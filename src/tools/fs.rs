use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::validate_work_path;

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

pub fn write_file(workdir: &Path, args: WriteFileArgs) -> Result<String> {
    let path = if Path::new(&args.path).is_absolute() {
        validate_work_path(workdir, Path::new(&args.path))?
    } else {
        let target = workdir.join(&args.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dirs for {}", target.display()))?;
        }
        target
    };
    std::fs::write(&path, &args.content)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(format!(
        "wrote {} bytes to {}",
        args.content.len(),
        path.display()
    ))
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