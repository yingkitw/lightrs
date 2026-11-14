use std::sync::OnceLock;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

struct Engine {
    syntax_set: SyntaxSet,
    theme: Theme,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

fn engine() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = syntect::highlighting::ThemeSet::load_defaults();
        let theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.get("TwoDark"))
            .or_else(|| theme_set.themes.get("Solarized (dark)"))
            .cloned()
            .unwrap_or_default();
        Engine { syntax_set, theme }
    })
}

pub(crate) fn normalize_lang(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "" => String::new(),
        "rs" => "rust".into(),
        "js" | "jsx" => "javascript".into(),
        "ts" | "tsx" => "typescript".into(),
        "py" => "python".into(),
        "sh" | "shell" | "zsh" | "bash" => "bash".into(),
        "yml" => "yaml".into(),
        "md" | "mkd" => "markdown".into(),
        "cpp" | "cc" | "cxx" | "h" | "hpp" => "cpp".into(),
        "cs" => "csharp".into(),
        "golang" => "go".into(),
        "docker" => "dockerfile".into(),
        "mmd" => "mermaid".into(),
        other => other.into(),
    }
}

pub(crate) fn highlight_code(code: &str, lang: &str) -> Vec<Line<'static>> {
    let engine = engine();
    let lang_key = normalize_lang(lang);
    let syntax = if lang_key.is_empty() {
        engine.syntax_set.find_syntax_plain_text()
    } else {
        engine
            .syntax_set
            .find_syntax_by_token(&lang_key)
            .or_else(|| engine.syntax_set.find_syntax_by_extension(&lang_key))
            .unwrap_or_else(|| engine.syntax_set.find_syntax_plain_text())
    };

    let mut highlighter = HighlightLines::new(syntax, &engine.theme);
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(code) {
        let highlighted = highlighter.highlight_line(line, &engine.syntax_set);
        let ranges = match highlighted {
            Ok(r) => r,
            Err(_) => vec![(SynStyle::default(), line)],
        };
        let spans: Vec<Span<'static>> = ranges
            .into_iter()
            .map(|(style, text)| Span::styled(text.to_string(), syntect_to_ratatui(style)))
            .collect();
        if spans.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(spans));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
}

fn syntect_to_ratatui(st: SynStyle) -> Style {
    use ratatui::style::Color;
    use crate::tui::theme::palette;

    let mut style = Style::default().bg(palette::CODE_BG);
    let c = st.foreground;
    style = style.fg(Color::Rgb(c.r, c.g, c.b));
    if st.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if st.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if st.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_fn_is_colored() {
        let lines = highlight_code("fn main() {}", "rust");
        assert!(!lines.is_empty());
        let has_color = lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.style.fg.is_some() && s.content.contains("fn"))
        });
        assert!(has_color);
    }
}