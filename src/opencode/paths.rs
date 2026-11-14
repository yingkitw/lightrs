use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn default_opencode_global_config() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(crate::paths::opencode_global_config_candidates());
    candidates.into_iter().find(|p| p.is_file())
}

pub fn discover_project_opencode() -> Result<Option<PathBuf>> {
    let mut dir = std::env::current_dir().context("current directory")?;
    loop {
        for candidate in [
            dir.join("opencode.json"),
            dir.join("opencode.jsonc"),
            dir.join(".opencode/opencode.json"),
            dir.join(".opencode/opencode.jsonc"),
        ] {
            if candidate.is_file() {
                return Ok(Some(candidate));
            }
        }
        if !dir.pop() {
            break;
        }
    }
    Ok(None)
}