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

/// Catppuccin Mocha — default; balanced contrast for long sessions.
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

/// Catppuccin Latte — light background feel (bright terminal / daytime).
pub const LATTE: Palette = Palette {
    text: rgb(76, 79, 105),
    muted: rgb(124, 127, 147),
    hint: rgb(156, 160, 176),
    border: rgb(204, 208, 218),
    user: rgb(30, 102, 245),
    assistant: rgb(64, 160, 43),
    tool: rgb(223, 142, 29),
    system: rgb(140, 144, 162),
    error: rgb(210, 15, 57),
    accent: rgb(23, 146, 153),
    busy: rgb(223, 142, 29),
    link: rgb(35, 102, 217),
    code_fg: rgb(76, 79, 105),
    code_bg: rgb(239, 241, 245),
    inline_code_fg: rgb(136, 57, 150),
    inline_code_bg: rgb(220, 224, 232),
    heading_1: rgb(23, 146, 153),
    heading_2: rgb(64, 160, 43),
    heading_3: rgb(136, 57, 150),
    heading_other: rgb(223, 142, 29),
    mermaid_accent: rgb(136, 57, 150),
    mermaid_edge: rgb(35, 102, 217),
    tool_args: rgb(124, 127, 147),
    diff_add: rgb(64, 160, 43),
    diff_del: rgb(210, 15, 57),
    diff_hunk: rgb(35, 102, 217),
};

/// Nord — cool, low-blue glare.
pub const NORD: Palette = Palette {
    text: rgb(236, 239, 244),
    muted: rgb(216, 222, 233),
    hint: rgb(76, 86, 106),
    border: rgb(67, 76, 94),
    user: rgb(143, 188, 187),
    assistant: rgb(163, 190, 140),
    tool: rgb(235, 203, 139),
    system: rgb(129, 161, 193),
    error: rgb(191, 97, 106),
    accent: rgb(136, 192, 208),
    busy: rgb(235, 203, 139),
    link: rgb(129, 161, 193),
    code_fg: rgb(216, 222, 233),
    code_bg: rgb(46, 52, 64),
    inline_code_fg: rgb(180, 142, 173),
    inline_code_bg: rgb(59, 66, 82),
    heading_1: rgb(136, 192, 208),
    heading_2: rgb(163, 190, 140),
    heading_3: rgb(180, 142, 173),
    heading_other: rgb(235, 203, 139),
    mermaid_accent: rgb(180, 142, 173),
    mermaid_edge: rgb(129, 161, 193),
    tool_args: rgb(216, 222, 233),
    diff_add: rgb(163, 190, 140),
    diff_del: rgb(191, 97, 106),
    diff_hunk: rgb(136, 192, 208),
};

/// Rose Pine Moon — warm dusk tones.
pub const ROSEPINE: Palette = Palette {
    text: rgb(224, 222, 244),
    muted: rgb(110, 106, 134),
    hint: rgb(87, 82, 105),
    border: rgb(62, 56, 81),
    user: rgb(156, 207, 216),
    assistant: rgb(169, 193, 155),
    tool: rgb(246, 193, 119),
    system: rgb(196, 167, 231),
    error: rgb(235, 111, 146),
    accent: rgb(196, 167, 231),
    busy: rgb(246, 193, 119),
    link: rgb(196, 167, 231),
    code_fg: rgb(224, 222, 244),
    code_bg: rgb(35, 33, 54),
    inline_code_fg: rgb(235, 188, 186),
    inline_code_bg: rgb(57, 53, 82),
    heading_1: rgb(196, 167, 231),
    heading_2: rgb(169, 193, 155),
    heading_3: rgb(235, 188, 186),
    heading_other: rgb(246, 193, 119),
    mermaid_accent: rgb(235, 188, 186),
    mermaid_edge: rgb(156, 207, 216),
    tool_args: rgb(110, 106, 134),
    diff_add: rgb(169, 193, 155),
    diff_del: rgb(235, 111, 146),
    diff_hunk: rgb(156, 207, 216),
};

/// High contrast — crisp edges for bright rooms / accessibility.
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
        id: "latte",
        label: "Catppuccin Latte (light)",
        palette: LATTE,
    },
    ThemeEntry {
        id: "nord",
        label: "Nord",
        palette: NORD,
    },
    ThemeEntry {
        id: "rosepine",
        label: "Rosé Pine Moon",
        palette: ROSEPINE,
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
    fn all_theme_ids_resolve() {
        for t in THEMES {
            assert!(by_id(t.id).is_some(), "{}", t.id);
        }
    }
}