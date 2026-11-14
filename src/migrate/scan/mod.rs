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
    lines.push("Run `lightrs migrate` to import the highest-priority profile (OpenCode → Claude Code → Aider → Pi).".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::detect::parse_aider_config;

    #[test]
    fn aider_yaml_model() {
        let p = parse_aider_config("model: glm-4.6\n", Some("yml")).unwrap();
        assert_eq!(p.model.as_deref(), Some("glm-4.6"));
    }
}