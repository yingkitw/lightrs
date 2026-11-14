use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::tui::app::{App, RunState};
use crate::tui::highlight::render_message_body;

pub fn draw_transcript(frame: &mut Frame, area: Rect, agent: &Agent, app: &mut App) {
    let width = area.width.saturating_sub(2).max(1) as usize;
    app.transcript_viewport = area.height.saturating_sub(2).max(1);
    let viewport = usize::from(app.transcript_viewport);

    let mut lines = transcript_lines(agent.ui_messages(), width);
    if app.state == RunState::Busy {
        lines.push(Line::from(Span::styled(
            "● waiting for model…",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC),
        )));
    }

    let total = lines.len();
    let max_scroll = total.saturating_sub(viewport).min(u16::MAX as usize) as u16;
    app.clamp_scroll(max_scroll);

    let scroll = usize::from(app.scroll);
    let start = total.saturating_sub(viewport + scroll);
    let end = (start + viewport).min(total);
    let visible = lines[start..end].to_vec();

    let mut title = " chat ".to_string();
    if app.scroll > 0 {
        title.push_str(&format!(" ↑{} ", app.scroll));
    }

    let block = Block::default().title(title).borders(Borders::ALL);
    let paragraph = Paragraph::new(Text::from(visible))
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn transcript_lines(messages: &[UiMessage], width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for m in messages {
        let (label, color) = role_style(m.role);
        let label_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let body_style = match m.role {
            UiRole::System | UiRole::Tool => Style::default().fg(Color::DarkGray),
            UiRole::Error => Style::default().fg(Color::Red),
            _ => Style::default(),
        };
        lines.push(Line::from(vec![Span::styled(format!("[{label}]"), label_style)]));
        lines.extend(render_message_body(&m.content, width, body_style));
        lines.push(Line::from(""));
    }
    lines
}

fn role_style(role: UiRole) -> (&'static str, Color) {
    match role {
        UiRole::User => ("you", Color::Cyan),
        UiRole::Assistant => ("agent", Color::Green),
        UiRole::Tool => ("tool", Color::Yellow),
        UiRole::System => ("sys", Color::DarkGray),
        UiRole::Error => ("err", Color::Red),
    }
}