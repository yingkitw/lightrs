mod assets;
mod jsonc;
mod paths;
mod profile;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::config_path;

pub use jsonc::load_jsonc;
pub use paths::{default_opencode_global_config, discover_project_opencode};
pub use profile::extract_model_profile;

pub fn import(global: Option<PathBuf>, project: Option<PathBuf>) -> Result<()> {
    let mut docs = Vec::new();
    if let Some(path) = global {
        docs.push(load_jsonc(&path)?);
    } else if let Some(path) = default_opencode_global_config() {
        if path.exists() {
            docs.push(load_jsonc(&path)?);
        }
    }

    if let Some(path) = project {
        docs.push(load_jsonc(&path)?);
    } else if let Some(path) = discover_project_opencode()? {
        docs.push(load_jsonc(&path)?);
    }

    if docs.is_empty() {
        bail!("no OpenCode config found; pass --config or run from a project with opencode.json");
    }

    let merged = jsonc::merge_json_docs(&docs);
    let lightrs_dir = config_path()
        .parent()
        .map(Path::to_path_buf)
        .context("lightrs config directory")?;

    fs::create_dir_all(&lightrs_dir)?;

    profile::import_model_settings(&merged, &lightrs_dir)?;
    assets::import_mcp(&merged, &lightrs_dir)?;
    assets::import_skills(&merged, &lightrs_dir)?;
    assets::import_rules(&merged, &lightrs_dir)?;
    assets::import_hooks(&merged, &lightrs_dir)?;

    println!("Imported OpenCode settings into {}", lightrs_dir.display());
    Ok(())
}