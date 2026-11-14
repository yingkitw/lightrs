use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::chat::FileDiffUi;
use crate::config::validate_work_path;
use crate::tools::diff::build_file_diff;

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

#[derive(Deserialize)]
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
    let data = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    if data.len() > max {
        let chunk = &data[..max];
        let text = String::from_utf8_lossy(chunk);
        return Ok(format!(
            "{text}\n\n[truncated at {max} bytes; file size {}]",
            data.len()
        ));
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

pub struct WriteFileOutcome {
    pub summary: String,
    pub file_diff: FileDiffUi,
}

pub fn patch_file(workdir: &Path, args: PatchFileArgs) -> Result<WriteFileOutcome> {
    let path = resolve_write_path(workdir, &args.path)?;
    let prior = std::fs::read_to_string(&path)
        .with_context(|| format!("read {} for patch", path.display()))?;
    let matches = prior.matches(&args.old_string).count();
    if matches == 0 {
        anyhow::bail!("old_string not found in {}", args.path);
    }
    if !args.replace_all && matches > 1 {
        anyhow::bail!(
            "old_string matched {matches} times in {}; make the snippet unique or set replace_all",
            args.path
        );
    }
    let content = if args.replace_all {
        prior.replace(&args.old_string, &args.new_string)
    } else {
        prior.replacen(&args.old_string, &args.new_string, 1)
    };
    commit_write(&path, &args.path, Some(prior.as_str()), &content)
}

pub fn write_file(workdir: &Path, args: WriteFileArgs) -> Result<WriteFileOutcome> {
    let path = resolve_write_path(workdir, &args.path)?;
    let prior = std::fs::read_to_string(&path).ok();
    commit_write(&path, &args.path, prior.as_deref(), &args.content)
}

fn resolve_write_path(workdir: &Path, rel: &str) -> Result<std::path::PathBuf> {
    if Path::new(rel).is_absolute() {
        Ok(validate_work_path(workdir, Path::new(rel))?)
    } else {
        let target = workdir.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dirs for {}", target.display()))?;
        }
        Ok(target)
    }
}

fn commit_write(
    path: &Path,
    rel: &str,
    prior: Option<&str>,
    content: &str,
) -> Result<WriteFileOutcome> {
    std::fs::write(path, content).with_context(|| format!("write {}", path.display()))?;
    let file_diff = build_file_diff(prior, content, rel);
    let summary = format!(
        "wrote {} bytes to {} (+{} −{})",
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