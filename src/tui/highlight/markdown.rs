use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::wrap::{wrap_plain_lines, wrap_styled_lines};

fn normalize_lang(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "" => String::new(),
        "rs" => "rust".into(),
        "js" | "jsx" => "javascript".into(),
        "ts" | "tsx" => "typescript".into(),
        "py" => "python".into(),
        "sh" | "shell" | "zsh" | "bash" => "bash".into(),
        "yml" => "yaml".into(),
        other => other.into(),
    }
}

fn code_block_lines(code: &str, width: usize) -> Vec<Line<'static>> {
    let style = Style::default()
        .fg(Color::Rgb(180, 210, 255))
        .bg(Color::Rgb(30, 32, 42));
    wrap_plain_lines(code.trim_end_matches('\n'), width, style)
}

pub(crate) fn render_markdown(content: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(content, opts);
    let mut out = MdLines::new(width, base);

    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_body = String::new();
    let mut heading_level = HeadingLevel::H1;
    let mut heading_buf = String::new();
    let mut in_heading = false;
    let mut in_link = false;

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
            Event::Start(Tag::Paragraph) => out.flush_paragraph(),
            Event::End(TagEnd::Paragraph) => out.flush_paragraph(),
            Event::Start(Tag::List(_)) => out.list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                out.list_depth = out.list_depth.saturating_sub(1);
            }
            Event::Start(Tag::Item) => {
                out.flush_paragraph();
                out.para_prefix = "  ".repeat(out.list_depth.saturating_sub(1)) + "• ";
            }
            Event::End(TagEnd::Item) => out.flush_paragraph(),
            Event::Start(Tag::Strong) => out.strong_depth += 1,
            Event::End(TagEnd::Strong) => out.strong_depth = out.strong_depth.saturating_sub(1),
            Event::Start(Tag::Emphasis) => out.emph_depth += 1,
            Event::End(TagEnd::Emphasis) => out.emph_depth = out.emph_depth.saturating_sub(1),
            Event::Code(code) => {
                out.flush_paragraph();
                out.out.push(Line::from(Span::styled(
                    format!(" `{code}` "),
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::Rgb(40, 42, 54)),
                )));
            }
            Event::Start(Tag::Link { .. }) => in_link = true,
            Event::End(TagEnd::Link) => in_link = false,
            Event::SoftBreak | Event::HardBreak => {
                if in_code_block {
                    code_body.push('\n');
                } else if in_heading {
                    heading_buf.push(' ');
                } else {
                    out.para.push('\n');
                }
            }
            Event::Text(text) => {
                if in_code_block {
                    code_body.push_str(&text);
                } else if in_heading {
                    heading_buf.push_str(&text);
                } else {
                    let style = out.inline_style(in_link);
                    out.push_styled_text(&text, style);
                }
            }
            Event::Rule => {
                out.flush_paragraph();
                let rule = "─".repeat(width.max(8).min(48));
                out.out.push(Line::styled(rule, Style::default().fg(Color::DarkGray)));
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
    para: String,
    para_prefix: String,
    strong_depth: usize,
    emph_depth: usize,
    list_depth: usize,
}

impl MdLines {
    fn new(width: usize, base: Style) -> Self {
        Self {
            width,
            base,
            out: Vec::new(),
            para: String::new(),
            para_prefix: String::new(),
            strong_depth: 0,
            emph_depth: 0,
            list_depth: 0,
        }
    }

    fn inline_style(&self, link: bool) -> Style {
        let mut s = self.base;
        if self.strong_depth > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.emph_depth > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if link {
            s = s.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED);
        }
        s
    }

    fn push_styled_text(&mut self, text: &str, style: Style) {
        self.para.push_str(text);
        let _ = style;
    }

    fn flush_paragraph(&mut self) {
        if self.para.is_empty() && self.para_prefix.is_empty() {
            return;
        }
        let text = format!("{}{}", self.para_prefix, self.para);
        self.para.clear();
        self.para_prefix.clear();
        let style = self.inline_style(false);
        for line in wrap_plain_lines(&text, self.width, style) {
            self.out.push(line);
        }
    }

    fn push_heading(&mut self, text: &str, level: HeadingLevel) {
        let color = match level {
            HeadingLevel::H1 => Color::Cyan,
            HeadingLevel::H2 => Color::Green,
            HeadingLevel::H3 => Color::Magenta,
            _ => Color::Yellow,
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

    fn push_code_block(&mut self, lang: &str, code: &str) {
        let lang_key = normalize_lang(lang);
        let header = if lang_key.is_empty() {
            "code".to_string()
        } else {
            lang_key.clone()
        };
        self.out.push(Line::from(Span::styled(
            format!("── {header} ──"),
            Style::default().fg(Color::DarkGray),
        )));
        self.out.extend(code_block_lines(code, self.width));
        self.out.push(Line::from(""));
    }
}