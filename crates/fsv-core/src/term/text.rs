//! Rich-compatible text measurement and wrapping.

use unicode_width::UnicodeWidthChar;

use super::markup::{Style, Text};
use crate::pystr::is_space;

/// One rendered line: characters with their resolved styles.
pub type Line = Vec<(char, Style)>;

/// Terminal cell width of a character.
pub fn char_width(c: char) -> usize {
    if c < ' ' { 0 } else { c.width().unwrap_or(0) }
}

/// Terminal cell width of a string.
pub fn cell_len(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn line_cells(line: &[(char, Style)]) -> usize {
    line.iter().map(|(c, _)| char_width(*c)).sum()
}

/// Horizontal alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    #[default]
    Left,
    Center,
    Right,
}

/// What to do with words longer than the width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overflow {
    /// Truncate with `…` (Rich's column default).
    #[default]
    Ellipsis,
    /// Break the word across lines.
    Fold,
    /// Hard crop.
    Crop,
}

/// `(minimum, maximum)` widths (longest word, longest line) of plain text.
pub fn measure(plain: &str) -> (usize, usize) {
    let max = plain.lines().map(cell_len).max().unwrap_or(0);
    let min = plain.split(is_space).filter(|w| !w.is_empty()).map(cell_len).max().unwrap_or(max);
    (min, max)
}

/// Crop or pad `line` to exactly `total` cells (Rich `set_cell_size`).
fn set_cell_size(line: &mut Line, total: usize, pad_style: Style) {
    let mut cells = 0;
    let mut keep = 0;
    for (c, _) in line.iter() {
        let w = char_width(*c);
        if cells + w > total {
            break;
        }
        cells += w;
        keep += 1;
    }
    line.truncate(keep);
    while cells < total {
        line.push((' ', pad_style));
        cells += 1;
    }
}

fn truncate(line: &mut Line, width: usize, overflow: Overflow, pad: bool, pad_style: Style) {
    let length = line_cells(line);
    if length > width {
        match overflow {
            Overflow::Ellipsis => {
                set_cell_size(line, width.saturating_sub(1), pad_style);
                let style = line.last().map_or(pad_style, |(_, s)| *s);
                if width > 0 {
                    line.push(('…', style));
                }
            }
            Overflow::Fold | Overflow::Crop => set_cell_size(line, width, pad_style),
        }
    }
    if pad && length < width {
        for _ in length..width {
            line.push((' ', pad_style));
        }
    }
}

/// Break offsets for one paragraph (Rich `divide_line`).
fn divide_line(chars: &[char], width: usize, fold: bool) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut cell_offset = 0usize;
    let n = chars.len();
    let mut pos = 0;
    while pos < n {
        // word = \s*\S+\s*
        let start = pos;
        let mut i = pos;
        while i < n && is_space(chars[i]) {
            i += 1;
        }
        if i == n {
            break;
        }
        while i < n && !is_space(chars[i]) {
            i += 1;
        }
        let word_end_nonspace = i;
        while i < n && is_space(chars[i]) {
            i += 1;
        }
        let end = i;
        pos = end;
        let word = &chars[start..end];
        let word_len: usize = chars[start..word_end_nonspace].iter().map(|c| char_width(*c)).sum();
        let full_len: usize = word.iter().map(|c| char_width(*c)).sum();
        if width.saturating_sub(cell_offset) >= word_len && width >= cell_offset {
            cell_offset += full_len;
        } else if word_len > width {
            if fold {
                // chop the word into width-sized chunks
                let mut chunks: Vec<(usize, usize)> = Vec::new();
                let mut cs = start;
                let mut cw = 0;
                for (k, c) in word.iter().enumerate() {
                    let w = char_width(*c);
                    if cw + w > width && cw > 0 {
                        chunks.push((cs, start + k));
                        cs = start + k;
                        cw = 0;
                    }
                    cw += w;
                }
                chunks.push((cs, end));
                let mut s = start;
                let last = chunks.len() - 1;
                for (idx, (a, b)) in chunks.iter().enumerate() {
                    if s > 0 {
                        breaks.push(s);
                    }
                    if idx == last {
                        cell_offset = chars[*a..*b].iter().map(|c| char_width(*c)).sum();
                    } else {
                        s += b - a;
                    }
                }
            } else {
                if start > 0 {
                    breaks.push(start);
                }
                cell_offset = full_len;
            }
        } else if cell_offset > 0 && start > 0 {
            breaks.push(start);
            cell_offset = full_len;
        }
    }
    breaks
}

fn rstrip_end(line: &mut Line, size: usize) {
    if line.len() > size {
        let excess = line.len() - size;
        let ws = line.iter().rev().take_while(|(c, _)| is_space(*c)).count();
        let crop = ws.min(excess);
        line.truncate(line.len() - crop);
    }
}

fn rstrip(line: &mut Line) {
    while line.last().is_some_and(|(c, _)| is_space(*c)) {
        line.pop();
    }
}

/// Wrap styled text to `width` cells, each output line padded to `width`.
pub fn wrap(text: &Text, base: Style, width: usize, justify: Justify, overflow: Overflow, no_wrap: bool) -> Vec<Line> {
    let styles = text.char_styles(base);
    let chars: Vec<char> = text.plain.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut paragraphs: Vec<(usize, usize)> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            paragraphs.push((start, i));
            start = i + 1;
        }
    }
    paragraphs.push((start, chars.len()));
    for (a, b) in paragraphs {
        let para: Line = chars[a..b].iter().copied().map(expand_tab).zip(styles[a..b].iter().copied()).collect();
        let pieces: Vec<Line> = if no_wrap {
            vec![para]
        } else {
            let plain: Vec<char> = para.iter().map(|(c, _)| *c).collect();
            let offsets = divide_line(&plain, width, overflow == Overflow::Fold);
            let mut v = Vec::new();
            let mut s = 0;
            for o in offsets.into_iter().chain(std::iter::once(para.len())) {
                v.push(para[s..o].to_vec());
                s = o;
            }
            v
        };
        for mut line in pieces {
            rstrip_end(&mut line, width);
            match justify {
                Justify::Left => truncate(&mut line, width, overflow, true, base),
                Justify::Center => {
                    rstrip(&mut line);
                    truncate(&mut line, width, overflow, false, base);
                    let len = line_cells(&line);
                    let left = (width.saturating_sub(len)) / 2;
                    let mut padded: Line = std::iter::repeat_n((' ', base), left).collect();
                    padded.extend(line);
                    let len = line_cells(&padded);
                    padded.extend(std::iter::repeat_n((' ', base), width.saturating_sub(len)));
                    line = padded;
                }
                Justify::Right => {
                    rstrip(&mut line);
                    truncate(&mut line, width, overflow, false, base);
                    let len = line_cells(&line);
                    let mut padded: Line = std::iter::repeat_n((' ', base), width.saturating_sub(len)).collect();
                    padded.extend(line);
                    line = padded;
                }
            }
            truncate(&mut line, width, overflow, false, base);
            out.push(line);
        }
    }
    out
}

fn expand_tab(c: char) -> char {
    if c == '\t' { ' ' } else { c }
}

/// Lines of `text` without wrapping (for plain `console.print`).
pub fn lines(text: &Text, base: Style) -> Vec<Line> {
    let styles = text.char_styles(base);
    let mut out = vec![Vec::new()];
    for (c, s) in text.plain.chars().zip(styles) {
        if c == '\n' {
            out.push(Vec::new());
        } else if let Some(last) = out.last_mut() {
            last.push((c, s));
        }
    }
    out
}

/// Render a line to a string, with SGR codes when `color`.
pub fn to_ansi(line: &[(char, Style)], color: bool) -> String {
    let mut out = String::new();
    if !color {
        out.extend(line.iter().map(|(c, _)| *c));
        return out;
    }
    let mut i = 0;
    while i < line.len() {
        let style = line[i].1;
        let mut j = i;
        while j < line.len() && line[j].1 == style {
            j += 1;
        }
        let chunk: String = line[i..j].iter().map(|(c, _)| *c).collect();
        let sgr = style.sgr();
        if sgr.is_empty() {
            out.push_str(&chunk);
        } else {
            out.push_str(&format!("\x1b[{sgr}m{chunk}\x1b[0m"));
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.iter().map(|(c, _)| *c).collect()).collect()
    }

    #[test]
    fn wraps_words_and_pads() {
        let t = Text::plain("hello brave new world");
        let l = wrap(&t, Style::default(), 11, Justify::Left, Overflow::Ellipsis, false);
        assert_eq!(plain(&l), vec!["hello brave", "new world  "]);
        let l = wrap(&Text::plain("abcdefghij"), Style::default(), 4, Justify::Left, Overflow::Fold, false);
        assert_eq!(plain(&l), vec!["abcd", "efgh", "ij  "]);
        let l = wrap(&Text::plain("abcdefghij"), Style::default(), 4, Justify::Left, Overflow::Ellipsis, false);
        assert_eq!(plain(&l), vec!["abc…"]);
        let l = wrap(&Text::plain("ab"), Style::default(), 6, Justify::Center, Overflow::Ellipsis, false);
        assert_eq!(plain(&l), vec!["  ab  "]);
    }
}
