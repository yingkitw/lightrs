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

/// Build the message list actually sent on the wire (lean history + one-turn expansions).
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
    out
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
}