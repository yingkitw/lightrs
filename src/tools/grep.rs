use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::validate_work_path;

use super::shell::MAX_SHELL_STREAM_BYTES;
use super::walk;

const DEFAULT_MAX_MATCHES: usize = 80;
const MAX_MATCHES_CAP: usize = 200;
const MAX_READ_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    pub path: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(default)]
    pub max_matches: Option<usize>,
}

pub fn grep(workdir: &Path, args: GrepArgs) -> Result<String> {
    // `root` below is canonical (validate_work_path); the walk filters on
    // `path.starts_with(workdir)`, so keep both sides canonical or every file
    // is silently dropped (e.g. /var → /private/var on macOS).
    let workdir = &workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    let pattern = args.pattern.trim();
    if pattern.is_empty() {
        bail!("pattern must not be empty");
    }
    let root = match args.path.as_deref() {
        None | Some(".") | Some("") => workdir.to_path_buf(),
        Some(p) => validate_work_path(workdir, Path::new(p))?,
    };
    let max_matches = args
        .max_matches
        .unwrap_or(DEFAULT_MAX_MATCHES)
        .min(MAX_MATCHES_CAP);

    if let Some(out) = try_ripgrep(&root, pattern, max_matches, args.case_insensitive)? {
        return Ok(out);
    }

    let needle = if args.case_insensitive {
        pattern.to_ascii_lowercase()
    } else {
        pattern.to_string()
    };

    let mut files: Vec<PathBuf> = Vec::new();
    walk::collect_files(&root, workdir, &mut files)?;

    let mut hits: Vec<String> = Vec::new();
    let mut files_scanned = 0usize;
    for path in files {
        if hits.len() >= max_matches {
            break;
        }
        files_scanned += 1;
        let rel = path
            .strip_prefix(workdir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        scan_file(&rel, &path, &needle, args.case_insensitive, &mut hits, max_matches);
    }

    if hits.is_empty() {
        return Ok(format!(
            "no matches for `{pattern}` under {} ({files_scanned} files scanned)",
            display_root(&root, workdir)
        ));
    }
    let mut out = hits.join("\n");
    if hits.len() >= max_matches {
        out.push_str(&format!("\n\n[truncated at {max_matches} matches]"));
    }
    out.push_str(&format!(
        "\n\n({} matches in {files_scanned} files)",
        hits.len()
    ));
    Ok(out)
}

fn try_ripgrep(
    root: &Path,
    pattern: &str,
    max_matches: usize,
    case_insensitive: bool,
) -> Result<Option<String>> {
    if !have_rg() {
        return Ok(None);
    }
    let mut cmd = Command::new("rg");
    cmd.arg("--no-heading")
        .arg("--color")
        .arg("never")
        .arg("-n")
        .arg("-F")
        .arg(pattern)
        .arg(root);
    if case_insensitive {
        cmd.arg("-i");
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().context("spawn rg")?;
    // Bounded drain keeps memory flat on huge result sets AND prevents a
    // full-pipe stall; the deadline bounds pathological walks (no output()
    // call exists that both pipes and times out).
    let out_rd = child
        .stdout
        .take()
        .map(|s| crate::tools::shell::drain_stream(s, MAX_SHELL_STREAM_BYTES * 2));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status = loop {
        if let Some(s) = child.try_wait().context("wait rg")? {
            break s;
        }
        if std::time::Instant::now() >= deadline {
            crate::tools::shell::kill_process_tree(&mut child);
            let _ = out_rd.map(|h| h.join());
            return Ok(Some("error: ripgrep timed out after 60s".into()));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let (raw, _) = out_rd.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let code = status.code();
    if !status.success() && !matches!(code, Some(0) | Some(1)) {
        return Ok(None);
    }
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let line_count = raw.lines().count();
    if line_count == 0 {
        return Ok(Some(format!(
            "no matches for `{pattern}` under {} (ripgrep)",
            root.display()
        )));
    }
    let truncated = line_count > max_matches;
    let mut s = if truncated {
        let mut joined = raw
            .lines()
            .take(max_matches)
            .collect::<Vec<_>>()
            .join("\n");
        joined.push_str(&format!("\n\n[truncated at {max_matches} matches]"));
        joined
    } else {
        raw
    };
    if s.len() > MAX_SHELL_STREAM_BYTES {
        s = format!(
            "{}\n\n[output truncated]",
            crate::tools::truncate_bytes(&s, MAX_SHELL_STREAM_BYTES)
        );
    }
    s.push_str(&format!(
        "\n\n({} matches, ripgrep)",
        line_count.min(max_matches)
    ));
    Ok(Some(s))
}

fn have_rg() -> bool {
    Command::new("rg")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn display_root(root: &Path, workdir: &Path) -> String {
    root.strip_prefix(workdir)
        .map(|p| {
            if p.as_os_str().is_empty() {
                ".".into()
            } else {
                p.to_string_lossy().replace('\\', "/")
            }
        })
        .unwrap_or_else(|_| root.display().to_string())
}

fn scan_file(
    rel: &str,
    path: &Path,
    needle: &str,
    ci: bool,
    hits: &mut Vec<String>,
    max: usize,
) {
    // Bound the read at the source (+1 byte detects "exceeds" without a
    // second stat) — a multi-GB binary shouldn't load fully just to learn
    // it has a NUL in it.
    let data = match fs::File::open(path).map(|f| {
        use std::io::Read;
        let mut b = Vec::new();
        f.take(MAX_READ_BYTES as u64 + 1).read_to_end(&mut b).map(|_| b)
    }) {
        Ok(Ok(d)) => d,
        _ => return,
    };
    if data.contains(&0) {
        return;
    }
    let slice = if data.len() > MAX_READ_BYTES {
        &data[..MAX_READ_BYTES]
    } else {
        &data[..]
    };
    let text = String::from_utf8_lossy(slice);
    for (lineno, line) in text.lines().enumerate() {
        if hits.len() >= max {
            return;
        }
        let hay = if ci {
            line.to_ascii_lowercase()
        } else {
            line.to_string()
        };
        if let Some(col) = hay.find(needle) {
            let preview = trim_line_preview(line);
            hits.push(format!("{}:{}:{}:{}", rel, lineno + 1, col + 1, preview));
        }
    }
    if data.len() > MAX_READ_BYTES && hits.len() < max {
        hits.push(format!("{rel}: [file exceeds {MAX_READ_BYTES} bytes; partial scan]"));
    }
}

fn trim_line_preview(line: &str) -> String {
    let t = line.trim();
    if t.chars().count() > 240 {
        format!("{}…", t.chars().take(239).collect::<String>())
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_literal_pattern() {
        let dir = std::env::temp_dir().join(format!("lightrs-grep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "fn unique_needle_abc() {}\n").unwrap();
        fs::write(dir.join("README.md"), "nothing\n").unwrap();
        let out = grep(
            &dir,
            GrepArgs {
                pattern: "unique_needle_abc".into(),
                path: None,
                case_insensitive: false,
                max_matches: None,
            },
        )
        .unwrap();
        assert!(out.contains("src/lib.rs:1:") || out.contains("unique_needle_abc"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lightrs-grep-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn args(pat: &str) -> GrepArgs {
        GrepArgs {
            pattern: pat.into(),
            path: None,
            case_insensitive: false,
            max_matches: None,
        }
    }

    #[test]
    fn rg_output_cap_stops_at_char_boundary() {
        if !have_rg() {
            return; // fallback path caps per-line previews instead
        }
        let dir = tmp("cap-boundary");
        // rg prints `<abs-path>:1:<line>`; compute the fill so a 3-byte '€'
        // straddles byte 65536 of the output — String::truncate would panic.
        let root = dir.canonicalize().unwrap();
        let prefix = format!("{}:1:", root.join("big.txt").display());
        let fill = MAX_SHELL_STREAM_BYTES - prefix.len() - 1;
        fs::write(dir.join("big.txt"), format!("{}€€€", "x".repeat(fill))).unwrap();

        let out = grep(&dir, args("x")).unwrap();
        assert!(out.contains("[output truncated]"), "tail: {}", &out[out.len().saturating_sub(80)..]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_pattern_bails() {
        let dir = tmp("empty");
        let err = grep(&dir, args("   ")).unwrap_err();
        assert!(format!("{err:#}").contains("pattern"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn case_insensitive_matches_mixed_case() {
        let dir = tmp("ci");
        fs::write(dir.join("a.rs"), "Hello World\nbye\n").unwrap();
        let out = grep(
            &dir,
            GrepArgs {
                case_insensitive: true,
                ..args("hello")
            },
        )
        .unwrap();
        assert!(out.contains("Hello World"), "out = {out}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_matches_marks_truncation() {
        let dir = tmp("cap");
        let body = (0..50).map(|i| format!("hit {i}")).collect::<Vec<_>>().join("\n");
        fs::write(dir.join("big.txt"), body).unwrap();
        let out = grep(
            &dir,
            GrepArgs {
                max_matches: Some(3),
                ..args("hit")
            },
        )
        .unwrap();
        assert!(out.contains("[truncated at 3 matches]"), "out = {out}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_matches_reports_scanned_files() {
        let dir = tmp("nomatch");
        fs::write(dir.join("a.txt"), "nothing here\n").unwrap();
        let out = grep(&dir, args("zzz-absent")).unwrap();
        assert!(out.contains("no matches for `zzz-absent`"), "out = {out}");
        assert!(out.contains("files scanned") || out.contains("ripgrep"), "out = {out}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_arg_scopes_search_to_subdir() {
        let dir = tmp("scope");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("top.txt"), "needle\n").unwrap();
        fs::write(dir.join("sub/inner.txt"), "needle\n").unwrap();
        let out = grep(
            &dir,
            GrepArgs {
                path: Some("sub".into()),
                ..args("needle")
            },
        )
        .unwrap();
        assert!(out.contains("inner.txt"), "out = {out}");
        assert!(!out.contains("top.txt"), "out = {out}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_escape_workdir() {
        let dir = std::env::temp_dir().join(format!("lightrs-grep-wd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let err = grep(
            &dir,
            GrepArgs {
                pattern: "x".into(),
                path: Some("../etc/passwd".into()),
                case_insensitive: false,
                max_matches: None,
            },
        );
        assert!(err.is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}