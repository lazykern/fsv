//! SGR-coloured text (`\x1b[1;36m…\x1b[0m`) → ratatui lines, as Rich's
//! `Text.from_ansi` reads what a console printed.

use fsv_core::term::Color as TColor;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::markup::color;

fn sgr_color(codes: &[u16], i: &mut usize) -> Option<Color> {
    match codes.get(*i + 1) {
        Some(5) => {
            let n = *codes.get(*i + 2)?;
            *i += 2;
            Some(Color::Indexed(u8::try_from(n).ok()?))
        }
        Some(2) => {
            let rgb = codes.get(*i + 2..*i + 5)?;
            *i += 4;
            let c = |v: u16| u8::try_from(v).unwrap_or(u8::MAX);
            Some(Color::Rgb(c(rgb[0]), c(rgb[1]), c(rgb[2])))
        }
        _ => None,
    }
}

fn apply(style: Style, params: &str) -> Style {
    let codes: Vec<u16> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
    let mut s = style;
    let mut i = 0;
    while i < codes.len() {
        let ansi = |n: u16| color(TColor::Ansi(n as u8));
        s = match codes[i] {
            0 => Style::default(),
            1 => s.add_modifier(Modifier::BOLD),
            2 => s.add_modifier(Modifier::DIM),
            3 => s.add_modifier(Modifier::ITALIC),
            4 => s.add_modifier(Modifier::UNDERLINED),
            5 => s.add_modifier(Modifier::SLOW_BLINK),
            7 => s.add_modifier(Modifier::REVERSED),
            8 => s.add_modifier(Modifier::HIDDEN),
            9 => s.add_modifier(Modifier::CROSSED_OUT),
            n @ 30..=37 => s.fg(ansi(n - 30)),
            n @ 90..=97 => s.fg(ansi(n - 90 + 8)),
            n @ 40..=47 => s.bg(ansi(n - 40)),
            n @ 100..=107 => s.bg(ansi(n - 100 + 8)),
            38 => sgr_color(&codes, &mut i).map_or(s, |c| s.fg(c)),
            48 => sgr_color(&codes, &mut i).map_or(s, |c| s.bg(c)),
            39 => s.fg(Color::Reset),
            49 => s.bg(Color::Reset),
            _ => s,
        };
        i += 1;
    }
    s
}

/// One printed line; other escape sequences are dropped.
pub fn line(text: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![];
    let mut style = Style::default();
    let mut cur = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('\x1b') {
        cur.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let Some(body) = rest.strip_prefix('[') else {
            continue;
        };
        let end = body
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(body.len());
        if body[end..].starts_with('m') {
            if !cur.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut cur), style));
            }
            style = apply(style, &body[..end]);
        }
        rest = body.get(end + 1..).unwrap_or("");
    }
    cur.push_str(rest);
    if !cur.is_empty() {
        spans.push(Span::styled(cur, style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_follow_the_codes() {
        let l = line("\x1b[36mSubject \x1b[0m\x1b[36m \x1b[0mx \x1b[1;38;2;1;2;3mb\x1b[0m");
        let got: Vec<(&str, Style)> = l.spans.iter().map(|s| (&*s.content, s.style)).collect();
        assert_eq!(
            got,
            [
                ("Subject ", Style::default().fg(Color::Cyan)),
                (" ", Style::default().fg(Color::Cyan)),
                ("x ", Style::default()),
                (
                    "b",
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Rgb(1, 2, 3))
                ),
            ]
        );
        assert_eq!(line("plain").spans[0].content, "plain");
        assert_eq!(
            line("\x1b[38;5;9mr").spans[0].style.fg,
            Some(Color::Indexed(9))
        );
    }
}
