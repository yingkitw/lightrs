use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::theme;

use super::mermaid;
use super::syntect::{highlight_code, normalize_lang};
use super::wrap::wrap_styled_lines;

fn code_block_lines(code: &str, lang: &str, width: usize) -> Vec<Line<'static>> {
    let trimmed = code.trim_end_matches('\n');
    let highlighted = highlight_code(trimmed, lang);
    super::wrap::wrap_code_lines(highlighted, width)
}

pub(crate) fn render_markdown(content: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION;
    let parser = Parser::new_ext(content, opts);
    let mut out = MdLines::new(width, base);

    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_body = String::new();
    let mut heading_level = HeadingLevel::H1;
    let mut heading_buf = String::new();
    let mut in_heading = false;
    let mut in_link = false;
    let mut in_table = false;
    let mut in_table_cell = false;
    let mut table_cell = String::new();
    let mut table_row: Vec<String> = Vec::new();
    let mut table_head_rows: Vec<Vec<String>> = Vec::new();
    let mut table_body_rows: Vec<Vec<String>> = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                out.flush_paragraph();
                heading_level = level;
                heading_buf.clear();
                in_heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                out.push_heading(&heading_buf, heading_level);
                heading_buf.clear();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                out.flush_paragraph();
                in_code_block = true;
                code_body.clear();
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out.push_code_block(&code_lang, &code_body);
                code_body.clear();
                code_lang.clear();
            }
            Event::Start(Tag::Paragraph) => {
                if !in_table {
                    out.flush_paragraph();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if !in_table {
                    out.flush_paragraph();
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                out.flush_paragraph();
                out.blockquote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                out.flush_paragraph();
                out.blockquote_depth = out.blockquote_depth.saturating_sub(1);
            }
            Event::Start(Tag::List(_)) => out.list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                out.list_depth = out.list_depth.saturating_sub(1);
            }
            Event::Start(Tag::Item) => {
                out.flush_paragraph();
                let pad = "  ".repeat(out.list_depth.saturating_sub(1));
                out.para_prefix = format!("{pad}• ");
            }
            Event::End(TagEnd::Item) => out.flush_paragraph(),
            Event::Start(Tag::Strong) => out.strong_depth += 1,
            Event::End(TagEnd::Strong) => out.strong_depth = out.strong_depth.saturating_sub(1),
            Event::Start(Tag::Emphasis) => out.emph_depth += 1,
            Event::End(TagEnd::Emphasis) => out.emph_depth = out.emph_depth.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => out.strike_depth += 1,
            Event::End(TagEnd::Strikethrough) => {
                out.strike_depth = out.strike_depth.saturating_sub(1);
            }
            Event::Code(code) => {
                out.push_span(Span::styled(
                    format!("`{code}`"),
                    theme::inline_code(),
                ));
            }
            Event::Start(Tag::Link { .. }) => in_link = true,
            Event::End(TagEnd::Link) => in_link = false,
            Event::SoftBreak => {
                if in_code_block {
                    code_body.push('\n');
                } else if in_heading {
                    heading_buf.push(' ');
                } else {
                    out.push_span(Span::styled(" ".to_string(), out.inline_style(false)));
                }
            }
            Event::HardBreak => {
                if in_code_block {
                    code_body.push('\n');
                } else if in_heading {
                    heading_buf.push(' ');
                } else {
                    // Model replies often use hard breaks mid-sentence; treat as a space in prose.
                    out.push_span(Span::styled(" ".to_string(), out.inline_style(false)));
                }
            }
            Event::Start(Tag::Table(_)) => {
                out.flush_paragraph();
                in_table = true;
                table_head_rows.clear();
                table_body_rows.clear();
            }
            Event::End(TagEnd::Table) => {
                in_table = false;
                out.push_table(&table_head_rows, &table_body_rows);
                table_head_rows.clear();
                table_body_rows.clear();
            }
            Event::Start(Tag::TableHead) => table_row.clear(),
            Event::End(TagEnd::TableHead) => {
                if !table_row.is_empty() {
                    table_head_rows.push(table_row.clone());
                }
                table_row.clear();
            }
            Event::Start(Tag::TableRow) => table_row.clear(),
            Event::End(TagEnd::TableRow) => table_body_rows.push(table_row.clone()),
            Event::Start(Tag::TableCell) => {
                in_table_cell = true;
                table_cell.clear();
            }
            Event::End(TagEnd::TableCell) => {
                in_table_cell = false;
                table_row.push(table_cell.trim().to_string());
            }
            Event::Text(text) => {
                if in_code_block {
                    code_body.push_str(&text);
                } else if in_heading {
                    heading_buf.push_str(&text);
                } else if in_table && in_table_cell {
                    table_cell.push_str(&text);
                } else {
                    let style = out.inline_style(in_link);
                    out.push_span(Span::styled(text.to_string(), style));
                }
            }
            Event::Rule => {
                out.flush_paragraph();
                let rule = "─".repeat(width.clamp(8, 48));
                out.out.push(Line::styled(rule, theme::border()));
            }
            Event::Html(_) | Event::InlineHtml(_) => {}
            _ => {}
        }
    }

    out.flush_paragraph();
    out.out
}

struct MdLines {
    width: usize,
    base: Style,
    out: Vec<Line<'static>>,
    para_spans: Vec<Span<'static>>,
    para_prefix: String,
    strong_depth: usize,
    emph_depth: usize,
    strike_depth: usize,
    list_depth: usize,
    blockquote_depth: usize,
}

impl MdLines {
    fn new(width: usize, base: Style) -> Self {
        Self {
            width,
            base,
            out: Vec::new(),
            para_spans: Vec::new(),
            para_prefix: String::new(),
            strong_depth: 0,
            emph_depth: 0,
            strike_depth: 0,
            list_depth: 0,
            blockquote_depth: 0,
        }
    }

    fn inline_style(&self, link: bool) -> Style {
        let mut s = if self.blockquote_depth > 0 {
            theme::body_muted()
        } else {
            self.base
        };
        if self.strong_depth > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.emph_depth > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strike_depth > 0 {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        if link {
            s = theme::link();
        }
        s
    }

    fn push_span(&mut self, span: Span<'static>) {
        if span.content.is_empty() {
            return;
        }
        if let Some(last) = self.para_spans.last_mut()
            && last.style == span.style
                && !span.content.starts_with('\n')
                && !last.content.ends_with('\n')
            {
                let mut merged = last.content.to_string();
                merged.push_str(&span.content);
                last.content = merged.into();
                return;
            }
        self.para_spans.push(span);
    }

    fn flush_paragraph(&mut self) {
        if self.para_spans.is_empty() && self.para_prefix.is_empty() {
            return;
        }
        let mut spans = Vec::new();
        if !self.para_prefix.is_empty() {
            spans.push(Span::styled(self.para_prefix.clone(), self.base));
            self.para_prefix.clear();
        }
        if self.blockquote_depth > 0 {
            spans.insert(
                0,
                Span::styled(
                    "▏ ",
                    Style::default().fg(theme::active().border),
                ),
            );
        }
        spans.append(&mut self.para_spans);
        self.para_spans.clear();
        self.out
            .extend(wrap_styled_lines(vec![Line::from(spans)], self.width));
    }

    fn push_heading(&mut self, text: &str, level: HeadingLevel) {
        let color = match level {
            HeadingLevel::H1 => theme::active().heading_1,
            HeadingLevel::H2 => theme::active().heading_2,
            HeadingLevel::H3 => theme::active().heading_3,
            _ => theme::active().heading_other,
        };
        let indent = match level {
            HeadingLevel::H1 => "",
            HeadingLevel::H2 => "",
            HeadingLevel::H3 => "  ",
            _ => "    ",
        };
        let mut style = Style::default()
            .fg(color)
            .add_modifier(Modifier::BOLD);
        if level == HeadingLevel::H1 {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        let line = Line::from(vec![Span::styled(format!("{indent}{text}"), style)]);
        self.out.extend(wrap_styled_lines(vec![line], self.width));
        self.out.push(Line::from(""));
    }

    fn push_table(&mut self, head: &[Vec<String>], body: &[Vec<String>]) {
        let head: Vec<Vec<String>> = head
            .iter()
            .filter(|row| !is_alignment_row(row))
            .cloned()
            .collect();
        let body: Vec<Vec<String>> = body
            .iter()
            .filter(|row| !is_alignment_row(row))
            .cloned()
            .collect();
        if head.is_empty() && body.is_empty() {
            return;
        }
        let mut rows = head;
        let head_count = rows.len();
        rows.extend(body);
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let mut widths = vec![2usize; cols];
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count().min(28));
            }
        }
        for (idx, row) in rows.iter().enumerate() {
            let style = if idx < head_count {
                self.base.add_modifier(Modifier::BOLD)
            } else {
                theme::body_muted()
            };
            let cells: Vec<String> = (0..cols)
                .map(|i| {
                    let text = row.get(i).map(String::as_str).unwrap_or("");
                    pad_display_cell(text, widths[i])
                })
                .collect();
            self.out.push(Line::styled(
                format!("│ {} │", cells.join(" │ ")),
                style,
            ));
        }
        self.out.push(Line::from(""));
    }

    fn push_code_block(&mut self, lang: &str, code: &str) {
        let lang_key = normalize_lang(lang);
        if lang_key == "mermaid" {
            self.out
                .extend(mermaid::render_mermaid(code, self.width));
            return;
        }
        let header = if lang_key.is_empty() {
            "code".to_string()
        } else {
            lang_key.clone()
        };
        self.out.push(Line::from(Span::styled(
            format!("── {header} ──"),
            theme::fence_header(),
        )));
        self.out.extend(code_block_lines(code, &lang_key, self.width));
        self.out.push(Line::from(""));
    }
}

fn is_alignment_row(row: &[String]) -> bool {
    row.iter().all(|cell| {
        let t = cell.trim();
        t.is_empty() || t.chars().all(|c| c == '-' || c == ':' || c.is_whitespace())
    })
}

fn pad_display_cell(text: &str, width: usize) -> String {
    let n = text.chars().count();
    if n > width {
        let end: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{end}…")
    } else {
        format!("{text}{}", " ".repeat(width - n))
    }
}