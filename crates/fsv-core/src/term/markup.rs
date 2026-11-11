//! Rich console markup (`[bold red]x[/]`) → styled text.

use std::sync::LazyLock;

use regex::Regex;

/// A terminal color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Standard (0-7) or bright (8-15) palette entry.
    Ansi(u8),
    /// 256-color palette entry.
    Indexed(u8),
    Rgb(u8, u8, u8),
    /// Terminal default.
    Default,
}

/// Attributes and colors of a span (`None` = inherit).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: Option<bool>,
    pub dim: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub blink: Option<bool>,
    pub reverse: Option<bool>,
    pub conceal: Option<bool>,
    pub strike: Option<bool>,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

const NAMES: &[&str] = &[
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

fn parse_color(word: &str) -> Option<Color> {
    let w = word.to_ascii_lowercase();
    if w == "default" {
        return Some(Color::Default);
    }
    if let Some(i) = NAMES.iter().position(|n| *n == w) {
        return Some(Color::Ansi(i as u8));
    }
    if let Some(rest) = w.strip_prefix("bright_")
        && let Some(i) = NAMES.iter().position(|n| *n == rest)
    {
        return Some(Color::Ansi(i as u8 + 8));
    }
    if matches!(w.as_str(), "grey" | "gray") {
        return Some(Color::Ansi(8));
    }
    if let Some(hex) = w.strip_prefix('#') {
        if hex.len() == 6 {
            let p = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            return Some(Color::Rgb(p(0)?, p(2)?, p(4)?));
        }
        return None;
    }
    if let Some(n) = w.strip_prefix("color(").and_then(|r| r.strip_suffix(')')) {
        let n: u8 = n.trim().parse().ok()?;
        return Some(if n < 16 {
            Color::Ansi(n)
        } else {
            Color::Indexed(n)
        });
    }
    if let Some(n) = w.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<u8> = n.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if let [r, g, b] = parts[..] {
            return Some(Color::Rgb(r, g, b));
        }
    }
    None
}

impl Style {
    /// Parse a Rich style definition (`"bold cyan"`, `"red on white"`, `"not dim"`).
    /// Unknown words are ignored.
    pub fn parse(def: &str) -> Style {
        let mut s = Style::default();
        let mut words = def.split_whitespace();
        while let Some(word) = words.next() {
            let lw = word.to_ascii_lowercase();
            match lw.as_str() {
                "on" => {
                    if let Some(c) = words.next().and_then(parse_color) {
                        s.bg = Some(c);
                    }
                }
                "not" => {
                    if let Some(attr) = words.next() {
                        s.set_attr(&attr.to_ascii_lowercase(), false);
                    }
                }
                "link" => {
                    words.next();
                }
                _ => {
                    if !s.set_attr(&lw, true)
                        && let Some(c) = parse_color(&lw)
                    {
                        s.fg = Some(c);
                    }
                }
            }
        }
        s
    }

    fn set_attr(&mut self, name: &str, on: bool) -> bool {
        let slot = match name {
            "bold" | "b" => &mut self.bold,
            "dim" | "d" => &mut self.dim,
            "italic" | "i" => &mut self.italic,
            "underline" | "u" => &mut self.underline,
            "blink" => &mut self.blink,
            "reverse" | "r" => &mut self.reverse,
            "conceal" | "c" => &mut self.conceal,
            "strike" | "s" => &mut self.strike,
            _ => return false,
        };
        *slot = Some(on);
        true
    }

    /// `self + other`: `other` wins where set.
    pub fn combine(&self, other: &Style) -> Style {
        Style {
            bold: other.bold.or(self.bold),
            dim: other.dim.or(self.dim),
            italic: other.italic.or(self.italic),
            underline: other.underline.or(self.underline),
            blink: other.blink.or(self.blink),
            reverse: other.reverse.or(self.reverse),
            conceal: other.conceal.or(self.conceal),
            strike: other.strike.or(self.strike),
            fg: other.fg.or(self.fg),
            bg: other.bg.or(self.bg),
        }
    }

    /// SGR parameters (`"1;36"`), empty for a null style.
    pub fn sgr(&self) -> String {
        let mut codes: Vec<String> = Vec::new();
        for (flag, code) in [
            (self.bold, "1"),
            (self.dim, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
            (self.blink, "5"),
            (self.reverse, "7"),
            (self.conceal, "8"),
            (self.strike, "9"),
        ] {
            if flag == Some(true) {
                codes.push(code.into());
            }
        }
        let color = |c: Color, base: u8| -> String {
            match c {
                Color::Ansi(n) if n < 8 => format!("{}", base + n),
                Color::Ansi(n) => format!("{}", base + 60 + (n - 8)),
                Color::Indexed(n) => format!("{};5;{n}", base + 8),
                Color::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", base + 8),
                Color::Default => format!("{}", base + 9),
            }
        };
        if let Some(c) = self.fg {
            codes.push(color(c, 30));
        }
        if let Some(c) = self.bg {
            codes.push(color(c, 40));
        }
        codes.join(";")
    }
}

/// A styled span over char indices `[start, end)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

/// Plain text with styled spans (Rich `Text`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub plain: String,
    pub spans: Vec<Span>,
}

impl Text {
    pub fn plain(s: &str) -> Text {
        Text {
            plain: s.to_string(),
            spans: Vec::new(),
        }
    }

    /// Plain text styled as a whole.
    pub fn styled(s: &str, style: Style) -> Text {
        let n = s.chars().count();
        let spans = if n > 0 && style != Style::default() {
            vec![Span {
                start: 0,
                end: n,
                style,
            }]
        } else {
            Vec::new()
        };
        Text {
            plain: s.to_string(),
            spans,
        }
    }

    /// Per-character styles (spans applied in order over `base`).
    pub fn char_styles(&self, base: Style) -> Vec<Style> {
        let n = self.plain.chars().count();
        let mut out = vec![base; n];
        for span in &self.spans {
            for s in out.iter_mut().take(span.end.min(n)).skip(span.start) {
                *s = s.combine(&span.style);
            }
        }
        out
    }
}

static TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"((\\*)\[([a-z#/@][^\[]*?)])").expect("regex"));

/// `rich.markup.escape`: backslash-escape anything that looks like a tag.
pub fn escape(markup: &str) -> String {
    static ESC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(\\*)(\[[a-z#/@][^\[]*?])").expect("regex"));
    let out = ESC.replace_all(markup, |c: &regex::Captures<'_>| {
        let bs = c.get(1).map_or("", |m| m.as_str());
        let text = c.get(2).map_or("", |m| m.as_str());
        format!("{bs}{bs}\\{text}")
    });
    if out.ends_with('\\') && !out.ends_with("\\\\") {
        format!("{out}\\")
    } else {
        out.into_owned()
    }
}

fn normalize_style_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Parse Rich markup. Unknown tags vanish like in Rich; an unmatched closing
/// tag is dropped (Rich raises `MarkupError`).
pub fn render(markup: &str) -> Text {
    if !markup.contains('[') {
        return Text::plain(markup);
    }
    let mut plain = String::new();
    let mut len = 0usize;
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let push_text = |plain: &mut String, len: &mut usize, s: &str| {
        let s = s.replace("\\[", "[");
        *len += s.chars().count();
        plain.push_str(&s);
    };
    let mut position = 0;
    for caps in TAGS.captures_iter(markup) {
        let Some(m) = caps.get(0) else { continue };
        let full = m.as_str();
        let escapes = caps.get(2).map_or("", |m| m.as_str());
        let tag = caps.get(3).map_or("", |m| m.as_str());
        if m.start() > position {
            push_text(&mut plain, &mut len, &markup[position..m.start()]);
        }
        position = m.end();
        if !escapes.is_empty() {
            let (backslashes, escaped) = (escapes.len() / 2, escapes.len() % 2);
            if backslashes > 0 {
                let lit = "\\".repeat(backslashes);
                len += backslashes;
                plain.push_str(&lit);
            }
            if escaped == 1 {
                let lit = &full[escapes.len()..];
                len += lit.chars().count();
                plain.push_str(lit);
                continue;
            }
        }
        if let Some(close) = tag.strip_prefix('/') {
            let name = normalize_style_name(close);
            let found = if name.is_empty() {
                stack.pop()
            } else {
                stack
                    .iter()
                    .rposition(|(_, t)| normalize_style_name(t) == name)
                    .map(|i| stack.remove(i))
            };
            if let Some((start, open)) = found
                && !open.starts_with('@')
            {
                let style = Style::parse(&open.replacen('=', " ", 1));
                spans.push(Span {
                    start,
                    end: len,
                    style,
                });
            }
        } else {
            stack.push((len, tag.to_string()));
        }
    }
    if position < markup.len() {
        push_text(&mut plain, &mut len, &markup[position..]);
    }
    while let Some((start, open)) = stack.pop() {
        if !open.starts_with('@') {
            spans.push(Span {
                start,
                end: len,
                style: Style::parse(&open.replacen('=', " ", 1)),
            });
        }
    }
    spans.reverse();
    spans.sort_by_key(|s| s.start);
    Text { plain, spans }
}

/// Markup with tags removed (what Rich prints without a terminal).
pub fn strip(markup: &str) -> String {
    render(markup).plain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_like_rich() {
        assert_eq!(strip("[foo]bar[/foo] x"), "bar x");
        assert_eq!(strip("[1] a"), "[1] a");
        assert_eq!(strip("list[int]"), "list");
        assert_eq!(strip("[bold]b[/] [dim]d"), "b d");
        assert_eq!(strip("a \\[b] c"), "a [b] c");
        assert_eq!(strip("[link=http://x]l[/link]"), "l");
        let t = render("[bold cyan]a[/] b");
        assert_eq!(t.spans[0].style.sgr(), "1;36");
    }

    #[test]
    fn escape_like_rich() {
        assert_eq!(escape("[bold]x"), "\\[bold]x");
        assert_eq!(escape("[1]"), "[1]");
        assert_eq!(escape("a\\"), "a\\\\");
        assert_eq!(strip(&escape("list[int] \\[x]")), "list[int] \\[x]");
    }
}
