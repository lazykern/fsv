//! Textual's `textual-dark` / `textual-light` colors, alpha pre-blended.

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub warning: Color,
    pub error: Color,
    pub success: Color,
    pub foreground: Color,
    pub background: Color,
    pub surface: Color,
    pub panel: Color,
    pub boost: Color,
    pub text: Color,
    pub text_muted: Color,
    pub footer_key: Color,
    pub footer_bg: Color,
    pub border: Color,
    pub border_blurred: Color,
}

const fn rgb(hex: u32) -> (u8, u8, u8) {
    ((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn c(hex: u32) -> Color {
    let (r, g, b) = rgb(hex);
    Color::Rgb(r, g, b)
}

/// `fg` at `alpha` over `bg`.
pub fn blend(fg: u32, bg: u32, alpha: f32) -> Color {
    let (f, b) = (rgb(fg), rgb(bg));
    let mix = |x: u8, y: u8| (f32::from(x) * alpha + f32::from(y) * (1.0 - alpha)).round() as u8;
    Color::Rgb(mix(f.0, b.0), mix(f.1, b.1), mix(f.2, b.2))
}

impl Theme {
    pub fn textual_dark() -> Theme {
        let bg = 0x121212;
        Theme {
            name: "textual-dark",
            primary: c(0x0178D4),
            secondary: c(0x004578),
            accent: c(0xFEA62B),
            warning: c(0xFEA62B),
            error: c(0xB93C5B),
            success: c(0x4EBF71),
            foreground: c(0xE0E0E0),
            background: c(bg),
            surface: c(0x1E1E1E),
            panel: c(0x242F38),
            boost: blend(0xFFFFFF, bg, 0.04),
            text: blend(0xFFFFFF, bg, 0.87),
            text_muted: blend(0xFFFFFF, bg, 0.60),
            footer_key: c(0xFFA62B),
            footer_bg: c(0x242F38),
            border: c(0x0178D4),
            border_blurred: c(0x191919),
        }
    }

    pub fn textual_light() -> Theme {
        let bg = 0xE0E0E0;
        Theme {
            name: "textual-light",
            primary: c(0x004578),
            secondary: c(0x0178D4),
            accent: c(0xFEA62B),
            warning: c(0xFEA62B),
            error: c(0xB93C5B),
            success: c(0x4EBF71),
            foreground: c(0x1F1F1F),
            background: c(bg),
            surface: c(0xD8D8D8),
            panel: c(0xD0D0D0),
            boost: c(bg),
            text: blend(0x000000, bg, 0.87),
            text_muted: blend(0x000000, bg, 0.60),
            footer_key: c(0x0178D4),
            footer_bg: c(0xD0D0D0),
            border: c(0x004578),
            border_blurred: c(0xD0D1D0),
        }
    }

    /// `forms.theme`; unknown names keep the default (as Python).
    pub fn by_name(name: &str) -> Theme {
        match name {
            "textual-light" => Theme::textual_light(),
            _ => Theme::textual_dark(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::textual_dark()
    }
}
