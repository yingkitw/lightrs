use ratatui::style::Style;
use ratatui::text::Line;

use crate::tui::theme::{self, palette};

use super::wrap::wrap_plain_lines;

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
        out.extend(wrap_plain_lines(line, width, style));
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
        Style::default().fg(palette::DIFF_HUNK)
    } else if line.starts_with('+') {
        Style::default().fg(palette::DIFF_ADD)
    } else if line.starts_with('-') {
        Style::default().fg(palette::DIFF_DEL)
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
}