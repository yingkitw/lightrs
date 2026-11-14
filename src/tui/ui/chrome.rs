use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::agent::{Agent, context_usage_text};
use crate::tui::app::{App, RunState};
use crate::tui::info::{header_compact, header_lines};
use crate::tui::slash::{is_slash_input, slash_command_hint};

pub fn draw_header(frame: &mut Frame, area: Rect, app: &App, agent: &Agent, rows: u16) {
    let state_style = if app.state == RunState::Busy {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let ctx = context_usage_text(agent.token_summary(), agent.api_context_len());
    let text = if rows == 1 {
        Text::from(Line::styled(
            header_compact(&app.config, &ctx, app.state, area.width),
            state_style,
        ))
    } else {
        let lines = header_lines(&app.config, &ctx, app.state, area.width);
        Text::from(vec![
            Line::styled(lines[0].clone(), state_style),
            Line::styled(lines[1].clone(), Style::default().fg(Color::DarkGray)),
        ])
    };
    let block = Block::default().borders(Borders::BOTTOM);
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// Returns the inner rect of the input block for cursor placement.
pub fn draw_input(frame: &mut Frame, area: Rect, app: &App) -> Rect {
    let title = if app.state == RunState::Busy {
        " input (busy — read-only) "
    } else if is_slash_input(&app.input) {
        " command "
    } else {
        " input "
    };
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);

    let content = if app.input.is_empty() && app.state == RunState::Ready {
        Line::from(vec![Span::styled(
            "Message…  Enter send · Shift+Enter newline · Ctrl+P history · / commands",
            Style::default().fg(Color::DarkGray),
        )])
    } else {
        Line::from(app.input.as_str())
    };

    let style = if app.state == RunState::Busy {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };
    let paragraph = Paragraph::new(Text::from(vec![content]))
        .style(style)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
    inner
}

pub fn place_input_cursor(frame: &mut Frame, inner: Rect, app: &App) {
    let col = u16::try_from(app.input.chars().count()).unwrap_or(u16::MAX);
    let x = inner.x.saturating_add(col.min(inner.width.saturating_sub(1)));
    let y = inner.y;
    frame.set_cursor_position(Position::new(x, y));
}

pub fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let style = if app.state == RunState::Busy {
        Style::default().fg(Color::Yellow)
    } else if !app.status.is_empty() {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let hint = if app.state == RunState::Ready {
        if let Some(slash) = slash_command_hint(&app.input) {
            slash
        } else if !app.input.is_empty() {
            "Enter send · Esc clear · Ctrl+U clear".into()
        } else {
            "↑↓ scroll · click/o tool detail · /help · q quit".into()
        }
    } else {
        let phase = if app.status.is_empty() {
            "thinking"
        } else {
            app.status.as_str()
        };
        format!("{} {phase} · Esc or Ctrl+C stop", app.busy_spinner())
    };
    frame.render_widget(Paragraph::new(hint).style(style), area);
}