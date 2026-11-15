use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::gitignore::Gitignore;

pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".cursor"];
pub const MAX_WALK_FILES: usize = 2_000;

/// Depth-first file list under `dir`, skipping known heavy dirs; paths must stay under `workdir`.
pub fn collect_files(dir: &Path, workdir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let ignore = Gitignore::load(workdir);
    collect_files_inner(dir, workdir, &ignore, out)
}

fn collect_files_inner(
    dir: &Path,
    workdir: &Path,
    ignore: &Gitignore,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
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
        let rel = path
            .strip_prefix(workdir)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if meta.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if !rel.is_empty() && ignore.is_ignored(&rel, true) {
                continue;
            }
            collect_files_inner(&path, workdir, ignore, out)?;
        } else if meta.is_file() && path.starts_with(workdir) {
            if !rel.is_empty() && ignore.is_ignored(&rel, false) {
                continue;
            }
            out.push(path);
        }
    }
    Ok(())
}