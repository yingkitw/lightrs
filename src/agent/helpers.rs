use std::sync::{Arc, Mutex};

use crate::chat::{UiMessage, UiRole};

use super::ObserveEvent;

pub(crate) const MAX_TOOL_STEPS: usize = 16;
const TOOL_UI_TRUNCATE: usize = 8_000;

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

pub(crate) fn remove_trailing_empty_assistant(ui_history: &Arc<Mutex<Vec<UiMessage>>>) {
    let mut ui = ui_history.lock().unwrap();
    if let Some(last) = ui.last() {
        if last.role == UiRole::Assistant && last.content.is_empty() {
            ui.pop();
        }
    }
}

pub(crate) fn is_interrupted(err: &anyhow::Error) -> bool {
    err.to_string().contains("turn interrupted")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end: String = s.chars().take(max).collect();
    format!("{end}…")
}