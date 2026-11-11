//! Drawing helpers: wrapped lines, buttons, dialog frames, toasts.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthChar;

use super::theme::Theme;

type Cells = Vec<(char, Style)>;

fn to_spans(cells: &[(char, Style)]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = vec![];
    let mut cur = String::new();
    let mut style = None;
    for &(ch, st) in cells {
        if style.is_some_and(|s| s != st) {
            spans.push(Span::styled(
                std::mem::take(&mut cur),
                style.unwrap_or_default(),
            ));
        }
        style = Some(st);
        cur.push(ch);
    }
    if !cur.is_empty() {
        spans.push(Span::styled(cur, style.unwrap_or_default()));
    }
    spans
}

fn cells_width(cells: &[(char, Style)]) -> usize {
    cells.iter().map(|(c, _)| c.width().unwrap_or(0)).sum()
}

/// Wrap a line at spaces into rows no wider than `width`; a word longer
/// than a row is folded (Rich's default wrapping).
pub fn wrap_line(line: &Line<'static>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    // words with the spaces before them
    let mut words: Vec<(Cells, Cells)> = vec![];
    let mut i = 0;
    while i < cells.len() {
        let start = i;
        while i < cells.len() && cells[i].0 == ' ' {
            i += 1;
        }
        let gap = cells[start..i].to_vec();
        let wstart = i;
        while i < cells.len() && cells[i].0 != ' ' {
            i += 1;
        }
        words.push((gap, cells[wstart..i].to_vec()));
    }
    let mut rows: Vec<Vec<(char, Style)>> = vec![];
    let mut row: Vec<(char, Style)> = vec![];
    for (gap, word) in words {
        let (gw, ww) = (cells_width(&gap), cells_width(&word));
        let rw = cells_width(&row);
        if rw + gw + ww <= width {
            row.extend(gap);
            row.extend(word);
            continue;
        }
        if !row.is_empty() && ww <= width {
            rows.push(std::mem::take(&mut row));
            row = word;
            continue;
        }
        // a word longer than a row starts its own row and is folded
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
        } else {
            row.extend(gap.into_iter().take(width.saturating_sub(1)));
        }
        for c in word {
            let cw = c.0.width().unwrap_or(0);
            if cells_width(&row) + cw > width && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            row.push(c);
        }
    }
    rows.push(row);
    rows.into_iter()
        .map(|r| Line::from(to_spans(&r)).style(line.style))
        .collect()
}

pub fn wrap_lines(lines: &[Line<'static>], width: u16) -> Vec<Line<'static>> {
    lines.iter().flat_map(|l| wrap_line(l, width)).collect()
}

/// Draw pre-wrapped lines from `skip`.
pub fn draw_lines(area: Rect, buf: &mut Buffer, lines: &[Line<'static>], skip: usize) {
    for (i, l) in lines
        .iter()
        .skip(skip)
        .take(usize::from(area.height))
        .enumerate()
    {
        buf.set_line(area.x, area.y + i as u16, l, area.width);
    }
}

pub fn fill(area: Rect, buf: &mut Buffer, style: Style) {
    buf.set_style(area, style);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y))
                && cell.symbol().is_empty()
            {
                cell.set_symbol(" ");
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Default,
    Primary,
    Success,
    Error,
}

/// A Textual-style button (3 rows).
#[derive(Clone, Copy, Debug)]
pub struct Button<'a> {
    pub label: &'a str,
    pub variant: Variant,
    pub focused: bool,
    pub disabled: bool,
}

impl Button<'_> {
    /// Draw at (`x`, `y`); returns the width.
    pub fn draw(&self, x: u16, y: u16, buf: &mut Buffer, theme: &Theme) -> u16 {
        button(x, y, buf, *self, theme)
    }
}

fn button(x: u16, y: u16, buf: &mut Buffer, b: Button<'_>, theme: &Theme) -> u16 {
    let Button {
        label,
        variant,
        focused,
        disabled,
    } = b;
    let w = label.chars().count() as u16 + 4;
    let bg = match variant {
        Variant::Default => theme.panel,
        Variant::Primary => theme.primary,
        Variant::Success => theme.success,
        Variant::Error => theme.error,
    };
    let mut style = Style::default()
        .bg(bg)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    if disabled {
        style = style
            .add_modifier(Modifier::DIM)
            .remove_modifier(Modifier::BOLD);
    }
    if focused {
        style = style.add_modifier(Modifier::REVERSED);
    }
    let area = Rect::new(x, y, w, 3).intersection(buf.area);
    fill(area, buf, style);
    if area.height >= 2 {
        buf.set_stringn(
            x + 2,
            y + 1,
            label,
            usize::from(area.width.saturating_sub(2)),
            style,
        );
    }
    w
}

/// Width of [`button`] for `label`.
pub fn button_width(label: &str) -> u16 {
    label.chars().count() as u16 + 4
}

/// A centered dialog box `w` wide (at most `max_pct` % of the screen) and
/// `h` tall; clears it and returns the inner area.
pub fn dialog(area: Rect, buf: &mut Buffer, w: u16, max_pct: u16, h: u16, theme: &Theme) -> Rect {
    let w = w.min(area.width * max_pct / 100).max(10).min(area.width);
    let h = h.min(area.height);
    let r = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    dim_background(area, buf);
    Clear.render(r, buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(theme.primary))
        .style(Style::default().bg(theme.surface).fg(theme.foreground));
    let inner = block.inner(r);
    block.render(r, buf);
    Rect::new(
        inner.x + 2,
        inner.y + 1,
        inner.width.saturating_sub(4),
        inner.height.saturating_sub(2),
    )
}

/// The screen behind a modal, dimmed.
pub fn dim_background(area: Rect, buf: &mut Buffer) {
    buf.set_style(area, Style::default().add_modifier(Modifier::DIM));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Information,
    Warning,
    Error,
}

/// A `notify` message.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub text: String,
    pub title: String,
    pub severity: Severity,
    pub until: Duration,
}

/// Stack toasts in the bottom-right corner, newest at the bottom.
pub fn toasts(area: Rect, buf: &mut Buffer, list: &[Toast], theme: &Theme) {
    let width = 50.min(area.width);
    let mut bottom = area.bottom().saturating_sub(1);
    for t in list.iter().rev() {
        let mut lines: Vec<Line<'static>> = vec![];
        if !t.title.is_empty() {
            lines.push(Line::styled(
                t.title.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        for l in t.text.split('\n') {
            lines.extend(wrap_line(
                &Line::from(l.to_string()),
                width.saturating_sub(4),
            ));
        }
        let h = lines.len() as u16 + 2;
        if bottom < area.y + h {
            break;
        }
        let r = Rect::new(area.right() - width, bottom - h, width, h);
        let color = match t.severity {
            Severity::Information => theme.primary,
            Severity::Warning => theme.warning,
            Severity::Error => theme.error,
        };
        Clear.render(r, buf);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::LEFT | Borders::RIGHT)
                    .border_type(BorderType::Thick)
                    .border_style(Style::default().fg(color))
                    .style(Style::default().bg(theme.panel).fg(theme.foreground)),
            )
            .render(Rect::new(r.x, r.y + 1, r.width, r.height - 2), buf);
        bottom = r.y;
    }
}

/// The text of a buffer row (tests, snapshots).
pub fn row_text(buf: &Buffer, y: u16) -> String {
    let a = buf.area;
    (a.left()..a.right())
        .map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()).to_string())
        .collect::<String>()
}

/// The whole buffer as text, rows joined by `\n`, trailing spaces cut.
pub fn buffer_text(buf: &Buffer) -> String {
    (buf.area.top()..buf.area.bottom())
        .map(|y| row_text(buf, y).trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(rows: &[Line<'static>]) -> Vec<String> {
        rows.iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn fold_keeps_styles() {
        let l = Line::from(vec![
            Span::raw("abc"),
            Span::styled("defg", Style::default().fg(Color::Red)),
        ]);
        let rows = wrap_line(&l, 4);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].spans[1].content, "d");
        assert_eq!(rows[1].spans[0].content, "efg");
        assert_eq!(rows[1].spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn wraps_at_spaces() {
        let l = Line::from("Type to search · move · Enter picks");
        assert_eq!(
            texts(&wrap_line(&l, 16)),
            vec!["Type to search ·", "move · Enter", "picks"]
        );
        assert_eq!(
            texts(&wrap_line(&Line::from("ab abcdefgh"), 4)),
            vec!["ab", "abcd", "efgh"]
        );
        assert_eq!(texts(&wrap_line(&Line::from(""), 4)), vec![""]);
        assert_eq!(texts(&wrap_line(&Line::from("  x"), 4)), vec!["  x"]);
    }
}
