//! Compact layer — shrinks the *stored* `api_history`: tool-result stubbing,
//! tool_call argument stubbing, and whole-turn drops. Unlike `wire`, this is
//! destructive by design (it is what `/compact` and `auto_compact_context` use).

use crate::chat::{ApiMessage, ApiRole};

/// Max chars per tool result in API history (UI may show more).
pub(crate) const TOOL_API_MAX: usize = 6_144;

pub(crate) const COMPACTED_TOOL: &str = "[compacted tool result — use read_file if needed]";
pub(crate) const COMPACT_KEEP_TOOLS: usize = 6;

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
    format!(
        "{}\n\n[tool result truncated at {TOOL_API_MAX} chars]",
        crate::tools::truncate_bytes(&body, TOOL_API_MAX)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
            reasoning_content: None,
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
}
