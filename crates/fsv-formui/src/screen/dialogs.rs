//! The small modal dialogs: confirm, save path, text (with Copy) and the
//! date picker.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fsv_forms::Value;
use fsv_forms::tz::Zone;
use fsv_forms::when::{self, TIME_SUGGESTIONS, WhenOptions};
use jiff::civil::Date;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use crate::files::{Paths, strerror};
use crate::kit::draw::{Button, Variant, dialog, wrap_line};
use crate::kit::{InputEvent, LineInput, Theme};
use crate::widgets::{CalendarEvent, CalendarGrid};

/// What a dialog key did.
#[derive(Debug, PartialEq)]
pub enum Dlg<T> {
    Open,
    Done(T),
}

fn is_tab(k: &KeyEvent) -> Option<isize> {
    match k.code {
        KeyCode::Tab if k.modifiers.contains(KeyModifiers::SHIFT) => Some(-1),
        KeyCode::Tab => Some(1),
        KeyCode::BackTab => Some(-1),
        _ => None,
    }
}

fn step(cur: usize, n: usize, by: isize) -> usize {
    (cur as isize + by).rem_euclid(n as isize) as usize
}

fn press(k: &KeyEvent) -> bool {
    matches!(k.code, KeyCode::Enter | KeyCode::Char(' '))
}

/// Buttons left to right with a 1-cell gap.
fn buttons(x: u16, y: u16, buf: &mut Buffer, list: &[(&str, Variant, bool, bool)], theme: &Theme) {
    let mut x = x;
    for (label, variant, focused, disabled) in list {
        x += Button {
            label,
            variant: *variant,
            focused: *focused,
            disabled: *disabled,
        }
        .draw(x, y, buf, theme)
            + 1;
    }
}

fn text_lines(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    text.split('\n')
        .flat_map(|l| wrap_line(&Line::styled(l.to_string(), style), width))
        .collect()
}

// ---------------------------------------------------------------------------

pub struct ConfirmDialog {
    pub question: String,
    pub yes: String,
    pub no: String,
    /// 0 yes, 1 no.
    pub focus: usize,
}

impl ConfirmDialog {
    pub fn new(question: &str, yes: &str, no: &str) -> Self {
        ConfirmDialog {
            question: question.into(),
            yes: yes.into(),
            no: no.into(),
            focus: 1,
        }
    }

    pub fn handle_key(&mut self, k: &KeyEvent) -> Dlg<bool> {
        if let Some(by) = is_tab(k) {
            self.focus = step(self.focus, 2, by);
            return Dlg::Open;
        }
        match k.code {
            KeyCode::Esc => self.action_no(),
            KeyCode::Char('y') => self.action_yes(),
            KeyCode::Left | KeyCode::Right => {
                self.focus = 1 - self.focus;
                Dlg::Open
            }
            _ if press(k) => self.pressed(),
            _ => Dlg::Open,
        }
    }

    /// `ConfirmDialog._pressed`: the focused button answers.
    pub fn pressed(&self) -> Dlg<bool> {
        Dlg::Done(self.focus == 0)
    }

    /// `ConfirmDialog.action_yes` (`y`).
    pub fn action_yes(&self) -> Dlg<bool> {
        Dlg::Done(true)
    }

    /// `ConfirmDialog.action_no` (Esc).
    pub fn action_no(&self) -> Dlg<bool> {
        Dlg::Done(false)
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let w = 60;
        let inner_w = w.min(area.width * 95 / 100).saturating_sub(6);
        let lines = text_lines(&self.question, inner_w, Style::default());
        let h = lines.len() as u16 + 1 + 3 + 4;
        let inner = dialog(area, buf, w, 95, h, theme);
        for (i, l) in lines.iter().enumerate() {
            buf.set_line(inner.x, inner.y + i as u16, l, inner.width);
        }
        buttons(
            inner.x,
            inner.y + lines.len() as u16 + 1,
            buf,
            &[
                (&self.yes, Variant::Error, self.focus == 0, false),
                (&self.no, Variant::Primary, self.focus == 1, false),
            ],
            theme,
        );
    }
}

// ---------------------------------------------------------------------------

/// Ask where to save the draft (asks again before overwriting). The screen
/// writes the file and reports a failure back with [`SavePathDialog::failed`]:
/// the dialog stays open with the error (try another path).
pub struct SavePathDialog {
    pub title: String,
    pub input: LineInput,
    pub note: String,
    confirm_overwrite: Option<PathBuf>,
    /// 0 input, 1 Save, 2 Cancel.
    pub focus: usize,
}

impl SavePathDialog {
    pub fn new(default: &Path, title: &str) -> Self {
        let mut input = LineInput::new("");
        input.set_value(&default.to_string_lossy());
        input.select_all();
        SavePathDialog {
            title: title.into(),
            input,
            note: String::new(),
            confirm_overwrite: None,
            focus: 0,
        }
    }

    /// `Done(Some(path))`: write there now; `Done(None)`: cancelled.
    pub fn handle_key(&mut self, k: &KeyEvent, paths: &Paths) -> Dlg<Option<PathBuf>> {
        if let Some(by) = is_tab(k) {
            self.focus = step(self.focus, 3, by);
            if self.focus == 0 {
                self.input.select_all();
            }
            return Dlg::Open;
        }
        if k.code == KeyCode::Esc {
            return self.cancel_btn();
        }
        match self.focus {
            0 => match self.input.handle_key(k) {
                InputEvent::Submitted => self.enter(paths),
                _ => Dlg::Open,
            },
            1 if press(k) => self.save_btn(paths),
            2 if press(k) => self.cancel_btn(),
            _ => Dlg::Open,
        }
    }

    /// `SavePathDialog._enter`: Enter in the path box saves.
    pub fn enter(&mut self, paths: &Paths) -> Dlg<Option<PathBuf>> {
        self.save(paths)
    }

    /// `SavePathDialog._save_btn`.
    pub fn save_btn(&mut self, paths: &Paths) -> Dlg<Option<PathBuf>> {
        self.save(paths)
    }

    /// `SavePathDialog._cancel_btn` (and `action_cancel`, Esc).
    pub fn cancel_btn(&self) -> Dlg<Option<PathBuf>> {
        Dlg::Done(None)
    }

    /// `SavePathDialog._note`: the line under the path box.
    pub fn set_note(&mut self, text: &str) {
        self.note = text.to_string();
    }

    pub fn paste(&mut self, text: &str) {
        if self.focus == 0 {
            self.input.paste(text);
        }
    }

    /// `SavePathDialog._save`: the path checks before a write. The screen
    /// writes and reports a failure with [`SavePathDialog::failed`].
    pub fn save(&mut self, paths: &Paths) -> Dlg<Option<PathBuf>> {
        let value = self.input.value();
        let raw = value.trim();
        if raw.is_empty() {
            self.set_note("type a file path");
            return Dlg::Open;
        }
        let path = paths.abspath(raw);
        let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if path.is_dir() {
            self.set_note(&format!("{} is a folder: add a file name", path.display()));
            return Dlg::Open;
        }
        if !folder.is_dir() {
            self.set_note(&format!(
                "cannot save: the folder {} does not exist",
                folder.display()
            ));
            return Dlg::Open;
        }
        if path.exists() && self.confirm_overwrite.as_ref() != Some(&path) {
            self.set_note(&format!(
                "{} exists: press Save again to overwrite",
                path.display()
            ));
            self.confirm_overwrite = Some(path);
            return Dlg::Open;
        }
        Dlg::Done(Some(path))
    }

    pub fn failed(&mut self, path: &Path, e: &std::io::Error) {
        self.confirm_overwrite = None;
        self.set_note(&format!(
            "cannot save to {}: {}",
            path.display(),
            strerror(e)
        ));
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) -> Option<Position> {
        let w = 76;
        let inner_w = w.min(area.width * 95 / 100).saturating_sub(6);
        let note = text_lines(&self.note, inner_w, Style::default().fg(theme.warning));
        let h = 1 + 3 + note.len() as u16 + 1 + 3 + 4;
        let inner = dialog(area, buf, w, 95, h, theme);
        buf.set_stringn(
            inner.x,
            inner.y,
            &self.title,
            usize::from(inner.width),
            Style::default().add_modifier(Modifier::BOLD),
        );
        let cursor = self.input.render(
            Rect::new(inner.x, inner.y + 1, inner.width, 3),
            buf,
            self.focus == 0,
            theme,
        );
        let mut y = inner.y + 4;
        for l in &note {
            buf.set_line(inner.x, y, l, inner.width);
            y += 1;
        }
        buttons(
            inner.x,
            y + 1,
            buf,
            &[
                ("Save", Variant::Primary, self.focus == 1, false),
                ("Cancel", Variant::Default, self.focus == 2, false),
            ],
            theme,
        );
        cursor
    }
}

// ---------------------------------------------------------------------------

/// Show a command (or any text) with a Copy button.
pub struct TextDialog {
    pub title: String,
    pub text: String,
    pub note: String,
    /// 0 Copy, 1 Close.
    pub focus: usize,
}

pub enum TextOut {
    Copy,
    Close,
}

impl TextDialog {
    pub fn new(title: &str, text: &str, note: &str) -> Self {
        TextDialog {
            title: title.into(),
            text: text.into(),
            note: note.into(),
            focus: 0,
        }
    }

    pub fn handle_key(&mut self, k: &KeyEvent) -> Dlg<TextOut> {
        if let Some(by) = is_tab(k) {
            self.focus = step(self.focus, 2, by);
            return Dlg::Open;
        }
        match k.code {
            KeyCode::Esc => self.close_btn(),
            KeyCode::Left | KeyCode::Right => {
                self.focus = 1 - self.focus;
                Dlg::Open
            }
            _ if press(k) && self.focus == 0 => self.copy(),
            _ if press(k) => self.close_btn(),
            _ => Dlg::Open,
        }
    }

    /// `TextDialog._copy`: the screen puts [`TextDialog::text`] on the
    /// clipboard and says so; the dialog stays open.
    pub fn copy(&self) -> Dlg<TextOut> {
        Dlg::Done(TextOut::Copy)
    }

    /// `TextDialog._close_btn` (and `action_close`, Esc).
    pub fn close_btn(&self) -> Dlg<TextOut> {
        Dlg::Done(TextOut::Close)
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let w = area.width * 90 / 100;
        let inner_w = w.saturating_sub(6);
        let text = text_lines(&self.text, inner_w.saturating_sub(2), Style::default());
        let note = text_lines(
            &self.note,
            inner_w,
            Style::default().add_modifier(Modifier::DIM),
        );
        let h = 1 + text.len() as u16 + 2 + note.len() as u16 + 1 + 3 + 4;
        let inner = dialog(area, buf, w, 90, h, theme);
        buf.set_stringn(
            inner.x,
            inner.y,
            &self.title,
            usize::from(inner.width),
            Style::default().add_modifier(Modifier::BOLD),
        );
        let box_ = Rect::new(inner.x, inner.y + 1, inner.width, text.len() as u16 + 2);
        buf.set_style(
            box_.intersection(buf.area),
            Style::default().bg(theme.panel),
        );
        for (i, l) in text.iter().enumerate() {
            buf.set_line(
                inner.x + 1,
                inner.y + 2 + i as u16,
                l,
                inner.width.saturating_sub(2),
            );
        }
        let mut y = box_.bottom();
        for l in &note {
            buf.set_line(inner.x, y, l, inner.width);
            y += 1;
        }
        buttons(
            inner.x,
            y + 1,
            buf,
            &[
                ("Copy", Variant::Primary, self.focus == 0, false),
                ("Close", Variant::Default, self.focus == 1, false),
            ],
            theme,
        );
    }
}

// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerFocus {
    Calendar,
    Time,
    Suggest,
    Ok,
    Cancel,
}

/// Calendar (+ time suggestions for date-times). Done with a `Value::Date`
/// / `Value::DateTime`, or `None` when cancelled.
pub struct DatePicker {
    pub label: String,
    pub date_only: bool,
    tz: Zone,
    tz_source: String,
    pub grid: CalendarGrid,
    pub time: LineInput,
    pub suggest: usize,
    pub focus: PickerFocus,
    pub preview: String,
    error: String,
}

impl DatePicker {
    pub fn new(label: &str, value: &Value, date_only: bool, tz: &Zone, tz_source: &str) -> Self {
        let today = jiff::Timestamp::now().to_zoned(tz.tz.clone()).date();
        Self::with_today(label, value, date_only, tz, tz_source, today)
    }

    pub fn with_today(
        label: &str,
        value: &Value,
        date_only: bool,
        tz: &Zone,
        tz_source: &str,
        today: Date,
    ) -> Self {
        let (day, time) = match value {
            Value::DateTime(v) => {
                // the same instant in the form zone: convert the whole
                // date-time, not only the date (an explicit offset would
                // shift the time)
                let local = v.timestamp().to_zoned(tz.tz.clone()).datetime();
                let t = if local.second() != 0 {
                    format!(
                        "{:02}:{:02}:{:02}",
                        local.hour(),
                        local.minute(),
                        local.second()
                    )
                } else {
                    format!("{:02}:{:02}", local.hour(), local.minute())
                };
                (local.date(), t)
            }
            Value::Date(d) => (*d, "09:00".to_string()),
            _ => (today, "09:00".to_string()),
        };
        let mut input = LineInput::new("HH:MM");
        input.set_value(&time);
        let mut p = DatePicker {
            label: label.into(),
            date_only,
            tz: tz.clone(),
            tz_source: tz_source.into(),
            grid: CalendarGrid::new(day, today),
            time: input,
            suggest: 0,
            focus: PickerFocus::Calendar,
            preview: String::new(),
            error: String::new(),
        };
        p.update();
        p
    }

    pub fn value(&mut self) -> Option<Value> {
        let day = self.grid.cursor;
        if self.date_only {
            return Some(Value::Date(day));
        }
        let text = format!("{} {}", fsv_forms::pydt::date_iso(day), self.time.value());
        let p = when::parse_when(
            &text,
            &self.tz,
            WhenOptions {
                tz_source: &self.tz_source,
                date_only: false,
                today: None,
            },
        );
        self.error = p.error.clone();
        p.error.is_empty().then_some(p.value)
    }

    fn update(&mut self) {
        self.error.clear();
        self.preview = match self.value() {
            Some(v) => when::preview(&v, &self.tz_source, false),
            None if !self.error.is_empty() => self.error.clone(),
            None => "type a time like 19:00".into(),
        };
    }

    fn ring(&self) -> Vec<PickerFocus> {
        if self.date_only {
            vec![PickerFocus::Calendar, PickerFocus::Ok, PickerFocus::Cancel]
        } else {
            vec![
                PickerFocus::Calendar,
                PickerFocus::Time,
                PickerFocus::Suggest,
                PickerFocus::Ok,
                PickerFocus::Cancel,
            ]
        }
    }

    fn ok(&mut self) -> Dlg<Option<Value>> {
        match self.value() {
            Some(v) => Dlg::Done(Some(v)),
            None => Dlg::Open,
        }
    }

    /// `DatePickerScreen._picked`: a date-only field is done, else on to
    /// the time.
    fn picked(&mut self, d: Date) -> Dlg<Option<Value>> {
        self.update();
        if self.date_only {
            return Dlg::Done(Some(Value::Date(d)));
        }
        self.focus = PickerFocus::Time;
        self.time.select_all();
        Dlg::Open
    }

    /// `DatePickerScreen._time`: a suggested time, then focus on OK.
    fn time(&mut self, index: usize) {
        self.time.set_value(TIME_SUGGESTIONS[index]);
        self.update();
        self.focus = PickerFocus::Ok;
    }

    /// `DatePickerScreen.on_key`: every key refreshes the preview.
    pub fn handle_key(&mut self, k: &KeyEvent) -> Dlg<Option<Value>> {
        let out = self.key(k);
        if out == Dlg::Open {
            self.update();
        }
        out
    }

    /// `DatePickerScreen._time_enter`: Enter in the time box is OK.
    fn time_enter(&mut self) -> Dlg<Option<Value>> {
        self.ok()
    }

    /// `DatePickerScreen._cancel` (and `action_cancel`, Esc).
    fn cancel(&self) -> Dlg<Option<Value>> {
        Dlg::Done(None)
    }

    fn key(&mut self, k: &KeyEvent) -> Dlg<Option<Value>> {
        if k.code == KeyCode::Esc {
            return self.cancel();
        }
        if let Some(by) = is_tab(k) {
            let ring = self.ring();
            let i = ring.iter().position(|f| *f == self.focus).unwrap_or(0);
            self.focus = ring[step(i, ring.len(), by)];
            if self.focus == PickerFocus::Time {
                self.time.select_all();
            }
            return Dlg::Open;
        }
        match self.focus {
            PickerFocus::Calendar => match self.grid.handle_key(k) {
                CalendarEvent::Picked(d) => self.picked(d),
                _ => Dlg::Open,
            },
            PickerFocus::Time => match self.time.handle_key(k) {
                InputEvent::Submitted => self.time_enter(),
                _ => Dlg::Open,
            },
            PickerFocus::Suggest => {
                let n = TIME_SUGGESTIONS.len();
                match k.code {
                    KeyCode::Down => self.suggest = (self.suggest + 1).min(n - 1),
                    KeyCode::Up => self.suggest = self.suggest.saturating_sub(1),
                    KeyCode::Home => self.suggest = 0,
                    KeyCode::End => self.suggest = n - 1,
                    KeyCode::Enter => self.time(self.suggest),
                    _ => {}
                }
                Dlg::Open
            }
            PickerFocus::Ok if press(k) => self.ok(),
            PickerFocus::Cancel if press(k) => self.cancel(),
            _ => Dlg::Open,
        }
    }

    pub fn paste(&mut self, text: &str) {
        if self.focus == PickerFocus::Time {
            self.time.paste(text);
            self.update();
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) -> Option<Position> {
        let w = 60;
        let inner_w = w.min(area.width * 98 / 100).saturating_sub(6);
        let preview = text_lines(&self.preview, inner_w, Style::default().fg(theme.success));
        let h = 1 + 9 + 1 + preview.len() as u16 + 1 + 1 + 3 + 4;
        let inner = dialog(area, buf, w, 98, h, theme);
        buf.set_stringn(
            inner.x,
            inner.y,
            &self.label,
            usize::from(inner.width),
            Style::default().add_modifier(Modifier::BOLD),
        );
        let body_y = inner.y + 1;
        self.grid.render(
            Rect::new(
                inner.x,
                body_y,
                crate::widgets::calendar::WIDTH,
                crate::widgets::calendar::HEIGHT,
            ),
            buf,
            self.focus == PickerFocus::Calendar,
            theme,
        );
        let mut cursor = None;
        if !self.date_only {
            let tx = inner.x + crate::widgets::calendar::WIDTH + 2;
            let tw = inner.right().saturating_sub(tx);
            cursor = self.time.render(
                Rect::new(tx, body_y, tw, 3),
                buf,
                self.focus == PickerFocus::Time,
                theme,
            );
            for (i, t) in TIME_SUGGESTIONS.iter().enumerate().take(6) {
                let mut style = Style::default();
                if i == self.suggest {
                    style = if self.focus == PickerFocus::Suggest {
                        style.bg(theme.primary).add_modifier(Modifier::BOLD)
                    } else {
                        style.add_modifier(Modifier::REVERSED)
                    };
                }
                buf.set_stringn(tx + 1, body_y + 3 + i as u16, t, usize::from(tw), style);
            }
        }
        let mut y = body_y + 9 + 1;
        for l in &preview {
            buf.set_line(inner.x, y, l, inner.width);
            y += 1;
        }
        buf.set_stringn(
            inner.x,
            y,
            "arrows move · PgUp/PgDn month · Home today · Enter picks",
            usize::from(inner.width),
            Style::default().add_modifier(Modifier::DIM),
        );
        buttons(
            inner.x,
            y + 2,
            buf,
            &[
                ("OK", Variant::Primary, self.focus == PickerFocus::Ok, false),
                (
                    "Cancel",
                    Variant::Default,
                    self.focus == PickerFocus::Cancel,
                    false,
                ),
            ],
            theme,
        );
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::parse_key;

    fn picker(date_only: bool) -> DatePicker {
        let (_acme, s) = crate::acme::acme();
        assert_eq!(s.tz_source, "Freshservice user time zone (Tokyo)");
        let today = jiff::civil::date(2030, 11, 26);
        DatePicker::with_today(
            "Planned Start",
            &Value::Null,
            date_only,
            &s.tz,
            &s.tz_source,
            today,
        )
    }

    /// `DatePickerScreen.value` / `_update`: the preview of what OK would
    /// give, or why it cannot.
    #[test]
    fn the_preview_follows_the_typed_time() {
        let mut p = picker(false);
        assert_eq!(p.time.value(), "09:00");
        assert_eq!(
            p.preview,
            "Tue 26 Nov 2030 09:00 (+09:00 from your Freshservice profile tz)"
        );
        match p.value() {
            Some(Value::DateTime(d)) => {
                assert_eq!(d.local, jiff::civil::date(2030, 11, 26).at(9, 0, 0, 0));
                assert_eq!(d.offset, jiff::tz::Offset::constant(9));
            }
            other => panic!("{other:?}"),
        }
        for (time, preview) in [
            (
                "9pm",
                "Tue 26 Nov 2030 21:00 (+09:00 from your Freshservice profile tz)",
            ),
            ("", "Tue 26 Nov 2030: add a time, e.g. 2030-11-26 19:00"),
            ("25:00", "'25:00' is not a time of day"),
        ] {
            p.time.set_value(time);
            p.update();
            assert_eq!(p.preview, preview, "{time:?}");
        }
        assert_eq!(p.value(), None);
        assert_eq!(p.ok(), Dlg::Open);
        p.time.set_value("19:00");
        assert!(matches!(p.ok(), Dlg::Done(Some(Value::DateTime(_)))));
    }

    #[test]
    fn a_date_only_picker_is_done_on_the_day() {
        let mut p = picker(true);
        assert_eq!(p.preview, "Tue 26 Nov 2030");
        assert_eq!(p.handle_key(&parse_key("right")), Dlg::Open);
        assert_eq!(
            p.handle_key(&parse_key("enter")),
            Dlg::Done(Some(Value::Date(jiff::civil::date(2030, 11, 27))))
        );
    }

    /// `_picked` moves on to the time (all of it selected), `_time` takes a
    /// suggestion and focuses OK, `_time_enter` is OK.
    #[test]
    fn picking_a_day_then_a_time() {
        let mut p = picker(false);
        assert_eq!(p.handle_key(&parse_key("enter")), Dlg::Open);
        assert_eq!(p.focus, PickerFocus::Time);
        assert_eq!(p.time.selection(), Some((0, 5)));
        p.handle_key(&parse_key("1"));
        assert_eq!(p.time.value(), "1");
        p.time(5);
        assert_eq!(p.time.value(), TIME_SUGGESTIONS[5]);
        assert_eq!(p.focus, PickerFocus::Ok);
        assert_eq!(
            p.preview,
            "Tue 26 Nov 2030 19:00 (+09:00 from your Freshservice profile tz)"
        );
        p.focus = PickerFocus::Time;
        assert!(matches!(
            p.handle_key(&parse_key("enter")),
            Dlg::Done(Some(Value::DateTime(_)))
        ));
        assert_eq!(p.handle_key(&parse_key("escape")), Dlg::Done(None));
    }

    /// `SavePathDialog`: the path is selected on open; Save and Cancel.
    #[test]
    fn save_path_buttons() {
        let dir = tempfile::tempdir().expect("tmp");
        let paths = Paths::new(None, dir.path().to_path_buf());
        let mut d = SavePathDialog::new(&dir.path().join("d.yaml"), "Save draft to");
        assert!(d.input.selection().is_some());
        d.handle_key(&parse_key("tab"), &paths);
        assert_eq!(
            d.handle_key(&parse_key("enter"), &paths),
            Dlg::Done(Some(dir.path().join("d.yaml")))
        );
        d.handle_key(&parse_key("tab"), &paths);
        assert_eq!(d.handle_key(&parse_key("enter"), &paths), Dlg::Done(None));
        d.handle_key(&parse_key("tab"), &paths);
        assert_eq!(d.focus, 0);
        assert!(d.input.selection().is_some());
        d.input.set_value(" ");
        assert_eq!(d.save(&paths), Dlg::Open);
        assert_eq!(d.note, "type a file path");
    }

    /// `SavePathDialog._save` / `_enter` / `_save_btn` / `_note`, as a
    /// Textual pilot of the Python dialog shows them: each check's note,
    /// Save twice to overwrite, a failed write keeps the dialog open.
    #[test]
    fn save_path_enter_and_save_button_check_the_path() {
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path().to_path_buf();
        let existing = root.join("a.yaml");
        std::fs::write(&existing, "").expect("write");
        let paths = Paths::new(None, root.clone());
        let mut d = SavePathDialog::new(&existing, "Save draft to");
        let try_ = |d: &mut SavePathDialog, v: &str, btn: bool| {
            d.input.set_value(v);
            let r = if btn {
                d.save_btn(&paths)
            } else {
                d.enter(&paths)
            };
            (r, d.note.clone())
        };
        assert_eq!(
            try_(&mut d, " ", false),
            (Dlg::Open, "type a file path".into())
        );
        let root_s = root.display().to_string();
        assert_eq!(
            try_(&mut d, &root_s, false),
            (Dlg::Open, format!("{root_s} is a folder: add a file name"))
        );
        let nope = root.join("nope").join("x.yaml");
        assert_eq!(
            try_(&mut d, &nope.display().to_string(), false),
            (
                Dlg::Open,
                format!(
                    "cannot save: the folder {} does not exist",
                    root.join("nope").display()
                )
            )
        );
        let ex = existing.display().to_string();
        assert_eq!(
            try_(&mut d, &ex, false),
            (
                Dlg::Open,
                format!("{ex} exists: press Save again to overwrite")
            )
        );
        assert_eq!(try_(&mut d, &ex, true).0, Dlg::Done(Some(existing.clone())));

        let b = root.join("b.yaml");
        let mut d = SavePathDialog::new(&b, "Save draft to");
        assert_eq!(d.enter(&paths), Dlg::Done(Some(b.clone())));
        d.failed(&b, &std::io::Error::from_raw_os_error(13));
        assert_eq!(
            d.note,
            format!("cannot save to {}: Permission denied", b.display())
        );
        d.set_note("x");
        assert_eq!(d.note, "x");
        assert_eq!(d.cancel_btn(), Dlg::Done(None));
        assert_eq!(d.handle_key(&parse_key("escape"), &paths), Dlg::Done(None));
    }

    /// `ConfirmDialog._pressed` / `action_yes` / `action_no`: a click on
    /// Discard is `True`, on Keep editing `False`; `y` yes, Esc no; Enter
    /// on the first focus (No) is `False`.
    #[test]
    fn confirm_buttons_and_actions() {
        let mut c = ConfirmDialog::new("Discard?", "Discard", "Keep editing");
        assert_eq!(c.pressed(), Dlg::Done(false));
        c.focus = 0;
        assert_eq!(c.pressed(), Dlg::Done(true));
        assert_eq!(c.action_yes(), Dlg::Done(true));
        assert_eq!(c.action_no(), Dlg::Done(false));
    }

    /// `TextDialog._copy` keeps the dialog open (the screen copies);
    /// `_close_btn` closes it.
    #[test]
    fn text_dialog_copy_and_close_buttons() {
        let t = TextDialog::new("Equivalent command", "fsv x", "");
        assert!(matches!(t.copy(), Dlg::Done(TextOut::Copy)));
        assert!(matches!(t.close_btn(), Dlg::Done(TextOut::Close)));
    }

    /// `DatePickerScreen._time_enter`: Enter in the time box is OK (19:00
    /// Tokyo), and an unreadable time keeps the picker open with its
    /// error. `_cancel` is `None`.
    #[test]
    fn picker_time_enter_and_cancel() {
        let mut p = picker(false);
        p.time.set_value("19:00");
        match p.time_enter() {
            Dlg::Done(Some(Value::DateTime(d))) => {
                assert_eq!(d.local, jiff::civil::date(2030, 11, 26).at(19, 0, 0, 0));
                assert_eq!(d.offset, jiff::tz::Offset::constant(9));
            }
            other => panic!("{other:?}"),
        }
        p.focus = PickerFocus::Time;
        p.time.set_value("25:00");
        assert_eq!(p.handle_key(&parse_key("enter")), Dlg::Open);
        assert_eq!(p.preview, "'25:00' is not a time of day");
        assert_eq!(p.cancel(), Dlg::Done(None));
        p.focus = PickerFocus::Cancel;
        assert_eq!(p.handle_key(&parse_key("enter")), Dlg::Done(None));
    }

    /// `ConfirmDialog`: `y` is Discard, Esc keeps editing, arrows move.
    #[test]
    fn confirm_keys() {
        let mut c = ConfirmDialog::new("Discard?", "Discard", "Keep editing");
        assert_eq!(c.focus, 1);
        assert_eq!(c.handle_key(&parse_key("enter")), Dlg::Done(false));
        assert_eq!(c.handle_key(&parse_key("y")), Dlg::Done(true));
        assert_eq!(c.handle_key(&parse_key("escape")), Dlg::Done(false));
        c.handle_key(&parse_key("left"));
        assert_eq!(c.handle_key(&parse_key("space")), Dlg::Done(true));
    }

    /// `TextDialog`: Copy has focus; Tab then Enter closes, Esc too.
    #[test]
    fn text_dialog_keys() {
        let mut t = TextDialog::new("Equivalent command", "fsv x", "");
        assert!(matches!(
            t.handle_key(&parse_key("enter")),
            Dlg::Done(TextOut::Copy)
        ));
        t.handle_key(&parse_key("tab"));
        assert!(matches!(
            t.handle_key(&parse_key("enter")),
            Dlg::Done(TextOut::Close)
        ));
        assert!(matches!(
            t.handle_key(&parse_key("escape")),
            Dlg::Done(TextOut::Close)
        ));
    }
}
