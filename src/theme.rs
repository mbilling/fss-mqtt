use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub accent: Style,
    pub accent_b: Style,
    pub text: Style,
    pub bold: Style,
    pub dim: Style,
    pub faint: Style,
    pub border: Style,       // panes without focus
    pub focus_border: Style, // the pane your keys go to
    pub green: Style,
    pub red: Style,
    pub yellow: Style,
    pub blue: Style,
    pub purple: Style,
    pub value: Style,
    pub number: Style,
    pub keyword: Style,
    pub section: Style,
    pub cursor: Style,
    pub matched: Style,
    pub key: Style,     // key names in the hint line
    pub sel: Style,     // selected row in the focused pane (background bar)
    pub sel_dim: Style, // selected row in other panes
}

fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Theme {
    /// Picks light or dark from $COLORFGBG ("fg;bg"), defaulting to dark.
    pub fn detect() -> Theme {
        let light = std::env::var("COLORFGBG")
            .ok()
            .and_then(|v| v.rsplit(';').next().and_then(|b| b.parse::<u8>().ok()))
            .is_some_and(|bg| bg == 7 || bg == 15);
        Theme::new(light)
    }

    pub fn new(light: bool) -> Theme {
        let c = |l: u32, d: u32| Style::new().fg(rgb(if light { l } else { d }));
        let accent = c(0xC15F3C, 0xD77757); // Claude orange
        let dim = c(0x8A8A8A, 0x7A7A7A);
        Theme {
            accent,
            accent_b: accent.add_modifier(Modifier::BOLD),
            text: c(0x1F1F1F, 0xE8E6E3),
            bold: c(0x1F1F1F, 0xE8E6E3).add_modifier(Modifier::BOLD),
            dim,
            faint: c(0xB8B8B8, 0x4A4A4A),
            border: c(0xC8C8C8, 0x4E4E4E),
            focus_border: dim,
            green: c(0x2E8B47, 0x5FBF77),
            red: c(0xC0392B, 0xFF6B6B),
            yellow: c(0xA07800, 0xE5C07B),
            blue: c(0x2F6FB0, 0x7AB4E8),
            purple: c(0x6F5FC4, 0xB1A8F5),
            value: c(0x2B7A78, 0x7FD1C7),
            number: c(0xB35A1F, 0xE8A36B),
            keyword: c(0x8A3FA0, 0xD49BE8),
            section: dim.add_modifier(Modifier::BOLD),
            cursor: Style::new().add_modifier(Modifier::REVERSED),
            matched: accent.add_modifier(Modifier::BOLD),
            key: c(0x1F1F1F, 0xE8E6E3),
            // No background bars: the selected row is marked by the cursor glyph.
            sel: Style::new(),
            sel_dim: Style::new(),
        }
    }
}
