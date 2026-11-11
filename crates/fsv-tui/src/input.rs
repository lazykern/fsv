//! Text input with Textual `Input` editing keys. Setting the value from
//! code does not emit a change (Python needed `_tab_lock` /
//! `_history_nav_lock` counters to undo that; the port has no such counters).

use crossterm::event::KeyEvent;

use crate::keys;

#[derive(Debug, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    Submitted,
    /// Moved the cursor only.
    Moved,
    /// Not an editing key; the caller may handle it.
    Ignored,
}

#[derive(Clone, Debug, Default)]
pub struct LineInput {
    value: String,
    cursor: usize,
    /// Enter inserts a newline instead of submitting (TextArea).
    pub multiline: bool,
    pub placeholder: String,
}

impl LineInput {
    pub fn new(placeholder: &str) -> LineInput {
        LineInput {
            placeholder: placeholder.to_string(),
            ..Default::default()
        }
    }

    pub fn multiline() -> LineInput {
        LineInput {
            multiline: true,
            ..Default::default()
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Replace the text and put the cursor at the end (no change event).
    pub fn set_value(&mut self, v: &str) {
        self.value = v.to_string();
        self.cursor = self.len();
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn len(&self) -> usize {
        self.value.chars().count()
    }

    fn byte(&self, idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(idx)
            .map_or(self.value.len(), |(b, _)| b)
    }

    fn insert(&mut self, c: char) {
        let b = self.byte(self.cursor);
        self.value.insert(b, c);
        self.cursor += 1;
    }

    fn remove_range(&mut self, from: usize, to: usize) {
        let (a, b) = (self.byte(from), self.byte(to));
        self.value.replace_range(a..b, "");
        self.cursor = from;
    }

    fn word_left(&self) -> usize {
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    pub fn handle(&mut self, ev: &KeyEvent) -> InputEvent {
        if let Some(c) = keys::printable(ev) {
            self.insert(c);
            return InputEvent::Changed;
        }
        let before = self.value.clone();
        match keys::name(ev).as_str() {
            "enter" if self.multiline => {
                self.insert('\n');
            }
            "enter" => return InputEvent::Submitted,
            "backspace" if self.cursor > 0 => self.remove_range(self.cursor - 1, self.cursor),
            "backspace" => {}
            "delete" | "ctrl+d" if self.cursor < self.len() => {
                self.remove_range(self.cursor, self.cursor + 1)
            }
            "delete" | "ctrl+d" => {}
            "ctrl+w" => {
                let to = self.cursor;
                let from = self.word_left();
                self.remove_range(from, to);
            }
            "ctrl+u" => {
                let to = self.cursor;
                self.remove_range(0, to);
            }
            "ctrl+k" if !self.multiline => {
                let from = self.cursor;
                let len = self.len();
                self.remove_range(from, len);
            }
            "left" => {
                self.cursor = self.cursor.saturating_sub(1);
                return InputEvent::Moved;
            }
            "right" => {
                self.cursor = (self.cursor + 1).min(self.len());
                return InputEvent::Moved;
            }
            "home" | "ctrl+a" => {
                self.cursor = 0;
                return InputEvent::Moved;
            }
            "end" | "ctrl+e" => {
                self.cursor = self.len();
                return InputEvent::Moved;
            }
            _ => return InputEvent::Ignored,
        }
        if self.value == before {
            InputEvent::Moved
        } else {
            InputEvent::Changed
        }
    }

    /// Display text and the cursor column for a single-line input of `width`
    /// cells (scrolled so the cursor stays visible).
    pub fn window(&self, width: usize) -> (String, usize) {
        let chars: Vec<char> = self.value.chars().collect();
        let width = width.max(1);
        let start = self.cursor.saturating_sub(width - 1);
        let shown: String = chars.iter().skip(start).take(width).collect();
        (shown, self.cursor - start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(i: &mut LineInput, k: &str) -> InputEvent {
        i.handle(&keys::parse(k))
    }

    #[test]
    fn editing_keys() {
        let mut i = LineInput::new("");
        for c in ["a", "b", "space", "c"] {
            press(&mut i, c);
        }
        assert_eq!(i.value(), "ab c");
        assert_eq!(press(&mut i, "ctrl+w"), InputEvent::Changed);
        assert_eq!(i.value(), "ab ");
        press(&mut i, "home");
        press(&mut i, "x");
        assert_eq!(i.value(), "xab ");
        assert_eq!(press(&mut i, "enter"), InputEvent::Submitted);
        assert_eq!(press(&mut i, "ctrl+p"), InputEvent::Ignored);
    }
}
