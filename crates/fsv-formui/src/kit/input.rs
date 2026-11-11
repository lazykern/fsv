//! A one-line text input with Textual `Input` bindings: editing keys,
//! placeholder, horizontal scroll, an optional ghost completion (→ at the
//! end accepts it) and a number filter.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Widget};
use unicode_width::UnicodeWidthChar;

use super::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// The text changed (typed, deleted, pasted, completion accepted).
    Changed,
    Submitted,
    /// The key was used without changing the text (cursor moves).
    Handled,
    Ignored,
}

#[derive(Clone, Debug, Default)]
pub struct LineInput {
    chars: Vec<char>,
    cursor: usize,
    pub placeholder: String,
    pub disabled: bool,
    pub number: bool,
    /// The whole completed text, shown dim after the cursor.
    pub suggestion: Option<String>,
    scroll: usize,
    /// Selected `(start, end)`: all of the text on focus, so typing
    /// replaces it (Textual's `select_on_focus`).
    selection: Option<(usize, usize)>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Textual's `type="number"` pattern: a float as it is being typed.
fn number_like(s: &str) -> bool {
    let b = s.trim_start_matches(['+', '-']);
    let (mant, exp) = match b.find(['e', 'E']) {
        Some(i) => (&b[..i], Some(&b[i + 1..])),
        None => (b, None),
    };
    let mut dots = 0;
    for ch in mant.chars() {
        match ch {
            '.' => dots += 1,
            c if c.is_ascii_digit() => {}
            _ => return false,
        }
    }
    if dots > 1 || b.len() < s.len().saturating_sub(1) {
        return false;
    }
    match exp {
        None => true,
        Some(e) => {
            !mant.is_empty()
                && e.trim_start_matches(['+', '-'])
                    .chars()
                    .all(|c| c.is_ascii_digit())
        }
    }
}

impl LineInput {
    pub fn new(placeholder: impl Into<String>) -> Self {
        LineInput {
            placeholder: placeholder.into(),
            ..Default::default()
        }
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    /// Set the text from code (the cursor goes to the end).
    pub fn set_value(&mut self, v: &str) {
        self.chars = v.chars().collect();
        self.cursor = self.chars.len();
        self.selection = None;
    }

    /// Select all of the text, as focusing a Textual `Input` does.
    pub fn select_all(&mut self) {
        self.selection = (!self.chars.is_empty()).then_some((0, self.chars.len()));
        self.cursor = self.chars.len();
        self.suggestion = None;
    }

    /// The selected `(start, end)`, if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.selection
    }

    /// Drop the selection (`true` when there was one).
    fn delete_selection(&mut self) -> bool {
        match self.selection.take() {
            Some((a, b)) => {
                self.chars.drain(a..b);
                self.cursor = a;
                true
            }
            None => false,
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn insert(&mut self, s: &str) -> bool {
        let mut next = self.chars.clone();
        let add: Vec<char> = s.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        let (a, b) = self.selection.unwrap_or((self.cursor, self.cursor));
        next.splice(a..b, add.iter().copied());
        if self.number && !number_like(&next.iter().collect::<String>()) {
            return false;
        }
        self.chars = next;
        self.cursor = a + add.len();
        self.selection = None;
        true
    }

    pub fn paste(&mut self, text: &str) -> InputEvent {
        if self.disabled || !self.insert(text) {
            return InputEvent::Ignored;
        }
        InputEvent::Changed
    }

    fn word_left(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && !is_word(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.chars[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let n = self.chars.len();
        let mut i = self.cursor;
        while i < n && !is_word(self.chars[i]) {
            i += 1;
        }
        while i < n && is_word(self.chars[i]) {
            i += 1;
        }
        i
    }

    pub fn handle_key(&mut self, k: &KeyEvent) -> InputEvent {
        if self.disabled {
            return InputEvent::Ignored;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = self.chars.len();
        let moved = |me: &mut Self, to: usize| {
            me.cursor = to;
            me.selection = None;
            InputEvent::Handled
        };
        let deletes = matches!(k.code, KeyCode::Backspace | KeyCode::Delete)
            || (ctrl && matches!(k.code, KeyCode::Char('u' | 'k' | 'w' | 'd')));
        if deletes && self.delete_selection() {
            return InputEvent::Changed;
        }
        if let Some((a, b)) = self.selection
            && matches!(k.code, KeyCode::Left | KeyCode::Right)
            && !ctrl
            && !(k.code == KeyCode::Right && self.suggestion.is_some())
        {
            return moved(self, if k.code == KeyCode::Left { a } else { b });
        }
        match k.code {
            KeyCode::Enter => InputEvent::Submitted,
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                if self.insert(&c.to_string()) {
                    InputEvent::Changed
                } else {
                    InputEvent::Handled
                }
            }
            KeyCode::Char('a') if ctrl => moved(self, 0),
            KeyCode::Char('e') if ctrl => moved(self, n),
            KeyCode::Char('u') if ctrl => {
                self.chars.drain(..self.cursor);
                self.cursor = 0;
                InputEvent::Changed
            }
            KeyCode::Char('k') if ctrl => {
                self.chars.truncate(self.cursor);
                InputEvent::Changed
            }
            KeyCode::Char('w') if ctrl => {
                let to = self.word_left();
                self.chars.drain(to..self.cursor);
                self.cursor = to;
                InputEvent::Changed
            }
            KeyCode::Char('d') if ctrl => self.delete_right(),
            KeyCode::Backspace if ctrl => {
                let to = self.word_right();
                self.chars.drain(self.cursor..to);
                InputEvent::Changed
            }
            KeyCode::Backspace => {
                if self.cursor == 0 {
                    return InputEvent::Handled;
                }
                self.cursor -= 1;
                self.chars.remove(self.cursor);
                InputEvent::Changed
            }
            KeyCode::Delete => self.delete_right(),
            KeyCode::Left if ctrl => moved(self, self.word_left()),
            KeyCode::Right if ctrl => moved(self, self.word_right()),
            KeyCode::Left => moved(self, self.cursor.saturating_sub(1)),
            KeyCode::Right => {
                if self.cursor == n {
                    let value = self.value();
                    if let Some(s) = self
                        .suggestion
                        .take()
                        .filter(|s| s.starts_with(&value) && *s != value)
                    {
                        self.set_value(&s);
                        return InputEvent::Changed;
                    }
                    return InputEvent::Handled;
                }
                moved(self, self.cursor + 1)
            }
            KeyCode::Home => moved(self, 0),
            KeyCode::End => moved(self, n),
            _ => InputEvent::Ignored,
        }
    }

    fn delete_right(&mut self) -> InputEvent {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
            return InputEvent::Changed;
        }
        InputEvent::Handled
    }

    /// Draw a bordered input (3 rows); returns the cursor cell when focused.
    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focused: bool,
        theme: &Theme,
    ) -> Option<Position> {
        let border = if focused {
            theme.border
        } else {
            theme.border_blurred
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(border));
        let inner = block.inner(area);
        block.render(area, buf);
        self.render_line(inner, buf, focused, theme)
    }

    /// Draw the text alone on the first row of `area`.
    pub fn render_line(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focused: bool,
        theme: &Theme,
    ) -> Option<Position> {
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let width = usize::from(area.width.saturating_sub(1)).max(1);
        let base = if self.disabled {
            Style::default()
                .fg(theme.text_muted)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(theme.foreground)
        };
        if self.chars.is_empty() && !self.placeholder.is_empty() {
            buf.set_stringn(
                area.x,
                area.y,
                &self.placeholder,
                usize::from(area.width),
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::DIM),
            );
            return focused.then_some(Position::new(area.x, area.y));
        }
        let w = |c: &char| c.width().unwrap_or(0);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        }
        while self.chars[self.scroll..self.cursor]
            .iter()
            .map(w)
            .sum::<usize>()
            >= width
        {
            self.scroll += 1;
        }
        let mut x = area.x;
        let right = area.x + area.width;
        let mut cursor_at = None;
        for (i, ch) in self.chars.iter().enumerate().skip(self.scroll) {
            if i == self.cursor {
                cursor_at = Some(Position::new(x, area.y));
            }
            let cw = w(ch) as u16;
            if x + cw > right {
                break;
            }
            let selected = focused && self.selection.is_some_and(|(a, b)| (a..b).contains(&i));
            let st = if selected {
                base.bg(theme.input_selection())
            } else {
                base
            };
            buf.set_string(x, area.y, ch.to_string(), st);
            x += cw;
        }
        if self.cursor == self.chars.len() {
            cursor_at = Some(Position::new(x.min(right.saturating_sub(1)), area.y));
            let value = self.value();
            if focused && let Some(s) = self.suggestion.as_ref().filter(|s| s.starts_with(&value)) {
                let ghost: String = s.chars().skip(self.chars.len()).collect();
                if x < right {
                    buf.set_stringn(
                        x,
                        area.y,
                        &ghost,
                        usize::from(right - x),
                        Style::default()
                            .fg(theme.text_muted)
                            .add_modifier(Modifier::DIM),
                    );
                }
            }
        }
        if focused { cursor_at } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::keys::parse_key;

    fn typed(s: &str) -> LineInput {
        let mut i = LineInput::default();
        for c in s.chars() {
            i.handle_key(&parse_key(&c.to_string()));
        }
        i
    }

    #[test]
    fn editing_keys() {
        let mut i = typed("hello world");
        assert_eq!(i.handle_key(&parse_key("ctrl+w")), InputEvent::Changed);
        assert_eq!(i.value(), "hello ");
        i.handle_key(&parse_key("home"));
        i.handle_key(&parse_key("delete"));
        assert_eq!(i.value(), "ello ");
        i.handle_key(&parse_key("ctrl+k"));
        assert_eq!(i.value(), "");
        assert_eq!(i.handle_key(&parse_key("enter")), InputEvent::Submitted);
    }

    #[test]
    fn number_filter_and_suggestion() {
        let mut n = LineInput {
            number: true,
            ..Default::default()
        };
        for c in "-1.5x".chars() {
            n.handle_key(&parse_key(&c.to_string()));
        }
        assert_eq!(n.value(), "-1.5");
        let mut f = typed("/tmp/ro");
        f.suggestion = Some("/tmp/roll".into());
        assert_eq!(f.handle_key(&parse_key("right")), InputEvent::Changed);
        assert_eq!(f.value(), "/tmp/roll");
    }

    /// Textual `Input`: focus selects all; typing or a paste replaces the
    /// selection, a delete key removes it, an arrow drops it.
    #[test]
    fn a_selection_is_replaced_or_dropped() {
        let mut i = typed("Normal");
        i.select_all();
        assert_eq!(i.selection(), Some((0, 6)));
        i.handle_key(&parse_key("e"));
        assert_eq!(i.value(), "e");
        assert_eq!(i.selection(), None);

        let mut i = typed("Hello");
        i.select_all();
        assert_eq!(i.paste("x"), InputEvent::Changed);
        assert_eq!(i.value(), "x");

        for key in ["backspace", "delete", "ctrl+u", "ctrl+k", "ctrl+w"] {
            let mut i = typed("09:00");
            i.select_all();
            assert_eq!(i.handle_key(&parse_key(key)), InputEvent::Changed, "{key}");
            assert_eq!(i.value(), "", "{key}");
        }

        let mut i = typed("2030-01-01");
        i.select_all();
        i.handle_key(&parse_key("left"));
        assert_eq!((i.cursor(), i.selection()), (0, None));
        i.select_all();
        i.handle_key(&parse_key("right"));
        assert_eq!((i.cursor(), i.selection()), (10, None));
        i.select_all();
        i.handle_key(&parse_key("home"));
        assert_eq!((i.cursor(), i.selection()), (0, None));
        i.handle_key(&parse_key("x"));
        assert_eq!(i.value(), "x2030-01-01");

        let mut empty = LineInput::default();
        empty.select_all();
        assert_eq!(empty.selection(), None);
    }

    #[test]
    fn the_selection_is_drawn_while_focused() {
        let theme = Theme::textual_dark();
        let mut i = typed("ab");
        i.select_all();
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        i.render_line(area, &mut buf, true, &theme);
        assert_eq!(buf[(0, 0)].bg, theme.input_selection());
        assert_eq!(buf[(1, 0)].bg, theme.input_selection());
        let mut buf = Buffer::empty(area);
        i.render_line(area, &mut buf, false, &theme);
        assert_ne!(buf[(0, 0)].bg, theme.input_selection());
    }
}
