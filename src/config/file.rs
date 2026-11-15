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
    /// Secondary model retried once when the primary errors mid-turn.
    pub fallback_model: Option<String>,
    pub shell: Option<FileShell>,
    pub skills: Option<FileSkills>,
    pub tui: Option<FileTui>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileTui {
    pub theme: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileSkills {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct FileShell {
    /// Command prefix allowlist for `run_command`; empty = allow all.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// When true, TUI prompts before every `run_command` (y/n).
    #[serde(default)]
    pub require_approval: bool,
    /// Post-edit check run after write/edit tools; failures go back to the model.
    pub test_command: Option<String>,
    /// Max heal runs per user turn (default 3).
    pub heal_steps: Option<u32>,
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

pub fn load_tui_theme() -> Option<String> {
    let file = load_config_file().ok().flatten()?;
    file.tui.and_then(|t| t.theme)
}

pub fn save_tui_theme(theme: &str) -> Result<()> {
    let path = config_path();
    let mut root: toml::Value = if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let toml::Value::Table(ref mut table) = root else {
        bail!("config.toml root must be a table");
    };
    let ui = table
        .entry("tui")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if let toml::Value::Table(ui_table) = ui {
        ui_table.insert("theme".to_string(), toml::Value::String(theme.to_string()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&path, toml::to_string_pretty(&root)?).with_context(|| {
        format!("write {}", path.display())
    })?;
    Ok(())
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

    if let Ok(k) = std::env::var(provider.env_var())
        && !k.is_empty() {
            return Ok(k);
        }

    if let Some(alt) = provider.alt_env_var()
        && let Ok(k) = std::env::var(alt)
            && !k.is_empty() {
                return Ok(k);
            }

    bail!(
        "missing API key for {:?}; set {} or pass --*-api-key",
        provider,
        provider.env_var()
    )
}