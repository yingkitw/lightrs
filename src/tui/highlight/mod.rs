mod markdown;
mod syntect;
mod wrap;

use ratatui::style::{Color, Style};
use ratatui::text::Line;

/// Render message body as ratatui lines (Markdown + highlighted fenced code).
pub fn render_message_body(content: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    if !looks_rich(content) {
        return wrap::wrap_plain_lines(content, width, base);
    }

    let lines = markdown::render_markdown(content, width, base);
    if lines.is_empty() {
        wrap::wrap_plain_lines(content, width, base)
    } else {
        lines
    }
}

fn looks_rich(content: &str) -> bool {
    content.contains("```")
        || content.contains('`')
        || content.contains('#')
        || content.contains('*')
        || content.contains('[')
        || content.contains('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_fence_produces_colored_spans() {
        let lines = render_message_body("```rust\nfn main() {}\n```", 80, Style::default());
        assert!(
            lines.iter().any(|l| {
                l.spans
                    .iter()
                    .any(|s| matches!(s.style.fg, Some(Color::Rgb(_, _, _))))
            }),
            "expected RGB syntax colors"
        );
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
}