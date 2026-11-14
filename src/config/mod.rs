mod file;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::ModelFlags;
use crate::migrate::{self, format_scan_report, infer_provider_from_env, scan_local_tools};

pub use file::config_path;

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

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub provider: Provider,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub workdir: PathBuf,
}

impl AppConfig {
    pub fn load(flags: &ModelFlags, auto_migrate: bool) -> Result<Self> {
        if auto_migrate {
            let _ = migrate::bootstrap_if_needed();
        }
        let file = file::load_config_file()?;
        let provider = flags
            .provider
            .or_else(|| file.as_ref().and_then(|f| f.provider))
            .unwrap_or_else(infer_provider_from_env);
        let model = flags
            .model
            .clone()
            .or_else(|| file.as_ref().and_then(|f| f.model.clone()));
        let model = model.ok_or_else(|| {
            let hint = provider.default_model();
            anyhow::anyhow!(
                "no model configured (pass --model e.g. `{hint}` or run `lightrs migrate`).\n\n{}",
                format_scan_report(&scan_local_tools())
            )
        })?;

        let base_url = flags
            .base_url
            .clone()
            .or_else(|| file.as_ref().and_then(|f| f.base_url.clone()))
            .unwrap_or_else(|| provider.default_base_url().to_string());

        let workdir = if flags.workdir == PathBuf::from(".") {
            file.as_ref()
                .and_then(|f| f.workdir.clone())
                .unwrap_or_else(|| flags.workdir.clone())
        } else {
            flags.workdir.clone()
        }
        .canonicalize()
        .or_else(|_| std::env::current_dir())
        .context("resolve working directory")?;

        let api_key = file::resolve_api_key(flags, file.as_ref(), provider)?;

        Ok(Self {
            provider,
            model,
            base_url,
            api_key,
            workdir,
        })
    }
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