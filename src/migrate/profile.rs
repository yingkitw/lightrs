use crate::config::Provider;

#[derive(Clone, Debug, Default)]
pub struct ModelProfile {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub keys: Keys,
}

#[derive(Clone, Debug, Default)]
pub struct Keys {
    pub glm: Option<String>,
    pub kimi: Option<String>,
    pub minimax: Option<String>,
}

impl ModelProfile {
    pub fn provider_or_default(&self) -> Provider {
        self.provider.unwrap_or_else(infer_provider_from_env)
    }
}

pub fn infer_provider_from_env() -> Provider {
    if std::env::var("ZAI_API_KEY")
        .or_else(|_| std::env::var("GLM_API_KEY"))
        .is_ok_and(|v| !v.is_empty())
    {
        return Provider::Glm;
    }
    if std::env::var("MOONSHOT_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Provider::Kimi;
    }
    if std::env::var("MINIMAX_API_KEY").is_ok_and(|v| !v.is_empty()) {
        return Provider::Minimax;
    }
    Provider::Kimi
}

pub fn map_opencode_provider_id(id: &str) -> Option<Provider> {
    let id = id.to_ascii_lowercase();
    if id.contains("zai") || id.contains("glm") || id.contains("zhipu") {
        Some(Provider::Glm)
    } else if id.contains("moonshot") || id.contains("kimi") {
        Some(Provider::Kimi)
    } else if id.contains("minimax") {
        Some(Provider::Minimax)
    } else {
        None
    }
}

/// Provider inference from an API endpoint (`api.z.ai` → GLM, `moonshot` →
/// Kimi, `minimax` → MiniMax). Host substrings differ from provider *ids*:
/// `api.z.ai` does not contain contiguous `zai`.
pub(crate) fn provider_from_base_url(base_url: &str) -> Option<Provider> {
    let b = base_url.to_ascii_lowercase();
    if b.contains("z.ai") || b.contains("bigmodel") {
        Some(Provider::Glm)
    } else if b.contains("moonshot") {
        Some(Provider::Kimi)
    } else if b.contains("minimax") {
        Some(Provider::Minimax)
    } else {
        None
    }
}

pub fn anthropic_base_to_profile(base_url: &str, token: &str) -> ModelProfile {
    let base = base_url.trim_end_matches('/').to_string();
    let mut keys = Keys::default();
    if base.contains("z.ai") || base.contains("bigmodel") {
        keys.glm = Some(token.to_string());
        ModelProfile {
            provider: Some(Provider::Glm),
            model: None,
            base_url: Some(if base.contains("/api/paas") {
                base
            } else if base.contains("/api/anthropic") {
                base.replace("/api/anthropic", "/api/paas/v4")
            } else if base.contains("anthropic") {
                base.replace("/anthropic", "/api/paas/v4")
            } else {
                "https://api.z.ai/api/paas/v4".into()
            }),
            keys,
        }
    } else if base.contains("moonshot") {
        keys.kimi = Some(token.to_string());
        ModelProfile {
            provider: Some(Provider::Kimi),
            model: None,
            base_url: Some(base),
            keys,
        }
    } else {
        ModelProfile::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_ids_map_by_substring() {
        for (id, want) in [
            ("zai/glm-4.6", Provider::Glm),
            ("Zhipu GLM", Provider::Glm),
            ("moonshot/kimi-k2", Provider::Kimi),
            ("KIMI", Provider::Kimi),
            ("minimax-m2", Provider::Minimax),
        ] {
            assert_eq!(map_opencode_provider_id(id), Some(want), "{id}");
        }
        assert_eq!(map_opencode_provider_id("openai/gpt-5"), None);
    }

    #[test]
    fn anthropic_base_glm_variants() {
        // Direct paas URL is kept as-is.
        let p = anthropic_base_to_profile("https://api.z.ai/api/paas/v4/", "tok");
        assert_eq!(p.provider, Some(Provider::Glm));
        assert_eq!(p.base_url.as_deref(), Some("https://api.z.ai/api/paas/v4"));
        assert_eq!(p.keys.glm.as_deref(), Some("tok"));

        // `/anthropic` suffix rewrites to the paas path.
        let p = anthropic_base_to_profile("https://api.z.ai/api/anthropic", "tok");
        assert_eq!(p.provider, Some(Provider::Glm));
        assert_eq!(
            p.base_url.as_deref(),
            Some("https://api.z.ai/api/paas/v4")
        );
    }

    #[test]
    fn anthropic_base_moonshot_and_unknown() {
        let p = anthropic_base_to_profile("https://api.moonshot.ai/anthropic/", "k");
        assert_eq!(p.provider, Some(Provider::Kimi));
        assert_eq!(
            p.base_url.as_deref(),
            Some("https://api.moonshot.ai/anthropic")
        );
        assert_eq!(p.keys.kimi.as_deref(), Some("k"));

        let p = anthropic_base_to_profile("https://api.openai.com", "k");
        assert!(p.provider.is_none() && p.keys.kimi.is_none());
    }
}