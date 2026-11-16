use ratatui::style::Style;
use ratatui::text::Line;

use crate::tui::theme;

use super::wrap::wrap_code_lines;

const COLLAPSED_DIFF_LINES: usize = 14;

pub fn collapse_diff_preview(unified: &str) -> String {
    let lines: Vec<&str> = unified.lines().collect();
    if lines.len() <= COLLAPSED_DIFF_LINES {
        return unified.to_string();
    }
    let head: Vec<&str> = lines.iter().take(COLLAPSED_DIFF_LINES).copied().collect();
    format!("{}\n… ({} more lines; expand)", head.join("\n"), lines.len() - COLLAPSED_DIFF_LINES)
}

pub fn render_unified_diff(unified: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for line in unified.lines() {
        let style = diff_line_style(line);
        out.extend(wrap_code_lines(
            vec![ratatui::text::Line::styled(line.to_string(), style)],
            width,
        ));
    }
    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

fn diff_line_style(line: &str) -> Style {
    if line.starts_with("+++") || line.starts_with("---") {
        theme::fence_header()
    } else if line.starts_with("@@") {
        Style::default().fg(theme::active().diff_hunk)
    } else if line.starts_with('+') {
        Style::default().fg(theme::active().diff_add)
    } else if line.starts_with('-') {
        Style::default().fg(theme::active().diff_del)
    } else {
        theme::body_muted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_diff() {
        let big = (0..30).map(|i| format!("+line {i}")).collect::<Vec<_>>().join("\n");
        let c = collapse_diff_preview(&big);
        assert!(c.contains("more lines"));
    }

    #[test]
    fn render_preserves_indent_inside_diff_lines() {
        let diff = "@@ -1,3 +1,4 @@\n fn main() {\n+        let x = 1;\n         let y = 2;\n }";
        let lines = render_unified_diff(diff, 80);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("+        let x"), "indent lost: {joined:?}");
        assert!(joined.contains("         let y"), "context indent lost: {joined:?}");
    }
}