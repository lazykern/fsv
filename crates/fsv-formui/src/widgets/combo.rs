//! Combo box for dropdowns, multi-selects and people / group / agent
//! lookups: type to filter (choices) or to search (lookups); arrows move,
//! Enter picks, Esc closes. Multi-selects toggle with Enter and stay open.
//!
//! Results of an earlier remote search are never picked for the text typed
//! now, and typed person text is never committed as a value.

use crossterm::event::{KeyCode, KeyEvent};
use fsv_forms::Value;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use super::{COMBO_CLOSE, DEBOUNCE_LOOKUP, EdCx, EdMsg, Editor, TimerId};
use crate::kit::draw::wrap_line;
use crate::kit::{InputEvent, LineInput, Theme};
use crate::session::{FormItem, ItemView, Suggestion, WidgetKind};

const LIST_ROWS: usize = 7;

pub struct ComboBox {
    item: FormItem,
    pub multi: bool,
    pub remote: bool,
    /// Port of fsv.formui.widgets.ComboBox.input
    pub input: LineInput,
    /// Port of fsv.formui.widgets.ComboBox.is_open
    pub is_open: bool,
    pub options: Vec<Suggestion>,
    pub highlighted: Option<usize>,
    list_top: usize,
    /// The input text that reflects the stored value.
    pub shown_text: String,
    /// The query the shown (remote) options answer.
    pub options_query: Option<String>,
    pub note: String,
    chosen: String,
    /// Lookups started (tests).
    pub searches: usize,
    generation: u64,
}

impl ComboBox {
    pub fn new(item: &FormItem) -> Self {
        let multi = item.widget == WidgetKind::Multi;
        let remote = item.widget == WidgetKind::Lookup;
        let hint = if remote {
            "type to search"
        } else if multi {
            "type to filter, Enter toggles"
        } else {
            "type to filter, Enter picks"
        };
        let ph = if item.placeholder.is_empty() {
            hint.to_string()
        } else {
            item.placeholder.clone()
        };
        ComboBox {
            item: item.clone(),
            multi,
            remote,
            input: LineInput::new(ph),
            is_open: false,
            options: vec![],
            highlighted: None,
            list_top: 0,
            shown_text: String::new(),
            options_query: None,
            note: String::new(),
            chosen: String::new(),
            searches: 0,
            generation: 0,
        }
    }

    fn key(&self) -> String {
        self.item.key.clone()
    }

    /// A picked value lists everything.
    pub fn query(&self) -> String {
        let q = self.input.value();
        if !self.multi && q == self.shown_text {
            String::new()
        } else {
            q
        }
    }

    pub fn open(&mut self, cx: &mut EdCx<'_>) {
        if self.input.disabled {
            return;
        }
        if !self.is_open {
            self.is_open = true;
            cx.emit(EdMsg::Opened(self.key()));
        }
        if self.remote {
            let immediate = matches!(self.item.entity.as_deref(), Some("group" | "agent"));
            self.schedule_search(immediate, cx);
        } else {
            self.refresh_options(cx);
        }
    }

    /// The highlight is kept: a multi-select opens again on the same row.
    pub fn close(&mut self, focused: bool) {
        self.is_open = false;
        self.options.clear();
        self.options_query = None;
        self.note.clear();
        if !self.multi && self.input.value() != self.shown_text && !focused {
            let shown = self.shown_text.clone();
            self.input.set_value(&shown);
        }
    }

    fn refresh_options(&mut self, cx: &mut EdCx<'_>) {
        match cx.session.suggestions(&self.item.key, &self.query()) {
            Ok((sugg, note)) => self.show_options(sugg, &note),
            Err(e) => self.show_options(vec![], &e),
        }
    }

    pub fn show_options(&mut self, sugg: Vec<Suggestion>, note: &str) {
        let old = self.highlighted;
        self.options = sugg;
        self.highlighted = if self.options.is_empty() {
            None
        } else {
            match old {
                Some(i) if self.multi && i < self.options.len() => Some(i),
                _ => Some(0),
            }
        };
        if !self.multi || old != self.highlighted {
            self.list_top = 0;
        }
        self.set_note(note);
    }

    /// `ComboBox._note`: the highlighted choice's description, then `note`.
    fn set_note(&mut self, note: &str) {
        let desc = self
            .highlighted
            .and_then(|i| self.options.get(i))
            .map(|s| s.detail.clone())
            .filter(|d| !d.is_empty() && !self.remote)
            .unwrap_or_default();
        self.note = [desc.as_str(), note]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
    }

    fn typed(&mut self, cx: &mut EdCx<'_>) {
        if !self.multi && self.input.value() == self.shown_text && !self.is_open {
            return;
        }
        if !self.is_open {
            self.is_open = true;
            cx.emit(EdMsg::Opened(self.key()));
        }
        if self.remote {
            if self.options_query.as_deref() != Some(self.query().as_str()) {
                self.options_query = None;
                self.show_options(vec![], "searching…");
            }
            self.schedule_search(false, cx);
        } else {
            self.refresh_options(cx);
        }
    }

    fn schedule_search(&mut self, immediate: bool, cx: &mut EdCx<'_>) {
        let id = TimerId::Search(self.key());
        cx.timers.cancel(&id);
        if immediate {
            self.search(cx);
        } else {
            cx.timer(id, DEBOUNCE_LOOKUP);
        }
    }

    fn search(&mut self, cx: &mut EdCx<'_>) {
        let q = self.query();
        self.searches += 1;
        self.set_note("searching…");
        self.generation += 1;
        cx.emit(EdMsg::Search {
            key: self.key(),
            query: q,
            generation: self.generation,
        });
    }

    /// A finished search for `q` (generation `generation`): dropped when stale.
    pub fn found(&mut self, generation: Option<u64>, q: &str, sugg: Vec<Suggestion>, note: &str) {
        if generation.is_some_and(|g| g != self.generation) || q != self.query() || !self.is_open {
            return;
        }
        self.options_query = Some(q.to_string());
        self.show_options(sugg, note);
    }

    /// Remote options that do not answer what is typed now.
    pub fn stale(&self) -> bool {
        self.remote && self.options_query.as_deref() != Some(self.query().as_str())
    }

    fn enter(&mut self, cx: &mut EdCx<'_>) {
        if !self.is_open {
            self.open(cx);
            return;
        }
        if !self.multi && self.input.value().trim().is_empty() {
            self.shown_text.clear();
            cx.emit(EdMsg::Edited(self.key(), Value::Null));
            self.close(true);
            return;
        }
        if self.stale() {
            self.set_note("still searching… (Enter again when the results are shown)");
            return;
        }
        if let Some(i) = self.highlighted.filter(|i| *i < self.options.len()) {
            self.pick(i, cx);
        }
    }

    pub fn pick(&mut self, index: usize, cx: &mut EdCx<'_>) {
        let Some(s) = self.options.get(index).cloned() else {
            return;
        };
        if self.multi {
            let mut current: Vec<Value> = cx
                .session
                .view(&self.item.key)
                .value
                .as_list()
                .map(<[Value]>::to_vec)
                .unwrap_or_default();
            if let Some(pos) = current.iter().position(|v| *v == s.value) {
                current.remove(pos);
            } else {
                current.push(s.value.clone());
            }
            let v = if current.is_empty() {
                Value::Null
            } else {
                Value::List(current)
            };
            cx.emit(EdMsg::Edited(self.key(), v));
            self.input.set_value("");
            return;
        }
        let text = cx.session.text_for(&self.item, &s.value);
        self.shown_text = if text.is_empty() {
            s.label.clone()
        } else {
            text
        };
        let shown = self.shown_text.clone();
        self.input.set_value(&shown);
        cx.emit(EdMsg::Edited(self.key(), s.value));
        self.close(true);
    }

    /// Move the highlight `n` rows (opens a closed list first).
    fn action_cursor(&mut self, n: isize, cx: &mut EdCx<'_>) {
        if !self.is_open {
            self.open(cx);
            return;
        }
        if self.options.is_empty() {
            return;
        }
        let cur = self.highlighted.map_or(-1, |h| h as isize);
        let next = (cur + n).clamp(0, self.options.len() as isize - 1) as usize;
        if self.highlighted != Some(next) {
            self.highlighted = Some(next);
            self.on_highlight();
        }
    }

    /// `ComboBox._highlighted`: the note follows the highlighted choice.
    fn on_highlight(&mut self) {
        self.set_note("");
    }

    /// `ComboBox._maybe_close`: shortly after the input lost focus.
    fn maybe_close(&mut self, focused: bool) {
        if !focused {
            self.close(false);
        }
    }

    fn list_height(&self) -> u16 {
        if self.is_open && !self.options.is_empty() {
            self.options.len().min(LIST_ROWS) as u16 + 2
        } else {
            0
        }
    }

    fn note_lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.note.is_empty() {
            return vec![];
        }
        wrap_line(&Line::from(self.note.clone()), width.saturating_sub(2))
    }
}

impl Editor for ComboBox {
    fn disabled(&self) -> bool {
        self.input.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, _slot: usize, cx: &mut EdCx<'_>) -> bool {
        match k.code {
            KeyCode::Down => self.action_cursor(1, cx),
            KeyCode::Up => self.action_cursor(-1, cx),
            KeyCode::PageDown => self.action_cursor(8, cx),
            KeyCode::PageUp => self.action_cursor(-8, cx),
            KeyCode::Esc => {
                if !self.is_open {
                    return false;
                }
                self.close(true);
            }
            KeyCode::Enter => self.enter(cx),
            _ => match self.input.handle_key(k) {
                InputEvent::Changed => self.typed(cx),
                InputEvent::Handled | InputEvent::Submitted => {}
                InputEvent::Ignored => return false,
            },
        }
        true
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if self.input.paste(text) == InputEvent::Changed {
            self.typed(cx);
        }
    }

    /// `ComboBox.on_descendant_focus`: the list opens.
    fn focus(&mut self, _slot: usize, cx: &mut EdCx<'_>) {
        cx.timers.cancel(&TimerId::ComboClose(self.key()));
        self.input.select_all();
        self.open(cx);
    }

    /// `ComboBox.on_descendant_blur`: closes after [`COMBO_CLOSE`].
    fn blur(&mut self, _slot: usize, cx: &mut EdCx<'_>) {
        cx.timer(TimerId::ComboClose(self.key()), COMBO_CLOSE);
    }

    fn timer(&mut self, id: &TimerId, focused: bool, cx: &mut EdCx<'_>) {
        match id {
            TimerId::Search(_) => self.search(cx),
            TimerId::ComboClose(_) => self.maybe_close(focused),
            _ => {}
        }
    }

    fn sync(&mut self, view: &ItemView, focused: bool, cx: &mut EdCx<'_>) {
        self.input.disabled = view.disabled;
        if self.multi {
            self.chosen = view.text.clone();
            if self.is_open && !self.remote {
                self.refresh_options(cx);
            }
            return;
        }
        if !focused && self.input.value() != view.text {
            self.shown_text = view.text.clone();
            self.input.set_value(&view.text);
        } else if self.is_open && !self.remote {
            self.refresh_options(cx);
        }
    }

    fn dirty(&self) -> bool {
        let text = self.input.value();
        !text.trim().is_empty() && (self.multi || text != self.shown_text)
    }

    fn height(&self, width: u16) -> u16 {
        u16::from(self.multi) + 3 + self.list_height() + self.note_lines(width).len() as u16
    }

    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        let mut y = area.y;
        let pad = area.width.saturating_sub(2);
        if self.multi {
            let line = if self.chosen.is_empty() {
                Line::styled(
                    "nothing selected",
                    Style::default().add_modifier(Modifier::DIM),
                )
            } else {
                Line::styled(
                    format!("✓ {}", self.chosen),
                    Style::default().fg(theme.accent),
                )
            };
            buf.set_line(area.x + 1, y, &line, pad);
            y += 1;
        }
        let cursor = self.input.render(
            Rect::new(area.x, y, area.width, 3),
            buf,
            focus.is_some(),
            theme,
        );
        y += 3;
        let lh = self.list_height();
        if lh > 0 {
            let r = Rect::new(area.x, y, area.width, lh);
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.accent));
            let inner = block.inner(r);
            block.render(r, buf);
            let h = self.highlighted.unwrap_or(0);
            if h < self.list_top {
                self.list_top = h;
            } else if h >= self.list_top + LIST_ROWS {
                self.list_top = h + 1 - LIST_ROWS;
            }
            for (i, s) in self
                .options
                .iter()
                .enumerate()
                .skip(self.list_top)
                .take(LIST_ROWS)
            {
                let mut spans = vec![];
                if self.multi {
                    if s.selected {
                        spans.push(Span::styled(
                            "[x] ",
                            Style::default().add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(Span::raw("[ ] "));
                    }
                }
                spans.push(Span::raw(s.label.clone()));
                if !s.detail.is_empty() {
                    spans.push(Span::styled(
                        format!("  {}", s.detail),
                        Style::default().add_modifier(Modifier::DIM),
                    ));
                }
                let row = inner.y + (i - self.list_top) as u16;
                let mut line = Line::from(spans);
                if Some(i) == self.highlighted {
                    line = line.style(Style::default().bg(theme.primary).fg(theme.text));
                    buf.set_style(
                        Rect::new(inner.x, row, inner.width, 1),
                        Style::default().bg(theme.primary),
                    );
                }
                buf.set_line(inner.x, row, &line, inner.width);
            }
            y += lh;
        }
        for l in self.note_lines(area.width) {
            let l = l.style(
                Style::default()
                    .fg(theme.text_muted)
                    .add_modifier(Modifier::DIM),
            );
            buf.set_line(area.x + 1, y, &l, pad);
            y += 1;
        }
        cursor
    }

    fn as_combo(&mut self) -> Option<&mut ComboBox> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::TestCx;

    fn combo(h: &TestCx, key: &str) -> ComboBox {
        ComboBox::new(&h.item(key))
    }

    fn labels(c: &ComboBox) -> Vec<&str> {
        c.options.iter().map(|s| s.label.as_str()).collect()
    }

    /// `ComboBox.refresh_options` and `_note`: the filtered choices, the
    /// first highlighted, its description first in the note.
    #[test]
    fn refresh_options_and_note() {
        let mut h = TestCx::new();
        let mut c = combo(&h, "priority");
        h.cx(|cx| c.refresh_options(cx));
        assert_eq!(labels(&c), ["Low", "Medium", "High", "Urgent"]);
        assert_eq!(c.highlighted, Some(0));
        assert_eq!(c.note, "Can wait");
        c.set_note("x");
        assert_eq!(c.note, "Can wait · x");

        let mut c = combo(&h, "change_type");
        c.input.set_value("em");
        h.cx(|cx| c.refresh_options(cx));
        assert_eq!(labels(&c), ["Emergency"]);
        assert_eq!(c.note, "");
        c.input.set_value("zzz");
        h.cx(|cx| c.refresh_options(cx));
        assert!(c.options.is_empty());
        assert_eq!(c.highlighted, None);
        assert_eq!(c.note, "no choice matches");
    }

    /// `ComboBox._query`: the text of the picked value lists everything on
    /// a single box; a multi-select always filters by what is typed.
    #[test]
    fn query_ignores_the_picked_text() {
        let h = TestCx::new();
        let mut c = combo(&h, "change_type");
        assert_eq!(c.query(), "");
        c.input.set_value("em");
        assert_eq!(c.query(), "em");
        c.shown_text = "Emergency".into();
        c.input.set_value("Emergency");
        assert_eq!(c.query(), "");
        c.input.set_value("Emergenc");
        assert_eq!(c.query(), "Emergenc");

        let mut m = combo(&h, "services");
        assert!(m.multi);
        m.shown_text = "Email".into();
        m.input.set_value("Email");
        assert_eq!(m.query(), "Email");
    }

    /// `ComboBox.pick`: a single box stores the value, shows its text and
    /// closes; a multi-select toggles the value and clears the input.
    #[test]
    fn pick_stores_or_toggles() {
        let mut h = TestCx::new();
        let mut c = combo(&h, "change_type");
        h.cx(|cx| c.open(cx));
        h.take();
        let i = labels(&c)
            .iter()
            .position(|l| *l == "Emergency")
            .expect("Emergency");
        let want = c.options[i].value.clone();
        h.cx(|cx| c.pick(i, cx));
        assert_eq!(h.take(), [EdMsg::Edited("change_type".into(), want)]);
        assert_eq!(c.shown_text, "Emergency");
        assert_eq!(c.input.value(), "Emergency");
        assert!(!c.is_open);
        h.cx(|cx| c.pick(99, cx));
        assert!(h.take().is_empty());

        let mut m = combo(&h, "services");
        h.cx(|cx| m.open(cx));
        h.take();
        let first = m.options[0].value.clone();
        m.input.set_value("x");
        h.cx(|cx| m.pick(0, cx));
        assert_eq!(
            h.take(),
            [EdMsg::Edited(
                "services".into(),
                Value::List(vec![first.clone()])
            )]
        );
        assert_eq!(m.input.value(), "");
        assert!(m.is_open);
        h.s.set("services", Value::List(vec![first]));
        h.cx(|cx| m.pick(0, cx));
        assert_eq!(h.take(), [EdMsg::Edited("services".into(), Value::Null)]);
    }

    /// `ComboBox.action_cursor` and `_highlighted`: a closed list opens
    /// first; then the highlight moves (clamped) and the note follows it.
    #[test]
    fn action_cursor_opens_then_moves() {
        let mut h = TestCx::new();
        let mut c = combo(&h, "priority");
        h.cx(|cx| c.action_cursor(1, cx));
        assert!(c.is_open);
        assert_eq!(h.take(), [EdMsg::Opened("priority".into())]);
        assert_eq!(c.highlighted, Some(0));
        h.cx(|cx| c.action_cursor(1, cx));
        assert_eq!((c.highlighted, c.note.as_str()), (Some(1), "Planned work"));
        h.cx(|cx| c.action_cursor(8, cx));
        assert_eq!((c.highlighted, c.note.as_str()), (Some(3), "Outage"));
        h.cx(|cx| c.action_cursor(-8, cx));
        assert_eq!(c.highlighted, Some(0));
        c.highlighted = Some(2);
        c.on_highlight();
        assert_eq!(c.note, "Revenue at risk");
    }

    /// A pilot of the Python form: Down twice in Services, leave, come back:
    /// the third row is highlighted again. A move that stays put (Up on the
    /// first row) keeps the note, as no highlight event fires.
    #[test]
    fn multi_reopens_on_the_same_row_and_a_stuck_cursor_keeps_the_note() {
        let mut h = TestCx::new();
        let mut m = combo(&h, "services");
        h.cx(|cx| m.open(cx));
        h.cx(|cx| m.action_cursor(2, cx));
        assert_eq!(m.highlighted, Some(2));
        m.close(false);
        h.cx(|cx| m.open(cx));
        assert_eq!(m.highlighted, Some(2));

        let mut c = combo(&h, "priority");
        h.cx(|cx| c.open(cx));
        c.set_note("searching…");
        h.cx(|cx| c.action_cursor(-1, cx));
        assert_eq!(c.note, "Can wait · searching…");
    }

    /// `ComboBox._maybe_close`: closes only once the input lost focus, and
    /// puts back the text of the stored value.
    #[test]
    fn maybe_close_after_blur() {
        let mut h = TestCx::new();
        let mut c = combo(&h, "change_type");
        h.cx(|cx| c.open(cx));
        c.input.set_value("Sta");
        c.maybe_close(true);
        assert!(c.is_open);
        c.maybe_close(false);
        assert!(!c.is_open);
        assert!(c.options.is_empty());
        assert_eq!(c.input.value(), "");
    }

    /// `ComboBox._schedule_search` / `_search`: a pause before a people
    /// search, at once for groups and agents; each search is a new
    /// generation and says "searching…".
    #[test]
    fn schedule_search_and_search() {
        let mut h = TestCx::new();
        let mut c = combo(&h, "requester");
        assert!(c.remote);
        c.input.set_value("jan");
        h.cx(|cx| c.schedule_search(false, cx));
        assert!(h.timers.is_set(&TimerId::Search("requester".into())));
        assert_eq!(c.searches, 0);
        h.cx(|cx| c.schedule_search(true, cx));
        assert!(!h.timers.is_set(&TimerId::Search("requester".into())));
        assert_eq!(c.searches, 1);
        assert_eq!(c.note, "searching…");
        h.cx(|cx| c.search(cx));
        assert_eq!(
            h.take(),
            [
                EdMsg::Search {
                    key: "requester".into(),
                    query: "jan".into(),
                    generation: 1
                },
                EdMsg::Search {
                    key: "requester".into(),
                    query: "jan".into(),
                    generation: 2
                },
            ]
        );
        // a group lookup searches as soon as it opens
        let mut g = combo(&h, "group");
        h.cx(|cx| g.open(cx));
        assert_eq!(g.searches, 1);
    }

    /// `ComboBox._enter`: opens a closed list; on an emptied single box
    /// clears the value; refuses stale remote results; else picks the
    /// highlighted option.
    #[test]
    fn enter_opens_clears_refuses_stale_and_picks() {
        let mut h = TestCx::new();
        let mut c = combo(&h, "change_type");
        h.cx(|cx| c.enter(cx));
        assert!(c.is_open);
        h.take();
        c.input.set_value("em");
        h.cx(|cx| c.refresh_options(cx));
        h.cx(|cx| c.enter(cx));
        assert!(matches!(&h.take()[..], [EdMsg::Edited(k, _)] if k == "change_type"));
        assert_eq!(c.input.value(), "Emergency");
        assert!(!c.is_open);
        h.cx(|cx| c.enter(cx));
        h.take();
        c.input.set_value(" ");
        h.cx(|cx| c.enter(cx));
        assert_eq!(h.take(), [EdMsg::Edited("change_type".into(), Value::Null)]);
        assert!(!c.is_open);

        let mut r = combo(&h, "requester");
        h.cx(|cx| r.open(cx));
        r.input.set_value("jan");
        h.cx(|cx| r.enter(cx));
        assert_eq!(
            r.note,
            "still searching… (Enter again when the results are shown)"
        );
    }
}
