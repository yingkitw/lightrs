use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::tui::app::{App, RunState};
use crate::tui::highlight::render_message_body;

const COLLAPSED_RESULT_CHARS: usize = 480;

pub fn draw_transcript(frame: &mut Frame, area: Rect, agent: &Agent, app: &mut App) {
    let width = area.width.saturating_sub(2).max(1) as usize;
    app.transcript_viewport = area.height.saturating_sub(2).max(1);
    let viewport = usize::from(app.transcript_viewport);

    let messages = agent.ui_messages();
    let (mut lines, tool_spans) = transcript_lines(messages.as_slice(), width, app);
    app.focus_tool_msg_idx = pick_visible_tool(
        &tool_spans,
        lines.len(),
        viewport,
        usize::from(app.scroll),
    );
    if app.state == RunState::Busy {
        let tail = messages
            .last()
            .filter(|m| m.role == UiRole::Assistant && !m.content.is_empty());
        if tail.is_none() {
            lines.push(Line::from(Span::styled(
                "● waiting for model…",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
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

/// `(first_line, last_line_exclusive, message_index)` for each tool block.
fn transcript_lines(
    messages: &[UiMessage],
    width: usize,
    app: &App,
) -> (Vec<Line<'static>>, Vec<(usize, usize, usize)>) {
    let mut lines = Vec::new();
    let mut tool_spans = Vec::new();
    for (idx, m) in messages.iter().enumerate() {
        if m.is_tool_block() {
            let tc = m.tool_call.as_ref().expect("tool_call when is_tool_block");
            let start = lines.len();
            render_tool_block(&mut lines, idx, m, tc, width, app);
            tool_spans.push((start, lines.len(), idx));
            continue;
        }
        if m.role == UiRole::Tool && m.content.starts_with("→") {
            lines.push(Line::from(Span::styled(
                m.content.clone(),
                Style::default().fg(Color::Yellow),
            )));
            lines.push(Line::from(""));
            continue;
        }
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
    (lines, tool_spans)
}

fn pick_visible_tool(
    spans: &[(usize, usize, usize)],
    total_lines: usize,
    viewport: usize,
    scroll: usize,
) -> Option<usize> {
    if spans.is_empty() {
        return None;
    }
    let start = total_lines.saturating_sub(viewport + scroll);
    let end = (start + viewport).min(total_lines);
    spans
        .iter()
        .filter(|(a, b, _)| *a < end && *b > start)
        .max_by_key(|(a, _, _)| a)
        .map(|(_, _, idx)| *idx)
}

fn render_tool_block(
    lines: &mut Vec<Line<'static>>,
    idx: usize,
    m: &UiMessage,
    tc: &crate::chat::ToolCallUi,
    width: usize,
    app: &App,
) {
    let expanded = app.expanded_tools.contains(&idx);
    let marker = if expanded { "▼" } else { "▸" };
    let mut header = vec![Span::styled(
        format!("{marker} {} ", tc.name),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    if !expanded {
        header.push(Span::styled(
            truncate_inline(&tc.arguments, 72),
            Style::default().fg(Color::DarkGray),
        ));
    }
    header.push(Span::styled(
        if expanded { "  (o collapse)" } else { "  (o expand)" },
        Style::default().fg(Color::Rgb(80, 80, 80)),
    ));
    lines.push(Line::from(header));
    if expanded {
        lines.push(Line::from(Span::styled(
            "arguments:",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        lines.extend(render_message_body(
            &format_args_detail(&tc.arguments),
            width,
            Style::default().fg(Color::Rgb(200, 200, 200)),
        ));
    }
    let body = if expanded {
        m.content.clone()
    } else {
        collapse_result(&m.content)
    };
    lines.push(Line::from(Span::styled(
        "result:",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.extend(render_message_body(
        &body,
        width,
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::from(""));
}

fn collapse_result(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= COLLAPSED_RESULT_CHARS {
        return t.to_string();
    }
    let head: String = t.chars().take(COLLAPSED_RESULT_CHARS).collect();
    format!("{head}…")
}

fn truncate_inline(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let head: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn format_args_detail(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| trimmed.to_string())
    } else {
        trimmed.to_string()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::UiMessage;

    #[test]
    fn collapses_long_tool_output() {
        let long = "x".repeat(600);
        let c = collapse_result(&long);
        assert!(c.chars().count() < 600);
        assert!(c.ends_with('…'));
    }

    #[test]
    fn pretty_prints_json_args() {
        let s = format_args_detail(r#"{"path":"a.rs"}"#);
        assert!(s.contains("path"));
    }

    #[test]
    fn tool_block_detection() {
        let m = UiMessage::tool_result("read_file", "{}", "ok");
        assert!(m.is_tool_block());
    }
}