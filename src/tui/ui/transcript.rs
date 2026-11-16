use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::tui::app::{App, RunState};
use crate::tui::highlight::{
    collapse_diff_preview, render_file_result_body, render_message_body, render_pre_body,
    render_system_body, render_unified_diff,
};
use crate::tui::theme;

const COLLAPSED_RESULT_CHARS: usize = 480;

pub fn draw_transcript(frame: &mut Frame, area: Rect, agent: &Agent, app: &mut App) {
    let width = area.width.saturating_sub(2).max(1) as usize;
    app.transcript_viewport = area.height.saturating_sub(2).max(1);
    let viewport = usize::from(app.transcript_viewport);

    let messages = agent.ui_messages();
    // Rebuild only when rendered inputs change — markdown/syntect parsing of
    // the whole history on every 16ms busy frame is the expensive part.
    let key = transcript_fingerprint(messages.as_slice(), width, &app.expanded_tools);
    let (mut lines, tool_spans) = match &app.transcript_cache {
        Some((k, l, s)) if *k == key => (l.clone(), s.clone()),
        _ => {
            let (l, s) = transcript_lines(messages.as_slice(), width, app);
            app.transcript_cache = Some((key, l.clone(), s.clone()));
            (l, s)
        }
    };
    app.focus_tool_msg_idx = pick_visible_tool(
        &tool_spans,
        lines.len(),
        viewport,
        usize::from(app.scroll),
    );
    if app.state == RunState::Busy {
        let extra = match (app.busy_elapsed(), app.stream_chars) {
            (Some(s), c) if c > 0 => format!(" · {s}s · {c} chars"),
            (Some(s), _) => format!(" · {s}s"),
            (_, c) if c > 0 => format!(" · {c} chars"),
            _ => String::new(),
        };
        let progress = format!(
            "▏ {} {}{}{}",
            app.busy_spinner(),
            app.busy_phase(),
            app.busy_dots(),
            extra
        );
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
/// Fingerprint of every input `transcript_lines` reads — width, the expanded
/// set, and each message's rendered fields. A collision only costs a stale
/// frame, but full content bytes are hashed so an in-place same-length edit
/// can't slip through.
fn transcript_fingerprint(messages: &[UiMessage], width: usize, expanded: &HashSet<usize>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    width.hash(&mut h);
    // Order-independent: HashSet iteration order isn't a rendered input.
    let exp: u64 = expanded
        .iter()
        .fold(0u64, |acc, i| acc ^ (*i as u64).wrapping_mul(0x9e37_79b9));
    exp.hash(&mut h);
    for m in messages {
        std::mem::discriminant(&m.role).hash(&mut h);
        m.content.hash(&mut h);
        m.pending.hash(&mut h);
        m.elapsed_ms.hash(&mut h);
        if let Some(tc) = &m.tool_call {
            tc.name.hash(&mut h);
            tc.arguments.hash(&mut h);
        }
        if let Some(fd) = &m.file_diff {
            fd.path.hash(&mut h);
            fd.unified.hash(&mut h);
            fd.lines_added.hash(&mut h);
            fd.lines_removed.hash(&mut h);
            fd.is_new_file.hash(&mut h);
        }
    }
    h.finish()
}

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
    let marker = if m.pending {
        "◌"
    } else if expanded {
        "▼"
    } else {
        "▸"
    };
    let mut header = vec![Span::styled(
        format!("{marker} {} ", tc.name),
        theme::tool_header(expanded),
    )];
    // One-line call summary stays in the header expanded or collapsed.
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
    if let Some(ms) = m.elapsed_ms {
        header.push(Span::styled(
            format!(" · {}", crate::chat::fmt_elapsed(ms)),
            Style::default().fg(theme::active().tool_args),
        ));
    }
    header.push(Span::styled(
        if m.pending {
            "  running…"
        } else if expanded {
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
        lines.extend(render_pre_body(
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
            lines.extend(render_pre_body(&m.content, width, result_style(&m.content)));
        }
    } else {
        let body = if expanded {
            m.content.clone()
        } else {
            collapse_result(&m.content)
        };
        lines.push(Line::from(Span::styled(
            "result:",
            result_style(&body).add_modifier(Modifier::ITALIC),
        )));
        lines.extend(render_result_body(tc, &body, width));
    }
    lines.push(Line::from(""));
}

/// `read_file` results are source text — syntax-highlight them with the path
/// extension. Everything else stays literal pre: tool output is data, never
/// markdown (a shell result's `#`/`|`/backticks would mangle under parsing).
/// `error:` bodies keep the flat error style.
fn render_result_body(tc: &crate::chat::ToolCallUi, body: &str, width: usize) -> Vec<Line<'static>> {
    let style = result_style(body);
    if body.trim_start().starts_with("error") {
        return render_pre_body(body, width, style);
    }
    match file_result_lang(tc) {
        Some(lang) => render_file_result_body(body, &lang, width),
        None => render_pre_body(body, width, style),
    }
}

/// Language hint from a file-tool call's `path` argument. The extension goes
/// straight to syntect (token → extension → plain-text fallback inside
/// `highlight_code`), so uncommon extensions still resolve.
fn file_result_lang(tc: &crate::chat::ToolCallUi) -> Option<String> {
    if tc.name != "read_file" {
        return None;
    }
    let args: serde_json::Value = serde_json::from_str(&tc.arguments).ok()?;
    let path = args.get("path")?.as_str()?;
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_string)
}

/// Tool bodies that begin `error:` render in the error color; successes in
/// normal body color (more legible than muting everything).
fn result_style(body: &str) -> Style {
    if body.trim_start().starts_with("error") {
        theme::error()
    } else {
        theme::body()
    }
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
    fn result_style_colors_errors() {
        assert_eq!(result_style("error: denied").fg, Some(theme::active().error));
        assert_eq!(result_style("  error: x").fg, Some(theme::active().error));
        assert_eq!(result_style("wrote 3 files").fg, Some(theme::active().text));
    }

    #[test]
    fn line_hit_maps_to_tool_message() {
        use crate::tui::app::tool_msg_index_at_line;
        let spans = vec![(10, 20, 3), (30, 45, 7)];
        assert_eq!(tool_msg_index_at_line(12, &spans), Some(3));
        assert_eq!(tool_msg_index_at_line(9, &spans), None);
        assert_eq!(tool_msg_index_at_line(30, &spans), Some(7));
    }

    #[test]
    fn fingerprint_stable_for_identical_inputs() {
        let no_exp = HashSet::new();
        let msgs = vec![UiMessage::with_role(UiRole::User, "hello")];
        assert_eq!(
            transcript_fingerprint(&msgs, 80, &no_exp),
            transcript_fingerprint(&msgs, 80, &no_exp)
        );
        // A rebuilt Vec with identical content must hit the cache.
        let msgs2 = vec![UiMessage::with_role(UiRole::User, "hello")];
        assert_eq!(
            transcript_fingerprint(&msgs, 80, &no_exp),
            transcript_fingerprint(&msgs2, 80, &no_exp)
        );
    }

    #[test]
    fn fingerprint_invalidates_on_each_rendered_input() {
        let msgs = || vec![UiMessage::with_role(UiRole::User, "hello")];
        let no_exp = || HashSet::new();
        let base = transcript_fingerprint(&msgs(), 80, &no_exp());
        assert_ne!(base, transcript_fingerprint(&msgs(), 81, &no_exp()), "width");
        let mut grown = msgs();
        grown.push(UiMessage::with_role(UiRole::Assistant, "reply"));
        assert_ne!(base, transcript_fingerprint(&grown, 80, &no_exp()), "append");
        let mut edited = msgs();
        edited[0].content = "hello2".into();
        assert_ne!(base, transcript_fingerprint(&edited, 80, &no_exp()), "edit");
        assert_ne!(
            base,
            transcript_fingerprint(&msgs(), 80, &HashSet::from([3])),
            "expand"
        );
        let mut timed = msgs();
        timed[0].elapsed_ms = Some(42);
        assert_ne!(base, transcript_fingerprint(&timed, 80, &no_exp()), "elapsed");
    }

    #[test]
    fn file_result_lang_reads_path_extension() {
        let tc = crate::chat::ToolCallUi {
            name: "read_file".into(),
            arguments: r#"{"path":"src/main.ts"}"#.into(),
        };
        assert_eq!(file_result_lang(&tc).as_deref(), Some("ts"));
    }

    #[test]
    fn file_result_lang_ignores_non_file_tools() {
        for (name, args) in [
            ("run_command", r#"{"command":"cat a.rs"}"#),
            ("write_file", r#"{"path":"a.rs","content":"x"}"#),
            ("read_file", r#"{"path":"LICENSE"}"#),
            ("read_file", "not json"),
        ] {
            let tc = crate::chat::ToolCallUi {
                name: name.into(),
                arguments: args.into(),
            };
            assert!(file_result_lang(&tc).is_none(), "{name} {args}");
        }
    }

    #[test]
    fn result_body_highlights_read_file_source() {
        let tc = crate::chat::ToolCallUi {
            name: "read_file".into(),
            arguments: r#"{"path":"a.rs"}"#.into(),
        };
        let lines = render_result_body(&tc, "fn main() {\n    let x = 1;\n}", 80);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("    let x"), "indent lost: {joined:?}");
        assert!(
            lines[0].spans.iter().any(|s| s.style.fg.is_some()),
            "expected syntax colors on code result"
        );
    }

    #[test]
    fn result_body_keeps_errors_and_shell_literal() {
        let err = crate::chat::ToolCallUi {
            name: "read_file".into(),
            arguments: r#"{"path":"a.rs"}"#.into(),
        };
        let lines = render_result_body(&err, "error: not found", 80);
        assert_eq!(lines[0].style.fg, Some(theme::active().error));

        let shell = crate::chat::ToolCallUi {
            name: "run_command".into(),
            arguments: r#"{"command":"ls"}"#.into(),
        };
        let lines = render_result_body(&shell, "# not a heading | literal", 80);
        assert_eq!(lines.len(), 1, "shell output must not reflow as markdown");
    }
}