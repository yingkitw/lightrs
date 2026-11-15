use crate::chat::{ApiMessage, ApiRole};

/// Max chars per tool result in API history (UI may show more).
pub(crate) const TOOL_API_MAX: usize = 6_144;

pub(crate) const COMPACTED_TOOL: &str = "[compacted tool result — use read_file if needed]";
pub(crate) const COMPACT_KEEP_TOOLS: usize = 6;

const LEGACY_OMIT: &str = "\n[attachments omitted from history]";

/// Suffix appended to the active turn's composer (`\n\n[attached]…`, `\n\n[skills]…`).
pub(crate) fn user_expansion_suffix(full: &str, composer: &str) -> Option<String> {
    if full == composer || !full.starts_with(composer) {
        return None;
    }
    let tail = &full[composer.len()..];
    if tail.is_empty() {
        return None;
    }
    Some(tail.to_string())
}

/// Wire payload bounds (history itself is NOT trimmed — `/rewind` and session
/// save keep everything): at most this many user turns reach the model…
pub(crate) const MAX_RESENT_TURNS: usize = 12;
/// …and at most this estimated size (`chars/4`); oldest whole turns drop first.
pub(crate) const WIRE_BUDGET_TOKENS: u32 = 48_000;

/// Build the message list actually sent on the wire: lean history + one-turn
/// expansions, then a sliding window — keep only the last `MAX_RESENT_TURNS`
/// user turns and stay under `WIRE_BUDGET_TOKENS`. Non-destructive: callers
/// get a trimmed copy while `api_history` keeps the full conversation.
pub(crate) fn messages_for_api(
    history: &[ApiMessage],
    active_user_expansion: Option<&str>,
) -> Vec<ApiMessage> {
    let last_user = history.iter().rposition(|m| m.role == ApiRole::User);
    let mut out = Vec::with_capacity(history.len());
    for (i, msg) in history.iter().enumerate() {
        if msg.role != ApiRole::User {
            out.push(msg.clone());
            continue;
        }
        let Some(text) = msg.text_content() else {
            out.push(msg.clone());
            continue;
        };
        let text = if Some(i) == last_user {
            if let Some(suffix) = active_user_expansion.filter(|s| !s.is_empty()) {
                format!("{text}{suffix}")
            } else {
                text
            }
        } else {
            strip_legacy_user_expansions(&text)
        };
        out.push(ApiMessage::user(text));
    }
    drop_excess_turns(&mut out, MAX_RESENT_TURNS);
    while super::tokens::estimate_context_tokens(&out) > WIRE_BUDGET_TOKENS
        && drop_oldest_turn(&mut out)
    {}
    out
}

/// Drop oldest complete user turns until at most `max_turns` user messages
/// remain. Returns turns dropped; the newest (in-flight) turn is never removed.
pub(crate) fn drop_excess_turns(history: &mut Vec<ApiMessage>, max_turns: usize) -> usize {
    let mut dropped = 0;
    while history.iter().filter(|m| m.role == ApiRole::User).count() > max_turns
        && drop_oldest_turn(history)
    {
        dropped += 1;
    }
    dropped
}

/// Count of `User` messages (one per turn) — used for wire-vs-history reporting.
pub(crate) fn user_turn_count(history: &[ApiMessage]) -> usize {
    history.iter().filter(|m| m.role == ApiRole::User).count()
}

/// Drop `@` / `$` attachment blocks (session save / restore migration).
pub(crate) fn slim_stored_user_content(text: &str) -> String {
    let mut s = text.to_string();
    for marker in ["\n\n[attached]", "\n[attached]", "\n\n[skills]", "\n[skills]"] {
        if let Some(i) = s.find(marker) {
            s.truncate(i);
        }
    }
    s.trim_end().to_string()
}

pub(crate) fn slim_api_history_for_storage(history: &mut [ApiMessage]) {
    for msg in history.iter_mut() {
        if msg.role != ApiRole::User {
            continue;
        }
        let Some(text) = msg.text_content() else {
            continue;
        };
        let slim = slim_stored_user_content(&text);
        if slim != text {
            *msg = ApiMessage::user(slim);
        }
    }
}

/// Strip attachment blocks when resending older user turns (legacy sessions stored full API text).
pub(crate) fn strip_legacy_user_expansions(text: &str) -> String {
    let trimmed = slim_stored_user_content(text);
    if trimmed != text.trim_end()
        && (text.contains("[attached]") || text.contains("[skills]"))
    {
        return format!("{trimmed}{LEGACY_OMIT}");
    }
    trimmed
}

/// Replace older tool bodies with a short placeholder; keeps the last `keep_recent` tool messages intact.
pub(crate) fn compact_tool_messages(history: &mut [ApiMessage], keep_recent: usize) -> usize {
    let tool_idxs: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == ApiRole::Tool)
        .map(|(i, _)| i)
        .collect();
    if tool_idxs.len() <= keep_recent {
        return 0;
    }
    let n_compact = tool_idxs.len() - keep_recent;
    for idx in tool_idxs.into_iter().take(n_compact) {
        let msg = &history[idx];
        let Some(body) = msg.text_content() else {
            continue;
        };
        if body == COMPACTED_TOOL {
            continue;
        }
        let id = msg.tool_call_id.clone().unwrap_or_default();
        let name = msg.name.clone().unwrap_or_else(|| "tool".into());
        history[idx] = ApiMessage::tool_result(id, name, COMPACTED_TOOL);
    }
    n_compact
}

/// Arguments blobs above this size get stubbed to `{}` by `compact_tool_call_args`.
const CALL_ARGS_STUB_MAX: usize = 512;

/// Stub `tool_calls[].function.arguments` on assistant messages older than the
/// last `keep_recent` tool results (same window as `compact_tool_messages`).
/// Write/patch/multi_edit args hold full file bodies and would otherwise be
/// resent forever; `{}` stays valid JSON for the API.
pub(crate) fn compact_tool_call_args(history: &mut [ApiMessage], keep_recent: usize) -> usize {
    let tool_idxs: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == ApiRole::Tool)
        .map(|(i, _)| i)
        .collect();
    if tool_idxs.len() <= keep_recent {
        return 0;
    }
    let cutoff = tool_idxs[tool_idxs.len() - keep_recent];
    let mut n = 0;
    for msg in history[..cutoff].iter_mut() {
        let Some(calls) = &mut msg.tool_calls else {
            continue;
        };
        for call in calls.iter_mut() {
            if call.function.arguments.len() > CALL_ARGS_STUB_MAX {
                call.function.arguments = "{}".into();
                n += 1;
            }
        }
    }
    n
}

/// Remove the oldest complete user turn (its user message plus following
/// assistant/tool messages up to the next user). Never splits tool_call
/// pairs, and refuses when only the in-flight turn remains.
/// `history[0]` (system) is always kept.
pub(crate) fn drop_oldest_turn(history: &mut Vec<ApiMessage>) -> bool {
    let Some(start) = history.iter().position(|m| m.role == ApiRole::User) else {
        return false;
    };
    let end = history[start + 1..]
        .iter()
        .position(|m| m.role == ApiRole::User)
        .map(|p| start + 1 + p)
        .unwrap_or(history.len());
    if end == history.len() {
        return false;
    }
    history.drain(start..end);
    true
}

pub(crate) fn truncate_tool_api(body: &str) -> String {
    let body = crate::tools::redact::scrub_tool_output(body);
    if body.len() <= TOOL_API_MAX {
        return body;
    }
    let mut end = TOOL_API_MAX;
    while !body.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!(
        "{}\n\n[tool result truncated at {TOOL_API_MAX} chars]",
        &body[..end]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_old_attachments_from_history() {
        let raw = "fix bug\n\n[attached]\n--- @a.rs (10B) ---\nfn main(){}";
        let lean = strip_legacy_user_expansions(raw);
        assert!(!lean.contains("@a.rs"));
        assert!(lean.contains(LEGACY_OMIT));
    }

    #[test]
    fn active_expansion_only_on_last_user() {
        let history = vec![
            ApiMessage::system("sys"),
            ApiMessage::user("first\n\n[attached]\nold"),
            ApiMessage::user("second"),
        ];
        let wire = messages_for_api(&history, Some("\n\n[skills]\nbody"));
        assert!(wire[1].text_content().unwrap().contains(LEGACY_OMIT));
        assert_eq!(wire[2].text_content().unwrap(), "second\n\n[skills]\nbody");
    }

    #[test]
    fn expansion_suffix_detects_blocks() {
        let full = "hi\n\n[attached]\n--- @x ---\ndata";
        assert_eq!(
            user_expansion_suffix(full, "hi").as_deref(),
            Some("\n\n[attached]\n--- @x ---\ndata")
        );
    }

    #[test]
    fn expansion_suffix_requires_prefix() {
        assert!(user_expansion_suffix("other\n\n[attached]", "hi").is_none());
    }

    #[test]
    fn tool_api_truncates() {
        let big = "x".repeat(TOOL_API_MAX + 100);
        let t = truncate_tool_api(&big);
        assert!(t.len() < big.len());
        assert!(t.contains("truncated"));
    }

    #[test]
    fn compact_drops_oldest_tools() {
        let mut h = vec![
            ApiMessage::system("s"),
            ApiMessage::tool_result("a", "read_file", "one"),
            ApiMessage::tool_result("b", "read_file", "two"),
            ApiMessage::tool_result("c", "read_file", "three"),
        ];
        let n = compact_tool_messages(&mut h, 1);
        assert_eq!(n, 2);
        assert_eq!(h[1].text_content().unwrap(), COMPACTED_TOOL);
        assert_eq!(h[3].text_content().unwrap(), "three");
    }

    fn assistant_call(args_len: usize) -> ApiMessage {
        use crate::chat::{FunctionCall, ToolCall};
        ApiMessage {
            role: ApiRole::Assistant,
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "c".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "write_file".into(),
                    arguments: "x".repeat(args_len),
                },
            }]),
            tool_call_id: None,
            name: None,
        }
    }

    #[test]
    fn compact_stubs_old_call_args_keeps_recent() {
        let mut h = vec![ApiMessage::system("s"), assistant_call(10_000)];
        for i in 0..6 {
            h.push(ApiMessage::tool_result(format!("t{i}"), "read_file", "b"));
        }
        h.push(assistant_call(10_000));
        h.push(ApiMessage::tool_result("t6", "read_file", "b"));

        let n = compact_tool_call_args(&mut h, 6);
        assert_eq!(n, 1);
        assert_eq!(h[1].tool_calls.as_ref().unwrap()[0].function.arguments, "{}");
        // The call inside the recent window keeps its arguments.
        assert_eq!(
            h[8].tool_calls.as_ref().unwrap()[0].function.arguments.len(),
            10_000
        );
    }

    #[test]
    fn compact_call_args_noop_under_keep_window() {
        let mut h = vec![
            ApiMessage::system("s"),
            assistant_call(10_000),
            ApiMessage::tool_result("t", "read_file", "b"),
        ];
        assert_eq!(compact_tool_call_args(&mut h, 6), 0);
        assert_eq!(
            h[1].tool_calls.as_ref().unwrap()[0].function.arguments.len(),
            10_000
        );
    }

    #[test]
    fn drop_oldest_removes_whole_turn() {
        let mut h = vec![
            ApiMessage::system("s"),
            ApiMessage::user("u1"),
            ApiMessage::tool_result("t1", "read_file", "b"),
            ApiMessage::tool_result("t2", "read_file", "b"),
            ApiMessage::user("u2"),
            ApiMessage::tool_result("t3", "read_file", "b"),
        ];
        assert!(drop_oldest_turn(&mut h));
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].role, ApiRole::System);
        assert_eq!(h[1].text_content().unwrap(), "u2");
    }

    #[test]
    fn drop_oldest_refuses_in_flight_only_turn() {
        let mut h = vec![
            ApiMessage::system("s"),
            ApiMessage::user("u1"),
            ApiMessage::tool_result("t1", "read_file", "b"),
        ];
        assert!(!drop_oldest_turn(&mut h));
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn wire_window_keeps_only_last_n_turns() {
        let mut history = vec![ApiMessage::system("s")];
        for i in 0..20 {
            history.push(ApiMessage::user(format!("u{i}")));
            history.push(ApiMessage::tool_result(format!("t{i}"), "x", "r"));
        }
        let original_len = history.len();
        let wire = messages_for_api(&history, None);
        assert_eq!(user_turn_count(&wire), MAX_RESENT_TURNS);
        assert_eq!(wire[0].role, ApiRole::System);
        // Last turns survive, oldest dropped at whole-turn boundaries.
        let last_user = wire.iter().rev().find(|m| m.role == ApiRole::User).unwrap();
        assert_eq!(last_user.text_content().unwrap(), "u19");
        // History itself untouched — windowing is wire-only.
        assert_eq!(history.len(), original_len);
    }

    #[test]
    fn wire_budget_drops_oldest_turns() {
        let mut history = vec![ApiMessage::system("s")];
        for i in 0..6 {
            history.push(ApiMessage::user("x".repeat(40_000)));
            history.push(ApiMessage::tool_result(format!("t{i}"), "x", "r"));
        }
        let wire = messages_for_api(&history, None);
        assert!(super::super::tokens::estimate_context_tokens(&wire) <= WIRE_BUDGET_TOKENS + 1);
        assert_eq!(wire[0].role, ApiRole::System);
        // In-flight turn is never dropped even when over budget.
        assert!(wire.iter().any(|m| m.role == ApiRole::User));
    }

    #[test]
    fn wire_window_under_limits_is_identity() {
        let history = vec![
            ApiMessage::system("s"),
            ApiMessage::user("u1"),
            ApiMessage::tool_result("t1", "x", "r"),
        ];
        let wire = messages_for_api(&history, None);
        assert_eq!(wire.len(), history.len());
    }
}