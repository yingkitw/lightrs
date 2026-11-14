//! Dark, low-glare palette tuned for long coding sessions (Catppuccin Mocha–inspired).

use ratatui::style::{Modifier, Style};

use crate::chat::UiRole;

pub mod palette {
    use ratatui::style::Color;

    pub const TEXT: Color = Color::Rgb(205, 214, 244);
    pub const MUTED: Color = Color::Rgb(127, 132, 156);
    pub const HINT: Color = Color::Rgb(88, 91, 112);
    pub const BORDER: Color = Color::Rgb(69, 71, 90);

    pub const USER: Color = Color::Rgb(137, 220, 235);
    pub const ASSISTANT: Color = Color::Rgb(166, 227, 161);
    pub const TOOL: Color = Color::Rgb(250, 179, 135);
    pub const SYSTEM: Color = Color::Rgb(147, 153, 178);
    pub const ERROR: Color = Color::Rgb(243, 139, 168);

    pub const ACCENT: Color = Color::Rgb(148, 226, 213);
    pub const BUSY: Color = Color::Rgb(249, 226, 175);
    pub const LINK: Color = Color::Rgb(137, 180, 250);

    pub const CODE_FG: Color = Color::Rgb(186, 199, 218);
    pub const CODE_BG: Color = Color::Rgb(30, 30, 46);
    pub const INLINE_CODE_FG: Color = Color::Rgb(245, 194, 231);
    pub const INLINE_CODE_BG: Color = Color::Rgb(49, 50, 68);

    pub const HEADING_1: Color = Color::Rgb(148, 226, 213);
    pub const HEADING_2: Color = Color::Rgb(166, 227, 161);
    pub const HEADING_3: Color = Color::Rgb(203, 166, 247);
    pub const HEADING_OTHER: Color = Color::Rgb(250, 179, 135);

    pub const MERMAID_ACCENT: Color = Color::Rgb(203, 166, 247);
    pub const MERMAID_EDGE: Color = Color::Rgb(166, 199, 236);
    pub const TOOL_ARGS: Color = Color::Rgb(186, 194, 216);
}

pub fn body() -> Style {
    Style::default().fg(palette::TEXT)
}

pub fn body_muted() -> Style {
    Style::default().fg(palette::MUTED)
}

pub fn hint() -> Style {
    Style::default().fg(palette::HINT)
}

pub fn border() -> Style {
    Style::default().fg(palette::BORDER)
}

pub fn error() -> Style {
    Style::default().fg(palette::ERROR)
}

pub fn busy() -> Style {
    Style::default().fg(palette::BUSY)
}

pub fn ready_accent() -> Style {
    Style::default().fg(palette::ACCENT)
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
        UiRole::User => palette::USER,
        UiRole::Assistant => palette::ASSISTANT,
        UiRole::Tool => palette::TOOL,
        UiRole::System => palette::SYSTEM,
        UiRole::Error => palette::ERROR,
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
        .fg(palette::TOOL)
        .add_modifier(if expanded {
            Modifier::BOLD | Modifier::UNDERLINED
        } else {
            Modifier::BOLD
        })
}

pub fn code_block() -> Style {
    Style::default().fg(palette::CODE_FG).bg(palette::CODE_BG)
}

pub fn inline_code() -> Style {
    Style::default()
        .fg(palette::INLINE_CODE_FG)
        .bg(palette::INLINE_CODE_BG)
}

pub fn fence_header() -> Style {
    Style::default().fg(palette::HINT)
}

pub fn link() -> Style {
    Style::default()
        .fg(palette::LINK)
        .add_modifier(Modifier::UNDERLINED)
}