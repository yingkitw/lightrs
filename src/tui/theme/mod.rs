//! TUI palettes — `mocha` (default) and `hc`. `/theme` + `config.toml` `[tui]`.

mod palettes;

use std::cell::Cell;

use ratatui::style::{Modifier, Style};

use crate::chat::UiRole;
use crate::config;

pub use palettes::{Palette, THEMES};

thread_local! {
    static CURRENT: Cell<&'static str> = const { Cell::new("mocha") };
}

/// Call once per frame from `draw()` — ponytail: no `RwLock` on a single-threaded TUI.
pub fn sync(theme_id: &str) {
    let key = theme_id.trim().to_lowercase();
    let id = THEMES
        .iter()
        .find(|t| t.id == key.as_str())
        .map(|t| t.id)
        .unwrap_or("mocha");
    CURRENT.with(|c| c.set(id));
}

pub fn active() -> &'static Palette {
    let id = CURRENT.with(|c| c.get());
    palettes::by_id(id).unwrap_or(&palettes::MOCHA)
}

pub fn normalize_theme_id(raw: Option<String>) -> String {
    raw.filter(|s| palettes::by_id(s).is_some())
        .unwrap_or_else(|| "mocha".into())
}

pub fn apply(theme_id: &str, app: &mut crate::tui::App) -> Result<String, String> {
    let key = theme_id.trim().to_lowercase();
    let entry = THEMES
        .iter()
        .find(|t| t.id == key)
        .ok_or_else(|| format!("unknown theme `{theme_id}` — /theme lists: mocha, hc"))?;
    app.theme = entry.id.to_string();
    sync(&app.theme);
    config::save_tui_theme(entry.id).map_err(|e| format!("{e:#}"))?;
    Ok(entry.id.to_string())
}

pub fn list_message(current: &str) -> String {
    let lines: Vec<String> = THEMES
        .iter()
        .map(|t| {
            let mark = if t.id == current {
                " ← current"
            } else {
                ""
            };
            format!("  {} — {}{mark}", t.id, t.label)
        })
        .collect();
    format!(
        "TUI themes:\n{}\n\n  /theme <id>   (saved under [tui] in config.toml)",
        lines.join("\n")
    )
}

pub fn body() -> Style {
    Style::default().fg(active().text)
}

pub fn body_muted() -> Style {
    Style::default().fg(active().muted)
}

pub fn hint() -> Style {
    Style::default().fg(active().hint)
}

pub fn border() -> Style {
    Style::default().fg(active().border)
}

pub fn error() -> Style {
    Style::default().fg(active().error)
}

pub fn busy() -> Style {
    Style::default().fg(active().busy)
}

pub fn ready_accent() -> Style {
    Style::default().fg(active().accent)
}

pub fn system_tag() -> Style {
    Style::default()
        .fg(active().system)
        .add_modifier(Modifier::BOLD)
}

pub fn system_metric() -> Style {
    Style::default()
        .fg(active().accent)
        .add_modifier(Modifier::BOLD)
}

pub fn role_tag(role: UiRole) -> (&'static str, Style) {
    let label = match role {
        UiRole::User => "you",
        UiRole::Assistant => "agent",
        UiRole::Tool => "tool",
        UiRole::System => "sys",
        UiRole::Error => "err",
        UiRole::Thinking => "think",
    };
    let color = match role {
        UiRole::User => active().user,
        UiRole::Assistant => active().assistant,
        UiRole::Tool => active().tool,
        UiRole::System => active().system,
        UiRole::Error => active().error,
        UiRole::Thinking => active().muted,
    };
    (
        label,
        Style::default()
            .fg(color)
            .add_modifier(Modifier::BOLD),
    )
}

pub fn tool_header(expanded: bool) -> Style {
    Style::default()
        .fg(active().tool)
        .add_modifier(if expanded {
            Modifier::BOLD | Modifier::UNDERLINED
        } else {
            Modifier::BOLD
        })
}

pub fn code_block() -> Style {
    Style::default()
        .fg(active().code_fg)
        .bg(active().code_bg)
}

pub fn inline_code() -> Style {
    Style::default()
        .fg(active().inline_code_fg)
        .bg(active().inline_code_bg)
}

pub fn fence_header() -> Style {
    Style::default().fg(active().hint)
}

pub fn link() -> Style {
    Style::default()
        .fg(active().link)
        .add_modifier(Modifier::UNDERLINED)
}