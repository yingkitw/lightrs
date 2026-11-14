use ratatui::style::Style;
use ratatui::text::{Line, Span};

pub(crate) fn wrap_plain_lines(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::styled(text.to_string(), style)];
    }
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut rest = paragraph.to_string();
        if rest.is_empty() {
            out.push(Line::from(""));
            continue;
        }
        while !rest.is_empty() {
            let char_count = rest.chars().count();
            if char_count <= width {
                out.push(Line::styled(rest, style));
                break;
            }
            let chunk: String = rest.chars().take(width).collect();
            out.push(Line::styled(chunk, style));
            rest = rest.chars().skip(width).collect();
        }
    }
    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

pub(crate) fn wrap_styled_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return lines;
    }
    let mut out = Vec::new();
    for line in lines {
        out.extend(wrap_spans_line(line, width));
    }
    out
}

fn wrap_spans_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;

    let flush = |rows: &mut Vec<Line<'static>>, current: &mut Vec<Span<'static>>| {
        if !current.is_empty() {
            rows.push(Line::from(std::mem::take(current)));
        }
    };

    for span in line.spans {
        let style = span.style;
        let mut rest: String = span.content.into_owned();
        while !rest.is_empty() {
            let space = width.saturating_sub(col);
            if space == 0 {
                flush(&mut rows, &mut current);
                col = 0;
                continue;
            }
            let take = space.min(rest.chars().count());
            let chunk: String = rest.chars().take(take).collect();
            let remainder: String = rest.chars().skip(take).collect();
            current.push(Span::styled(chunk, style));
            col += take;
            if col >= width {
                flush(&mut rows, &mut current);
                col = 0;
            }
            rest = remainder;
        }
    }
    flush(&mut rows, &mut current);
    if rows.is_empty() {
        rows.push(Line::from(""));
    }
    rows
}