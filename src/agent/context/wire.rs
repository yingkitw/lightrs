//! Wire layer — the message list actually sent per request. Non-destructive:
//! `api_history` keeps the full conversation (`/rewind`, session save); this
//! module builds the trimmed copy (expansions + window + byte budget).

use crate::chat::{ApiMessage, ApiRole};
use super::compact::drop_oldest_turn;
use super::tokens::estimate_context_tokens;

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
    while estimate_context_tokens(&out) > WIRE_BUDGET_TOKENS && drop_oldest_turn(&mut out) {}
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
    if trimmed != text.trim_end() && (text.contains("[attached]") || text.contains("[skills]")) {
        return format!("{trimmed}{LEGACY_OMIT}");
    }
    trimmed
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
