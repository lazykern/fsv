//! A multi-line text editor with soft wrap (Textual `TextArea` with
//! `tab_behavior="focus"`: Tab is left to the screen).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Widget};
use unicode_width::UnicodeWidthChar;

use super::input::InputEvent;
use super::theme::Theme;

#[derive(Clone, Debug)]
pub struct TextArea {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    pub disabled: bool,
    scroll: usize,
}

impl Default for TextArea {
    fn default() -> Self {
        TextArea {
            lines: vec![vec![]],
            row: 0,
            col: 0,
            disabled: false,
            scroll: 0,
        }
    }
}

impl TextArea {
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `load_text`: replace everything, cursor at the start.
    pub fn load_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(|l| l.chars().collect()).collect();
        self.row = 0;
        self.col = 0;
        self.scroll = 0;
    }

    pub fn insert(&mut self, s: &str) {
        for ch in s.chars() {
            match ch {
                '\r' => {}
                '\n' => {
                    let rest = self.lines[self.row].split_off(self.col);
                    self.row += 1;
                    self.lines.insert(self.row, rest);
                    self.col = 0;
                }
                c => {
                    self.lines[self.row].insert(self.col, c);
                    self.col += 1;
                }
            }
        }
    }

    pub fn handle_key(&mut self, k: &KeyEvent) -> InputEvent {
        if self.disabled {
            return InputEvent::Ignored;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let len = |me: &Self| me.lines[me.row].len();
        match k.code {
            KeyCode::Enter => {
                self.insert("\n");
                InputEvent::Changed
            }
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                self.insert(&c.to_string());
                InputEvent::Changed
            }
            KeyCode::Backspace => {
                if self.col > 0 {
                    self.col -= 1;
                    self.lines[self.row].remove(self.col);
                } else if self.row > 0 {
                    let line = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = len(self);
                    self.lines[self.row].extend(line);
                } else {
                    return InputEvent::Handled;
                }
                InputEvent::Changed
            }
            KeyCode::Delete => {
                if self.col < len(self) {
                    self.lines[self.row].remove(self.col);
                } else if self.row + 1 < self.lines.len() {
                    let next = self.lines.remove(self.row + 1);
                    self.lines[self.row].extend(next);
                } else {
                    return InputEvent::Handled;
                }
                InputEvent::Changed
            }
            KeyCode::Char('k') if ctrl => {
                self.lines[self.row].truncate(self.col);
                InputEvent::Changed
            }
            KeyCode::Char('u') if ctrl => {
                self.lines[self.row].drain(..self.col);
                self.col = 0;
                InputEvent::Changed
            }
            KeyCode::Left => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = len(self);
                }
                InputEvent::Handled
            }
            KeyCode::Right => {
                if self.col < len(self) {
                    self.col += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
                InputEvent::Handled
            }
            KeyCode::Up => {
                self.row = self.row.saturating_sub(1);
                self.col = self.col.min(len(self));
                InputEvent::Handled
            }
            KeyCode::Down => {
                self.row = (self.row + 1).min(self.lines.len() - 1);
                self.col = self.col.min(len(self));
                InputEvent::Handled
            }
            KeyCode::Home => {
                self.col = 0;
                InputEvent::Handled
            }
            KeyCode::Char('a') if ctrl => {
                self.col = 0;
                InputEvent::Handled
            }
            KeyCode::End => {
                self.col = len(self);
                InputEvent::Handled
            }
            KeyCode::Char('e') if ctrl => {
                self.col = len(self);
                InputEvent::Handled
            }
            _ => InputEvent::Ignored,
        }
    }

    /// Wrapped rows: (line, start col, chars).
    fn wrapped(&self, width: usize) -> Vec<(usize, usize, Vec<char>)> {
        let mut out = vec![];
        for (li, line) in self.lines.iter().enumerate() {
            let mut start = 0;
            let mut cur = vec![];
            let mut w = 0;
            for (ci, ch) in line.iter().enumerate() {
                let cw = ch.width().unwrap_or(0);
                if w + cw > width && !cur.is_empty() {
                    out.push((li, start, std::mem::take(&mut cur)));
                    start = ci;
                    w = 0;
                }
                cur.push(*ch);
                w += cw;
            }
            out.push((li, start, cur));
        }
        out
    }

    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focused: bool,
        theme: &Theme,
    ) -> Option<Position> {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused {
                theme.border
            } else {
                theme.border_blurred
            }));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        let width = usize::from(inner.width.saturating_sub(1)).max(1);
        let rows = self.wrapped(width);
        let cur_row = rows
            .iter()
            .rposition(|(li, start, _)| *li == self.row && *start <= self.col)
            .unwrap_or(0);
        let height = usize::from(inner.height);
        if cur_row < self.scroll {
            self.scroll = cur_row;
        } else if cur_row >= self.scroll + height {
            self.scroll = cur_row + 1 - height;
        }
        let style = Style::default().fg(if self.disabled {
            theme.text_muted
        } else {
            theme.foreground
        });
        let mut cursor = None;
        for (i, (li, start, chars)) in rows.iter().enumerate().skip(self.scroll).take(height) {
            let y = inner.y + (i - self.scroll) as u16;
            let mut x = inner.x;
            for (off, ch) in chars.iter().enumerate() {
                if *li == self.row && start + off == self.col {
                    cursor = Some(Position::new(x, y));
                }
                buf.set_string(x, y, ch.to_string(), style);
                x += ch.width().unwrap_or(0) as u16;
            }
            if i == cur_row && cursor.is_none() {
                cursor = Some(Position::new(x.min(inner.right().saturating_sub(1)), y));
            }
        }
        if focused { cursor } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::keys::parse_key;

    #[test]
    fn lines_join_and_split() {
        let mut t = TextArea::default();
        t.insert("ab\ncd");
        assert_eq!(t.text(), "ab\ncd");
        t.handle_key(&parse_key("home"));
        t.handle_key(&parse_key("backspace"));
        assert_eq!(t.text(), "abcd");
        t.load_text("x\ny");
        t.handle_key(&parse_key("end"));
        t.handle_key(&parse_key("delete"));
        assert_eq!(t.text(), "xy");
    }
}
