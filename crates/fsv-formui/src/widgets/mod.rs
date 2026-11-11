//! Field editors and the form row around them.
//!
//! Every editor reports [`EdMsg::Edited`] (key, value) when the person
//! commits a value; the screen applies it to the session and re-syncs every
//! row (visibility, required star, disabled, choices, errors). Editors
//! never change the engine themselves.

pub mod calendar;
mod combo;
mod date;
mod text;

use std::time::Duration;

use crossterm::event::KeyEvent;
use fsv_forms::Value;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};

pub use calendar::{CalendarEvent, CalendarGrid, center};
pub use combo::ComboBox;
pub use date::DateEditor;
pub use text::{CheckEditor, DescriptionEditor, FileEditor, ParagraphEditor, TextEditor};

use crate::kit::draw::wrap_line;
use crate::kit::{Theme, Timers};
use crate::session::{FormItem, FormSession, ItemView, WidgetKind};

pub const DEBOUNCE_TEXT: Duration = Duration::from_millis(300);
pub const DEBOUNCE_LOOKUP: Duration = Duration::from_millis(250);
pub const COMBO_CLOSE: Duration = Duration::from_millis(150);

/// What an editor tells the screen.
#[derive(Clone, Debug, PartialEq)]
pub enum EdMsg {
    /// A committed value.
    Edited(String, Value),
    /// Input the editor cannot read (empty: withdrawn). The stored value
    /// is unchanged and the form may not be submitted until it is fixed.
    Invalid(String, String),
    OpenCalendar(String),
    /// Run a remote lookup for `query` (generation `generation` of this combo).
    Search {
        key: String,
        query: String,
        generation: u64,
    },
    FocusNext,
    /// A suggestion list opened: scroll it into view.
    Opened(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TimerId {
    Commit(String),
    Search(String),
    ComboClose(String),
    NotesClear,
    Toast(u64),
}

/// What an editor may use while handling an event.
pub struct EdCx<'a> {
    pub session: &'a FormSession,
    pub now: Duration,
    pub timers: &'a mut Timers<TimerId>,
    pub out: &'a mut Vec<EdMsg>,
}

impl EdCx<'_> {
    pub fn emit(&mut self, m: EdMsg) {
        self.out.push(m);
    }

    pub fn timer(&mut self, id: TimerId, delay: Duration) {
        self.timers.set(id, self.now, delay);
    }
}

/// A field editor. `slot` is the focusable part (DateEditor: 0 input,
/// 1 calendar button; the others have one).
pub trait Editor {
    fn slots(&self) -> usize {
        1
    }
    fn disabled(&self) -> bool;
    /// `true` when the key was used (else it goes up to the screen).
    fn handle_key(&mut self, k: &KeyEvent, slot: usize, cx: &mut EdCx<'_>) -> bool;
    fn paste(&mut self, _text: &str, _slot: usize, _cx: &mut EdCx<'_>) {}
    fn focus(&mut self, _slot: usize, _cx: &mut EdCx<'_>) {}
    fn blur(&mut self, _slot: usize, _cx: &mut EdCx<'_>) {}
    fn timer(&mut self, _id: &TimerId, _focused: bool, _cx: &mut EdCx<'_>) {}
    fn sync(&mut self, view: &ItemView, focused: bool, cx: &mut EdCx<'_>);
    /// Typed text that is not committed to the form yet.
    fn dirty(&self) -> bool {
        false
    }
    /// Commit pending text (Ctrl+S / F4 / Esc on the focused row).
    fn commit(&mut self, _cx: &mut EdCx<'_>) {}
    fn height(&self, width: u16) -> u16;
    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position>;
    /// Footer entries while focused.
    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![]
    }
    fn as_combo(&mut self) -> Option<&mut ComboBox> {
        None
    }
    fn as_date(&mut self) -> Option<&mut DateEditor> {
        None
    }
}

fn editor_for(item: &FormItem, session: &FormSession) -> Option<Box<dyn Editor>> {
    Some(match item.widget {
        WidgetKind::Notice => return None,
        WidgetKind::Text => Box::new(TextEditor::new(item, false)),
        WidgetKind::Number => Box::new(TextEditor::new(item, true)),
        WidgetKind::Textarea => Box::new(ParagraphEditor::new(item)),
        WidgetKind::Richtext => Box::new(DescriptionEditor::new(item)),
        WidgetKind::Checkbox => Box::new(CheckEditor::new(item)),
        WidgetKind::Choice | WidgetKind::Multi | WidgetKind::Lookup => {
            Box::new(ComboBox::new(item))
        }
        WidgetKind::Date | WidgetKind::Datetime => Box::new(DateEditor::new(item, session)),
        WidgetKind::File => Box::new(FileEditor::new(item)),
    })
}

/// Label (+ red `*` when required), editor, help line, note, inline error.
pub struct FieldRow {
    pub item: FormItem,
    pub editor: Option<Box<dyn Editor>>,
    pub view: ItemView,
}

fn first_sentence(help: &str) -> String {
    let first = help.split(". ").next().unwrap_or("");
    if first.chars().count() < 160 {
        first.to_string()
    } else {
        first.chars().take(159).collect::<String>() + "…"
    }
}

impl FieldRow {
    pub fn new(item: &FormItem, session: &FormSession) -> Self {
        FieldRow {
            editor: editor_for(item, session),
            item: item.clone(),
            view: ItemView::default(),
        }
    }

    pub fn key(&self) -> &str {
        &self.item.key
    }

    pub fn sync(&mut self, focused: bool, cx: &mut EdCx<'_>) {
        self.view = cx.session.view(&self.item.key);
        if let Some(ed) = self.editor.as_mut() {
            ed.sync(&self.view, focused, cx);
        }
    }

    /// Focusable now (visible, not disabled, has an editor).
    pub fn focusable(&self) -> bool {
        self.view.visible && self.editor.as_ref().is_some_and(|e| !e.disabled())
    }

    fn notice_lines(&self, width: u16) -> Vec<Line<'static>> {
        let content = self
            .item
            .field
            .as_ref()
            .map(|f| f.content.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| self.item.label.clone());
        let text = fsv_forms::spec::plain_text(&content);
        text.split('\n')
            .flat_map(|l| wrap_line(&Line::from(l.to_string()), width.saturating_sub(4)))
            .collect()
    }

    fn note_text(&self) -> String {
        if self.view.disabled {
            self.view.disabled_reason.clone()
        } else {
            self.view.note.clone()
        }
    }

    fn wrapped(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
        if text.is_empty() {
            return vec![];
        }
        wrap_line(&Line::styled(text.to_string(), style), width)
    }

    /// Rows this field takes at `width` (0 when hidden), margin included.
    pub fn height(&self, width: u16) -> u16 {
        if !self.view.visible {
            return 0;
        }
        let inner = width.saturating_sub(2);
        let Some(ed) = &self.editor else {
            return self.notice_lines(inner).len() as u16 + 2 + 1;
        };
        let help = u16::from(!self.item.help.is_empty());
        let note = Self::wrapped(&self.note_text(), inner, Style::default()).len() as u16;
        let err = Self::wrapped(&self.view.error, inner, Style::default()).len() as u16;
        1 + ed.height(inner) + help + note + err + 1
    }

    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        if focus.is_some() {
            buf.set_style(
                Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1)),
                Style::default().bg(theme.boost),
            );
        }
        let inner = Rect::new(
            area.x + 1,
            area.y,
            area.width.saturating_sub(2),
            area.height,
        );
        if self.editor.is_none() {
            let lines = self.notice_lines(inner.width);
            let h = lines.len() as u16 + 2;
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.secondary)),
                )
                .render(Rect::new(inner.x, inner.y, inner.width, h), buf);
            return None;
        }
        let mut y = inner.y;
        let mut label = vec![ratatui::text::Span::styled(
            self.item.label.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        if self.view.required {
            label.push(ratatui::text::Span::styled(
                " *",
                Style::default()
                    .fg(ratatui::style::Color::Red)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if self.view.disabled {
            label.push(ratatui::text::Span::styled(
                "  (read-only)",
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
        buf.set_line(inner.x, y, &Line::from(label), inner.width);
        y += 1;
        let mut cursor = None;
        if let Some(ed) = self.editor.as_mut() {
            let h = ed.height(inner.width);
            cursor = ed.render(Rect::new(inner.x, y, inner.width, h), buf, focus, theme);
            y += h;
        }
        if !self.item.help.is_empty() {
            buf.set_stringn(
                inner.x,
                y,
                first_sentence(&self.item.help),
                usize::from(inner.width),
                Style::default().fg(theme.text_muted),
            );
            y += 1;
        }
        let note = self.note_text();
        for (text, color) in [
            (note.as_str(), theme.warning),
            (self.view.error.as_str(), theme.error),
        ] {
            for l in Self::wrapped(text, inner.width, Style::default().fg(color)) {
                buf.set_line(inner.x, y, &l, inner.width);
                y += 1;
            }
        }
        cursor
    }
}

/// `#row-<key>` id (non `[A-Za-z0-9_-]` → `_`).
pub fn safe_id(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
