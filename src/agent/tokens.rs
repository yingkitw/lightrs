use crate::chat::{ApiMessage, TokenUsage};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenStats {
    pub last_prompt: u32,
    pub last_completion: u32,
    pub session_prompt: u64,
    pub session_completion: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenSummary {
    pub stats: TokenStats,
    /// Rough context size from API history (~chars/4).
    pub estimate: u32,
}

impl TokenStats {
    pub fn session_total(&self) -> u64 {
        self.session_prompt + self.session_completion
    }

    pub fn last_total(&self) -> u32 {
        self.last_prompt + self.last_completion
    }
}

pub fn estimate_context_tokens(messages: &[ApiMessage]) -> u32 {
    let mut chars = 0usize;
    for m in messages {
        if let Some(text) = m.text_content() {
            chars += text.chars().count();
        }
        if let Some(calls) = &m.tool_calls {
            for call in calls {
                chars += call.function.name.chars().count();
                chars += call.function.arguments.chars().count();
            }
        }
        if let Some(name) = &m.name {
            chars += name.chars().count();
        }
    }
    ((chars + 3) / 4).max(1) as u32
}

pub fn apply_usage(stats: &mut TokenStats, usage: Option<&TokenUsage>) {
    stats.last_prompt = 0;
    stats.last_completion = 0;
    if let Some(u) = usage {
        stats.last_prompt = u.prompt_tokens.unwrap_or(0);
        stats.last_completion = u.completion_tokens.unwrap_or(0);
        stats.session_prompt += u64::from(stats.last_prompt);
        stats.session_completion += u64::from(stats.last_completion);
    }
}

pub fn format_token_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        format!("{:.2}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn context_usage_text(summary: TokenSummary, api_messages: usize) -> String {
    let TokenSummary { stats, estimate } = summary;
    let mut parts = vec![
        format!("{api_messages} msg"),
        format!("~{} ctx est", format_token_count(u64::from(estimate))),
    ];
    if stats.session_total() > 0 {
        parts.push(format!(
            "{} session",
            format_token_count(stats.session_total())
        ));
        if stats.last_total() > 0 {
            parts.push(format!(
                "last {}+{}",
                format_token_count(u64::from(stats.last_prompt)),
                format_token_count(u64::from(stats.last_completion))
            ));
        }
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_from_text() {
        let msgs = vec![ApiMessage::user("a".repeat(400))];
        assert!(estimate_context_tokens(&msgs) >= 100);
    }
}