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
use crate::chat::TokenUsage;

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
        let mut out = format!(
            "limits: wire ≤{MAX_RESENT_TURNS} turns / ~{}k est · stored stub ≥{}k / drop ≥{}k · tool ≤{}B · ui ≤{}\nwire: {wire_turns} of {history_turns} turns · {} msgs\nbreakdown: {}",
            WIRE_BUDGET_TOKENS / 1_000,
            AUTO_COMPACT_TOKENS / 1_000,
            HISTORY_HARD_CAP_TOKENS / 1_000,
            TOOL_API_MAX,
            crate::agent::helpers::MAX_UI_MESSAGES,
            msgs.len(),
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
