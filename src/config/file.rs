use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::ModelFlags;
use crate::config::Provider;
use crate::paths::lightrs_config_toml;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileConfig {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub workdir: Option<PathBuf>,
    pub keys: Option<FileKeys>,
    pub shell: Option<FileShell>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileShell {
    /// Command prefix allowlist for `run_command`; empty = allow all.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// When true, TUI prompts before every `run_command` (y/n).
    #[serde(default)]
    pub require_approval: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileKeys {
    pub glm: Option<String>,
    pub kimi: Option<String>,
    pub minimax: Option<String>,
}

pub fn load_config_file() -> Result<Option<FileConfig>> {
    let path = config_path();
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let cfg: FileConfig = toml::from_str(&text).context("parse config.toml")?;
    Ok(Some(cfg))
}

pub fn config_path() -> PathBuf {
    lightrs_config_toml()
}

pub fn resolve_api_key(
    flags: &ModelFlags,
    file: Option<&FileConfig>,
    provider: Provider,
) -> Result<String> {
    let from_cli = match provider {
        Provider::Glm => flags.glm_api_key.clone(),
        Provider::Kimi => flags.kimi_api_key.clone(),
        Provider::Minimax => flags.minimax_api_key.clone(),
    };

    let from_file = file.and_then(|f| match provider {
        Provider::Glm => f.keys.as_ref().and_then(|k| k.glm.clone()),
        Provider::Kimi => f.keys.as_ref().and_then(|k| k.kimi.clone()),
        Provider::Minimax => f.keys.as_ref().and_then(|k| k.minimax.clone()),
    });

    if let Some(k) = from_cli.or(from_file) {
        return Ok(k);
    }

    if let Ok(k) = std::env::var(provider.env_var()) {
        if !k.is_empty() {
            return Ok(k);
        }
    }

    if let Some(alt) = provider.alt_env_var() {
        if let Ok(k) = std::env::var(alt) {
            if !k.is_empty() {
                return Ok(k);
            }
        }
    }

    bail!(
        "missing API key for {:?}; set {} or pass --*-api-key",
        provider,
        provider.env_var()
    )
}