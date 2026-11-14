use std::path::PathBuf;

use serde_json::Value;

use crate::migrate::profile::{self, ModelProfile};
use crate::migrate::scan::DetectedTool;
use crate::opencode;

pub(crate) fn push_all(out: &mut Vec<DetectedTool>) {
    push_opencode(out);
    push_claude_code(out);
    push_aider(out);
    push_pi(out);
    push_cursor_mcp(out);
}

fn push_opencode(out: &mut Vec<DetectedTool>) {
    let path = opencode::default_opencode_global_config().unwrap_or_default();
    let installed = has_binary("opencode");
    let profile = if path.is_file() {
        opencode::load_jsonc(&path)
            .ok()
            .map(|doc| opencode::extract_model_profile(&doc))
    } else {
        None
    };
    if path.is_file() || installed {
        out.push(DetectedTool {
            name: "opencode",
            config_path: path,
            installed,
            mcp_json: None,
            profile,
        });
    }
}

fn push_claude_code(out: &mut Vec<DetectedTool>) {
    let home = home_dir();
    let path = home.join(".claude/settings.json");
    let installed = has_binary("claude") || has_binary("claude-code");
    if !path.is_file() && !installed {
        return;
    }
    let mut profile = None;
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(v) = serde_json::from_str::<Value>(&text) {
            let mut p = ModelProfile::default();
            if let Some(m) = v.get("model").and_then(Value::as_str) {
                p.model = Some(m.to_string());
            }
            if let Some(env) = v.get("env").and_then(Value::as_object) {
                let base = env
                    .get("ANTHROPIC_BASE_URL")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let token = env
                    .get("ANTHROPIC_AUTH_TOKEN")
                    .or_else(|| env.get("ANTHROPIC_API_KEY"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !base.is_empty() && !token.is_empty() {
                    p = profile::anthropic_base_to_profile(base, token);
                    if let Some(m) = v.get("model").and_then(Value::as_str) {
                        p.model = Some(normalize_claude_model(m));
                    }
                }
            }
            if p.model.is_some() || p.keys.glm.is_some() || p.keys.kimi.is_some() {
                profile = Some(p);
            }
        }
    out.push(DetectedTool {
        name: "claude-code",
        config_path: path,
        installed,
        profile,
        mcp_json: None,
    });
}

fn push_aider(out: &mut Vec<DetectedTool>) {
    let home = home_dir();
    let candidates = [
        home.join(".aider.conf.yml"),
        home.join(".aider.conf"),
        home.join(".aider.model.settings.yml"),
    ];
    let path = candidates
        .iter()
        .find(|p| p.is_file())
        .cloned()
        .unwrap_or(candidates[0].clone());
    let installed = has_binary("aider");
    let mut profile = None;
    if path.is_file()
        && let Ok(text) = std::fs::read_to_string(&path) {
            profile = parse_aider_config(&text, path.extension().and_then(|s| s.to_str()));
        }
    if path.is_file() || installed {
        out.push(DetectedTool {
            name: "aider",
            config_path: path,
            installed,
            profile,
            mcp_json: None,
        });
    }
}

fn push_pi(out: &mut Vec<DetectedTool>) {
    let home = home_dir();
    let path = home.join(".pi/agent/settings.json");
    let installed = has_binary("pi") || home.join(".pi/agent").is_dir();
    if !installed && !path.is_file() {
        return;
    }
    out.push(DetectedTool {
        name: "pi",
        config_path: path,
        installed,
        profile: None,
        mcp_json: None,
    });
}

fn push_cursor_mcp(out: &mut Vec<DetectedTool>) {
    let home = home_dir();
    let path = home.join(".cursor/mcp.json");
    let installed = home.join(".cursor").is_dir();
    let mcp = path
        .is_file()
        .then(|| std::fs::read_to_string(&path).ok())
        .flatten()
        .and_then(|t| serde_json::from_str(&t).ok());
    if installed || path.is_file() {
        out.push(DetectedTool {
            name: "cursor",
            config_path: path,
            installed,
            profile: None,
            mcp_json: mcp,
        });
    }
}

pub(crate) fn parse_aider_config(text: &str, ext: Option<&str>) -> Option<ModelProfile> {
    let mut p = ModelProfile::default();
    if ext == Some("yml") || ext == Some("yaml") || text.contains("model:") {
        for line in text.lines() {
            let line = line.trim();
            if let Some(m) = line.strip_prefix("model:") {
                p.model = Some(m.trim().trim_matches('"').to_string());
            }
            if let Some(m) = line.strip_prefix("-m ") {
                p.model = Some(m.trim().to_string());
            }
        }
    }
    for line in text.lines() {
        if let Some(m) = line.strip_prefix("model = ") {
            p.model = Some(m.trim().trim_matches('"').to_string());
        }
    }
    if let Some(m) = &p.model {
        let lower = m.to_ascii_lowercase();
        p.provider = if lower.contains("glm") || lower.contains("zai") {
            Some(crate::config::Provider::Glm)
        } else if lower.contains("kimi") || lower.contains("moonshot") {
            Some(crate::config::Provider::Kimi)
        } else if lower.contains("minimax") {
            Some(crate::config::Provider::Minimax)
        } else {
            None
        };
    }
    (p.model.is_some()).then_some(p)
}

fn normalize_claude_model(model: &str) -> String {
    model
        .trim()
        .strip_prefix("claude-")
        .map(|s| s.to_string())
        .unwrap_or_else(|| model.to_string())
}

fn has_binary(name: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}