use std::sync::{Arc, Mutex};

use crate::chat::{UiMessage, UiRole};

use super::ObserveEvent;

pub(crate) const MAX_TOOL_STEPS: usize = 16;
const TOOL_UI_TRUNCATE: usize = 8_000;
/// Transcript messages kept in memory (and persisted): oldest drop first.
/// Tool blocks are self-contained `UiMessage`s, so front-dropping can't split
/// a call/result pair. Bounds session JSON size and per-frame render cost.
pub(crate) const MAX_UI_MESSAGES: usize = 500;

/// Keep `ui_history` at most `MAX_UI_MESSAGES` long by dropping the oldest.
pub(crate) fn trim_ui_history(ui_history: &Arc<Mutex<Vec<UiMessage>>>) {
    let mut ui = ui_history.lock().unwrap();
    let excess = ui.len().saturating_sub(MAX_UI_MESSAGES);
    if excess > 0 {
        ui.drain(..excess);
    }
}

/// Drop trailing whitespace/newlines from model text (avoids extra blank lines in the TUI).
pub(crate) fn normalize_assistant_text(s: &str) -> String {
    s.trim_end().to_string()
}

pub(crate) fn truncate_tool_ui(s: &str) -> String {
    truncate(s, TOOL_UI_TRUNCATE)
}

pub(crate) fn append_assistant_delta(
    ui_history: &Arc<Mutex<Vec<UiMessage>>>,
    chunk: &str,
    on: &mut impl FnMut(ObserveEvent<'_>),
) {
    if chunk.is_empty() {
        return;
    }
    let mut ui = ui_history.lock().unwrap();
    let needs_row = ui.last().is_none_or(|m| m.role != UiRole::Assistant);
    if needs_row {
        ui.push(UiMessage::with_role(UiRole::Assistant, String::new()));
    }
    if let Some(last) = ui.last_mut() {
        last.content.push_str(chunk);
    }
    drop(ui);
    on(ObserveEvent::AssistantDelta { text: chunk });
}

/// Append streamed reasoning text to the trailing `[think]` row, creating it
/// on first delta. Reasoning arrives before content (providers emit it
/// first), so the think row naturally precedes the assistant row.
pub(crate) fn append_thinking_delta(
    ui_history: &Arc<Mutex<Vec<UiMessage>>>,
    chunk: &str,
    on: &mut impl FnMut(ObserveEvent<'_>),
) {
    if chunk.is_empty() {
        return;
    }
    let mut ui = ui_history.lock().unwrap();
    if ui.last().is_none_or(|m| m.role != UiRole::Thinking) {
        ui.push(UiMessage::with_role(UiRole::Thinking, String::new()));
    }
    if let Some(last) = ui.last_mut() {
        last.content.push_str(chunk);
    }
    drop(ui);
    on(ObserveEvent::ThinkingDelta { text: chunk });
}

pub(crate) fn remove_trailing_empty_assistant(ui_history: &Arc<Mutex<Vec<UiMessage>>>) {
    let mut ui = ui_history.lock().unwrap();
    if let Some(last) = ui.last()
        && last.role == UiRole::Assistant && last.content.is_empty() {
            ui.pop();
        }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end: String = s.chars().take(max).collect();
    format!("{end}…")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::chat::UiRole;

    use super::{append_assistant_delta, ObserveEvent};

    #[test]
    fn append_assistant_delta_emits_observe_and_merges_rows() {
        let ui = Arc::new(Mutex::new(Vec::new()));
        let mut delta_texts = Vec::new();
        append_assistant_delta(&ui, "ab", &mut |ev| {
            if let ObserveEvent::AssistantDelta { text } = ev {
                delta_texts.push(text.to_string());
            }
        });
        append_assistant_delta(&ui, "c", &mut |ev| {
            if let ObserveEvent::AssistantDelta { text } = ev {
                delta_texts.push(text.to_string());
            }
        });

        let lock = ui.lock().unwrap();
        assert_eq!(lock.len(), 1);
        assert_eq!(lock[0].role, UiRole::Assistant);
        assert_eq!(lock[0].content, "abc");
        assert_eq!(delta_texts, ["ab", "c"]);
    }

    #[test]
    fn append_thinking_delta_builds_separate_row() {
        let ui = Arc::new(Mutex::new(Vec::new()));
        super::append_thinking_delta(&ui, "plan", &mut |_| {});
        super::append_thinking_delta(&ui, " A", &mut |_| {});
        super::append_thinking_delta(&ui, "", &mut |_| {}); // empty: no-op
        append_assistant_delta(&ui, "answer", &mut |_| {});
        let lock = ui.lock().unwrap();
        assert_eq!(lock.len(), 2);
        assert_eq!(lock[0].role, UiRole::Thinking);
        assert_eq!(lock[0].content, "plan A");
        assert_eq!(lock[1].role, UiRole::Assistant);
        assert_eq!(lock[1].content, "answer");
    }

    #[test]
    fn append_thinking_delta_emits_thinking_not_assistant() {
        // ACP maps ThinkingDelta → AgentThoughtChunk; regressing to
        // AssistantDelta would leak reasoning into the message stream.
        let ui = Arc::new(Mutex::new(Vec::new()));
        let mut thinking = Vec::new();
        let mut assistant = 0;
        super::append_thinking_delta(&ui, "why", &mut |ev| match ev {
            ObserveEvent::ThinkingDelta { text } => thinking.push(text.to_string()),
            ObserveEvent::AssistantDelta { .. } => assistant += 1,
            _ => {}
        });
        assert_eq!(thinking, ["why"]);
        assert_eq!(assistant, 0);
    }

    #[test]
    fn append_assistant_delta_skips_empty_chunks() {
        let ui = Arc::new(Mutex::new(Vec::new()));
        let mut n = 0;
        append_assistant_delta(&ui, "", &mut |_| n += 1);
        assert_eq!(n, 0);
        assert!(ui.lock().unwrap().is_empty());
    }

    #[test]
    fn trim_ui_history_drops_oldest_beyond_cap() {
        use crate::chat::UiMessage;
        use super::{MAX_UI_MESSAGES, trim_ui_history};
        let ui = Arc::new(Mutex::new(
            (0..MAX_UI_MESSAGES + 7)
                .map(|i| UiMessage::with_role(UiRole::User, format!("m{i}")))
                .collect::<Vec<_>>(),
        ));
        trim_ui_history(&ui);
        let g = ui.lock().unwrap();
        assert_eq!(g.len(), MAX_UI_MESSAGES);
        assert_eq!(g.first().unwrap().content, "m7"); // oldest dropped
        assert_eq!(g.last().unwrap().content, format!("m{}", MAX_UI_MESSAGES + 6));
    }
}