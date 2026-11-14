use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::ModelFlags;

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Glm,
    Kimi,
    Minimax,
}

impl Provider {
    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::Glm => "https://api.z.ai/api/paas/v4",
            Provider::Kimi => "https://api.moonshot.ai/v1",
            Provider::Minimax => "https://api.minimax.io/v1",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::Glm => "glm-4.6",
            Provider::Kimi => "kimi-k2-0905-preview",
            Provider::Minimax => "MiniMax-M2.1",
        }
    }

    pub fn env_var(self) -> &'static str {
        match self {
            Provider::Glm => "ZAI_API_KEY",
            Provider::Kimi => "MOONSHOT_API_KEY",
            Provider::Minimax => "MINIMAX_API_KEY",
        }
    }

    pub fn alt_env_var(self) -> Option<&'static str> {
        match self {
            Provider::Glm => Some("GLM_API_KEY"),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    provider: Option<Provider>,
    model: Option<String>,
    base_url: Option<String>,
    workdir: Option<PathBuf>,
    keys: Option<FileKeys>,
}

#[derive(Debug, Default, Deserialize)]
struct FileKeys {
    glm: Option<String>,
    kimi: Option<String>,
    minimax: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub workdir: PathBuf,
}

impl AppConfig {
    pub fn load(flags: &ModelFlags) -> Result<Self> {
        let file = load_config_file()?;
        let provider = flags.provider;
        let model = flags
            .model
            .clone()
            .or_else(|| file.as_ref().and_then(|f| f.model.clone()))
            .unwrap_or_else(|| provider.default_model().to_string());

        let base_url = flags
            .base_url
            .clone()
            .or_else(|| file.as_ref().and_then(|f| f.base_url.clone()))
            .unwrap_or_else(|| provider.default_base_url().to_string());

        let workdir = flags
            .workdir
            .canonicalize()
            .or_else(|_| std::env::current_dir())
            .context("resolve working directory")?;

        if let Some(ref f) = file {
            if let Some(wd) = &f.workdir {
                let _ = wd;
            }
        }

        let api_key = resolve_api_key(flags, file.as_ref(), provider)?;

        Ok(Self {
            provider,
            model,
            base_url,
            api_key,
            workdir,
        })
    }
}

fn load_config_file() -> Result<Option<FileConfig>> {
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
    directories::ProjectDirs::from("", "", "lightrs")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

fn resolve_api_key(flags: &ModelFlags, file: Option<&FileConfig>, provider: Provider) -> Result<String> {
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
    );
}

pub fn validate_work_path(workdir: &Path, user_path: &Path) -> Result<PathBuf> {
    let joined = if user_path.is_absolute() {
        user_path.to_path_buf()
    } else {
        workdir.join(user_path)
    };
    let canonical = joined.canonicalize().with_context(|| {
        format!("path not found: {}", user_path.display())
    })?;
    let work = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    if !canonical.starts_with(&work) {
        bail!("path escapes workdir: {}", user_path.display());
    }
    Ok(canonical)
}