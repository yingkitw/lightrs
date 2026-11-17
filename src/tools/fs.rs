use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::chat::FileDiffUi;
use crate::config::{resolve_work_output_path, validate_work_path};
use crate::tools::backup;
use crate::tools::diff::build_file_diff;
use crate::tools::edit_match;

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
    pub max_bytes: Option<usize>,
}

#[derive(Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PatchFileArgs {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Deserialize)]
pub struct ListDirArgs {
    pub path: Option<String>,
}

pub fn read_file(workdir: &Path, args: ReadFileArgs) -> Result<String> {
    let path = validate_work_path(workdir, Path::new(&args.path))?;
    let max = args.max_bytes.unwrap_or(65_536).min(512_000);
    // Bound the read at the source: the model picks the path, so `fs::read`
    // of a multi-GB file would OOM before the output cap ever applies.
    let file =
        std::fs::File::open(&path).with_context(|| format!("read {}", path.display()))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut data = Vec::new();
    use std::io::Read;
    file.take(max as u64)
        .read_to_end(&mut data)
        .with_context(|| format!("read {}", path.display()))?;
    if size > max as u64 {
        let text = String::from_utf8_lossy(&data);
        return Ok(format!("{text}\n\n[truncated at {max} bytes; file size {size}]"));
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

pub struct WriteFileOutcome {
    pub summary: String,
    pub file_diff: FileDiffUi,
}

/// Diff/backup needs the prior file whole, but a model-chosen path can be
/// huge — bound it. `patch_file` requires the prior (it must match against
/// the real content); `write_file` degrades to create-style, no backup.
const WRITE_PRIOR_MAX: u64 = 1024 * 1024;

fn prior_content(path: &Path, required: bool) -> Result<Option<String>> {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) if required => {
            return Err(e).with_context(|| format!("read {} for patch", path.display()))
        }
        Err(_) => return Ok(None),
    };
    if size > WRITE_PRIOR_MAX {
        if required {
            anyhow::bail!(
                "{} is {size} bytes (> {WRITE_PRIOR_MAX}); too large to patch",
                path.display()
            );
        }
        return Ok(None);
    }
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if required => {
            Err(e).with_context(|| format!("read {} for patch", path.display()))
        }
        Err(_) => Ok(None),
    }
}

pub fn patch_file(workdir: &Path, args: PatchFileArgs) -> Result<WriteFileOutcome> {
    let path = resolve_write_path(workdir, &args.path)?;
    let prior = prior_content(&path, true)?.unwrap_or_default();
    let matches = prior.matches(&args.old_string).count();
    let content = if args.replace_all {
        if matches == 0 {
            anyhow::bail!("old_string not found in {}", args.path);
        }
        prior.replace(&args.old_string, &args.new_string)
    } else if matches > 1 {
        anyhow::bail!(
            "old_string matched {matches} times in {}; make the snippet unique or set replace_all",
            args.path
        );
    } else if matches == 1 {
        prior.replacen(&args.old_string, &args.new_string, 1)
    } else if let Some(patched) =
        edit_match::replace_once(&prior, &args.old_string, &args.new_string)
    {
        patched
    } else {
        anyhow::bail!("old_string not found in {}", args.path);
    };
    commit_write(workdir, &path, &args.path, Some(prior.as_str()), &content)
}

pub fn write_file(workdir: &Path, args: WriteFileArgs) -> Result<WriteFileOutcome> {
    let path = resolve_write_path(workdir, &args.path)?;
    let existed = path.exists();
    let prior = prior_content(&path, false)?;
    let mut out = commit_write(workdir, &path, &args.path, prior.as_deref(), &args.content)?;
    if existed && prior.is_none() {
        out.summary
            .push_str("; prior file too large — no backup or diff");
    }
    Ok(out)
}

fn resolve_write_path(workdir: &Path, rel: &str) -> Result<std::path::PathBuf> {
    let path = resolve_work_output_path(workdir, Path::new(rel))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dirs for {}", path.display()))?;
    }
    Ok(path)
}

fn commit_write(
    workdir: &Path,
    path: &Path,
    rel: &str,
    prior: Option<&str>,
    content: &str,
) -> Result<WriteFileOutcome> {
    let backup_note = if let Some(prior_text) = prior {
        match backup::snapshot_prior(workdir, path, prior_text) {
            Ok(Some(b)) => format!(" (backup: {})", b.display()),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    std::fs::write(path, content).with_context(|| format!("write {}", path.display()))?;
    let file_diff = build_file_diff(prior, content, rel);
    let summary = format!(
        "wrote {} bytes to {} (+{} −{}){backup_note}",
        content.len(),
        path.display(),
        file_diff.lines_added,
        file_diff.lines_removed,
    );
    Ok(WriteFileOutcome {
        summary,
        file_diff,
    })
}

pub fn list_dir(workdir: &Path, args: ListDirArgs) -> Result<String> {
    let rel = args.path.as_deref().unwrap_or(".");
    let path = validate_work_path(workdir, Path::new(rel))?;
    let mut names: Vec<String> = std::fs::read_dir(&path)?
        .filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let kind = if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                "dir"
            } else {
                "file"
            };
            format!("{name} ({kind})")
        })
        .collect();
    names.sort();
    Ok(names.join("\n"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn testdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lightrs-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_file_bounds_the_read() {
        let dir = testdir("read");
        std::fs::write(dir.join("big.txt"), "x".repeat(2048)).unwrap();
        let out = read_file(
            &dir,
            ReadFileArgs {
                path: "big.txt".into(),
                max_bytes: Some(100),
            },
        )
        .unwrap();
        assert!(out.contains("[truncated at 100 bytes; file size 2048]"), "{out}");
        assert!(out.len() < 300);
    }

    #[test]
    fn patch_file_refuses_oversized_prior() {
        let dir = testdir("patch");
        std::fs::write(dir.join("big.txt"), "x".repeat(WRITE_PRIOR_MAX as usize + 1)).unwrap();
        let res = patch_file(
            &dir,
            PatchFileArgs {
                path: "big.txt".into(),
                old_string: "x".into(),
                new_string: "y".into(),
                replace_all: false,
            },
        );
        match res {
            Err(e) => assert!(e.to_string().contains("too large to patch"), "{e}"),
            Ok(_) => panic!("oversized patch must be refused"),
        }
    }

    #[test]
    fn write_file_over_oversized_prior_skips_backup() {
        let dir = testdir("write");
        std::fs::write(dir.join("big.txt"), "x".repeat(WRITE_PRIOR_MAX as usize + 1)).unwrap();
        let out = write_file(
            &dir,
            WriteFileArgs {
                path: "big.txt".into(),
                content: "new".into(),
            },
        )
        .unwrap();
        assert!(out.summary.contains("no backup or diff"), "{}", out.summary);
        assert_eq!(std::fs::read_to_string(dir.join("big.txt")).unwrap(), "new");
    }
}
