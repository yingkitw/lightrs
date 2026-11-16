use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::config::Provider;
use crate::migrate::{self, ModelProfile, config_lines, write_config};

pub fn extract_model_profile(doc: &Value) -> ModelProfile {
    let mut profile = ModelProfile::default();

    if let Some(model_ref) = doc.get("model").and_then(Value::as_str)
        && let Some((p, m)) = model_ref.split_once('/') {
            profile.provider = migrate::map_opencode_provider_id(p);
            profile.model = Some(m.to_string());
        }

    let provider_map = doc
        .get("providers")
        .or_else(|| doc.get("provider"))
        .and_then(Value::as_object);

    if let Some(map) = provider_map {
        for (id, cfg) in map {
            if profile.provider.is_none() {
                profile.provider = migrate::map_opencode_provider_id(id);
            }
            let provider = migrate::map_opencode_provider_id(id)
                .or_else(|| {
                    cfg.get("options")
                        .and_then(|o| o.get("baseURL").or_else(|| o.get("base_url")))
                        .and_then(Value::as_str)
                        .and_then(crate::migrate::provider_from_base_url)
                });
            if let Some(p) = provider {
                // Every provider's baseURL lands in `[endpoints]`; the
                // profile's own base_url is picked from it below.
                for key in ["options", "settings"] {
                    if let Some(url) = cfg
                        .get(key)
                        .and_then(|o| o.get("baseURL").or_else(|| o.get("base_url")))
                        .and_then(Value::as_str)
                    {
                        profile
                            .endpoints
                            .entry(p)
                            .or_insert_with(|| url.trim_end_matches('/').to_string());
                        break;
                    }
                }
                if let Some(options) = cfg.get("options").and_then(Value::as_object)
                    && let Some(key) = options
                        .get("apiKey")
                        .or_else(|| options.get("api_key"))
                        .and_then(Value::as_str)
                        .and_then(resolve_key)
                {
                    set_key(&mut profile.keys, p, key);
                }
            }
        }
    }

    if profile.base_url.is_none()
        && let Some(p) = profile.provider
    {
        profile.base_url = profile.endpoints.get(&p).cloned();
    }
    profile
}

fn set_key(keys: &mut crate::migrate::Keys, provider: Provider, key: String) {
    match provider {
        Provider::Glm => keys.glm = Some(key),
        Provider::Kimi => keys.kimi = Some(key),
        Provider::Minimax => keys.minimax = Some(key),
    }
}

/// `{env:NAME}` placeholders resolve against the environment at import time;
/// an unset var drops the entry rather than carrying the literal placeholder
/// through as an API key.
fn resolve_key(value: &str) -> Option<String> {
    if let Some(name) = value
        .strip_prefix("{env:")
        .and_then(|v| v.strip_suffix('}'))
    {
        return std::env::var(name).ok().filter(|v| !v.is_empty());
    }
    Some(value.to_string())
}

/// Merge `opencode auth login` credentials (`auth.json`: `{ "<provider>":
/// {"type":"api","key":"…"} }`) into the profile. Explicit `options.apiKey`
/// entries from opencode.json win — auth entries only fill missing slots.
pub fn merge_opencode_auth(profile: &mut ModelProfile) {
    for path in crate::paths::opencode_auth_candidates() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(doc) = serde_json::from_str::<Value>(&text) {
            merge_auth_doc(profile, &doc);
        }
        return;
    }
}

fn merge_auth_doc(profile: &mut ModelProfile, doc: &Value) {
    let Some(map) = doc.as_object() else {
        return;
    };
    for (id, entry) in map {
        // Only `type: "api"` entries carry a plain key; oauth entries are
        // provider-specific tokens we cannot map.
        if entry.get("type").and_then(Value::as_str) != Some("api") {
            continue;
        }
        let (Some(key), Some(p)) = (
            entry.get("key").and_then(Value::as_str),
            migrate::map_opencode_provider_id(id),
        ) else {
            continue;
        };
        let slot = match p {
            Provider::Glm => &mut profile.keys.glm,
            Provider::Kimi => &mut profile.keys.kimi,
            Provider::Minimax => &mut profile.keys.minimax,
        };
        if slot.is_none() {
            *slot = Some(key.to_string());
            // Plan/region variants carry their own endpoint (e.g. a
            // kimi-code-plan-global key only works on api.kimi.ai/coding/v1,
            // not the moonshot default).
            if let Some(url) = auth_endpoint(id) {
                profile
                    .endpoints
                    .entry(p)
                    .or_insert_with(|| url.to_string());
            }
        }
    }
}

/// OpenCode auth ids that imply a non-default provider endpoint (coding-plan
/// and region variants). Keep entries minimal — only ids that differ from
/// `Provider::default_base_url`.
fn auth_endpoint(id: &str) -> Option<&'static str> {
    match id {
        "kimi-code-plan-global" | "kimi-for-coding-global" => {
            Some("https://api.kimi.ai/coding/v1")
        }
        "kimi-for-coding" | "kimi-coding" | "kimi-code-plan" => {
            Some("https://api.kimi.com/coding/v1")
        }
        "moonshotai-cn" | "moonshot-cn" => Some("https://api.moonshot.cn/v1"),
        // MiniMax CN host (coding-plan and regional keys) vs api.minimax.io.
        "minimax-cn-coding-plan" | "minimax-cn" | "minimaxi-cn" => {
            Some("https://api.minimaxi.com/v1")
        }
        _ => None,
    }
}

pub fn import_model_settings(doc: &Value, dest: &Path) -> Result<()> {
    let mut profile = extract_model_profile(doc);
    merge_opencode_auth(&mut profile);
    let path = dest.join("config.toml");
    if path.exists() {
        let alt = dest.join("opencode-import.toml");
        eprintln!("note: leaving existing {}; wrote {}", path.display(), alt.display());
        crate::config::write_private_file(
            &alt,
            &(config_lines(&profile, "# Generated by `lightrs import opencode`").join("\n") + "\n"),
        )?;
    } else {
        write_config(&profile, dest)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn providers_carry_all_keys() {
        let doc = json!({
            "model": "zai/glm-4.6",
            "provider": {
                "zai": {"options": {"apiKey": "glm-key", "baseURL": "https://api.z.ai/api/paas/v4/"}},
                "moonshot": {"options": {"apiKey": "kimi-key", "baseURL": "https://api.kimi.ai/coding/v1"}},
                "minimax": {"options": {"apiKey": "mm-key"}}
            }
        });
        let p = extract_model_profile(&doc);
        assert_eq!(p.keys.glm.as_deref(), Some("glm-key"));
        assert_eq!(p.keys.kimi.as_deref(), Some("kimi-key"));
        assert_eq!(p.keys.minimax.as_deref(), Some("mm-key"));
        assert_eq!(
            p.base_url.as_deref(),
            Some("https://api.z.ai/api/paas/v4")
        );
        // Every provider's baseURL lands in endpoints, not just the active one.
        assert_eq!(
            p.endpoints.get(&Provider::Glm).map(String::as_str),
            Some("https://api.z.ai/api/paas/v4")
        );
        assert_eq!(
            p.endpoints.get(&Provider::Kimi).map(String::as_str),
            Some("https://api.kimi.ai/coding/v1")
        );
    }

    #[test]
    fn env_placeholder_resolves_and_unset_drops() {
        let doc = json!({
            "provider": {
                "zai": {"options": {"apiKey": "{env:HOME}"}},
                "moonshot": {"options": {"apiKey": "{env:LIGHTRS_DEFINITELY_UNSET_XYZ}"}}
            }
        });
        let p = extract_model_profile(&doc);
        assert_eq!(
            p.keys.glm.as_deref(),
            Some(std::env::var("HOME").unwrap().as_str())
        );
        assert!(p.keys.kimi.is_none(), "unset env must not carry placeholder");
    }

    #[test]
    fn auth_doc_fills_only_missing_slots() {
        let mut p = ModelProfile::default();
        p.keys.glm = Some("explicit".into());
        let auth = json!({
            "zai": {"type": "api", "key": "auth-glm"},
            "kimi-code-plan-global": {"type": "api", "key": "auth-kimi"},
            "minimax-cn-coding-plan": {"type": "api", "key": "auth-mm"},
            "anthropic": {"type": "api", "key": "unmapped"},
            "openai": {"type": "oauth", "access": "tok"}
        });
        merge_auth_doc(&mut p, &auth);
        assert_eq!(p.keys.glm.as_deref(), Some("explicit"));
        assert_eq!(p.keys.kimi.as_deref(), Some("auth-kimi"));
        assert_eq!(p.keys.minimax.as_deref(), Some("auth-mm"));
        // Coding-plan ids carry their non-default endpoint.
        assert_eq!(
            p.endpoints.get(&Provider::Kimi).map(String::as_str),
            Some("https://api.kimi.ai/coding/v1")
        );
        assert_eq!(
            p.endpoints.get(&Provider::Minimax).map(String::as_str),
            Some("https://api.minimaxi.com/v1")
        );
        // …but not for the glm slot, which auth never filled.
        assert!(!p.endpoints.contains_key(&Provider::Glm));
    }
}
