use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::validate_work_path;

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".cursor"];
const DEFAULT_MAX_MATCHES: usize = 80;
const MAX_MATCHES_CAP: usize = 200;
const MAX_FILES: usize = 2_000;
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
    let needle = if args.case_insensitive {
        pattern.to_ascii_lowercase()
    } else {
        pattern.to_string()
    };

    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(&root, workdir, &mut files)?;

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

fn collect_files(dir: &Path, workdir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if out.len() >= MAX_FILES {
        return Ok(());
    }
    let entries = fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for ent in entries {
        if out.len() >= MAX_FILES {
            break;
        }
        let ent = ent?;
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().into_owned();
        let meta = ent.metadata()?;
        if meta.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_files(&path, workdir, out)?;
        } else if meta.is_file() {
            if path.starts_with(workdir) {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn scan_file(
    rel: &str,
    path: &Path,
    needle: &str,
    ci: bool,
    hits: &mut Vec<String>,
    max: usize,
) {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(_) => return,
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
        format!(
            "{}…",
            t.chars().take(239).collect::<String>()
        )
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
        assert!(out.contains("src/lib.rs:1:"));
        assert!(out.contains("unique_needle_abc"));
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