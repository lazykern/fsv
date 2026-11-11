//! Typed date / date-time with a live preview; F2 or the button opens the
//! calendar (with time suggestions for date-times).
//!
//! Text that does not parse is kept (with its error) until it is fixed or
//! cleared, and the form may not be submitted meanwhile: the stored value
//! stays what it was.

use crossterm::event::{KeyCode, KeyEvent};
use fsv_forms::Value;
use fsv_forms::when::{self, Parsed, WhenOptions};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::Line;

use super::{EdCx, EdMsg, Editor};
use crate::kit::draw::{Button, Variant, wrap_line};
use crate::kit::{InputEvent, LineInput, Theme};
use crate::session::{FormItem, FormSession, ItemView, WidgetKind};

const BUTTON: &str = "Calendar";
const BUTTON_W: u16 = 12;

pub struct DateEditor {
    key: String,
    pub date_only: bool,
    /// Port of fsv.formui.widgets.DateEditor.input
    pub input: LineInput,
    committed: String,
    /// Text that does not parse (kept, and reported).
    pub invalid: Option<String>,
    pub preview: Parsed,
}

impl DateEditor {
    pub fn new(item: &FormItem, _session: &FormSession) -> Self {
        let date_only = item.widget == WidgetKind::Date;
        let ph = if !item.placeholder.is_empty() {
            item.placeholder.clone()
        } else if date_only {
            "YYYY-MM-DD, 26 Nov 2026, tomorrow".into()
        } else {
            "YYYY-MM-DD HH:MM, tomorrow 19:00, fri 9am".into()
        };
        DateEditor {
            key: item.key.clone(),
            date_only,
            input: LineInput::new(ph),
            committed: String::new(),
            invalid: None,
            preview: Parsed::default(),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn parse(&self, text: &str, session: &FormSession) -> Parsed {
        when::parse_when(
            text,
            &session.tz,
            WhenOptions {
                tz_source: &session.tz_source,
                date_only: self.date_only,
                today: None,
            },
        )
    }

    fn update_preview(&mut self, text: &str, session: &FormSession) -> Parsed {
        self.preview = self.parse(text, session);
        self.preview.clone()
    }

    pub fn commit_text(&mut self, cx: &mut EdCx<'_>) {
        let text = self.input.value();
        if text == self.committed {
            if self.invalid.take().is_some() {
                // reverted to the stored value by hand
                cx.emit(EdMsg::Invalid(self.key.clone(), String::new()));
            }
            return;
        }
        let p = self.update_preview(&text, cx.session);
        if !p.error.is_empty() {
            self.invalid = Some(text);
            cx.emit(EdMsg::Invalid(self.key.clone(), p.error));
            return;
        }
        self.invalid = None;
        self.committed = text;
        cx.emit(EdMsg::Edited(self.key.clone(), p.value));
    }

    /// From the calendar.
    pub fn set_value(&mut self, value: Value, cx: &mut EdCx<'_>) {
        let text = when::to_text(&value, Some(&cx.session.tz));
        self.invalid = None;
        self.input.set_value(&text);
        self.update_preview(&text, cx.session);
        self.committed = text;
        cx.emit(EdMsg::Edited(self.key.clone(), value));
    }

    /// Ask the screen for the calendar (F2 or the Calendar button).
    pub fn action_calendar(&self, cx: &mut EdCx<'_>) {
        cx.emit(EdMsg::OpenCalendar(self.key.clone()));
    }

    /// `DateEditor._enter`: Enter in the box commits the typed date.
    fn enter(&mut self, cx: &mut EdCx<'_>) {
        self.commit_text(cx);
    }

    /// `DateEditor._button`: the Calendar button (disabled with the field).
    fn button(&self, cx: &mut EdCx<'_>) -> bool {
        if self.input.disabled {
            return false;
        }
        self.action_calendar(cx);
        true
    }

    fn preview_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let (text, color) = if self.preview.error.is_empty() {
            (&self.preview.preview, theme.success)
        } else {
            (&self.preview.error, theme.error)
        };
        if text.is_empty() {
            return vec![];
        }
        wrap_line(
            &Line::styled(text.clone(), Style::default().fg(color)),
            width.saturating_sub(2),
        )
    }
}

impl Editor for DateEditor {
    fn slots(&self) -> usize {
        2
    }

    fn disabled(&self) -> bool {
        self.input.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, slot: usize, cx: &mut EdCx<'_>) -> bool {
        if k.code == KeyCode::F(2) {
            self.action_calendar(cx);
            return true;
        }
        if slot == 1 {
            return matches!(k.code, KeyCode::Enter | KeyCode::Char(' ')) && self.button(cx);
        }
        match self.input.handle_key(k) {
            InputEvent::Changed => {
                let v = self.input.value();
                self.update_preview(&v, cx.session);
                true
            }
            InputEvent::Submitted => {
                self.enter(cx);
                true
            }
            InputEvent::Handled => true,
            InputEvent::Ignored => false,
        }
    }

    fn paste(&mut self, text: &str, slot: usize, cx: &mut EdCx<'_>) {
        if slot == 0 && self.input.paste(text) == InputEvent::Changed {
            let v = self.input.value();
            self.update_preview(&v, cx.session);
        }
    }

    fn focus(&mut self, slot: usize, _cx: &mut EdCx<'_>) {
        if slot == 0 {
            self.input.select_all();
        }
    }

    /// `DateEditor.on_descendant_blur`: leaving the input commits it.
    fn blur(&mut self, slot: usize, cx: &mut EdCx<'_>) {
        if slot == 0 {
            self.commit_text(cx);
        }
    }

    fn sync(&mut self, view: &ItemView, focused: bool, cx: &mut EdCx<'_>) {
        self.input.disabled = view.disabled;
        if self
            .invalid
            .as_deref()
            .is_some_and(|t| t == self.input.value())
        {
            return;
        }
        if !focused && self.input.value() != view.text {
            self.committed = view.text.clone();
            self.input.set_value(&view.text);
            let t = view.text.clone();
            self.update_preview(&t, cx.session);
        }
    }

    fn dirty(&self) -> bool {
        self.input.value() != self.committed
    }

    fn commit(&mut self, cx: &mut EdCx<'_>) {
        self.commit_text(cx);
    }

    fn height(&self, width: u16) -> u16 {
        let n = if self.preview.error.is_empty() {
            &self.preview.preview
        } else {
            &self.preview.error
        };
        let lines = if n.is_empty() {
            0
        } else {
            wrap_line(&Line::from(n.clone()), width.saturating_sub(2)).len() as u16
        };
        3 + lines
    }

    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        let bw = BUTTON_W.min(area.width / 2);
        let iw = area.width.saturating_sub(bw);
        let cursor = self.input.render(
            Rect::new(area.x, area.y, iw, 3),
            buf,
            focus == Some(0),
            theme,
        );
        let label_w = BUTTON.chars().count() as u16 + 4;
        let bx = area.x + iw + bw.saturating_sub(label_w) / 2;
        Button {
            label: BUTTON,
            variant: Variant::Default,
            focused: focus == Some(1),
            disabled: self.input.disabled,
        }
        .draw(bx, area.y, buf, theme);
        for (i, l) in self
            .preview_lines(area.width, theme)
            .into_iter()
            .enumerate()
        {
            buf.set_line(
                area.x + 1,
                area.y + 3 + i as u16,
                &l,
                area.width.saturating_sub(2),
            );
        }
        cursor
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("f2", "Calendar")]
    }

    fn as_date(&mut self) -> Option<&mut DateEditor> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::TestCx;

    /// `DateEditor._enter`: Enter commits a readable date, and reports an
    /// unreadable one (kept in the box) without changing the value.
    #[test]
    fn enter_commits_or_reports_the_typed_date() {
        let mut h = TestCx::new();
        let mut d = DateEditor::new(&h.item("planned_start_date"), &h.s);
        d.input.set_value("2030-11-26 19:00");
        h.cx(|cx| d.enter(cx));
        match &h.take()[..] {
            [EdMsg::Edited(k, Value::DateTime(v))] => {
                assert_eq!(k, "planned_start_date");
                assert_eq!(v.local, jiff::civil::date(2030, 11, 26).at(19, 0, 0, 0));
                assert_eq!(v.offset, jiff::tz::Offset::constant(9));
            }
            other => panic!("{other:?}"),
        }
        d.input.set_value("25:00");
        h.cx(|cx| d.enter(cx));
        assert_eq!(
            h.take(),
            [EdMsg::Invalid(
                "planned_start_date".into(),
                "'25:00' is not a time of day".into()
            )]
        );
        assert_eq!(d.invalid.as_deref(), Some("25:00"));
    }

    /// `DateEditor._button`: the Calendar button asks for the calendar,
    /// except on a read-only field.
    #[test]
    fn button_opens_the_calendar_unless_disabled() {
        let mut h = TestCx::new();
        let mut d = DateEditor::new(&h.item("planned_start_date"), &h.s);
        assert!(h.cx(|cx| d.button(cx)));
        assert_eq!(h.take(), [EdMsg::OpenCalendar("planned_start_date".into())]);
        d.input.disabled = true;
        assert!(!h.cx(|cx| d.button(cx)));
        assert!(h.take().is_empty());
    }
}
