use crate::chat::{ApiMessage, ApiRole, TokenUsage};

/// Rough context size above which the TUI shows a `[sys]` pressure hint (~80k chars).
pub const CONTEXT_WARN_TOKENS: u32 = 20_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenStats {
    pub last_prompt: u32,
    pub last_completion: u32,
    pub session_prompt: u64,
    pub session_completion: u64,
    /// Current-turn accumulators — reset by `begin_turn`, summed by `apply_usage`.
    #[serde(default)]
    pub turn_prompt: u64,
    #[serde(default)]
    pub turn_completion: u64,
    #[serde(default)]
    pub turn_calls: u32,
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

    /// Reset per-turn accumulators; called at the start of each user turn.
    pub fn begin_turn(&mut self) {
        self.turn_prompt = 0;
        self.turn_completion = 0;
        self.turn_calls = 0;
    }
}

/// Categorized context footprint (~chars/4) — answers "where do the tokens go".
#[derive(Clone, Copy, Debug, Default)]
pub struct ContextBreakdown {
    pub system: u32,
    pub user: u32,
    pub assistant: u32,
    pub tool_results: u32,
    pub call_args: u32,
}

/// Bucket each wire message by role; assistant `tool_calls` name+args count
/// separately since they often dwarf the surrounding text.
pub fn context_breakdown(messages: &[ApiMessage]) -> ContextBreakdown {
    let mut b = ContextBreakdown::default();
    for m in messages {
        let text_tok = m.text_content().map_or(0, |t| estimate_text_tokens(&t));
        match m.role {
            ApiRole::System => b.system += text_tok,
            ApiRole::User => b.user += text_tok,
            ApiRole::Assistant => b.assistant += text_tok,
            ApiRole::Tool => b.tool_results += text_tok,
        }
        if let Some(calls) = &m.tool_calls {
            for c in calls {
                b.call_args += estimate_text_tokens(&c.function.name)
                    + estimate_text_tokens(&c.function.arguments);
            }
        }
    }
    b
}

pub fn format_breakdown(b: &ContextBreakdown) -> String {
    format!(
        "sys {} · user {} · asst {} · tool-out {} · call-args {}",
        format_token_count(u64::from(b.system)),
        format_token_count(u64::from(b.user)),
        format_token_count(u64::from(b.assistant)),
        format_token_count(u64::from(b.tool_results)),
        format_token_count(u64::from(b.call_args)),
    )
}

/// Heaviest single messages in the outgoing payload, labeled `role/name #idx`.
pub fn top_consumers(messages: &[ApiMessage], n: usize) -> Vec<String> {
    let mut rows: Vec<(u32, String)> = Vec::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        let mut tok = m.text_content().map_or(0, |t| estimate_text_tokens(&t));
        let mut label = match m.role {
            ApiRole::System => "system prompt".to_string(),
            ApiRole::User => format!("user #{i}"),
            ApiRole::Assistant => format!("asst #{i}"),
            ApiRole::Tool => format!("tool {} #{i}", m.name.as_deref().unwrap_or("?")),
        };
        if let Some(calls) = &m.tool_calls {
            let arg_tok: u32 = calls
                .iter()
                .map(|c| {
                    estimate_text_tokens(&c.function.name)
                        + estimate_text_tokens(&c.function.arguments)
                })
                .sum();
            if arg_tok > 0 {
                tok += arg_tok;
                label = format!("call args #{i}");
            }
        }
        rows.push((tok, label));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    rows.into_iter()
        .filter(|(t, _)| *t > 0)
        .take(n)
        .map(|(t, l)| format!("{l} ~{}", format_token_count(u64::from(t))))
        .collect()
}

/// End-of-turn real-usage summary line (`turn · N calls · X in + Y out`).
pub fn turn_usage_line(stats: &TokenStats) -> Option<String> {
    if stats.turn_calls == 0 || stats.turn_prompt + stats.turn_completion == 0 {
        return None;
    }
    Some(format!(
        "turn · {} call(s) · {} in + {} out",
        stats.turn_calls,
        format_token_count(stats.turn_prompt),
        format_token_count(stats.turn_completion)
    ))
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
    chars.div_ceil(4).max(1) as u32
}

/// Rough token count from text length (~chars/4).
pub fn estimate_text_tokens(text: &str) -> u32 {
    text.chars().count().div_ceil(4).max(1) as u32
}

/// One-line transcript hint after an API completion when usage is present.
pub fn step_usage_line(usage: Option<&TokenUsage>) -> Option<String> {
    let u = usage?;
    let prompt = u.prompt_tokens.unwrap_or(0);
    let completion = u.completion_tokens.unwrap_or(0);
    if prompt == 0 && completion == 0 {
        return None;
    }
    Some(format!(
        "model · {} in + {} out",
        format_token_count(u64::from(prompt)),
        format_token_count(u64::from(completion)),
    ))
}

pub fn apply_usage(stats: &mut TokenStats, usage: Option<&TokenUsage>) {
    stats.last_prompt = 0;
    stats.last_completion = 0;
    stats.turn_calls += 1;
    if let Some(u) = usage {
        stats.last_prompt = u.prompt_tokens.unwrap_or(0);
        stats.last_completion = u.completion_tokens.unwrap_or(0);
        stats.session_prompt += u64::from(stats.last_prompt);
        stats.session_completion += u64::from(stats.last_completion);
        stats.turn_prompt += u64::from(stats.last_prompt);
        stats.turn_completion += u64::from(stats.last_completion);
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

pub fn context_pressure_line(estimate: u32) -> Option<String> {
    if estimate < CONTEXT_WARN_TOKENS {
        return None;
    }
    Some(format!(
        "context · ~{} est — large history; try /clear",
        format_token_count(u64::from(estimate))
    ))
}

pub fn context_usage_text(
    summary: TokenSummary,
    api_messages: usize,
    window: Option<u32>,
) -> String {
    let TokenSummary { stats, estimate } = summary;
    let est = match window {
        Some(w) if w > 0 => format!(
            "~{} ctx est ({}%)",
            format_token_count(u64::from(estimate)),
            u64::from(estimate) * 100 / u64::from(w)
        ),
        _ => format!("~{} ctx est", format_token_count(u64::from(estimate))),
    };
    let mut parts = vec![format!("{api_messages} msg"), est];
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
    #[test]
    fn usage_text_shows_window_percent() {
        let text = super::context_usage_text(
            super::TokenSummary {
                stats: super::TokenStats::default(),
                estimate: 12_288,
            },
            3,
            Some(204_800),
        );
        assert!(text.contains("ctx est (6%)"), "{text}");
        let plain = super::context_usage_text(
            super::TokenSummary {
                stats: super::TokenStats::default(),
                estimate: 100,
            },
            1,
            None,
        );
        assert!(!plain.contains('%'), "{plain}");
    }

    use super::*;

    #[test]
    fn estimate_from_text() {
        let msgs = vec![ApiMessage::user("a".repeat(400))];
        assert!(estimate_context_tokens(&msgs) >= 100);
    }

    #[test]
    fn context_pressure_threshold() {
        assert!(context_pressure_line(CONTEXT_WARN_TOKENS).is_some());
        assert!(context_pressure_line(CONTEXT_WARN_TOKENS - 1).is_none());
    }

    #[test]
    fn step_usage_formats() {
        let line = step_usage_line(Some(&TokenUsage {
            prompt_tokens: Some(1200),
            completion_tokens: Some(48),
            total_tokens: Some(1248),
        }));
        assert_eq!(line.as_deref(), Some("model · 1.20k in + 48 out"));
    }

    #[test]
    fn breakdown_buckets_by_role() {
        let msgs = vec![
            ApiMessage::system("s".repeat(400)),
            ApiMessage::user("u".repeat(400)),
            ApiMessage::tool_result("c1", "run_command", "t".repeat(800)),
            ApiMessage {
                role: ApiRole::Assistant,
                content: Some(serde_json::Value::String("a".repeat(200))),
                tool_calls: Some(vec![crate::chat::ToolCall {
                    id: "c1".into(),
                    call_type: "function".into(),
                    function: crate::chat::FunctionCall {
                        name: "x".into(),
                        arguments: "y".repeat(400),
                    },
                }]),
                tool_call_id: None,
                name: None,
            },
        ];
        let b = context_breakdown(&msgs);
        assert_eq!(b.system, 100);
        assert_eq!(b.user, 100);
        assert_eq!(b.tool_results, 200);
        assert_eq!(b.assistant, 50);
        assert!(b.call_args >= 100);
        assert!(format_breakdown(&b).contains("tool-out 200"));
    }

    #[test]
    fn top_consumers_ranks_heaviest() {
        let msgs = vec![
            ApiMessage::system("s".repeat(40)),
            ApiMessage::user("u".repeat(4000)),
            ApiMessage::tool_result("c1", "run_command", "t".repeat(800)),
        ];
        let top = top_consumers(&msgs, 2);
        assert_eq!(top.len(), 2);
        assert!(top[0].starts_with("user #1"), "{top:?}");
        assert!(top[1].starts_with("tool run_command #2"), "{top:?}");
    }

    #[test]
    fn turn_stats_accumulate_and_reset() {
        let mut stats = TokenStats::default();
        stats.begin_turn();
        apply_usage(
            &mut stats,
            Some(&TokenUsage {
                prompt_tokens: Some(100),
                completion_tokens: Some(10),
                total_tokens: Some(110),
            }),
        );
        apply_usage(
            &mut stats,
            Some(&TokenUsage {
                prompt_tokens: Some(200),
                completion_tokens: Some(20),
                total_tokens: Some(220),
            }),
        );
        assert_eq!(stats.turn_calls, 2);
        assert_eq!(stats.turn_prompt, 300);
        assert_eq!(stats.session_prompt, 300);
        let line = turn_usage_line(&stats).unwrap();
        assert!(line.contains("2 call(s)"), "{line}");
        stats.begin_turn();
        assert_eq!(stats.turn_calls, 0);
        assert_eq!(stats.session_prompt, 300); // session survives reset
        assert!(turn_usage_line(&stats).is_none());
    }
}