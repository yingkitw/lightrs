use ratatui::style::Color;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub text: Color,
    pub muted: Color,
    pub hint: Color,
    pub border: Color,
    pub user: Color,
    pub assistant: Color,
    pub tool: Color,
    pub system: Color,
    pub error: Color,
    pub accent: Color,
    pub busy: Color,
    pub link: Color,
    pub code_fg: Color,
    pub code_bg: Color,
    pub inline_code_fg: Color,
    pub inline_code_bg: Color,
    pub heading_1: Color,
    pub heading_2: Color,
    pub heading_3: Color,
    pub heading_other: Color,
    pub mermaid_accent: Color,
    pub mermaid_edge: Color,
    pub tool_args: Color,
    pub diff_add: Color,
    pub diff_del: Color,
    pub diff_hunk: Color,
}

pub struct ThemeEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub palette: Palette,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

pub const MOCHA: Palette = Palette {
    text: rgb(205, 214, 244),
    muted: rgb(127, 132, 156),
    hint: rgb(88, 91, 112),
    border: rgb(69, 71, 90),
    user: rgb(137, 220, 235),
    assistant: rgb(166, 227, 161),
    tool: rgb(250, 179, 135),
    system: rgb(147, 153, 178),
    error: rgb(243, 139, 168),
    accent: rgb(148, 226, 213),
    busy: rgb(249, 226, 175),
    link: rgb(137, 180, 250),
    code_fg: rgb(186, 199, 218),
    code_bg: rgb(24, 24, 37),
    inline_code_fg: rgb(245, 194, 231),
    inline_code_bg: rgb(49, 50, 68),
    heading_1: rgb(148, 226, 213),
    heading_2: rgb(166, 227, 161),
    heading_3: rgb(203, 166, 247),
    heading_other: rgb(250, 179, 135),
    mermaid_accent: rgb(203, 166, 247),
    mermaid_edge: rgb(166, 199, 236),
    tool_args: rgb(186, 194, 216),
    diff_add: rgb(166, 227, 161),
    diff_del: rgb(243, 139, 168),
    diff_hunk: rgb(137, 180, 250),
};

pub const HC: Palette = Palette {
    text: rgb(250, 250, 250),
    muted: rgb(200, 200, 200),
    hint: rgb(160, 160, 160),
    border: rgb(120, 120, 120),
    user: rgb(135, 206, 250),
    assistant: rgb(144, 238, 144),
    tool: rgb(255, 200, 100),
    system: rgb(200, 200, 255),
    error: rgb(255, 120, 120),
    accent: rgb(0, 255, 200),
    busy: rgb(255, 220, 120),
    link: rgb(100, 200, 255),
    code_fg: rgb(240, 240, 240),
    code_bg: rgb(20, 20, 20),
    inline_code_fg: rgb(255, 220, 120),
    inline_code_bg: rgb(40, 40, 40),
    heading_1: rgb(0, 255, 200),
    heading_2: rgb(144, 238, 144),
    heading_3: rgb(255, 180, 255),
    heading_other: rgb(255, 200, 100),
    mermaid_accent: rgb(255, 180, 255),
    mermaid_edge: rgb(135, 206, 250),
    tool_args: rgb(200, 200, 200),
    diff_add: rgb(120, 255, 120),
    diff_del: rgb(255, 120, 120),
    diff_hunk: rgb(135, 206, 250),
};

pub const THEMES: &[ThemeEntry] = &[
    ThemeEntry {
        id: "mocha",
        label: "Catppuccin Mocha (default)",
        palette: MOCHA,
    },
    ThemeEntry {
        id: "hc",
        label: "High contrast",
        palette: HC,
    },
];

pub fn by_id(id: &str) -> Option<&'static Palette> {
    let key = id.trim().to_lowercase();
    THEMES
        .iter()
        .find(|t| t.id == key.as_str())
        .map(|t| &t.palette)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_resolve() {
        for t in THEMES {
            assert!(by_id(t.id).is_some(), "{}", t.id);
        }
    }
}