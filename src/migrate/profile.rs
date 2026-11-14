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