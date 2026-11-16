mod diff;
mod markdown;
mod sys;

pub use diff::{collapse_diff_preview, render_unified_diff};
mod mermaid;
mod syntect;
mod wrap;

use ratatui::style::Style;
use ratatui::text::Line;

/// Render message body as ratatui lines (Markdown + highlighted fenced code).
pub use sys::render_system_body;

pub fn render_message_body(content: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    let content = content.trim_end();
    let lines = markdown::render_markdown(content, width, base);
    let lines = if lines.is_empty() {
        wrap::wrap_plain_lines(content, width, base)
    } else {
        lines
    };
    strip_trailing_blank_lines(lines)
}

/// Tool bodies render preformatted — command output is literal text, not
/// markdown; ANSI escapes are stripped (defense for non-shell tools).
/// Whitespace is content here (indented code, aligned tables) — verbatim wrap.
pub fn render_pre_body(content: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    let content = crate::tools::shell::strip_ansi(content);
    let lines: Vec<Line<'static>> = content
        .trim_end()
        .split('\n')
        .map(|l| Line::styled(l.to_string(), base))
        .collect();
    let lines = wrap::wrap_code_lines(lines, width);
    strip_trailing_blank_lines(lines)
}

/// File-reading tool results are source text — syntax-highlight with the
/// fenced-code path keyed on a language/extension token. ANSI is stripped
/// first: file bytes are untrusted input and escapes would inject terminal
/// control into the transcript.
pub fn render_file_result_body(content: &str, lang: &str, width: usize) -> Vec<Line<'static>> {
    let content = crate::tools::shell::strip_ansi(content);
    wrap::wrap_code_lines(syntect::highlight_code(&content, lang), width)
}

fn strip_trailing_blank_lines(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while let Some(last) = lines.last() {
        if last.spans.iter().all(|s| s.content.trim().is_empty()) {
            lines.pop();
        } else {
            break;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;
    #[test]
    fn mermaid_fence_renders_diagram_header() {
        let src = "```mermaid\nflowchart LR\n  A[One] --> B[Two]\n```";
        let lines = render_message_body(src, 80, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("mermaid"));
        assert!(joined.contains("One"));
    }

    #[test]
    fn pre_body_preserves_indent_and_tabs() {
        let lines =
            render_pre_body("fn main() {\n\tlet a = 1;\n        let b = 2;\n}", 80, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("\tlet a"), "indent lost: {joined:?}");
        assert!(joined.contains("        let b"), "indent lost: {joined:?}");
    }

    #[test]
    fn code_fence_preserves_indentation() {
        let lines = render_message_body(
            "```python\ndef f():\n    if x:\n        return 1\n```",
            80,
            Style::default(),
        );
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("    if x"), "indent lost: {joined:?}");
        assert!(joined.contains("        return"), "indent lost: {joined:?}");
    }

    #[test]
    fn long_code_line_hard_wraps_without_losing_chars() {
        let src = "        ".repeat(8) + &"x".repeat(100);
        let lines = render_pre_body(&src, 30, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        // Every char survives the wrap — including the leading spaces.
        assert_eq!(joined.replace('\n', ""), src);
        assert!(lines.len() > 4);
    }

    #[test]
    fn pre_body_keeps_markdown_chars_literal_and_strips_ansi() {
        let lines = render_pre_body("\u{1b}[31m# not a heading | a|b\u{1b}[0m", 80, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("# not a heading | a|b"));
        assert!(!joined.contains('\u{1b}'), "no raw escapes: {joined:?}");
    }

    #[test]
    fn trailing_newline_does_not_add_blank_body_line() {
        let lines = render_message_body("Hello world.\n\n", 80, Style::default());
        assert!(
            lines
                .last()
                .is_none_or(|l| l.spans.iter().any(|s| !s.content.trim().is_empty())),
            "should not end with an empty rendered line"
        );
    }

    #[test]
    fn rust_fence_renders_fence_header() {
        let lines = render_message_body("```rust\nfn main() {}\n```", 80, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("rust"));
        assert!(joined.contains("fn main"));
        #[cfg(feature = "syntect")]
        {
            let colored = lines.iter().any(|l| {
                l.spans.iter().any(|s| {
                    s.content.contains("fn") && s.style.fg.is_some()
                })
            });
            assert!(colored, "expected syntect-colored tokens");
        }
    }

    #[test]
    fn markdown_heading() {
        let lines = render_message_body("# Title\n\nbody", 80, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("Title"));
    }

    #[test]
    fn markdown_table_rows() {
        let md = "| one | two |\n| --- | --- |\n| 1 | 2 |";
        let lines = render_message_body(md, 48, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains('│'), "expected table pipes: {joined}");
        assert!(joined.contains("one"), "table header: {joined}");
        assert!(joined.contains('1') && joined.contains('2'), "table cells: {joined}");
    }

    #[test]
    fn inline_code_stays_in_paragraph() {
        let md = "logic (`stepDemo`, `types`, `constants`) vs layer";
        let lines = render_message_body(md, 100, Style::default());
        let flat: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(flat.contains("`stepDemo`"), "{flat}");
        assert!(flat.contains("`types`"), "{flat}");
        assert!(
            lines.len() <= 2,
            "inline code should not be one line per backtick: {:?}",
            lines
        );
    }

    #[test]
    fn markdown_bold_and_link_styles() {
        let lines = render_message_body("**bold** and [link](https://x.test)", 80, Style::default());
        let bold = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.content.contains("bold")
                    && s.style.add_modifier.contains(Modifier::BOLD)
            })
        });
        let link = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.content.contains("link")
                    && s.style.fg == Some(crate::tui::theme::active().link)
            })
        });
        assert!(bold, "expected bold span");
        assert!(link, "expected link-colored span");
    }
}