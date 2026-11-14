use ratatui::style::Style;
use ratatui::text::{Line, Span};

pub(crate) fn wrap_plain_lines(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::styled(text.to_string(), style)];
    }
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(Line::from(""));
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            push_token(&mut line, word, width, style, &mut out);
        }
        if !line.is_empty() {
            out.push(Line::styled(line, style));
        }
    }
    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

fn push_token(
    line: &mut String,
    token: &str,
    width: usize,
    style: Style,
    out: &mut Vec<Line<'static>>,
) {
    let token_len = token.chars().count();
    if token_len > width {
        if !line.is_empty() {
            out.push(Line::styled(std::mem::take(line), style));
        }
        push_long_token(token, width, style, out, line);
        return;
    }
    let need_sep = !line.is_empty();
    let next_len = line.chars().count() + if need_sep { 1 } else { 0 } + token_len;
    if need_sep && next_len > width {
        out.push(Line::styled(std::mem::take(line), style));
    }
    if !line.is_empty() {
        line.push(' ');
    }
    line.push_str(token);
}

fn push_long_token(
    token: &str,
    width: usize,
    style: Style,
    out: &mut Vec<Line<'static>>,
    pending: &mut String,
) {
    if !pending.is_empty() {
        out.push(Line::styled(std::mem::take(pending), style));
    }
    let mut chars = token.chars();
    while chars.clone().count() > 0 {
        let chunk: String = chars.by_ref().take(width).collect();
        out.push(Line::styled(chunk, style));
    }
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
    let words = explode_words(line);
    if words.is_empty() {
        return vec![Line::from("")];
    }

    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;

    for word in words {
        let wlen = word.content.chars().count();
        let sep = if col == 0 { 0 } else { 1 };
        if wlen > width.saturating_sub(col.max(1)) && col > 0 {
            rows.push(std::mem::take(&mut row));
            col = 0;
        }
        if wlen > width && col == 0 {
            for piece in chunk_span(word, width) {
                let plen = piece.content.chars().count();
                row.push(piece);
                col += plen;
                if col >= width {
                    rows.push(std::mem::take(&mut row));
                    col = 0;
                }
            }
            continue;
        }
        if col > 0 && col + sep + wlen > width {
            rows.push(std::mem::take(&mut row));
            col = 0;
        }
        if col > 0 {
            row.push(Span::styled(" ".to_string(), word.style));
            col += 1;
        }
        row.push(word);
        col += wlen;
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        return vec![Line::from("")];
    }
    rows.into_iter().map(Line::from).collect()
}

fn explode_words(line: Line<'static>) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for span in line.spans {
        let style = span.style;
        let content = span.content.into_owned();
        for piece in content.split_whitespace() {
            if !piece.is_empty() {
                out.push(Span::styled(piece.to_string(), style));
            }
        }
    }
    out
}

fn chunk_span(span: Span<'static>, width: usize) -> Vec<Span<'static>> {
    let style = span.style;
    span.content
        .chars()
        .collect::<Vec<_>>()
        .chunks(width)
        .map(|chunk| Span::styled(chunk.iter().collect::<String>(), style))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styled_wrap_uses_single_spaces() {
        let line = Line::from("Retro arcade CRT aesthetic");
        let rows = wrap_spans_line(line, 80);
        let flat: String = rows[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !flat.contains("  "),
            "double spaces in wrapped text: {:?}",
            flat
        );
    }

    #[test]
    fn plain_wrap_keeps_paren_with_following_word() {
        let lines = wrap_plain_lines("call(foo, bar)", 12, Style::default());
        assert!(
            !lines.first().is_some_and(|l| {
                let t: String = l
                    .spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect();
                t.ends_with('(')
            }),
            "should not orphan opening paren: {:?}",
            lines
        );
    }
}