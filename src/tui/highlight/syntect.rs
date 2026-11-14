use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
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
        other => other.into(),
    }
}

pub(crate) fn highlight_code(code: &str, lang: &str) -> Vec<Line<'static>> {
    let engine = engine();
    let syntax = if lang.is_empty() {
        engine.syntax_set.find_syntax_plain_text()
    } else {
        engine
            .syntax_set
            .find_syntax_by_token(lang)
            .or_else(|| engine.syntax_set.find_syntax_by_extension(lang))
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
            .map(|(style, text)| Span::styled(text.to_string(), syntect_style(style)))
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

fn syntect_style(st: SynStyle) -> Style {
    let mut style = Style::default();
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