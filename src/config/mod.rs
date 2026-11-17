mod file;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::ModelFlags;
use crate::migrate::{self, format_scan_report, infer_provider_from_env, scan_local_tools};
use crate::tools::ShellPolicy;

pub use file::{config_path, load_tui_theme, project_config_mtime, save_tui_theme};
pub(crate) use file::write_private_file;

/// Last modification time of `config.toml`, if present.
pub fn config_file_mtime() -> Option<SystemTime> {
    std::fs::metadata(config_path()).ok()?.modified().ok()
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, clap::ValueEnum, Deserialize,
)]
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

    /// Stable lowercase id used in `[endpoints]` config keys and CLI values.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Glm => "glm",
            Provider::Kimi => "kimi",
            Provider::Minimax => "minimax",
        }
    }

    pub fn from_id(s: &str) -> Option<Provider> {
        use clap::ValueEnum;
        Provider::from_str(s, true).ok()
    }
}

/// Published context-window size for a model name (prefix match, most-specific
/// first). `None` for unknown models — callers then show tokens without a %.
/// Numbers are provider-documented limits, not measured values.
pub fn model_context_window(model: &str) -> Option<u32> {
    let m = model.to_ascii_lowercase();
    const TABLE: &[(&str, u32)] = &[
        ("kimi-k2-0905", 262_144),
        ("kimi-k2", 131_072),
        ("moonshot-v1-128k", 131_072),
        ("moonshot-v1-32k", 32_768),
        ("moonshot-v1-8k", 8_192),
        ("glm-4.6", 204_800),
        ("glm-4.5", 131_072),
        ("glm-4", 131_072),
        ("minimax-m2", 204_800),
        ("minimax-m1", 1_048_576),
    ];
    TABLE.iter().find(|(p, _)| m.starts_with(p)).map(|(_, w)| *w)
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub provider: Provider,
    pub model: String,
    /// Secondary model retried once on provider errors (`--fallback-model` / `fallback_model`).
    pub fallback_model: Option<String>,
    /// Cheaper model for read-only steps (`--weak-model` / `weak_model`).
    pub weak_model: Option<String>,
    pub base_url: String,
    pub api_key: String,
    pub workdir: PathBuf,
    pub shell_policy: ShellPolicy,
    /// Post-edit check command from `[shell].test_command` (heal loop).
    pub test_command: Option<String>,
    /// Formatter run after each write/edit tool (`[shell].format_command`).
    pub format_command: Option<String>,
    /// `--read-only` — plan mode from launch; write/exec tools denied.
    pub read_only: bool,
    /// Max heal runs per user turn (`[shell].heal_steps`, default 3).
    pub heal_steps: u32,
    /// Per-provider endpoint overrides from `[endpoints]` (e.g. a
    /// kimi-for-coding key needs `api.kimi.ai/coding/v1`, not moonshot).
    /// Global config only — project overlays ignore it like keys/base_url.
    pub endpoints: std::collections::BTreeMap<Provider, String>,
}

impl AppConfig {
    /// Base URL to reach `provider` — the active provider's explicit
    /// `base_url` wins, then a `[endpoints]` override, then the provider
    /// default.
    pub fn endpoint_for(&self, provider: Provider) -> String {
        if provider == self.provider {
            return self.base_url.clone();
        }
        self.endpoints
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| provider.default_base_url().to_string())
    }
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

        let workdir = if flags.workdir == *"." {
            file.as_ref()
                .and_then(|f| f.workdir.clone())
                .unwrap_or_else(|| flags.workdir.clone())
        } else {
            flags.workdir.clone()
        }
        .canonicalize()
        .or_else(|_| std::env::current_dir())
        .context("resolve working directory")?;

        // Per-project overlay: `<workdir>/.lightrs/config.toml` (safe subset only).
        let project = file::load_project_config_file(&workdir)?;

        // Precedence: CLI flags > project overlay > global file.
        let model = flags
            .model
            .clone()
            .or_else(|| project.as_ref().and_then(|f| f.model.clone()))
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

        let api_key = file::resolve_api_key(flags, file.as_ref(), provider)?;

        let (allowlist, require_approval, test_command, format_command, heal_steps) =
            file::shell_from_files(project.as_ref(), file.as_ref());

        Ok(Self {
            provider,
            model,
            fallback_model: flags
                .fallback_model
                .clone()
                .or_else(|| project.as_ref().and_then(|f| f.fallback_model.clone()))
                .or_else(|| file.as_ref().and_then(|f| f.fallback_model.clone())),
            weak_model: flags
                .weak_model
                .clone()
                .or_else(|| project.as_ref().and_then(|f| f.weak_model.clone()))
                .or_else(|| file.as_ref().and_then(|f| f.weak_model.clone())),
            base_url,
            api_key,
            workdir,
            shell_policy: shell_policy_from_parts(allowlist, require_approval),
            test_command,
            format_command,
            read_only: flags.read_only,
            heal_steps,
            // Global file only — a project overlay `endpoints` is parsed but
            // never applied (same redirect-attack surface as base_url/keys).
            endpoints: file
                .as_ref()
                .and_then(|f| f.endpoints.as_ref())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| Provider::from_id(k).map(|p| (p, v.clone())))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Re-resolve an API key for `provider` from global config + env (no CLI
    /// flags — CLI keys only applied at startup). Used by `/model <provider>`.
    pub fn resolve_runtime_api_key(provider: Provider) -> Result<String> {
        let file = file::load_config_file()?;
        file::resolve_api_key(&ModelFlags::default(), file.as_ref(), provider)
    }

    /// Re-read `config.toml` + `.lightrs/config.toml` and apply `[shell]` (model/provider need restart).
    pub fn reload_shell_from_file(&mut self) -> Result<String> {
        let path = config_path();
        let file = file::load_config_file()?;
        let project = file::load_project_config_file(&self.workdir)?;
        let (allowlist, require_approval, test_command, format_command, heal_steps) =
            file::shell_from_files(project.as_ref(), file.as_ref());
        self.shell_policy = shell_policy_from_parts(allowlist, require_approval);
        self.test_command = test_command;
        self.format_command = format_command;
        self.heal_steps = heal_steps;
        let project_note = if project.is_some() {
            " + project .lightrs/config.toml"
        } else {
            ""
        };
        Ok(format!(
            "Reloaded [shell] from {}{} — allowlist {} · require_approval={}",
            path.display(),
            project_note,
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
    #[test]
    fn context_window_known_models() {
        assert_eq!(super::model_context_window("glm-4.6"), Some(204_800));
        assert_eq!(super::model_context_window("glm-4.5-air"), Some(131_072));
        assert_eq!(super::model_context_window("kimi-k2-0905-preview"), Some(262_144));
        assert_eq!(super::model_context_window("kimi-k2-0711"), Some(131_072));
        assert_eq!(super::model_context_window("MiniMax-M2.1"), Some(204_800));
        assert_eq!(super::model_context_window("some-other-model"), None);
    }

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

/// Resolve a path for reading or listing (target must exist on disk).
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

/// Resolve a path for writing a new file (target may not exist yet).
pub fn resolve_work_output_path(workdir: &Path, user_path: &Path) -> Result<PathBuf> {
    if user_path.is_absolute() {
        let work = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf());
        let target = user_path.to_path_buf();
        if target.exists() {
            let canon = target
                .canonicalize()
                .with_context(|| format!("path {}", target.display()))?;
            if !canon.starts_with(&work) {
                bail!("path escapes workdir: {}", user_path.display());
            }
            return Ok(canon);
        }
        if let Some(parent) = target.parent() {
            let parent_canon = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            if !parent_canon.starts_with(&work) {
                bail!("path escapes workdir: {}", user_path.display());
            }
        }
        return Ok(target);
    }
    use std::path::Component;
    let mut acc = workdir.to_path_buf();
    let mut depth = 0usize;
    for comp in user_path.components() {
        match comp {
            Component::ParentDir => {
                if depth == 0 {
                    bail!("path escapes workdir: {}", user_path.display());
                }
                depth -= 1;
                acc.pop();
            }
            Component::Normal(c) => {
                depth += 1;
                acc.push(c);
            }
            Component::CurDir => {}
            _ => bail!("invalid path: {}", user_path.display()),
        }
    }
    if acc.exists() {
        let work = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf());
        let canon = acc
            .canonicalize()
            .with_context(|| format!("path {}", acc.display()))?;
        if !canon.starts_with(&work) {
            bail!("path escapes workdir: {}", user_path.display());
        }
        return Ok(canon);
    }
    Ok(acc)
}

#[cfg(test)]
mod path_tests {
    use std::fs;

    use super::*;

    #[test]
    fn resolve_output_allows_new_file() {
        let dir = std::env::temp_dir().join(format!("lightrs-path-out-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = resolve_work_output_path(&dir, Path::new("out/new.md")).unwrap();
        assert_eq!(path, dir.join("out/new.md"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_output_rejects_parent_escape() {
        let dir = std::env::temp_dir().join(format!("lightrs-path-esc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let err = resolve_work_output_path(&dir, Path::new("../escape.md"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("escapes workdir"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_output_absolute_new_file_under_workdir() {
        let dir = std::env::temp_dir().join(format!("lightrs-path-abs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("new.md");
        let path = resolve_work_output_path(&dir, &target).unwrap();
        assert_eq!(path, target);
        let _ = fs::remove_dir_all(&dir);
    }
}
#[cfg(test)]
mod endpoint_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn cfg() -> AppConfig {
        AppConfig {
            provider: Provider::Glm,
            model: "glm-4.6".into(),
            fallback_model: None,
            weak_model: None,
            base_url: "https://custom.example/v1".into(),
            api_key: "k".into(),
            workdir: PathBuf::from("."),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            format_command: None,
            read_only: false,
            heal_steps: 3,
            endpoints: BTreeMap::from([(
                Provider::Kimi,
                "https://api.kimi.ai/coding/v1".into(),
            )]),
        }
    }

    #[test]
    fn endpoint_for_prefers_active_then_override_then_default() {
        let c = cfg();
        // Active provider: explicit base_url always wins.
        assert_eq!(c.endpoint_for(Provider::Glm), "https://custom.example/v1");
        // `[endpoints]` override for a non-active provider.
        assert_eq!(
            c.endpoint_for(Provider::Kimi),
            "https://api.kimi.ai/coding/v1"
        );
        // No override → provider default.
        assert_eq!(
            c.endpoint_for(Provider::Minimax),
            Provider::Minimax.default_base_url()
        );
    }

    #[test]
    fn provider_id_roundtrip() {
        for p in [Provider::Glm, Provider::Kimi, Provider::Minimax] {
            assert_eq!(Provider::from_id(p.id()), Some(p));
        }
        assert_eq!(Provider::from_id("bogus"), None);
    }
}
