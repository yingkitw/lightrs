mod file;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::ModelFlags;
use crate::migrate::{self, format_scan_report, infer_provider_from_env, scan_local_tools};
use crate::tools::ShellPolicy;

pub use file::config_path;

/// Last modification time of `config.toml`, if present.
pub fn config_file_mtime() -> Option<SystemTime> {
    std::fs::metadata(config_path()).ok()?.modified().ok()
}

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
    pub shell_policy: ShellPolicy,
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

        let (allowlist, require_approval) = file
            .as_ref()
            .and_then(|f| f.shell.as_ref())
            .map(|s| (s.allowlist.clone(), s.require_approval))
            .unwrap_or((Vec::new(), false));

        Ok(Self {
            provider,
            model,
            base_url,
            api_key,
            workdir,
            shell_policy: shell_policy_from_parts(allowlist, require_approval),
        })
    }

    /// Re-read `~/.config/lightrs/config.toml` and apply `[shell]` (model/provider need restart).
    pub fn reload_shell_from_file(&mut self) -> Result<String> {
        let path = config_path();
        let file = file::load_config_file()?;
        let (allowlist, require_approval) = file
            .as_ref()
            .and_then(|f| f.shell.as_ref())
            .map(|s| (s.allowlist.clone(), s.require_approval))
            .unwrap_or((Vec::new(), false));
        self.shell_policy = shell_policy_from_parts(allowlist, require_approval);
        Ok(format!(
            "Reloaded [shell] from {} — allowlist {} · require_approval={}",
            path.display(),
            if self.shell_policy.is_restrictive() {
                "on"
            } else {
                "off"
            },
            self.shell_policy.requires_all_commands_approved()
        ))
    }
}

fn shell_policy_from_parts(allowlist: Vec<String>, require_approval: bool) -> ShellPolicy {
    ShellPolicy::new(allowlist, require_approval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_policy_from_parts_matches_shell_policy_new() {
        let a = shell_policy_from_parts(vec!["cargo ".into()], true);
        let b = ShellPolicy::new(vec!["cargo ".into()], true);
        assert_eq!(a, b);
    }
}

/// `[skills]` from config file (defaults: enabled, no extra paths).
pub fn skills_settings() -> (bool, Vec<PathBuf>) {
    let file = file::load_config_file().ok().flatten();
    let sec = file.as_ref().and_then(|f| f.skills.as_ref());
    (
        sec.map(|s| s.enabled).unwrap_or(true),
        sec.map(|s| s.paths.clone()).unwrap_or_default(),
    )
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