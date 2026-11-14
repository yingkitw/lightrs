use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".cursor"];
pub const MAX_WALK_FILES: usize = 2_000;

/// Depth-first file list under `dir`, skipping known heavy dirs; paths must stay under `workdir`.
pub fn collect_files(dir: &Path, workdir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if out.len() >= MAX_WALK_FILES {
        return Ok(());
    }
    let entries = fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?;
    for ent in entries {
        if out.len() >= MAX_WALK_FILES {
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
        } else if meta.is_file() && path.starts_with(workdir) {
            out.push(path);
        }
    }
    Ok(())
}