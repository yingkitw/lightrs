use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::agent::{Agent, context_usage_text};
use crate::tui::app::{App, RunState};
use crate::tui::theme::{self, palette};
use crate::tui::info::{header_compact, header_lines};
use crate::skills::skill_hint;
use crate::tui::mentions::mention_hint;
use crate::tui::slash::{is_slash_input, slash_command_hint};

pub fn draw_header(frame: &mut Frame, area: Rect, app: &App, agent: &Agent, rows: u16) {
    let state_style = if app.state == RunState::Busy {
        theme::busy()
    } else {
        theme::ready_accent()
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
            Line::styled(lines[1].clone(), theme::body_muted()),
        ])
    };
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme::border());
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// Returns the inner rect of the input block for cursor placement.
pub fn draw_input(frame: &mut Frame, area: Rect, app: &App) -> Rect {
    let title = if app.pending_approval.is_some() {
        " approve shell — y/n "
    } else if app.state == RunState::Busy {
        " input (busy — read-only) "
    } else if is_slash_input(&app.input) {
        " command "
    } else {
        " input "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if app.state == RunState::Busy {
            theme::border()
        } else {
            Style::default().fg(palette::ACCENT)
        });
    let inner = block.inner(area);

    let content = if let Some(ref ap) = app.pending_approval {
        Line::from(Span::styled(&ap.command, theme::busy()))
    } else if app.input.is_empty() && app.state == RunState::Ready {
        Line::from(vec![Span::styled(
            "Message…  Enter send · @ file · $ skill · Tab · / commands",
            theme::hint(),
        )])
    } else {
        Line::from(app.input.as_str())
    };

    let style = if app.pending_approval.is_some() {
        theme::busy()
    } else if app.state == RunState::Busy {
        theme::body_muted()
    } else {
        theme::body()
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
        theme::busy()
    } else if !app.status.is_empty() {
        theme::error()
    } else {
        theme::hint()
    };
    let hint = if app.state == RunState::Ready {
        if let Some(slash) = slash_command_hint(&app.input) {
            slash
        } else if let Some(mention) = mention_hint(&app.input, app.workdir()) {
            mention
        } else if let Some(skill) = skill_hint(&app.input, &app.skill_catalog) {
            skill
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