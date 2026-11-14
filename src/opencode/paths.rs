use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn default_opencode_global_config() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(d) = directories::ProjectDirs::from("", "", "opencode") {
        candidates.push(d.config_dir().join("opencode.json"));
        candidates.push(d.config_dir().join("opencode.jsonc"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".config/opencode/opencode.json"));
        candidates.push(home.join(".config/opencode/opencode.jsonc"));
    }
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