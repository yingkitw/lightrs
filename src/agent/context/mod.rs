//! Context management — what lightrs stores vs. what it sends. See
//! `docs/context.md` for the full budget spec.
//!
//! Two layers, deliberately separate:
//! - **`wire`**: bounds each request payload (last-N turns + byte budget) on a
//!   *copy* — `api_history` stays intact for `/rewind` and session save.
//! - **`compact`**: shrinks stored history destructively (tool/arg stubs, then
//!   oldest-turn drops) so sessions stay small.
//! - **`tokens`**: `chars/4` estimation + real provider usage + breakdowns.

mod compact;
pub(crate) mod tokens;
mod wire;

pub(crate) use compact::*;
pub(crate) use tokens::*;
pub(crate) use wire::*;

use crate::agent::Agent;
use crate::chat::{ApiMessage, ApiRole, TokenUsage};

/// Raw-history estimate at which stored compaction stubs old tool data.
pub(crate) const AUTO_COMPACT_TOKENS: u32 = 32_000;
/// Raw-history estimate above which whole oldest turns are dropped permanently.
pub(crate) const HISTORY_HARD_CAP_TOKENS: u32 = 64_000;

impl Agent {
    /// Stored-history compaction run before every request (auto `/compact`):
    /// above `AUTO_COMPACT_TOKENS` stub old tool bodies + tool_call args;
    /// above `HISTORY_HARD_CAP_TOKENS` drop whole oldest turns permanently.
    /// Measured on the **raw** history estimate — the wire payload has its own
    /// tighter window in `wire::messages_for_api`.
    pub(crate) fn auto_compact_context(&mut self) {
        let raw_est = estimate_context_tokens(&self.api_history);
        if raw_est < AUTO_COMPACT_TOKENS {
            return;
        }
        let n_tools = compact_tool_messages(&mut self.api_history, COMPACT_KEEP_TOOLS);
        let n_args = compact_tool_call_args(&mut self.api_history, COMPACT_KEEP_TOOLS);
        let mut dropped = 0usize;
        while estimate_context_tokens(&self.api_history) > HISTORY_HARD_CAP_TOKENS
            && drop_oldest_turn(&mut self.api_history)
        {
            dropped += 1;
        }
        if n_tools + n_args + dropped > 0 {
            self.refresh_context_estimate();
            self.push_system(format!(
                "context auto-compact: {n_tools} tool result(s), {n_args} call arg(s), {dropped} old turn(s) dropped (~{raw_est} → ~{} est)",
                self.estimated_context_tokens
            ));
        }
        self.context_warned_above = self.context_warned_above.max(raw_est);
    }

    /// A turn interrupted mid-tool-batch leaves an assistant `tool_calls`
    /// message whose later calls never produced results — OpenAI 400s on that
    /// wire shape (every call needs a matching `tool` message directly
    /// following it). Insert a stub result for each unanswered call right
    /// after the batch's partial results; the model then sees the call was
    /// interrupted instead of the next request failing opaquely. Runs before
    /// every request so both live-session and restored orphans are repaired.
    pub(crate) fn close_orphaned_tool_calls(&mut self) {
        let Some(call_idx) = self.api_history.iter().rposition(|m| {
            m.role == ApiRole::Assistant && m.tool_calls.as_ref().is_some_and(|t| !t.is_empty())
        }) else {
            return;
        };
        // A later assistant message means the batch closed normally.
        if self.api_history[call_idx + 1..]
            .iter()
            .any(|m| m.role == ApiRole::Assistant)
        {
            return;
        }
        let answered: std::collections::HashSet<String> = self.api_history[call_idx + 1..]
            .iter()
            .filter_map(|m| m.tool_call_id.clone())
            .collect();
        let calls = self.api_history[call_idx]
            .tool_calls
            .clone()
            .unwrap_or_default();
        let mut insert_at = call_idx + 1;
        while self
            .api_history
            .get(insert_at)
            .is_some_and(|m| m.role == ApiRole::Tool)
        {
            insert_at += 1;
        }
        for call in calls {
            if !answered.contains(&call.id) {
                self.api_history.insert(
                    insert_at,
                    ApiMessage::tool_result(
                        call.id,
                        call.function.name,
                        "[interrupted — no result recorded]",
                    ),
                );
                insert_at += 1;
            }
        }
    }

    pub fn token_summary(&self) -> TokenSummary {
        TokenSummary {
            stats: self.token_stats,
            estimate: self.estimated_context_tokens,
        }
    }

    pub(crate) fn refresh_context_estimate(&mut self) {
        self.estimated_context_tokens = estimate_context_tokens(&self.messages_for_request());
    }

    /// Show a `[sys]` hint when API history is large (`/quiet` suppresses).
    pub(crate) fn maybe_context_pressure_sys(&mut self) {
        if self.quiet_token_sys {
            return;
        }
        let est = self.estimated_context_tokens;
        if est < CONTEXT_WARN_TOKENS {
            return;
        }
        const REWARN_DELTA: u32 = 4_000;
        if self.context_warned_above > 0
            && est < self.context_warned_above.saturating_add(REWARN_DELTA)
        {
            return;
        }
        if let Some(line) = context_pressure_line(est) {
            self.context_warned_above = est;
            self.push_system(line);
        }
    }

    pub fn set_quiet_token_sys(&mut self, quiet: bool) {
        self.quiet_token_sys = quiet;
    }

    pub fn quiet_token_sys(&self) -> bool {
        self.quiet_token_sys
    }

    pub(crate) fn record_send_estimate(&mut self, api_text: &str) {
        if self.quiet_token_sys {
            return;
        }
        let est = estimate_text_tokens(api_text);
        self.push_system(format!(
            "send · ~{} ctx",
            format_token_count(u64::from(est))
        ));
    }

    pub(crate) fn record_model_usage(&mut self, usage: Option<&TokenUsage>) {
        if self.quiet_token_sys {
            return;
        }
        if let Some(line) = step_usage_line(usage) {
            self.push_system(line);
        }
    }

    /// Count of stored API messages (history size, not wire size).
    pub fn api_context_len(&self) -> usize {
        self.api_history.len()
    }

    /// Manual `/compact`: stub old tool bodies + call args in stored history.
    pub fn compact_history(&mut self) -> usize {
        let n = compact_tool_messages(&mut self.api_history, COMPACT_KEEP_TOOLS)
            + compact_tool_call_args(&mut self.api_history, COMPACT_KEEP_TOOLS);
        self.ensure_system_prompt();
        self.context_warned_above = 0;
        self.refresh_context_estimate();
        n
    }

    /// `/context` detail: budget limits, wire-vs-history shape, per-role
    /// breakdown, heaviest messages, and the current turn's real usage.
    pub fn context_report(&self) -> String {
        let msgs = self.messages_for_request();
        let wire_turns = user_turn_count(&msgs);
        let history_turns = user_turn_count(&self.api_history);
        let window = crate::config::model_context_window(self.client.model());
        let win_label = window.map_or_else(
            || "window ?".to_string(),
            |w| format!("window {}k", w / 1_000),
        );
        let mut out = format!(
            "limits: {win_label} · wire ≤{MAX_RESENT_TURNS} turns / ~{}k est · stored stub ≥{}k / drop ≥{}k · tool ≤{}B · ui ≤{}\nwire: {wire_turns} of {history_turns} turns · {} msgs{}\nbreakdown: {}",
            WIRE_BUDGET_TOKENS / 1_000,
            AUTO_COMPACT_TOKENS / 1_000,
            HISTORY_HARD_CAP_TOKENS / 1_000,
            TOOL_API_MAX,
            crate::agent::helpers::MAX_UI_MESSAGES,
            msgs.len(),
            window.map_or_else(String::new, |w| format!(
                " · ~{}% of window",
                u64::from(estimate_context_tokens(&msgs)) * 100 / u64::from(w)
            )),
            format_breakdown(&context_breakdown(&msgs))
        );
        let top = top_consumers(&msgs, 3);
        if !top.is_empty() {
            out.push_str(&format!("\ntop: {}", top.join(" · ")));
        }
        if let Some(turn) = turn_usage_line(&self.token_stats) {
            out.push_str(&format!("\nturn: {turn}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::CONTEXT_WARN_TOKENS;
    use std::sync::Arc;

    use crate::agent::Agent;
    use crate::chat::ApiMessage;
    use crate::provider::LlmClient;
    use crate::tools::ShellPolicy;

    fn agent() -> Agent {
        Agent::new(
            LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into()),
            Arc::from(std::path::Path::new(".")),
            ShellPolicy::default(),
        )
    }

    fn last_sys(agent: &Agent) -> String {
        agent
            .ui_messages()
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }

    #[test]
    fn report_lists_limits_wire_and_breakdown() {
        let a = agent();
        let report = a.context_report();
        assert!(report.contains("limits:") && report.contains("wire ≤12 turns / ~48k est"));
        assert!(report.contains("wire: 0 of 0 turns"));
        assert!(report.contains("breakdown:"));
    }

    #[test]
    fn quiet_gates_all_routine_sys_lines() {
        let mut a = agent();
        a.set_quiet_token_sys(true);
        let before = a.ui_messages().len();
        a.record_send_estimate("hello");
        a.record_model_usage(Some(&crate::chat::TokenUsage {
            prompt_tokens: Some(5),
            completion_tokens: Some(2),
            total_tokens: None,
        }));
        a.maybe_context_pressure_sys();
        assert_eq!(a.ui_messages().len(), before);
        assert!(a.quiet_token_sys());
    }

    #[test]
    fn send_estimate_and_usage_push_sys_when_loud() {
        let mut a = agent();
        a.record_send_estimate("hello world");
        assert!(last_sys(&a).starts_with("send · ~"));
        a.record_model_usage(Some(&crate::chat::TokenUsage {
            prompt_tokens: Some(10),
            completion_tokens: Some(4),
            total_tokens: None,
        }));
        assert!(last_sys(&a).contains("in") && last_sys(&a).contains("out"));
    }

    #[test]
    fn pressure_warns_once_then_rewarns_on_growth() {
        let mut a = agent();
        // Fake a large wire estimate so the pressure check trips.
        a.estimated_context_tokens = CONTEXT_WARN_TOKENS + 1;
        a.maybe_context_pressure_sys();
        let warned = last_sys(&a);
        assert!(warned.contains("context") || warned.contains("~"));

        let n = a.ui_messages().len();
        // Same level → no rewarn.
        a.maybe_context_pressure_sys();
        assert_eq!(a.ui_messages().len(), n);
        // +4k growth → rewarn.
        a.estimated_context_tokens += 4_001;
        a.maybe_context_pressure_sys();
        assert!(a.ui_messages().len() > n);
    }

    #[test]
    fn manual_compact_stubs_and_resets_warn() {
        let mut a = agent();
        for i in 0..8 {
            a.api_history.push(ApiMessage::tool_result(
                format!("id{i}"),
                "read_file",
                "x".repeat(400),
            ));
        }
        a.context_warned_above = 50_000;
        let n = a.compact_history();
        assert!(n >= 1, "expected stubs, got {n}");
        assert_eq!(a.context_warned_above, 0);
        // Older tool bodies stubbed; the keep-window tail stays intact.
        let texts: Vec<String> = a
            .api_history
            .iter()
            .filter_map(|m| m.text_content())
            .collect();
        assert!(texts.iter().any(|t| t.contains("compacted tool result")));
        assert!(texts.iter().any(|t| t.len() == 400));
    }

    #[test]
    fn api_context_len_counts_stored_history() {
        let mut a = agent();
        let base = a.api_context_len();
        a.api_history.push(ApiMessage::user("hi"));
        assert_eq!(a.api_context_len(), base + 1);
        let s = a.token_summary();
        assert_eq!(s.stats, a.token_stats);
    }

    fn assistant_calls(ids: &[&str]) -> ApiMessage {
        use crate::chat::{ApiRole, FunctionCall, ToolCall};
        ApiMessage {
            role: ApiRole::Assistant,
            content: None,
            tool_calls: Some(
                ids.iter()
                    .map(|id| ToolCall {
                        id: (*id).into(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: "read_file".into(),
                            arguments: "{}".into(),
                        },
                    })
                    .collect(),
            ),
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    /// Interrupt mid-batch: call 2 answered, call 3 orphaned → stub inserted
    /// after the partial results, before any later user message.
    #[test]
    fn close_orphaned_stubs_missing_results_in_position() {
        let mut a = agent();
        a.api_history.push(assistant_calls(&["c1", "c2", "c3"]));
        a.api_history
            .push(ApiMessage::tool_result("c1", "read_file", "ok"));
        a.api_history
            .push(ApiMessage::tool_result("c2", "read_file", "ok"));
        a.api_history.push(ApiMessage::user("next question"));
        let before = a.api_history.len();
        a.close_orphaned_tool_calls();
        assert_eq!(a.api_history.len(), before + 1);
        // Stub sits between the last real result and the user message.
        let stub = &a.api_history[before - 1];
        assert_eq!(stub.tool_call_id.as_deref(), Some("c3"));
        assert!(
            stub.text_content()
                .unwrap_or_default()
                .contains("interrupted"),
            "stub: {stub:?}"
        );
    }

    #[test]
    fn close_orphaned_noop_when_batch_complete() {
        let mut a = agent();
        a.api_history.push(assistant_calls(&["c1"]));
        a.api_history
            .push(ApiMessage::tool_result("c1", "read_file", "ok"));
        a.api_history.push(ApiMessage {
            role: crate::chat::ApiRole::Assistant,
            content: Some(serde_json::Value::String("done".into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        });
        let before = a.api_history.len();
        a.close_orphaned_tool_calls();
        assert_eq!(a.api_history.len(), before);
    }

    /// Restored-session orphan: call recorded, process killed before any
    /// result — the next request must not 400 on the dangling tool_calls.
    #[test]
    fn close_orphaned_repairs_fully_unanswered_batch() {
        let mut a = agent();
        a.api_history.push(assistant_calls(&["c1", "c2"]));
        let before = a.api_history.len();
        a.close_orphaned_tool_calls();
        assert_eq!(a.api_history.len(), before + 2);
        assert_eq!(a.api_history[before].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(
            a.api_history[before + 1].tool_call_id.as_deref(),
            Some("c2")
        );
    }
}
