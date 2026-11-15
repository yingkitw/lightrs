use ratatui::text::Line;

#[cfg(feature = "syntect")]
use ratatui::style::{Modifier, Style};
#[cfg(feature = "syntect")]
use ratatui::text::Span;

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
    #[cfg(feature = "syntect")]
    {
        highlight_syntect(code, lang)
    }
    #[cfg(not(feature = "syntect"))]
    {
        let _ = lang;
        return highlight_plain(code);
    }
}

#[cfg(feature = "syntect")]
fn highlight_syntect(code: &str, lang: &str) -> Vec<Line<'static>> {
    use std::sync::OnceLock;

    use syntect::easy::HighlightLines;
    use syntect::highlighting::{Style as SynStyle, Theme};
    use syntect::parsing::SyntaxSet;
    use syntect::util::LinesWithEndings;

    struct Engine {
        syntax_set: SyntaxSet,
        theme: Theme,
    }

    static ENGINE: OnceLock<Engine> = OnceLock::new();

    let engine = ENGINE.get_or_init(|| {
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
    });

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

#[cfg(feature = "syntect")]
fn syntect_to_ratatui(st: syntect::highlighting::Style) -> Style {
    use ratatui::style::Color;
    use syntect::highlighting::FontStyle;

    let mut style = Style::default().bg(crate::tui::theme::active().code_bg);
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

#[cfg(not(feature = "syntect"))]
fn highlight_plain(code: &str) -> Vec<Line<'static>> {
    use crate::tui::theme;
    code.split_inclusive('\n')
        .map(|line| {
            let line = line.strip_suffix('\n').unwrap_or(line);
            Line::styled(line.to_string(), theme::code_block())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_fence_has_content() {
        let lines = highlight_code("fn main() {}", "rust");
        assert!(!lines.is_empty());
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(joined.contains("fn main"));
    }

    #[cfg(feature = "syntect")]
    #[test]
    fn rust_fn_is_colored() {
        let lines = highlight_code("fn main() {}", "rust");
        let has_color = lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.style.fg.is_some() && s.content.contains("fn"))
        });
        assert!(has_color);
    }
}