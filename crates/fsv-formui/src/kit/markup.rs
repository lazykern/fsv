//! Rich markup (`[bold red]x[/]`) → ratatui lines, through the one parser
//! in `fsv_core::term`.

use fsv_core::term::{Color as TColor, Style as TStyle, parse_markup};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

fn color(c: TColor) -> Color {
    const NAMED: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    match c {
        TColor::Ansi(i) => NAMED[usize::from(i) % 16],
        TColor::Indexed(i) => Color::Indexed(i),
        TColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        TColor::Default => Color::Reset,
    }
}

/// A term style as a ratatui style.
pub fn style(s: &TStyle) -> Style {
    let mut out = Style::default();
    if let Some(fg) = s.fg {
        out = out.fg(color(fg));
    }
    if let Some(bg) = s.bg {
        out = out.bg(color(bg));
    }
    for (flag, m) in [
        (s.bold, Modifier::BOLD),
        (s.dim, Modifier::DIM),
        (s.italic, Modifier::ITALIC),
        (s.underline, Modifier::UNDERLINED),
        (s.blink, Modifier::SLOW_BLINK),
        (s.reverse, Modifier::REVERSED),
        (s.conceal, Modifier::HIDDEN),
        (s.strike, Modifier::CROSSED_OUT),
    ] {
        match flag {
            Some(true) => out = out.add_modifier(m),
            Some(false) => out = out.remove_modifier(m),
            None => {}
        }
    }
    out
}

/// Markup → lines (split on `\n`), each span styled over `base`.
pub fn lines(markup: &str, base: Style) -> Vec<Line<'static>> {
    let text = parse_markup(markup);
    let styles = text.char_styles(TStyle::default());
    let mut out = vec![];
    let mut spans: Vec<Span<'static>> = vec![];
    let mut cur = String::new();
    let mut cur_style: Option<TStyle> = None;
    let flush = |spans: &mut Vec<Span<'static>>, cur: &mut String, st: Option<TStyle>| {
        if !cur.is_empty() {
            let s = st.map_or(base, |s| base.patch(style(&s)));
            spans.push(Span::styled(std::mem::take(cur), s));
        }
    };
    for (ch, st) in text.plain.chars().zip(styles) {
        if ch == '\n' {
            flush(&mut spans, &mut cur, cur_style);
            out.push(Line::from(std::mem::take(&mut spans)));
            cur_style = None;
            continue;
        }
        if cur_style != Some(st) {
            flush(&mut spans, &mut cur, cur_style);
            cur_style = Some(st);
        }
        cur.push(ch);
    }
    flush(&mut spans, &mut cur, cur_style);
    out.push(Line::from(spans));
    out
}

/// Markup → the plain text a reader sees.
pub fn plain(markup: &str) -> String {
    parse_markup(markup).plain
}

pub use fsv_core::term::escape;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_and_lines() {
        let ls = lines("[b]Label[/b] [bold red]*[/]\n[dim]x[/]", Style::default());
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[0].spans[0].content, "Label");
        assert!(ls[0].spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(ls[0].spans[2].style.fg, Some(Color::Red));
        assert!(ls[1].spans[0].style.add_modifier.contains(Modifier::DIM));
        assert_eq!(plain(&format!("[b]{}[/b]", escape("[x]"))), "[x]");
    }
}
