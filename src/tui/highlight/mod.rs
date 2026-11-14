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
    let lines = markdown::render_markdown(content, width, base);
    if lines.is_empty() {
        wrap::wrap_plain_lines(content, width, base)
    } else {
        lines
    }
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
    fn rust_fence_renders_fence_header() {
        let lines = render_message_body("```rust\nfn main() {}\n```", 80, Style::default());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("rust"));
        assert!(joined.contains("fn main"));
        let colored = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.content.contains("fn") && s.style.fg.is_some()
            })
        });
        assert!(colored, "expected syntect-colored tokens");
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
                s.content.contains("link") && s.style.fg == Some(crate::tui::theme::palette::LINK)
            })
        });
        assert!(bold, "expected bold span");
        assert!(link, "expected link-colored span");
    }
}