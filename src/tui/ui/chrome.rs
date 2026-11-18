use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::agent::{Agent, context_usage_text};
use crate::tui::app::{App, RunState};
use crate::tui::theme;
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
    let ctx = context_usage_text(
        agent.token_summary(),
        agent.api_context_len(),
        crate::config::model_context_window(&app.config.model),
    );
    let text = if rows == 1 {
        Text::from(Line::styled(
            header_compact(&app.config, &ctx, app, area.width),
            state_style,
        ))
    } else {
        let lines = header_lines(&app.config, &ctx, app, area.width);
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
    let busy_input_title = if app.state == RunState::Busy {
        format!(" {} input (read-only) ", app.busy_spinner())
    } else {
        String::new()
    };
    let title: Cow<'_, str> = if app.pending_approval.is_some() {
        Cow::Borrowed(" approve shell — y/n ")
    } else if app.state == RunState::Busy {
        Cow::Owned(busy_input_title)
    } else if is_slash_input(&app.input) {
        Cow::Borrowed(" command ")
    } else {
        Cow::Borrowed(" input ")
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if app.state == RunState::Busy {
            theme::border()
        } else {
            Style::default().fg(theme::active().accent)
        });
    let inner = block.inner(area);

    let input_lines: Vec<Line> = if let Some(ref ap) = app.pending_approval {
        // Inner height fits ~3 command lines; overflow is summarized.
        const MAX_APPROVAL_LINES: usize = 3;
        let mut rows: Vec<Line> = ap
            .commands
            .iter()
            .take(MAX_APPROVAL_LINES)
            .map(|c| Line::from(Span::styled(crate::tools::shell::escape_controls(c), theme::busy())))
            .collect();
        if ap.commands.len() > MAX_APPROVAL_LINES {
            rows.push(Line::from(Span::styled(
                format!("… +{} more", ap.commands.len() - MAX_APPROVAL_LINES),
                theme::busy(),
            )));
        }
        rows
    } else if app.input.is_empty() && app.state == RunState::Ready {
        vec![Line::from(vec![Span::styled(
            "Message…  Enter send · @ file · $ skill · Tab · / commands",
            theme::hint(),
        )])]
    } else if app.input.is_empty() {
        vec![Line::from("")]
    } else {
        app.input
            .split('\n')
            .map(|row| Line::from(row.to_string()))
            .collect()
    };

    let style = if app.pending_approval.is_some() {
        theme::busy()
    } else if app.state == RunState::Busy {
        theme::body_muted()
    } else {
        theme::body()
    };
    let paragraph = Paragraph::new(Text::from(input_lines))
        .style(style)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
    inner
}

pub fn place_input_cursor(frame: &mut Frame, inner: Rect, app: &App) {
    let (row, col) = input_cursor_at_end(&app.input);
    let x = inner
        .x
        .saturating_add((col as u16).min(inner.width.saturating_sub(1)));
    let y = inner
        .y
        .saturating_add((row as u16).min(inner.height.saturating_sub(1)));
    frame.set_cursor_position(Position::new(x, y));
}

fn input_cursor_at_end(input: &str) -> (usize, usize) {
    if input.is_empty() {
        return (0, 0);
    }
    if input.ends_with('\n') {
        let row = input.chars().filter(|c| *c == '\n').count();
        return (row, 0);
    }
    let row = input.matches('\n').count();
    let col = input
        .rsplit_once('\n')
        .map(|(_, tail)| tail.chars().count())
        .unwrap_or_else(|| input.chars().count());
    (row, col)
}

pub fn draw_status(frame: &mut Frame, area: Rect, app: &App, agent: &Agent) {
    let style = if app.state == RunState::Busy {
        theme::busy()
    } else if !app.status.is_empty() {
        theme::error()
    } else {
        theme::hint()
    };
    // Always-on session indicators: context estimate, files changed by
    // write/edit tools this session, and `$skill`s attached to the latest turn.
    let ctx = crate::agent::context_compact_text(
        agent.token_summary(),
        crate::config::model_context_window(&app.config.model),
    );
    let changed: Vec<&String> = agent.changed_files().collect();
    let files = if changed.is_empty() {
        String::new()
    } else {
        let shown: Vec<&str> = changed.iter().take(2).map(|s| s.as_str()).collect();
        let more = changed.len().saturating_sub(2);
        let list = if more > 0 {
            format!("{} +{more}", shown.join(", "))
        } else {
            shown.join(", ")
        };
        format!(" · Δ {list}")
    };
    let skills = if agent.turn_skills().is_empty() {
        String::new()
    } else {
        let ids = agent
            .turn_skills()
            .iter()
            .map(|id| format!("${id}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(" · {ids}")
    };
    let plan = if agent.read_only() { " · plan" } else { "" };
    let hint = if app.state == RunState::Ready {
        // An error/notice set on Ready (failed turn, slash error) outranks
        // the hint chain — the style already flags it, the text must too.
        if !app.status.is_empty() {
            app.status.clone()
        } else if let Some(slash) = slash_command_hint(&app.input, &app.commands) {
            slash
        } else if let Some(mention) = mention_hint(&app.input, app.workdir()) {
            mention
        } else if let Some(skill) = skill_hint(&app.input, &app.skill_catalog) {
            skill
        } else if !app.input.is_empty() {
            "Enter send · Esc clear · Ctrl+U clear".into()
        } else if app.scroll > 0 {
            format!("↑{} scrolled · End latest · /help · q quit", app.scroll)
        } else {
            "↑↓ scroll · click/o tool detail · /help · q quit".into()
        }
    } else {
        format!("{} · Esc or Ctrl+C stop", app.busy_label())
    };
    // `hint` and the Δ-file names can carry model/repo-controlled text —
    // escape controls so nothing reaches the terminal or forges rows.
    let text = crate::tui::info::truncate_end(
        &format!("{ctx}{files}{skills}{plan} · {hint}"),
        usize::from(area.width.max(1)),
    );
    let text = crate::tools::shell::escape_controls(&text);
    frame.render_widget(Paragraph::new(text).style(style), area);
}