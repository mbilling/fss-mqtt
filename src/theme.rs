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
        let pick = |l: u32, d: u32| rgb(if light { l } else { d });
        let c = |l: u32, d: u32| Style::new().fg(pick(l, d));
        let accent = c(0xC15F3C, 0xE0805C); // Claude orange
        Theme {
            accent,
            accent_b: accent.add_modifier(Modifier::BOLD),
            text: c(0x1F1F1F, 0xE8E6E3),
            bold: c(0x000000, 0xFFFFFF).add_modifier(Modifier::BOLD),
            dim: c(0x6E6E6E, 0x9A9A9A),
            faint: c(0xA0A0A0, 0x666666),
            border: c(0xD0D0D0, 0x3C3C3C),
            focus_border: c(0xC15F3C, 0xC86F50),
            green: c(0x2E8B47, 0x6FD488),
            red: c(0xC0392B, 0xFF6B6B),
            yellow: c(0xA07800, 0xE5C07B),
            blue: c(0x2F6FB0, 0x7AB4E8),
            purple: c(0x6F5FC4, 0xB8B0FF),
            value: c(0x2B7A78, 0x7FD1C7),
            number: c(0xB35A1F, 0xE8A36B),
            keyword: c(0x8A3FA0, 0xD49BE8),
            section: c(0x4A4A4A, 0xB0B0B0).add_modifier(Modifier::BOLD),
            cursor: Style::new().add_modifier(Modifier::REVERSED),
            matched: accent.add_modifier(Modifier::BOLD),
            key: accent.add_modifier(Modifier::BOLD),
            sel: Style::new().bg(pick(0xF8DCCF, 0x4A2E22)),
            sel_dim: Style::new().bg(pick(0xECECEC, 0x2A2A2A)),
        }
    }
}
