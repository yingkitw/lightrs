use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Snapshot `content` before overwrite; returns backup path relative to workdir when created.
pub fn snapshot_prior(workdir: &Path, file: &Path, content: &str) -> Result<Option<PathBuf>> {
    let rel = file
        .strip_prefix(workdir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.display().to_string());
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let safe = rel.replace('/', "__");
    let dest = workdir
        .join(".lightrs/backups")
        .join(format!("{safe}.{ts}.bak"));
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
    }
    fs::write(&dest, content.as_bytes()).with_context(|| dest.display().to_string())?;
    Ok(Some(
        dest.strip_prefix(workdir)
            .map(|p| p.to_path_buf())
            .unwrap_or(dest),
    ))
}