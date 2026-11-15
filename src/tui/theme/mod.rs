//! TUI color themes; switch at runtime with `/theme <name>` (saved to `~/.config/lightrs/tui.toml`).

mod palettes;
mod persist;

use std::sync::RwLock;

use ratatui::style::{Modifier, Style};

use crate::chat::UiRole;

pub use palettes::{Palette, THEMES};

static ACTIVE_ID: RwLock<String> = RwLock::new(String::new());

pub fn init() {
    let id = persist::load_theme_id();
    if let Ok(mut guard) = ACTIVE_ID.write() {
        *guard = id;
    }
}

pub fn active_id() -> String {
    ensure_init();
    ACTIVE_ID.read().unwrap().clone()
}

pub fn active() -> &'static Palette {
    ensure_init();
    let id = ACTIVE_ID.read().unwrap();
    palettes::by_id(&id).unwrap_or(&palettes::MOCHA)
}

fn ensure_init() {
    if ACTIVE_ID.read().map(|g| g.is_empty()).unwrap_or(true) {
        init();
    }
}

pub fn set_theme(id: &str) -> Result<String, String> {
    let key = id.trim().to_lowercase();
    let entry = THEMES
        .iter()
        .find(|t| t.id == key)
        .ok_or_else(|| format!("unknown theme `{id}` — try /theme"))?;
    if let Ok(mut guard) = ACTIVE_ID.write() {
        *guard = entry.id.to_string();
    }
    persist::save_theme_id(entry.id).map_err(|e| format!("{e:#}"))?;
    Ok(entry.id.to_string())
}

pub fn theme_list_message() -> String {
    let current = active_id();
    let mut lines: Vec<String> = THEMES
        .iter()
        .map(|t| {
            let mark = if t.id == current { " ← current" } else { "" };
            format!("  {} — {}{mark}", t.id, t.label)
        })
        .collect();
    lines.push(String::new());
    lines.push("  /theme <id>   apply and save".into());
    format!("TUI color themes:\n{}", lines.join("\n"))
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
    };
    let color = match role {
        UiRole::User => active().user,
        UiRole::Assistant => active().assistant,
        UiRole::Tool => active().tool,
        UiRole::System => active().system,
        UiRole::Error => active().error,
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

/// Back-compat alias for code that addressed `palette::DIFF_ADD` etc.
pub mod palette {
    use super::active;
    use ratatui::style::Color;

    pub fn text() -> Color {
        active().text
    }
    pub fn border() -> Color {
        active().border
    }
    pub fn accent() -> Color {
        active().accent
    }
    pub fn tool() -> Color {
        active().tool
    }
    pub fn tool_args() -> Color {
        active().tool_args
    }
    pub fn link() -> Color {
        active().link
    }
    pub fn code_bg() -> Color {
        active().code_bg
    }
    pub fn heading_1() -> Color {
        active().heading_1
    }
    pub fn heading_2() -> Color {
        active().heading_2
    }
    pub fn heading_3() -> Color {
        active().heading_3
    }
    pub fn heading_other() -> Color {
        active().heading_other
    }
    pub fn mermaid_accent() -> Color {
        active().mermaid_accent
    }
    pub fn mermaid_edge() -> Color {
        active().mermaid_edge
    }
    pub fn diff_add() -> Color {
        active().diff_add
    }
    pub fn diff_del() -> Color {
        active().diff_del
    }
    pub fn diff_hunk() -> Color {
        active().diff_hunk
    }
}