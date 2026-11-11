//! The form screen, its review screen and the small dialogs.
//!
//! A [`FormScreen`] is driven from outside: [`FormScreen::handle_key`],
//! [`FormScreen::handle_paste`], [`FormScreen::tick`] (timers and lookup
//! results) and [`FormScreen::render`]. It never reads the clock or the
//! terminal itself, so it runs the same under a real terminal and under
//! ratatui's `TestBackend`.

pub mod dialogs;
pub mod review;

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fsv_forms::Value;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::backend::{FormMode, Plan};
use crate::kit::draw::{
    Button, Severity, Toast, Variant, draw_lines, fill, toasts, wrap_line, wrap_lines,
};
use crate::kit::markup::{self, escape};
use crate::kit::{Theme, Timers, Workers, WorkersMode};
use crate::session::{FormSession, Suggestion, WidgetKind};
use crate::widgets::{EdCx, EdMsg, Editor, FieldRow, TimerId};
use dialogs::{ConfirmDialog, DatePicker, Dlg, SavePathDialog, TextDialog, TextOut};
use review::{ReviewAction, ReviewState};

pub const NOTE_SECONDS: Duration = Duration::from_secs(8);
pub const NARROW: u16 = 100;
pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 12;
const TOAST_SECONDS: Duration = Duration::from_secs(5);

/// How the form ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Submit,
    SaveDraft,
    Cancel,
}

/// What the form screen finishes with. Nothing is sent: a `Submit` result
/// carries the reviewed plan for the caller's send path.
pub struct FormResult {
    pub action: Action,
    pub plan: Option<Plan>,
    pub saved_path: Option<PathBuf>,
    pub session: FormSession,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormButton {
    Review,
    Save,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// Row index, editor slot.
    Field(usize, usize),
    Button(FormButton),
}

/// Where a save dialog came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveFor {
    Form,
    Review,
    Command,
}

enum Layer {
    Review(Box<ReviewState>),
    Confirm(ConfirmDialog),
    SavePath(SavePathDialog, SaveFor),
    Text(TextDialog),
    DatePicker(Box<DatePicker>, String),
}

type LookupMeta = (String, u64, String);
type LookupResult = Result<(Vec<Suggestion>, String), String>;

pub struct FormScreen {
    pub session: FormSession,
    pub rows: Vec<FieldRow>,
    pub focus: Option<Focus>,
    pub theme: Theme,
    now: Duration,
    timers: Timers<TimerId>,
    workers: Workers<LookupMeta, LookupResult>,
    pending: Vec<EdMsg>,
    layers: Vec<Layer>,
    pub notes: Vec<String>,
    pub status: String,
    review_disabled: bool,
    help_key: Option<String>,
    help_markup: String,
    pub hide_help: bool,
    toasts: Vec<(u64, Toast)>,
    toast_seq: u64,
    scroll: usize,
    scroll_to_focus: bool,
    clipboard: Option<String>,
    done: Option<Action>,
    saved_path: Option<PathBuf>,
}

/// `" ".join(text.split())[:200]`
fn squash(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

/// Footer key hints.
pub fn footer(area: Rect, buf: &mut Buffer, keys: &[(&str, &str)], theme: &Theme) {
    fill(
        area,
        buf,
        Style::default().bg(theme.footer_bg).fg(theme.foreground),
    );
    let mut spans = vec![];
    for (k, d) in keys {
        spans.push(Span::styled(
            format!(" {k} "),
            Style::default()
                .fg(theme.footer_key)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!("{d} ")));
    }
    buf.set_line(area.x, area.y, &Line::from(spans), area.width);
}

/// Draw with `f` on a copy of `area` with room to spare on the right and
/// below, then keep what falls inside `area`: content that does not fit a
/// small terminal is cut instead of written out of bounds.
fn clipped<R>(area: Rect, buf: &mut Buffer, f: impl FnOnce(&mut Buffer) -> R) -> R {
    const SPARE: u16 = 128;
    let big = Rect::new(
        area.x,
        area.y,
        area.width.saturating_add(SPARE),
        area.height.saturating_add(SPARE),
    );
    let mut scratch = Buffer::empty(big);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let (Some(src), Some(dst)) = (buf.cell((x, y)), scratch.cell_mut((x, y))) {
                *dst = src.clone();
            }
        }
    }
    let r = f(&mut scratch);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let (Some(src), Some(dst)) = (scratch.cell((x, y)), buf.cell_mut((x, y))) {
                *dst = src.clone();
            }
        }
    }
    r
}

fn kind_hint(kind: WidgetKind) -> Option<&'static str> {
    Some(match kind {
        WidgetKind::Choice => "Type to filter · ↑↓ move · Enter picks · Esc closes",
        WidgetKind::Multi => "Type to filter · ↑↓ move · Enter toggles · Esc closes",
        WidgetKind::Lookup => "Type to search · ↑↓ move · Enter picks",
        WidgetKind::Date => "Type a date or press F2 for the calendar",
        WidgetKind::Datetime => "Type a date and time or press F2 for the calendar",
        WidgetKind::File => "Paths separated by ';' · → completes the path",
        WidgetKind::Richtext => "Markdown, sent as HTML · F3 preview",
        _ => return None,
    })
}

impl FormScreen {
    pub fn new(session: FormSession, theme: Theme) -> Self {
        Self::with_workers(session, theme, WorkersMode::Threads)
    }

    /// `WorkersMode::Inline` runs lookups at once (tests).
    pub fn with_workers(session: FormSession, theme: Theme, mode: WorkersMode) -> Self {
        let rows = session
            .items
            .iter()
            .map(|it| FieldRow::new(it, &session))
            .collect();
        let mut s = FormScreen {
            session,
            rows,
            focus: None,
            theme,
            now: Duration::ZERO,
            timers: Timers::default(),
            workers: Workers::new(mode),
            pending: vec![],
            layers: vec![],
            notes: vec![],
            status: String::new(),
            review_disabled: false,
            help_key: None,
            help_markup: String::new(),
            hide_help: false,
            toasts: vec![],
            toast_seq: 0,
            scroll: 0,
            scroll_to_focus: false,
            clipboard: None,
            done: None,
            saved_path: None,
        };
        s.refresh_rows();
        let notes = s.session.load_notes.clone();
        s.show_notes(&notes);
        s.focus_first();
        s.process();
        s
    }

    // -- results -------------------------------------------------------------
    pub fn done(&self) -> Option<Action> {
        self.done
    }

    pub fn into_result(self) -> FormResult {
        let action = self.done.unwrap_or(Action::Cancel);
        let plan = match action {
            Action::Cancel => None,
            _ => self.session.plan.clone(),
        };
        let saved_path = match action {
            Action::SaveDraft => self
                .saved_path
                .clone()
                .or_else(|| self.session.saved_path.clone()),
            _ => None,
        };
        FormResult {
            action,
            plan,
            saved_path,
            session: self.session,
        }
    }

    fn finish(&mut self, action: Action) {
        self.done = Some(action);
    }

    /// Quit at once (the app's Ctrl+Q): a cancel, nothing asked.
    pub fn quit(&mut self) {
        self.finish(Action::Cancel);
    }

    /// Text the Copy button asked to put on the clipboard.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    pub fn now(&self) -> Duration {
        self.now
    }

    /// The next timer deadline (the app polls until then).
    pub fn next_deadline(&self) -> Option<Duration> {
        self.timers.next_deadline()
    }

    /// Lookups still running.
    pub fn busy(&self) -> bool {
        self.workers.pending() > 0
    }

    pub fn notify(&mut self, text: &str, title: &str, severity: Severity, timeout: Duration) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        self.toasts.push((
            id,
            Toast {
                text: text.into(),
                title: title.into(),
                severity,
                until: self.now + timeout,
            },
        ));
        self.timers.set(TimerId::Toast(id), self.now, timeout);
    }

    pub fn toast_texts(&self) -> Vec<String> {
        self.toasts.iter().map(|(_, t)| t.text.clone()).collect()
    }

    /// The open layer, by name (tests): `review`, `confirm`, `save`,
    /// `text`, `calendar`.
    pub fn layer(&self) -> Option<&'static str> {
        self.layers.last().map(|l| match l {
            Layer::Review(_) => "review",
            Layer::Confirm(_) => "confirm",
            Layer::SavePath(..) => "save",
            Layer::Text(_) => "text",
            Layer::DatePicker(..) => "calendar",
        })
    }

    pub fn review(&self) -> Option<&ReviewState> {
        self.layers.iter().rev().find_map(|l| match l {
            Layer::Review(r) => Some(&**r),
            _ => None,
        })
    }

    pub fn save_dialog(&mut self) -> Option<&mut SavePathDialog> {
        match self.layers.last_mut() {
            Some(Layer::SavePath(d, _)) => Some(d),
            _ => None,
        }
    }

    pub fn date_picker(&mut self) -> Option<&mut DatePicker> {
        match self.layers.last_mut() {
            Some(Layer::DatePicker(d, _)) => Some(d),
            _ => None,
        }
    }

    pub fn text_dialog(&self) -> Option<&TextDialog> {
        match self.layers.last() {
            Some(Layer::Text(d)) => Some(d),
            _ => None,
        }
    }

    pub fn row_index(&self, key: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.key() == key)
    }

    pub fn editor_mut(&mut self, key: &str) -> Option<&mut (dyn Editor + 'static)> {
        let i = self.row_index(key)?;
        self.rows[i].editor.as_deref_mut()
    }

    pub fn focused_key(&self) -> Option<&str> {
        match self.focus {
            Some(Focus::Field(i, _)) => Some(self.rows[i].key()),
            _ => None,
        }
    }

    // -- editor plumbing -----------------------------------------------------
    fn with_editor<R>(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut dyn Editor, &mut EdCx<'_>) -> R,
    ) -> Option<R> {
        let mut out = vec![];
        let r = {
            let FormScreen {
                rows,
                session,
                timers,
                now,
                ..
            } = self;
            let ed = rows.get_mut(i)?.editor.as_deref_mut()?;
            let mut cx = EdCx {
                session,
                now: *now,
                timers,
                out: &mut out,
            };
            f(ed, &mut cx)
        };
        self.pending.extend(out);
        Some(r)
    }

    fn input_focused(&self, i: usize) -> bool {
        self.focus == Some(Focus::Field(i, 0)) && self.layers.is_empty()
    }

    /// Apply what the editors reported, until nothing is left.
    fn process(&mut self) {
        while !self.pending.is_empty() {
            for m in std::mem::take(&mut self.pending) {
                match m {
                    EdMsg::Edited(key, value) => self.edited(&key, value),
                    EdMsg::Invalid(key, error) => self.invalid(&key, &error),
                    EdMsg::OpenCalendar(key) => self.open_calendar(&key),
                    EdMsg::Search {
                        key,
                        query,
                        generation,
                    } => self.start_search(key, query, generation),
                    EdMsg::FocusNext => self.focus_step(1),
                    EdMsg::Opened(_) => self.opened(),
                }
            }
        }
    }

    /// `FormScreen._edited`: apply a committed value, re-sync every row and
    /// show what the rules did.
    fn edited(&mut self, key: &str, value: Value) {
        let out = self.session.set(key, value);
        self.refresh_rows();
        if !out.notes.is_empty() {
            self.show_notes(&out.notes);
        }
    }

    /// `FormScreen._invalid`: unreadable input blocks the review.
    fn invalid(&mut self, key: &str, error: &str) {
        self.session.mark_invalid(key, error);
        self.refresh_rows();
    }

    /// `FormScreen._opened`: an opened list scrolls into view.
    fn opened(&mut self) {
        self.scroll_to_focus = true;
    }

    fn start_search(&mut self, key: String, query: String, generation: u64) {
        match self.session.suggest_job(&key) {
            Some(job) => {
                let q = query.clone();
                self.workers
                    .spawn((key, generation, query), move || job.run(&q));
            }
            None => {
                let r = self.session.suggestions(&key, &query);
                self.deliver(&key, generation, &query, r);
            }
        }
    }

    fn deliver(&mut self, key: &str, generation: u64, query: &str, r: LookupResult) {
        let (sugg, note) = match r {
            Ok(x) => x,
            Err(e) => (vec![], squash(&e)),
        };
        if let Some(combo) = self.editor_mut(key).and_then(|e| e.as_combo()) {
            combo.found(Some(generation), query, sugg, &note);
        }
    }

    // -- syncing -------------------------------------------------------------
    pub fn refresh_rows(&mut self) {
        for i in 0..self.rows.len() {
            let focused = self.input_focused(i);
            let mut out = vec![];
            {
                let FormScreen {
                    rows,
                    session,
                    timers,
                    now,
                    ..
                } = self;
                let mut cx = EdCx {
                    session,
                    now: *now,
                    timers,
                    out: &mut out,
                };
                rows[i].sync(focused, &mut cx);
            }
            self.pending.extend(out);
        }
        let problems = self.session.problems();
        let missing: Vec<String> = self
            .session
            .missing_keys()
            .iter()
            .filter_map(|k| self.session.item(k).map(|it| it.label.clone()))
            .collect();
        self.review_disabled = !problems.is_empty();
        let mut status = if !missing.is_empty() {
            let mut shown = missing
                .iter()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if missing.len() > 4 {
                shown.push_str(&format!(" +{}", missing.len() - 4));
            }
            format!("{} required field(s) missing: {shown}", missing.len())
        } else if let Some(p) = problems.first() {
            p.clone()
        } else {
            "Ready: Ctrl+S to review and submit".into()
        };
        status.push_str("  ·  Ctrl+S review · F4 save draft · Esc cancel");
        self.status = status;
        if let Some(k) = self.help_key.clone() {
            self.show_help(&k, true);
        }
        self.fix_focus();
    }

    /// A focused row that was hidden or disabled hands focus on.
    fn fix_focus(&mut self) {
        let ring = self.ring();
        match self.focus {
            Some(f) if !ring.contains(&f) => {
                let next = match f {
                    Focus::Field(i, _) => ring.iter().copied().find(|g| match g {
                        Focus::Field(j, _) => *j > i,
                        Focus::Button(_) => true,
                    }),
                    Focus::Button(_) => {
                        ring.iter().copied().find(|g| matches!(g, Focus::Button(_)))
                    }
                };
                self.focus = next;
                if let Some(Focus::Field(j, s)) = next {
                    self.with_editor(j, |e, cx| e.focus(s, cx));
                    self.on_descendant_focus(j);
                }
            }
            _ => {}
        }
    }

    pub fn show_notes(&mut self, notes: &[String]) {
        if notes.is_empty() {
            return;
        }
        let start = notes.len().saturating_sub(4);
        self.notes = notes[start..].to_vec();
        self.notify(
            &notes.join("\n"),
            "",
            Severity::Information,
            Duration::from_secs(6),
        );
        self.timers.set(TimerId::NotesClear, self.now, NOTE_SECONDS);
    }

    pub fn show_help(&mut self, key: &str, force: bool) {
        if self.help_key.as_deref() == Some(key) && !force {
            return;
        }
        self.help_key = Some(key.to_string());
        let h = self.session.help_for(key);
        let mut lines = vec![format!(
            "[b]{}[/b]{}",
            escape(&h.label),
            if h.required {
                " [bold red]* required[/]"
            } else {
                " [dim]optional[/]"
            }
        )];
        if !h.help.is_empty() {
            lines.push(escape(&h.help));
        }
        if !h.placeholder.is_empty() {
            lines.push(format!("[dim]e.g. {}[/]", escape(&h.placeholder)));
        }
        if !h.disabled.is_empty() {
            lines.push(format!("[yellow]read-only: {}[/]", escape(&h.disabled)));
        }
        if !h.note.is_empty() {
            lines.push(format!("[yellow]{}[/]", escape(&h.note)));
        }
        if !h.choices.is_empty() {
            lines.push(String::new());
            lines.push("[b]Choices[/b]".into());
            for (label, desc) in h.choices.iter().take(40) {
                let d = if desc.is_empty() {
                    String::new()
                } else {
                    format!(" [dim]— {}[/]", escape(desc))
                };
                lines.push(format!(" • {}{d}", escape(label)));
            }
            if h.choices.len() > 40 {
                lines.push(format!(
                    " [dim]… {} in all; type to filter[/]",
                    h.choices.len()
                ));
            }
        }
        if !h.effects.is_empty() {
            lines.push(String::new());
            lines.push("[b]Rule effects[/b]".into());
            lines.extend(h.effects.iter().map(|e| format!(" {}", escape(e))));
        }
        if !h.why.is_empty() {
            lines.push(String::new());
            lines.extend(h.why.iter().map(|w| format!("[dim]{}[/]", escape(w))));
        }
        if let Some(hint) = h.kind.and_then(kind_hint) {
            lines.push(String::new());
            lines.push(format!("[dim]{hint}[/]"));
        }
        self.help_markup = lines.join("\n");
    }

    pub fn help_text(&self) -> String {
        markup::plain(&self.help_markup)
    }

    // -- focus ---------------------------------------------------------------
    fn ring(&self) -> Vec<Focus> {
        let mut ring = vec![];
        for (i, row) in self.rows.iter().enumerate() {
            if row.focusable() {
                let slots = row.editor.as_ref().map_or(1, |e| e.slots());
                ring.extend((0..slots).map(|s| Focus::Field(i, s)));
            }
        }
        if !self.review_disabled {
            ring.push(Focus::Button(FormButton::Review));
        }
        ring.push(Focus::Button(FormButton::Save));
        ring.push(Focus::Button(FormButton::Cancel));
        ring
    }

    pub fn set_focus(&mut self, to: Option<Focus>) {
        if self.focus == to {
            return;
        }
        if let Some(Focus::Field(i, s)) = self.focus {
            self.with_editor(i, |e, cx| e.blur(s, cx));
        }
        self.focus = to;
        if let Some(Focus::Field(i, s)) = to {
            self.with_editor(i, |e, cx| e.focus(s, cx));
            self.on_descendant_focus(i);
        }
        self.scroll_to_focus = true;
        self.process();
    }

    /// `FormScreen.on_descendant_focus`: the help panel follows the
    /// focused row.
    fn on_descendant_focus(&mut self, row: usize) {
        let key = self.rows[row].key().to_string();
        self.show_help(&key, false);
    }

    /// Focus the editor of `key` (slot 0, its input), then [`Editor::focus`].
    ///
    /// Port of fsv.formui.widgets.FieldRow.focus_editor, and of the
    /// `focus_editor` of EditorMixin, ComboBox, DescriptionEditor,
    /// DateEditor and FileEditor.
    pub fn focus_key(&mut self, key: &str) {
        if let Some(i) = self.row_index(key)
            && self.rows[i].focusable()
        {
            self.set_focus(Some(Focus::Field(i, 0)));
        }
    }

    /// `FormScreen._focus_first`: the first visible, enabled field.
    fn focus_first(&mut self) {
        for key in self.session.visible_keys() {
            if let Some(i) = self.row_index(&key)
                && self.rows[i].editor.is_some()
                && !self.session.view(&key).disabled
            {
                self.set_focus(Some(Focus::Field(i, 0)));
                return;
            }
        }
    }

    fn focus_step(&mut self, by: isize) {
        let ring = self.ring();
        if ring.is_empty() {
            return;
        }
        let next = match self.focus.and_then(|f| ring.iter().position(|g| *g == f)) {
            Some(i) => ring[(i as isize + by).rem_euclid(ring.len() as isize) as usize],
            None if by > 0 => ring[0],
            None => ring[ring.len() - 1],
        };
        self.set_focus(Some(next));
    }

    /// A modal opens over the form: the focused editor loses focus (and
    /// commits), as in Textual.
    fn push(&mut self, layer: Layer) {
        if self.layers.is_empty()
            && let Some(Focus::Field(i, s)) = self.focus
        {
            self.with_editor(i, |e, cx| e.blur(s, cx));
        }
        self.layers.push(layer);
        self.process();
    }

    /// Back on the form: the editor that had focus gets it again.
    fn pop(&mut self) {
        self.layers.pop();
        if self.layers.is_empty()
            && let Some(Focus::Field(i, s)) = self.focus
        {
            self.with_editor(i, |e, cx| e.focus(s, cx));
            self.refresh_rows();
        }
        self.process();
    }

    // -- events --------------------------------------------------------------
    /// Advance the clock to `now`: fire timers, take lookup results.
    pub fn tick(&mut self, now: Duration) {
        self.now = self.now.max(now);
        for ((key, generation, query), r) in self.workers.drain() {
            let r = r.unwrap_or_else(Err);
            self.deliver(&key, generation, &query, r);
        }
        for id in self.timers.due(self.now) {
            match &id {
                TimerId::Commit(k) | TimerId::Search(k) | TimerId::ComboClose(k) => {
                    if let Some(i) = self.row_index(k) {
                        let focused = self.input_focused(i);
                        self.with_editor(i, |e, cx| e.timer(&id, focused, cx));
                    }
                }
                TimerId::NotesClear => self.notes.clear(),
                TimerId::Toast(t) => self.toasts.retain(|(x, _)| x != t),
            }
            self.process();
        }
        // a search started by a timer (inline workers) answers now
        for ((key, generation, query), r) in self.workers.drain() {
            let r = r.unwrap_or_else(Err);
            self.deliver(&key, generation, &query, r);
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        match self.layers.last_mut() {
            Some(Layer::SavePath(d, _)) => d.paste(text),
            Some(Layer::DatePicker(d, _)) => d.paste(text),
            Some(_) => {}
            None => {
                if let Some(Focus::Field(i, s)) = self.focus {
                    self.with_editor(i, |e, cx| e.paste(text, s, cx));
                }
            }
        }
        self.process();
    }

    pub fn handle_key(&mut self, k: KeyEvent) {
        if self.done.is_some() {
            return;
        }
        if self.layers.is_empty() {
            self.form_key(&k);
        } else {
            self.layer_key(&k);
        }
        self.process();
    }

    fn form_key(&mut self, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('s') {
            self.action_review();
            return;
        }
        match self.focus {
            Some(Focus::Field(i, s)) => {
                if self.with_editor(i, |e, cx| e.handle_key(k, s, cx)) == Some(true) {
                    return;
                }
            }
            Some(Focus::Button(b)) if matches!(k.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                self.press_button(b);
                return;
            }
            _ => {}
        }
        match k.code {
            KeyCode::Tab if k.modifiers.contains(KeyModifiers::SHIFT) => self.focus_step(-1),
            KeyCode::Tab => self.focus_step(1),
            KeyCode::BackTab => self.focus_step(-1),
            KeyCode::Esc => self.action_cancel(),
            KeyCode::F(1) => self.action_toggle_help(),
            KeyCode::F(4) => self.action_save(),
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll += 1,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::PageDown => self.scroll += 10,
            KeyCode::Left | KeyCode::Right if matches!(self.focus, Some(Focus::Button(_))) => {
                self.focus_step(if k.code == KeyCode::Left { -1 } else { 1 });
            }
            _ => {}
        }
    }

    fn layer_key(&mut self, k: &KeyEvent) {
        let Some(top) = self.layers.last_mut() else {
            return;
        };
        match top {
            Layer::Confirm(d) => {
                if let Dlg::Done(yes) = d.handle_key(k) {
                    self.pop();
                    if yes {
                        self.finish(Action::Cancel);
                    }
                }
            }
            Layer::Text(d) => match d.handle_key(k) {
                Dlg::Done(TextOut::Copy) => {
                    let text = d.text.clone();
                    self.copy(text);
                }
                Dlg::Done(TextOut::Close) => self.pop(),
                Dlg::Open => {}
            },
            Layer::DatePicker(d, key) => {
                if let Dlg::Done(value) = d.handle_key(k) {
                    let key = key.clone();
                    self.layers.pop();
                    if let Some(i) = self.row_index(&key) {
                        if let Some(v) = value {
                            self.with_editor(i, |e, cx| {
                                if let Some(d) = e.as_date() {
                                    d.set_value(v, cx);
                                }
                            });
                            self.process();
                        }
                        if self.layers.is_empty() {
                            self.focus = Some(Focus::Field(i, 0));
                            self.with_editor(i, |e, cx| e.focus(0, cx));
                            self.scroll_to_focus = true;
                            self.refresh_rows();
                        }
                    }
                }
            }
            Layer::SavePath(d, why) => {
                let why = *why;
                if let Dlg::Done(path) = d.handle_key(k, &self.session.paths) {
                    match path {
                        None => self.pop(),
                        Some(p) => self.save_to(&p, why),
                    }
                }
            }
            Layer::Review(r) => {
                if let Some(a) = r.handle_key(k) {
                    self.review_action(a);
                }
            }
        }
    }

    /// `TextDialog._copy`: the text goes to the clipboard, with a toast.
    fn copy(&mut self, text: String) {
        self.clipboard = Some(text);
        self.notify(
            "copied to the clipboard (terminal clipboard support permitting)",
            "",
            Severity::Information,
            Duration::from_secs(4),
        );
    }

    // -- actions -------------------------------------------------------------
    /// The Review & submit / Save draft / Cancel buttons under the fields.
    fn press_button(&mut self, b: FormButton) {
        match b {
            FormButton::Review => self.review_btn(),
            FormButton::Save => self.save_btn(),
            FormButton::Cancel => self.cancel_btn(),
        }
    }

    /// `FormScreen._review_btn` (Review & submit).
    fn review_btn(&mut self) {
        self.action_review();
    }

    /// `FormScreen._save_btn` (Save draft).
    fn save_btn(&mut self) {
        self.action_save();
    }

    /// `FormScreen._cancel_btn` (Cancel).
    fn cancel_btn(&mut self) {
        self.action_cancel();
    }

    /// Show or hide the help panel (F1).
    pub fn action_toggle_help(&mut self) {
        self.hide_help = !self.hide_help;
    }

    /// `FormScreen._commit_focused`: the focused editor commits its text.
    fn commit_focused(&mut self) {
        if let Some(Focus::Field(i, _)) = self.focus {
            self.with_editor(i, |e, cx| e.commit(cx));
        }
        self.process();
    }

    /// `FormScreen._dirty`: an editor holds text not committed yet (typed
    /// within the debounce, an unpicked lookup search, an invalid date).
    fn dirty(&self) -> bool {
        self.rows
            .iter()
            .any(|r| r.editor.as_ref().is_some_and(|e| e.dirty()))
    }

    pub fn action_review(&mut self) {
        self.commit_focused();
        self.open_review();
    }

    /// `FormScreen._open_review`: every error shows, then the review opens.
    fn open_review(&mut self) {
        self.session.show_all_errors = true;
        self.refresh_rows();
        self.process();
        match self.session.build_plan(None) {
            Ok(plan) => {
                let r = ReviewState::new(plan, &self.session);
                self.push(Layer::Review(Box::new(r)));
            }
            Err(e) => self.notify(&e, "cannot build the plan", Severity::Error, TOAST_SECONDS),
        }
    }

    pub fn action_save(&mut self) {
        self.commit_focused();
        let d = SavePathDialog::new(&self.session.default_draft_path(), "Save draft to");
        self.push(Layer::SavePath(d, SaveFor::Form));
    }

    pub fn action_cancel(&mut self) {
        let dirty = self.dirty();
        let touched = !self.session.touched.is_empty();
        self.commit_focused();
        if !touched && !dirty {
            self.finish(Action::Cancel);
            return;
        }
        self.push(Layer::Confirm(ConfirmDialog::new(
            "Discard this form? Nothing has been sent.",
            "Discard",
            "Keep editing",
        )));
    }

    /// `FormScreen._calendar`: the date picker for `key`, at its value.
    fn open_calendar(&mut self, key: &str) {
        let Some(it) = self.session.item(key) else {
            return;
        };
        let view = self.session.view(key);
        let picker = DatePicker::new(
            &it.label,
            &view.value,
            it.widget == WidgetKind::Date,
            &self.session.tz,
            &self.session.tz_source,
        );
        self.push(Layer::DatePicker(Box::new(picker), key.to_string()));
    }

    fn review_plan(&self) -> Option<Plan> {
        self.review().map(|r| r.plan.clone())
    }

    fn save_to(&mut self, path: &Path, why: SaveFor) {
        let text = path.to_string_lossy().into_owned();
        let res = match why {
            SaveFor::Form => self.session.save_draft(&text, None),
            SaveFor::Review | SaveFor::Command => {
                let plan = self.review_plan();
                self.session.save_draft(&text, plan.as_ref())
            }
        };
        let saved = match res {
            Ok(p) => p,
            Err(e) => {
                if let Some(d) = self.save_dialog() {
                    d.failed(path, &e);
                }
                return;
            }
        };
        self.layers.pop();
        match why {
            SaveFor::Form | SaveFor::Review => {
                self.saved_path = Some(saved);
                self.finish(Action::SaveDraft);
            }
            SaveFor::Command => {
                let Some(plan) = self.review_plan() else {
                    return;
                };
                let cmd = self
                    .session
                    .command_for(&self.session.paths.relpath(&saved), &plan);
                let note = format!(
                    "draft saved to {}; the command refuses to send if the plan changes",
                    saved.display()
                );
                self.layers.push(Layer::Text(TextDialog::new(
                    "Equivalent command",
                    &cmd,
                    &note,
                )));
            }
        }
    }

    /// `FormScreen._after_review`: finish, or back on the form at the first
    /// field to fix.
    fn after_review(&mut self, a: ReviewAction) {
        match a {
            ReviewAction::Submit => self.finish(Action::Submit),
            ReviewAction::Cancel => self.finish(Action::Cancel),
            _ => {
                self.pop();
                let s = &self.session;
                let mut missing = s.missing_keys();
                if missing.is_empty() {
                    missing = s
                        .errors_by_key
                        .keys()
                        .filter(|k| s.view(k).visible)
                        .cloned()
                        .collect();
                }
                if let Some(k) = missing.first().cloned() {
                    self.focus_key(&k);
                }
            }
        }
    }

    /// `ReviewScreen._ack`: the acknowledgement box rebuilds the plan.
    fn ack(&mut self) {
        self.session.ack = !self.session.ack;
        match self.session.build_plan(None) {
            Ok(plan) => {
                if let Some(Layer::Review(r)) = self.layers.last_mut() {
                    r.plan = plan;
                    r.refocus();
                }
            }
            Err(e) => self.notify(&e, "cannot build the plan", Severity::Error, TOAST_SECONDS),
        }
    }

    fn review_action(&mut self, a: ReviewAction) {
        match a {
            ReviewAction::Submit | ReviewAction::Cancel | ReviewAction::Back => {
                self.after_review(a)
            }
            ReviewAction::Save => {
                let d = SavePathDialog::new(&self.session.default_draft_path(), "Save draft to");
                self.layers.push(Layer::SavePath(d, SaveFor::Review));
            }
            ReviewAction::Command => {
                let d = SavePathDialog::new(
                    &self.session.default_draft_path(),
                    "Save the draft for the command to",
                );
                self.layers.push(Layer::SavePath(d, SaveFor::Command));
            }
            ReviewAction::Refused(msg) => {
                self.notify(msg, "", Severity::Warning, Duration::from_secs(4))
            }
            ReviewAction::ToggleAck => self.ack(),
        }
    }

    // -- drawing -------------------------------------------------------------
    fn hints(&mut self) -> Vec<(&'static str, &'static str)> {
        let mut keys = vec![];
        if let Some(Focus::Field(i, _)) = self.focus
            && let Some(e) = self.rows[i].editor.as_ref()
        {
            keys.extend(e.hints());
        }
        keys.extend([
            ("^s", "Review"),
            ("f4", "Save draft"),
            ("esc", "Cancel"),
            ("f1", "Help panel"),
        ]);
        keys
    }

    /// Draw everything; returns where the terminal cursor goes.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) -> Option<Position> {
        let theme = self.theme;
        let area = area.intersection(buf.area);
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            fill(
                area,
                buf,
                Style::default().bg(theme.background).fg(theme.foreground),
            );
            let msg = format!("terminal too small (needs {MIN_WIDTH}×{MIN_HEIGHT})");
            for (i, l) in wrap_line(&Line::from(msg), area.width)
                .iter()
                .take(usize::from(area.height))
                .enumerate()
            {
                buf.set_line(area.x, area.y + i as u16, l, area.width);
            }
            return None;
        }
        let mut cursor = if self
            .layers
            .first()
            .is_some_and(|l| matches!(l, Layer::Review(_)))
        {
            None
        } else {
            self.render_form(area, buf)
        };
        let n = self.layers.len();
        for i in 0..n {
            let FormScreen {
                layers, session, ..
            } = self;
            let c = clipped(area, buf, |buf| match &mut layers[i] {
                Layer::Review(r) => {
                    r.render(area, buf, session, &theme);
                    None
                }
                Layer::Confirm(d) => {
                    d.render(area, buf, &theme);
                    None
                }
                Layer::SavePath(d, _) => d.render(area, buf, &theme),
                Layer::Text(d) => {
                    d.render(area, buf, &theme);
                    None
                }
                Layer::DatePicker(d, _) => d.render(area, buf, &theme),
            });
            cursor = c.filter(|p| area.contains(*p));
        }
        let list: Vec<Toast> = self.toasts.iter().map(|(_, t)| t.clone()).collect();
        toasts(
            Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1)),
            buf,
            &list,
            &theme,
        );
        cursor
    }

    fn render_form(&mut self, area: Rect, buf: &mut Buffer) -> Option<Position> {
        let theme = self.theme;
        fill(
            area,
            buf,
            Style::default().bg(theme.background).fg(theme.foreground),
        );
        let mut y = area.y;
        // title
        let s = &self.session;
        let bar = Style::default()
            .bg(theme.primary)
            .fg(theme.text)
            .add_modifier(Modifier::BOLD);
        fill(Rect::new(area.x, y, area.width, 1), buf, bar);
        let sub = if s.mode == FormMode::Edit {
            " (edit)"
        } else {
            ""
        };
        let mut spans = vec![Span::styled(format!("{}{sub}", s.title), bar)];
        if s.dry_run {
            spans.push(Span::styled("  dry run", bar.add_modifier(Modifier::DIM)));
        }
        buf.set_line(
            area.x + 1,
            y,
            &Line::from(spans),
            area.width.saturating_sub(2),
        );
        y += 1;
        // notes
        let inner_w = area.width.saturating_sub(2);
        let notes: Vec<Line<'static>> = self
            .notes
            .iter()
            .flat_map(|n| {
                wrap_line(
                    &Line::styled(n.clone(), Style::default().fg(theme.accent)),
                    inner_w,
                )
            })
            .take(4)
            .collect();
        draw_lines(
            Rect::new(area.x + 1, y, inner_w, notes.len() as u16),
            buf,
            &notes,
            0,
        );
        y += notes.len() as u16;
        // body
        let bottom = area.bottom().saturating_sub(2);
        let body = Rect::new(area.x, y, area.width, bottom.saturating_sub(y));
        let narrow = area.width < NARROW;
        let (fields, help) = if self.hide_help {
            (body, None)
        } else if narrow {
            let hh = 9.min(body.height / 2);
            (
                Rect::new(body.x, body.y, body.width, body.height - hh),
                Some(Rect::new(body.x, body.bottom() - hh, body.width, hh)),
            )
        } else {
            let hw = (body.width / 3).clamp(30, 64).min(body.width / 2);
            (
                Rect::new(body.x, body.y, body.width - hw, body.height),
                Some(Rect::new(body.right() - hw, body.y, hw, body.height)),
            )
        };
        let cursor = self.render_fields(fields, buf);
        if let Some(h) = help {
            self.render_help(h, buf, narrow);
        }
        // status + footer
        let st = Rect::new(area.x, bottom, area.width, 1);
        fill(
            st,
            buf,
            Style::default().bg(theme.panel).fg(theme.text_muted),
        );
        buf.set_stringn(
            area.x + 1,
            bottom,
            &self.status,
            usize::from(inner_w),
            Style::default().bg(theme.panel).fg(theme.text_muted),
        );
        let hints = self.hints();
        footer(
            Rect::new(area.x, bottom + 1, area.width, 1),
            buf,
            &hints,
            &theme,
        );
        cursor
    }

    fn render_help(&self, area: Rect, buf: &mut Buffer, narrow: bool) {
        let theme = self.theme;
        if area.width < 4 || area.height < 2 {
            return;
        }
        let line = Style::default().fg(theme.primary);
        let inner = if narrow {
            buf.set_stringn(
                area.x,
                area.y,
                "─".repeat(usize::from(area.width)),
                usize::from(area.width),
                line,
            );
            Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(1),
            )
        } else {
            for yy in area.top()..area.bottom() {
                buf.set_string(area.x, yy, "│", line);
            }
            Rect::new(
                area.x + 2,
                area.y,
                area.width.saturating_sub(3),
                area.height,
            )
        };
        let lines = wrap_lines(
            &markup::lines(&self.help_markup, Style::default()),
            inner.width,
        );
        draw_lines(inner, buf, &lines, 0);
    }

    fn render_fields(&mut self, area: Rect, buf: &mut Buffer) -> Option<Position> {
        let theme = self.theme;
        let w = area.width;
        if w == 0 || area.height == 0 {
            return None;
        }
        let mut spans: Vec<(usize, u16, u16)> = vec![];
        let mut total: u16 = 0;
        for (i, row) in self.rows.iter().enumerate() {
            let h = row.height(w);
            if h > 0 {
                spans.push((i, total, h));
                total = total.saturating_add(h);
            }
        }
        let actions_y = total;
        total = total.saturating_add(4);
        // scroll the focused part into view
        let view_h = area.height;
        if self.scroll_to_focus {
            self.scroll_to_focus = false;
            let target = match self.focus {
                Some(Focus::Field(i, _)) => spans.iter().find(|s| s.0 == i).map(|s| (s.1, s.2)),
                Some(Focus::Button(_)) => Some((actions_y, 4)),
                None => None,
            };
            if let Some((start, h)) = target {
                let (start, h) = (usize::from(start), usize::from(h));
                let vh = usize::from(view_h);
                if start < self.scroll {
                    self.scroll = start;
                } else if start + h > self.scroll + vh {
                    self.scroll = start.min(start + h - vh);
                }
            }
        }
        let max_scroll = usize::from(total).saturating_sub(usize::from(view_h));
        self.scroll = self.scroll.min(max_scroll);
        let mut canvas = Buffer::empty(Rect::new(0, 0, w, total.max(1)));
        fill(
            canvas.area,
            &mut canvas,
            Style::default().bg(theme.background).fg(theme.foreground),
        );
        let mut cursor = None;
        for &(i, y, h) in &spans {
            let focus = match self.focus {
                Some(Focus::Field(j, s)) if j == i && self.layers.is_empty() => Some(s),
                _ => None,
            };
            let c = self.rows[i].render(Rect::new(0, y, w, h), &mut canvas, focus, &theme);
            if focus.is_some() {
                cursor = c;
            }
        }
        let mut x = 1;
        for (label, variant, b, disabled) in [
            (
                "Review & submit",
                Variant::Primary,
                FormButton::Review,
                self.review_disabled,
            ),
            ("Save draft", Variant::Default, FormButton::Save, false),
            ("Cancel", Variant::Error, FormButton::Cancel, false),
        ] {
            let focused = self.focus == Some(Focus::Button(b)) && self.layers.is_empty();
            x += Button {
                label,
                variant,
                focused,
                disabled,
            }
            .draw(x, actions_y, &mut canvas, &theme)
                + 1;
        }
        // copy the visible window
        let top = self.scroll as u16;
        for dy in 0..view_h.min(total.saturating_sub(top)) {
            for dx in 0..w {
                if let (Some(src), Some(dst)) = (
                    canvas.cell((dx, top + dy)).cloned(),
                    buf.cell_mut((area.x + dx, area.y + dy)),
                ) {
                    *dst = src;
                }
            }
        }
        cursor.and_then(|p: Position| {
            let py = p.y.checked_sub(top)?;
            (py < view_h).then(|| Position::new(area.x + p.x, area.y + py))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::parse_key;

    fn screen() -> FormScreen {
        let (_acme, session) = crate::acme::acme();
        FormScreen::with_workers(session, Theme::textual_dark(), WorkersMode::Inline)
    }

    fn typ(s: &mut FormScreen, text: &str) {
        for ch in text.chars() {
            s.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }
    }

    /// `FormScreen._focus_first` and `on_descendant_focus`: the first
    /// enabled visible field has focus, and the help panel follows focus.
    #[test]
    fn focus_first_and_the_help_follows_focus() {
        let mut s = screen();
        assert_eq!(s.focused_key(), Some("requester"));
        s.focus_key("subject");
        assert!(s.help_text().starts_with("Subject * required"));
        s.focus = None;
        s.focus_first();
        assert_eq!(s.focused_key(), Some("requester"));
        let i = s.row_index("planned_start_date").expect("row");
        s.on_descendant_focus(i);
        assert!(
            s.help_text().starts_with("Planned Start Date"),
            "{}",
            s.help_text()
        );
        assert!(
            s.help_text()
                .ends_with("Type a date and time or press F2 for the calendar")
        );
    }

    /// `FormScreen._opened`: an opened list is scrolled into view.
    #[test]
    fn opened_scrolls_to_the_focus() {
        let mut s = screen();
        s.scroll_to_focus = false;
        s.opened();
        assert!(s.scroll_to_focus);
    }

    /// `FormScreen._calendar`: the picker for the field (date-time, its
    /// label); Esc is back on the field's input with nothing changed.
    #[test]
    fn calendar_opens_the_picker_for_the_field() {
        let mut s = screen();
        s.open_calendar("planned_start_date");
        assert_eq!(s.layer(), Some("calendar"));
        let p = s.date_picker().expect("picker");
        assert_eq!(p.label, "Planned Start Date");
        assert!(!p.date_only);
        s.handle_key(parse_key("escape"));
        assert_eq!(s.layer(), None);
        assert_eq!(s.focused_key(), Some("planned_start_date"));
        assert_eq!(s.session.view("planned_start_date").value, Value::Null);
        s.open_calendar("no_such_field");
        assert_eq!(s.layer(), None);
    }

    /// `FormScreen._review_btn` / `_save_btn` / `_cancel_btn`.
    #[test]
    fn form_buttons() {
        let mut s = screen();
        s.review_btn();
        assert_eq!(s.layer(), Some("review"));
        s.handle_key(parse_key("escape"));
        s.save_btn();
        assert_eq!(s.layer(), Some("save"));
        assert_eq!(
            s.save_dialog().map(|d| d.title.clone()).as_deref(),
            Some("Save draft to")
        );
        s.handle_key(parse_key("escape"));
        assert_eq!(s.layer(), None);
        s.cancel_btn();
        assert_eq!(s.done(), Some(Action::Cancel));
    }

    /// `FormScreen._commit_focused` and `_dirty`: text typed within the
    /// debounce is dirty until the focused editor commits it.
    #[test]
    fn commit_focused_takes_pending_text() {
        let mut s = screen();
        assert!(!s.dirty());
        s.focus_key("subject");
        typ(&mut s, "Mail");
        assert!(s.dirty());
        assert_eq!(s.session.view("subject").value, Value::Null);
        s.commit_focused();
        assert!(!s.dirty());
        assert_eq!(s.session.view("subject").value, Value::str("Mail"));
    }

    /// `FormScreen._open_review`: every error shows, then the review opens.
    #[test]
    fn open_review_shows_all_errors() {
        let mut s = screen();
        assert!(!s.session.show_all_errors);
        s.open_review();
        assert!(s.session.show_all_errors);
        assert_eq!(s.layer(), Some("review"));
        assert_eq!(s.session.view("subject").error, "required");
    }

    /// `FormScreen._after_review`: Back focuses the first missing field;
    /// Submit and Cancel finish.
    #[test]
    fn after_review_back_submit_cancel() {
        let mut s = screen();
        s.focus_key("subject");
        s.open_review();
        s.after_review(ReviewAction::Back);
        assert_eq!(s.layer(), None);
        assert_eq!(s.focused_key(), Some("requester"));
        s.open_review();
        s.after_review(ReviewAction::Submit);
        assert_eq!(s.done(), Some(Action::Submit));
        let mut s = screen();
        s.open_review();
        s.after_review(ReviewAction::Cancel);
        assert_eq!(s.done(), Some(Action::Cancel));
        assert!(s.into_result().plan.is_none());
    }

    /// `TextDialog._copy`: the text for the clipboard and the toast.
    #[test]
    fn copy_sets_the_clipboard_and_says_so() {
        let mut s = screen();
        s.copy("fsv x".into());
        assert_eq!(s.take_clipboard().as_deref(), Some("fsv x"));
        assert!(s.toast_texts().contains(
            &"copied to the clipboard (terminal clipboard support permitting)".to_string()
        ));
    }
}
