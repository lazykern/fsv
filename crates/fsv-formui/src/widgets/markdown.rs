//! The description preview: Markdown laid out as Textual's `Markdown`
//! widget does (block margins collapse, `• ` bullets, centred `#`).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::kit::Theme;
use crate::kit::draw::wrap_line;

enum Block {
    Para(String),
    Heading(usize, String),
    List(Vec<(String, String)>),
    Fence(Vec<String>),
}

struct Colors {
    marker: Color,
    code: (Color, Color),
    muted: Color,
    fence: (Color, Color),
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn colors(theme: &Theme) -> Colors {
    if theme.name == "textual-light" {
        Colors {
            marker: rgb(0x004F8B),
            code: (rgb(0x7E2F43), rgb(0xDDD8D9)),
            muted: rgb(0x6C6C6C),
            fence: (rgb(0x000000), rgb(0xE9E9E9)),
        }
    } else {
        Colors {
            marker: rgb(0x57A5E2),
            code: (rgb(0xF4BB6E), rgb(0x241D13)),
            muted: rgb(0x8D8D8D),
            fence: (rgb(0xFFFFFF), rgb(0x101010)),
        }
    }
}

fn heading(line: &str) -> Option<(usize, String)> {
    let t = line.trim_start();
    let n = t.chars().take_while(|&c| c == '#').count();
    let rest = &t[n..];
    if !(1..=6).contains(&n) || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim_end();
    Some((n, text.to_string()))
}

/// `(marker kind, number, text)` of a list item line.
fn item(line: &str) -> Option<(String, Option<u64>, String)> {
    let t = line.trim_start();
    let mut cs = t.chars();
    let first = cs.next()?;
    if matches!(first, '-' | '*' | '+') {
        let rest = &t[1..];
        if rest.starts_with([' ', '\t']) {
            return Some((first.to_string(), None, rest.trim().to_string()));
        }
        return None;
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if !(1..=9).contains(&digits) {
        return None;
    }
    let delim = t[digits..].chars().next()?;
    let rest = &t[digits + 1..];
    if matches!(delim, '.' | ')') && rest.starts_with([' ', '\t']) {
        let n = t[..digits].parse().ok()?;
        return Some((delim.to_string(), Some(n), rest.trim().to_string()));
    }
    None
}

fn join(text: &mut String, line: &str) {
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(line);
}

fn blocks(md: &str) -> Vec<Block> {
    let mut out: Vec<Block> = vec![];
    let mut kind: Option<String> = None;
    let mut open = false;
    let mut fence: Option<(String, Vec<String>)> = None;
    for line in md.lines() {
        if let Some((mark, body)) = fence.as_mut() {
            if line.trim_start().starts_with(mark.as_str()) {
                out.push(Block::Fence(std::mem::take(body)));
                fence = None;
            } else {
                body.push(line.to_string());
            }
            continue;
        }
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some((t[..3].to_string(), vec![]));
            open = false;
            kind = None;
            continue;
        }
        if t.is_empty() {
            open = false;
            kind = None;
            continue;
        }
        if let Some((n, text)) = heading(line) {
            out.push(Block::Heading(n, text));
            open = false;
            kind = None;
            continue;
        }
        if let Some((k, n, text)) = item(line) {
            let marker = n.map_or_else(String::new, |n| format!("{n}{k}"));
            if open
                && kind.as_deref() == Some(k.as_str())
                && let Some(Block::List(items)) = out.last_mut()
            {
                items.push((marker, text));
                continue;
            }
            out.push(Block::List(vec![(marker, text)]));
            kind = Some(k);
            open = true;
            continue;
        }
        match out.last_mut() {
            Some(Block::Para(text)) if open => join(text, line.trim()),
            Some(Block::List(items)) if open => {
                if let Some((_, text)) = items.last_mut() {
                    join(text, line.trim());
                }
            }
            _ => {
                out.push(Block::Para(line.trim().to_string()));
                kind = None;
                open = true;
            }
        }
    }
    if let Some((_, body)) = fence {
        out.push(Block::Fence(body));
    }
    out
}

fn closing(chars: &[char], from: usize, mark: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(mark.len() - 1))
        .find(|&j| chars[j..j + mark.len()] == *mark && j > from && chars[j - 1] != ' ')
}

/// Inline Markdown: `code`, **bold**, *italic*, [link](url); a line
/// break inside a block is a space unless the line ended in two spaces.
fn inline(text: &str, base: Style, c: &Colors, out: &mut Vec<Span<'static>>) {
    let chars: Vec<char> = text.chars().collect();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<Span<'static>>| {
        if !cur.is_empty() {
            out.push(Span::styled(std::mem::take(cur), base));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let prev_word = i > 0 && chars[i - 1].is_alphanumeric();
        if ch == '`'
            && let Some(j) = (i + 1..chars.len()).find(|&j| chars[j] == '`')
        {
            flush(&mut cur, out);
            let code: String = chars[i + 1..j].iter().collect();
            out.push(Span::styled(code, base.fg(c.code.0).bg(c.code.1)));
            i = j + 1;
            continue;
        }
        if (ch == '*' || ch == '_') && !(ch == '_' && prev_word) {
            let double = chars.get(i + 1) == Some(&ch);
            let mark: &[char] = if double { &[ch, ch] } else { &[ch] };
            let start = i + mark.len();
            let opens = chars.get(start).is_some_and(|c| !c.is_whitespace());
            if let Some(j) = closing(&chars, start, mark).filter(|_| opens) {
                flush(&mut cur, out);
                let inner: String = chars[start..j].iter().collect();
                let m = if double {
                    Modifier::BOLD
                } else {
                    Modifier::ITALIC
                };
                inline(&inner, base.add_modifier(m), c, out);
                i = j + mark.len();
                continue;
            }
        }
        if ch == '[' {
            let close = (i + 1..chars.len()).find(|&j| chars[j] == ']');
            if let Some(j) = close.filter(|&j| chars.get(j + 1) == Some(&'('))
                && let Some(k) = (j + 2..chars.len()).find(|&k| chars[k] == ')')
            {
                flush(&mut cur, out);
                let label: String = chars[i + 1..j].iter().collect();
                inline(&label, base.add_modifier(Modifier::UNDERLINED), c, out);
                i = k + 1;
                continue;
            }
        }
        if ch == '\n' {
            let hard = cur.ends_with("  ") || cur.ends_with('\\');
            if hard {
                let keep = cur.trim_end_matches([' ', '\\']).len();
                cur.truncate(keep);
                cur.push('\n');
            } else {
                let keep = cur.trim_end().len();
                cur.truncate(keep);
                cur.push(' ');
            }
            i += 1;
            continue;
        }
        cur.push(ch);
        i += 1;
    }
    flush(&mut cur, out);
}

/// Spans split at hard line breaks.
fn rows(spans: Vec<Span<'static>>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::default()];
    for s in spans {
        let mut parts = s.content.split('\n');
        if let Some(first) = parts.next().filter(|p| !p.is_empty()) {
            lines
                .last_mut()
                .unwrap()
                .spans
                .push(Span::styled(first.to_string(), s.style));
        }
        for p in parts {
            lines.push(Line::from(Span::styled(p.to_string(), s.style)));
        }
    }
    lines
}

fn wrapped(text: &str, base: Style, c: &Colors, width: u16) -> Vec<Line<'static>> {
    let mut spans = vec![];
    inline(text, base, c, &mut spans);
    rows(spans)
        .iter()
        .flat_map(|l| wrap_line(l, width))
        .collect()
}

/// `(top, bottom)` margins of a block in rows.
fn margins(b: &Block) -> (usize, usize) {
    match b {
        Block::Heading(1 | 2, _) => (2, 1),
        Block::Heading(..) | Block::Fence(_) => (1, 1),
        Block::Para(_) | Block::List(_) => (0, 1),
    }
}

fn body(b: &Block, width: u16, theme: &Theme, c: &Colors) -> Vec<Line<'static>> {
    let fg = Style::default().fg(theme.foreground);
    match b {
        Block::Para(text) => wrapped(text, fg, c, width),
        Block::Heading(n, text) => {
            let st = match n {
                1 => Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
                2 => Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::UNDERLINED),
                3 => Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
                4 => fg.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                5 => fg.add_modifier(Modifier::BOLD),
                _ => Style::default().fg(c.muted).add_modifier(Modifier::BOLD),
            };
            let lines = wrapped(text, st, c, width);
            if *n != 1 {
                return lines;
            }
            lines
                .into_iter()
                .map(|mut l| {
                    let left = usize::from(width).saturating_sub(l.width()) / 2;
                    l.spans.insert(0, Span::raw(" ".repeat(left)));
                    l
                })
                .collect()
        }
        Block::List(items) => {
            let ordered = items.first().is_some_and(|(m, _)| !m.is_empty());
            let pad = items.iter().map(|(m, _)| m.width()).max().unwrap_or(0) + 1;
            let mut out = vec![];
            for (m, text) in items {
                let marker = if ordered {
                    format!("{m:>pad$} ")
                } else {
                    "• ".to_string()
                };
                let mw = marker.width();
                let rows = wrapped(text, fg, c, width.saturating_sub(mw as u16).max(1));
                for (i, mut l) in rows.into_iter().enumerate() {
                    let lead = if i == 0 {
                        Span::styled(marker.clone(), Style::default().fg(c.marker))
                    } else {
                        Span::raw(" ".repeat(mw))
                    };
                    l.spans.insert(0, lead);
                    out.push(l);
                }
            }
            out
        }
        Block::Fence(code) => {
            let st = Style::default().fg(c.fence.0).bg(c.fence.1);
            let w = usize::from(width);
            let row = |text: &str| {
                let cut: String = format!("  {text}").chars().take(w).collect();
                let fill = w.saturating_sub(cut.width());
                Line::from(Span::styled(format!("{cut}{}", " ".repeat(fill)), st))
            };
            let mut out = vec![row("")];
            out.extend(code.iter().map(|l| row(l)));
            out.push(row(""));
            out
        }
    }
}

/// The preview of `md` in `width` columns; empty text reads `(empty)`.
pub fn markdown_lines(md: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let c = colors(theme);
    let md = if md.is_empty() { "_(empty)_" } else { md };
    let mut out: Vec<Line<'static>> = vec![];
    let mut gap = 0;
    for b in blocks(md) {
        let (top, bottom) = margins(&b);
        out.extend((0..gap.max(top)).map(|_| Line::default()));
        out.extend(body(&b, width, theme, &c));
        gap = bottom;
    }
    out.extend((0..gap).map(|_| Line::default()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn style_of(lines: &[Line<'_>], row: usize, word: &str) -> Style {
        lines[row]
            .spans
            .iter()
            .find(|s| s.content.contains(word))
            .map(|s| s.style)
            .unwrap()
    }

    /// The Textual preview of the description in the Python form (dark),
    /// 72 columns inside the border and padding.
    #[test]
    fn blocks_are_laid_out_like_textual() {
        let t = Theme::textual_dark();
        let md = "Rotate the **billing** keys, *now*.\n\n- one\n- two `code`\n\n\
                  1. first\n2. second\n\n# Head\n\nSee [docs](https://x.io).";
        let lines = markdown_lines(md, 72, &t);
        let got = text(&lines);
        let head = format!("{}Head", " ".repeat(34));
        assert_eq!(
            got,
            [
                "Rotate the billing keys, now.",
                "",
                "• one",
                "• two code",
                "",
                " 1. first",
                " 2. second",
                "",
                "",
                &head,
                "",
                "See docs.",
                "",
            ]
        );
        assert!(
            style_of(&lines, 0, "billing")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of(&lines, 0, "now")
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        assert_eq!(style_of(&lines, 2, "•").fg, Some(rgb(0x57A5E2)));
        assert_eq!(style_of(&lines, 2, "one").fg, Some(rgb(0xE0E0E0)));
        let code = style_of(&lines, 3, "code");
        assert_eq!(
            (code.fg, code.bg),
            (Some(rgb(0xF4BB6E)), Some(rgb(0x241D13)))
        );
        assert_eq!(style_of(&lines, 5, "1.").fg, Some(rgb(0x57A5E2)));
        let h = style_of(&lines, 9, "Head");
        assert_eq!(h.fg, Some(rgb(0x0178D4)));
        assert!(h.add_modifier.contains(Modifier::BOLD));
        assert!(
            style_of(&lines, 11, "docs")
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn headings_fences_and_lists_follow_textual() {
        let t = Theme::textual_dark();
        let md = "a\n```\nx = 1\n```\n## Two\n### Three\n#### Four";
        let lines = markdown_lines(md, 72, &t);
        let pad = " ".repeat(72);
        let x = format!("  x = 1{}", " ".repeat(65));
        let want = [
            "a", "", &pad, &x, &pad, "", "", "Two", "", "Three", "", "Four",
        ];
        assert_eq!(&text(&lines)[..12], want);
        for line in &lines[2..5] {
            assert_eq!(line.spans[0].style.bg, Some(rgb(0x101010)));
        }
        assert_eq!(style_of(&lines, 3, "x = 1").fg, Some(rgb(0xFFFFFF)));
        assert!(
            style_of(&lines, 7, "Two")
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        let three = style_of(&lines, 9, "Three");
        assert_eq!(three.fg, Some(rgb(0x0178D4)));
        assert!(three.add_modifier.contains(Modifier::BOLD));
        let four = style_of(&lines, 11, "Four");
        assert_eq!(four.fg, Some(rgb(0xE0E0E0)));
        assert!(
            four.add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );

        let md = "##### Five\n###### Six\n- a\nlazy cont\n* b\n3. c\n4. d";
        let lines = markdown_lines(md, 72, &t);
        assert_eq!(
            text(&lines),
            [
                "",
                "Five",
                "",
                "Six",
                "",
                "• a lazy cont",
                "",
                "• b",
                "",
                " 3. c",
                " 4. d",
                ""
            ]
        );
        assert!(
            style_of(&lines, 1, "Five")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        let six = style_of(&lines, 3, "Six");
        assert_eq!(six.fg, Some(rgb(0x8D8D8D)));
        assert!(six.add_modifier.contains(Modifier::BOLD));
        assert_eq!(style_of(&lines, 9, "3.").fg, Some(rgb(0x57A5E2)));
    }

    #[test]
    fn light_theme_and_empty_text() {
        let t = Theme::textual_light();
        let lines = markdown_lines("###### Six\n```\nx\n```\n- `c` *i* [l](u)", 72, &t);
        let pad = " ".repeat(72);
        let x = format!("  x{}", " ".repeat(69));
        assert_eq!(
            text(&lines),
            ["", "Six", "", &pad, &x, &pad, "", "• c i l", ""]
        );
        assert_eq!(style_of(&lines, 1, "Six").fg, Some(rgb(0x6C6C6C)));
        let f = style_of(&lines, 4, "x");
        assert_eq!((f.fg, f.bg), (Some(rgb(0x000000)), Some(rgb(0xE9E9E9))));
        assert_eq!(style_of(&lines, 7, "•").fg, Some(rgb(0x004F8B)));
        let code = lines[7].spans[1].style;
        assert_eq!(
            (code.fg, code.bg),
            (Some(rgb(0x7E2F43)), Some(rgb(0xDDD8D9)))
        );
        assert!(
            style_of(&lines, 7, "i")
                .add_modifier
                .contains(Modifier::ITALIC)
        );

        let empty = markdown_lines("", 72, &t);
        assert_eq!(text(&empty), ["(empty)", ""]);
        // `text or "_(empty)_"`: blank text is not empty, and renders nothing
        assert!(
            text(&markdown_lines("  \n ", 72, &t))
                .iter()
                .all(|l| l.trim().is_empty())
        );
        assert!(
            empty[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
    }
}
