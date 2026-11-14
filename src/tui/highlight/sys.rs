use ratatui::text::{Line, Span};

use super::wrap::wrap_plain_lines;
use crate::tui::theme;

/// Transcript layout for `[sys]` / `UiRole::System` messages.
pub fn render_system_body(content: &str, width: usize) -> Vec<Line<'static>> {
    let text = content.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let usable = width.saturating_sub(2).max(1);
    if !text.contains('\n') {
        return vec![render_compact_sys(text, usable)];
    }
    render_block_sys(text, usable)
}

fn bar() -> Span<'static> {
    Span::styled("▏ ", theme::border())
}

fn tag_sys() -> Span<'static> {
    Span::styled("sys ", theme::system_tag())
}

fn render_compact_sys(text: &str, width: usize) -> Line<'static> {
    let mut head = vec![bar(), tag_sys()];
    head.extend(compact_content_spans(text));
    let line = Line::from(head);
    wrap_sys_line(line.clone(), width)
        .into_iter()
        .next()
        .unwrap_or(line)
}

fn compact_content_spans(text: &str) -> Vec<Span<'static>> {
    if let Some(rest) = text.strip_prefix("send · ") {
        return vec![
            Span::styled("send ", theme::body_muted()),
            Span::styled("· ", theme::hint()),
            metric_span(rest),
        ];
    }
    if let Some(rest) = text.strip_prefix("model · ") {
        return parse_model_usage(rest);
    }
    if text.starts_with("cwd: ") {
        return vec![
            Span::styled("cwd ", theme::body_muted()),
            Span::styled(
                text.strip_prefix("cwd: ").unwrap_or(text).to_string(),
                theme::link(),
            ),
        ];
    }
    if text.contains("per-step token lines:") {
        return vec![Span::styled(text.to_string(), theme::body_muted())];
    }
    if text == "Turn interrupted." {
        return vec![Span::styled(text.to_string(), theme::busy())];
    }
    vec![Span::styled(text.to_string(), theme::body_muted())]
}

fn metric_span(rest: &str) -> Span<'static> {
    Span::styled(rest.to_string(), theme::system_metric())
}

fn parse_model_usage(rest: &str) -> Vec<Span<'static>> {
    // `format_token in + format out` or legacy shapes
    if let Some((in_part, out_part)) = rest.split_once(" in + ") {
        let out_part = out_part.strip_suffix(" out").unwrap_or(out_part);
        return vec![
            Span::styled("model ", theme::body_muted()),
            Span::styled(in_part.to_string(), theme::system_metric()),
            Span::styled(" in · ", theme::hint()),
            Span::styled(out_part.to_string(), theme::system_metric()),
            Span::styled(" out", theme::body_muted()),
        ];
    }
    vec![Span::styled(format!("model · {rest}"), theme::body_muted())]
}

fn render_block_sys(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    out.push(Line::from(vec![bar(), tag_sys()]));

    let body_width = width.saturating_sub(2).max(1);
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            out.push(Line::from(vec![bar()]));
            continue;
        }
        let spans = highlight_sys_line(trimmed);
        let wrapped = wrap_spans_to_width(&spans, body_width);
        for (i, wspans) in wrapped.into_iter().enumerate() {
            let mut row = vec![bar()];
            if i > 0 {
                row.push(Span::raw("  "));
            }
            row.extend(wspans);
            out.push(Line::from(row));
        }
    }
    out
}

fn highlight_sys_line(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = line;
    let base = theme::body_muted();
    while let Some(idx) = rest.find('/') {
        if idx > 0 {
            spans.push(Span::styled(rest[..idx].to_string(), base));
        }
        rest = &rest[idx..];
        let end = rest[1..]
            .find(|c: char| c.is_whitespace() || c == '—' || c == ',')
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        spans.push(Span::styled(rest[..end].to_string(), theme::link()));
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), base));
    }
    if spans.is_empty() {
        spans.push(Span::styled(line.to_string(), base));
    }
    spans
}

fn wrap_sys_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line];
    }
    let width = width.max(8);
    let char_count: usize = line.spans.iter().map(|s| s.width()).sum();
    if char_count <= width {
        return vec![line];
    }
    // Fallback: plain wrap entire line as one style
    let plain: String = line
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    wrap_plain_lines(&plain, width, theme::body_muted())
        .into_iter()
        .map(|l| {
            let mut spans = vec![bar(), tag_sys()];
            spans.extend(l.spans);
            Line::from(spans)
        })
        .collect()
}

fn wrap_spans_to_width(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    if width == 0 {
        return vec![spans.to_vec()];
    }
    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;

    for span in spans {
        for word in split_span_words(span) {
            let w = word.content.chars().count();
            if col > 0 && col + w > width {
                rows.push(std::mem::take(&mut current));
                col = 0;
            }
            col += w;
            current.push(word);
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

fn split_span_words(span: &Span<'static>) -> Vec<Span<'static>> {
    let style = span.style;
    let mut out = Vec::new();
    for piece in span.content.split_whitespace() {
        out.push(Span::styled(format!("{piece} "), style));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_send_line() {
        let lines = render_system_body("send · ~1.2k ctx", 40);
        let joined: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("send"));
        assert!(joined.contains("1.2k"));
    }

    #[test]
    fn block_has_sys_bar() {
        let lines = render_system_body("Slash commands:\n  /help — hi", 48);
        assert!(lines.len() >= 2);
        let first: String = lines[0].spans[0].content.as_ref().into();
        assert_eq!(first, "▏ ");
    }
}