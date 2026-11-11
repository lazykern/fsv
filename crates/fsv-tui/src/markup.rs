//! Rich markup (`fsv_core::term`) → ratatui text. The only adapter between
//! the two style models.

use fsv_core::term::{self, Color as MColor, Style as MStyle};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

fn color(c: MColor) -> Color {
    match c {
        MColor::Ansi(n) => match n {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            9 => Color::LightRed,
            10 => Color::LightGreen,
            11 => Color::LightYellow,
            12 => Color::LightBlue,
            13 => Color::LightMagenta,
            14 => Color::LightCyan,
            _ => Color::White,
        },
        MColor::Indexed(n) => Color::Indexed(n),
        MColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        MColor::Default => Color::Reset,
    }
}

pub fn style(s: &MStyle) -> Style {
    let mut out = Style::default();
    let flags = [
        (s.bold, Modifier::BOLD),
        (s.dim, Modifier::DIM),
        (s.italic, Modifier::ITALIC),
        (s.underline, Modifier::UNDERLINED),
        (s.blink, Modifier::SLOW_BLINK),
        (s.reverse, Modifier::REVERSED),
        (s.conceal, Modifier::HIDDEN),
        (s.strike, Modifier::CROSSED_OUT),
    ];
    for (flag, m) in flags {
        match flag {
            Some(true) => out = out.add_modifier(m),
            Some(false) => out = out.remove_modifier(m),
            None => {}
        }
    }
    if let Some(fg) = s.fg {
        out = out.fg(color(fg));
    }
    if let Some(bg) = s.bg {
        out = out.bg(color(bg));
    }
    out
}

fn from_term(t: &term::Text) -> Text<'static> {
    let styles = t.char_styles(MStyle::default());
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur: Option<Style> = None;
    for (ch, st) in t.plain.chars().zip(styles.iter().map(style)) {
        if ch == '\n' {
            if let Some(s) = cur.take() {
                spans.push(Span::styled(std::mem::take(&mut buf), s));
            }
            lines.push(Line::from(std::mem::take(&mut spans)));
            continue;
        }
        if cur != Some(st) {
            if let Some(s) = cur {
                spans.push(Span::styled(std::mem::take(&mut buf), s));
            }
            cur = Some(st);
        }
        buf.push(ch);
    }
    if let Some(s) = cur {
        spans.push(Span::styled(buf, s));
    }
    lines.push(Line::from(spans));
    Text::from(lines)
}

/// Parse Rich markup into styled lines.
pub fn to_text(markup: &str) -> Text<'static> {
    from_term(&term::parse_markup(markup))
}

/// One line of markup (newlines become separate lines; the first is kept).
pub fn to_line(markup: &str) -> Line<'static> {
    to_text(markup).lines.into_iter().next().unwrap_or_default()
}

/// Plain text of markup, as a reader sees it.
pub fn plain(markup: &str) -> String {
    term::strip_markup(markup)
}

/// Unstyled user text (Rich `Text(value)`), split on newlines.
pub fn plain_text(s: &str) -> Text<'static> {
    Text::from(
        s.split('\n')
            .map(|l| Line::from(l.to_string()))
            .collect::<Vec<_>>(),
    )
}
