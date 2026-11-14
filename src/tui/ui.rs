use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::tui::app::{App, RunState};

pub fn draw(frame: &mut Frame, app: &mut App, agent: &Agent) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    draw_transcript(frame, chunks[0], agent, app);
    draw_input(frame, chunks[1], app);
    draw_status(frame, chunks[2], app);
}

fn draw_transcript(frame: &mut Frame, area: Rect, agent: &Agent, app: &mut App) {
    let width = area.width.saturating_sub(2).max(1) as usize;
    app.transcript_viewport = area.height.saturating_sub(2).max(1);
    let viewport = usize::from(app.transcript_viewport);

    let lines = transcript_lines(agent.ui_messages(), width);
    let total = lines.len();
    let max_scroll = total.saturating_sub(viewport).min(u16::MAX as usize) as u16;
    app.clamp_scroll(max_scroll);

    let scroll = usize::from(app.scroll);
    let start = total.saturating_sub(viewport + scroll);
    let end = (start + viewport).min(total);
    let visible = lines[start..end].to_vec();

    let mut title = " lightrs ".to_string();
    if app.scroll > 0 {
        title.push_str(&format!(" ↑{}", app.scroll));
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
        lines.push(Line::from(vec![Span::styled(
            format!("[{label}]"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )]));
        for chunk in wrap_text(&m.content, width) {
            lines.push(Line::from(chunk));
        }
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

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut rest = paragraph.to_string();
        if rest.is_empty() {
            out.push(String::new());
            continue;
        }
        while !rest.is_empty() {
            let char_count = rest.chars().count();
            if char_count <= width {
                out.push(rest);
                break;
            }
            let chunk: String = rest.chars().take(width).collect();
            out.push(chunk);
            rest = rest.chars().skip(width).collect();
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let title = if app.state == RunState::Busy {
        " input (busy) "
    } else {
        " input "
    };
    let paragraph = Paragraph::new(app.input.as_str())
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let style = if app.state == RunState::Busy {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let hint = if app.state == RunState::Ready {
        format!("{} · ↑/↓ scroll · PgUp/PgDn", app.status)
    } else {
        app.status.clone()
    };
    let paragraph = Paragraph::new(hint).style(style);
    frame.render_widget(paragraph, area);
}