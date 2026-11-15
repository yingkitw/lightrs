use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::tui::app::{App, RunState};
use crate::tui::highlight::{
    collapse_diff_preview, render_message_body, render_system_body, render_unified_diff,
};
use crate::tui::theme;

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
        let streaming = messages
            .last()
            .is_some_and(|m| m.role == UiRole::Assistant && !m.content.is_empty());
        let extra = match (app.busy_elapsed(), app.stream_chars) {
            (Some(s), c) if c > 0 => format!(" · {s}s · {c} chars"),
            (Some(s), _) => format!(" · {s}s"),
            (_, c) if c > 0 => format!(" · {c} chars"),
            _ => String::new(),
        };
        let progress = if streaming {
            format!(
                "▏ {} {}{}{}",
                app.busy_spinner(),
                app.busy_phase(),
                app.busy_dots(),
                extra
            )
        } else {
            format!(
                "▏ {} {}{}{}",
                app.busy_spinner(),
                app.busy_phase(),
                app.busy_dots(),
                extra
            )
        };
        lines.push(Line::from(Span::styled(
            progress,
            theme::busy().add_modifier(Modifier::ITALIC),
        )));
    }

    let total = lines.len();
    let max_scroll = total.saturating_sub(viewport).min(u16::MAX as usize) as u16;
    app.clamp_scroll(max_scroll);

    let scroll = usize::from(app.scroll);
    let start = total.saturating_sub(viewport + scroll);
    let end = (start + viewport).min(total);
    let visible = lines[start..end].to_vec();

    app.transcript_rect = area;
    app.transcript_visible_start = start;
    app.tool_hit_spans = tool_spans;

    let mut title = if app.state == RunState::Busy {
        format!(" {} chat ", app.busy_spinner())
    } else {
        " chat ".to_string()
    };
    if app.scroll > 0 {
        title.push_str(&format!("↑{} ", app.scroll));
    }

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme::border());
    let paragraph = Paragraph::new(Text::from(visible)).block(block);
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
                Style::default().fg(theme::active().tool),
            )));
            lines.push(Line::from(""));
            continue;
        }
        if m.role == UiRole::System {
            lines.extend(render_system_body(&m.content, width));
            lines.push(Line::from(""));
            continue;
        }
        let (label, label_style) = theme::role_tag(m.role);
        let body_style = match m.role {
            UiRole::Tool => theme::body_muted(),
            UiRole::Error => theme::error(),
            _ => theme::body(),
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
        theme::tool_header(expanded),
    )];
    if !expanded {
        if let Some(fd) = &m.file_diff {
            header.push(Span::styled(
                format!(
                    "{} +{} −{}",
                    fd.path, fd.lines_added, fd.lines_removed
                ),
                Style::default().fg(theme::active().tool_args),
            ));
        } else {
            header.push(Span::styled(
                truncate_inline(&tc.arguments, 72),
                Style::default().fg(theme::active().tool_args),
            ));
        }
    }
    header.push(Span::styled(
        if expanded {
            "  (click or o collapse)"
        } else {
            "  (click or o expand)"
        },
        theme::hint(),
    ));
    lines.push(Line::from(header));
    if expanded {
        lines.push(Line::from(Span::styled(
            "arguments:",
            theme::body_muted().add_modifier(Modifier::ITALIC),
        )));
        lines.extend(render_message_body(
            &format_args_detail(&tc.arguments),
            width,
            theme::body(),
        ));
    }
    if let Some(fd) = &m.file_diff {
        let label = if fd.is_new_file {
            format!("diff: {} (new file)", fd.path)
        } else {
            format!(
                "diff: {} (+{} −{})",
                fd.path, fd.lines_added, fd.lines_removed
            )
        };
        lines.push(Line::from(Span::styled(
            label,
            theme::body_muted().add_modifier(Modifier::ITALIC),
        )));
        let diff_text = if expanded {
            fd.unified.as_str()
        } else {
            &collapse_diff_preview(&fd.unified)
        };
        lines.extend(render_unified_diff(diff_text, width));
        if expanded {
            lines.push(Line::from(Span::styled(
                "summary:",
                theme::body_muted().add_modifier(Modifier::ITALIC),
            )));
            lines.extend(render_message_body(&m.content, width, theme::body_muted()));
        }
    } else {
        let body = if expanded {
            m.content.clone()
        } else {
            collapse_result(&m.content)
        };
        lines.push(Line::from(Span::styled(
            "result:",
            theme::body_muted().add_modifier(Modifier::ITALIC),
        )));
        lines.extend(render_message_body(&body, width, theme::body_muted()));
    }
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
        let m = UiMessage::tool_result("read_file", "{}", "ok", None);
        assert!(m.is_tool_block());
    }

    #[test]
    fn line_hit_maps_to_tool_message() {
        use crate::tui::app::tool_msg_index_at_line;
        let spans = vec![(10, 20, 3), (30, 45, 7)];
        assert_eq!(tool_msg_index_at_line(12, &spans), Some(3));
        assert_eq!(tool_msg_index_at_line(9, &spans), None);
        assert_eq!(tool_msg_index_at_line(30, &spans), Some(7));
    }
}