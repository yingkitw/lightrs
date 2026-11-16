mod detect;

use std::path::PathBuf;

use serde_json::Value;

use crate::migrate::profile::ModelProfile;

#[derive(Clone, Debug)]
pub struct DetectedTool {
    pub name: &'static str,
    pub config_path: PathBuf,
    pub installed: bool,
    pub profile: Option<ModelProfile>,
    pub mcp_json: Option<Value>,
}

pub fn scan_local_tools() -> Vec<DetectedTool> {
    let mut out = Vec::new();
    detect::push_all(&mut out);
    out.sort_by_key(|t| std::cmp::Reverse(t.profile.is_some()));
    out
}

pub fn format_scan_report(tools: &[DetectedTool]) -> String {
    if tools.is_empty() {
        return "No known coding-agent configs found on this machine.".into();
    }
    let mut lines = vec!["Detected local agent tooling:".to_string()];
    for t in tools {
        let model = t
            .profile
            .as_ref()
            .and_then(|p| p.model.as_deref())
            .unwrap_or("—");
        let prov = t
            .profile
            .as_ref()
            .and_then(|p| p.provider)
            .map(|p| format!("{p:?}"))
            .unwrap_or_else(|| "—".into());
        lines.push(format!(
            "  - {} (installed={}): {} · provider={prov} model={model}",
            t.name,
            t.installed,
            t.config_path.display()
        ));
    }
    lines.push("Run `lightrs migrate` to import the highest-priority profile (OpenCode → Claude Code → Codex → Aider → Pi).".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::detect::{parse_aider_config, parse_codex_config};
    use crate::config::Provider;

    #[test]
    fn aider_yaml_model() {
        let p = parse_aider_config("model: glm-4.6\n", Some("yml")).unwrap();
        assert_eq!(p.model.as_deref(), Some("glm-4.6"));
    }

    const CODEX_ZAI: &str = r#"
model = "glm-4.6"
model_provider = "zai"

[model_providers.zai]
name = "Z.AI"
base_url = "https://api.z.ai/api/paas/v4"
env_key = "ZAI_API_KEY"
"#;

    #[test]
    fn codex_config_maps_zai_provider() {
        let p = parse_codex_config(CODEX_ZAI).unwrap();
        assert_eq!(p.provider, Some(Provider::Glm));
        assert_eq!(p.model.as_deref(), Some("glm-4.6"));
        assert_eq!(
            p.base_url.as_deref(),
            Some("https://api.z.ai/api/paas/v4")
        );
        // env_key resolution follows whatever the machine actually exports.
        assert_eq!(p.keys.glm.is_some(), std::env::var("ZAI_API_KEY").is_ok());
    }

    #[test]
    fn codex_config_maps_moonshot_provider() {
        let p = parse_codex_config(
            "model_provider = \"ms\"\n[model_providers.ms]\nbase_url = \"https://api.moonshot.ai/v1\"\n",
        )
        .unwrap();
        assert_eq!(p.provider, Some(Provider::Kimi));
    }

    #[test]
    fn codex_config_falls_back_to_provider_name() {
        // Custom proxy endpoint: host is unrecognizable but the codex
        // provider name still maps (e.g. "glm-proxy" behind a gateway).
        let p = parse_codex_config(
            "model_provider = \"glm-proxy\"\n[model_providers.glm-proxy]\nbase_url = \"https://proxy.example.com/v1\"\n",
        )
        .unwrap();
        assert_eq!(p.provider, Some(Provider::Glm));
        assert_eq!(p.base_url.as_deref(), Some("https://proxy.example.com/v1"));
    }

    #[test]
    fn codex_config_rejects_unknown_provider() {
        // Foreign endpoints (e.g. stock OpenAI) would produce a broken
        // default-provider config — no profile is better than a wrong one.
        let p = parse_codex_config(
            "model_provider = \"openai\"\n[model_providers.openai]\nbase_url = \"https://api.openai.com/v1\"\n",
        );
        assert!(p.is_none());
        // Missing provider table or model_provider key → None too.
        assert!(parse_codex_config("model = \"x\"\n").is_none());
        assert!(parse_codex_config("model_provider = \"ghost\"\n").is_none());
    }
}