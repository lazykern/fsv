//! Modal screens (`tui/modals.py`, `HelpScreen`). Each handles its own keys
//! and dismisses with a [`ModalResult`]; keys never reach the main handler
//! while a modal is open.

use crossterm::event::KeyEvent;
use fsv_core::term::escape;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use serde_json::Value;

use crate::input::{InputEvent, LineInput};
use crate::keys;
use crate::markup::{plain_text, to_line, to_text};

pub const ACCENT: Color = Color::Rgb(0xFE, 0xA6, 0x2B);
pub const SURFACE: Color = Color::Rgb(0x1E, 0x1E, 0x1E);
pub const PANEL: Color = Color::Rgb(0x24, 0x2F, 0x38);
pub const PRIMARY: Color = Color::Rgb(0x01, 0x78, 0xD4);
pub const WARNING: Color = Color::Rgb(0xFE, 0xA6, 0x2B);
pub const ERROR: Color = Color::Rgb(0xB9, 0x3C, 0x5B);
pub const SUCCESS: Color = Color::Rgb(0x4E, 0xBF, 0x71);

pub const HELP_TEXT: &str = "[bold cyan]fsv[/] help

[bold]Navigation[/]
  [cyan]Tab[/]          switch pane
  [cyan]j/k[/]          move list or scroll detail
  [cyan]J/K[/]          move or scroll ×5
  [cyan]G/gg[/]         bottom/top in active pane
  [cyan]h/l[/]          entity in list pane, tabs in detail pane
  [cyan]gt/gp/gc[/]     jump to tickets/problems/changes
  [cyan]1-9[/]          switch detail tab (the tabs the web shows for the item)
  [cyan]v[/]            pick a saved view by name (the list opens with the default view)
  [cyan]i[/]            browse or review device/CI and service associations
  [cyan]\\[][/cyan]         resize list/detail pane

[bold]Search[/]
  [cyan]f[/]            fulltext search across all entities
  [cyan]h/l[/]          narrow results: all → tickets → problems → changes
  [cyan]s[/]            cycle sort: relevance → date created → last modified
  [cyan]Esc[/]          exit search mode
  pagination auto-loads at list bottom (per-entity only)

[bold]Filter[/]
  [cyan]/[/]            open filter input (FIELD=VALUE, space-separated)
  [cyan]↑/↓[/] or [cyan]Ctrl+P/N[/]  browse filter history
  [cyan]Tab[/]          autocomplete field/value
  [cyan]Esc[/]          cancel filter input
  [cyan]Enter[/]        apply filter and reload
  empty input clears active filter

[bold]Selection[/]
  moving list cursor loads detail automatically
  [cyan]y[/]            copy selected display id
  [cyan]o[/]            open selected item in browser

[bold]Forms[/] (off with --read-only)
  [cyan]N[/]            choose Change / Incident / Service Request
  [cyan]e[/]            edit the selected change

[bold]Write actions[/] (changes/tickets; off with --read-only)
  [cyan]s[/]            change status: pick, then confirm
  [cyan]n[/]            add note (private by default; Ctrl+T toggles public)
  [cyan]r[/]            reply to ticket (detail pane; public, confirmed)
  [cyan]a[/]            add/update/delete task / initiating-ticket / attachment actions
  [cyan]![/]            reopen the last persistent failure

[bold]App[/]
  [cyan]r[/]            reload list and clear caches (list pane)
  [cyan]q[/]/[cyan]ctrl+c[/]     quit
  [cyan]?[/]            close/open this help

Press any key to close.";

#[derive(Clone, Debug, PartialEq)]
pub enum ModalResult {
    Cancelled,
    Bool(bool),
    /// Status `(id, label)`.
    Status(Value, String),
    /// Note `(body, public)`.
    Note(String, bool),
    /// Ctrl+O on a ticket note: `(body, public, {"edit_options": True})`.
    NoteOptions(String, bool),
    /// Picker / choice value.
    Choice(Value),
    Text(String),
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Stay,
    Close(ModalResult),
    /// Stay and show a warning toast.
    Warn(String),
}

/// `OptionList` cursor.
#[derive(Clone, Debug, Default)]
pub struct OptionCursor {
    pub highlighted: Option<usize>,
}

impl OptionCursor {
    fn step(&mut self, len: usize, key: &str) -> bool {
        if len == 0 {
            return false;
        }
        let cur = self.highlighted.unwrap_or(0) as isize;
        let next = match key {
            "down" => cur + 1,
            "up" => cur - 1,
            "pagedown" => cur + 8,
            "pageup" => cur - 8,
            "home" => 0,
            "end" => len as isize - 1,
            _ => return false,
        };
        self.highlighted = Some(next.clamp(0, len as isize - 1) as usize);
        true
    }
}

#[derive(Clone, Debug)]
pub struct StatusModal {
    pub choices: Vec<(Value, String)>,
    pub current: String,
    pub cursor: OptionCursor,
}

impl StatusModal {
    pub fn new(choices: Vec<(Value, String)>, current: &str) -> StatusModal {
        let hl = choices
            .iter()
            .position(|(_, l)| l == current)
            .or(if choices.is_empty() { None } else { Some(0) });
        StatusModal {
            choices,
            current: current.to_string(),
            cursor: OptionCursor { highlighted: hl },
        }
    }

    /// Enter on the highlighted option: `(id, label)`.
    fn selected(&self) -> Outcome {
        match self.cursor.highlighted.and_then(|i| self.choices.get(i)) {
            Some((id, label)) => Outcome::Close(ModalResult::Status(id.clone(), label.clone())),
            None => Outcome::Stay,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
pub enum ConfirmFocus {
    Confirm,
    Cancel,
}

#[derive(Clone, Debug)]
pub struct ConfirmModal {
    pub title: String,
    pub lines: Vec<String>,
    pub focus: ConfirmFocus,
    pub scroll: u16,
}

impl ConfirmModal {
    pub fn new(title: &str, lines: Vec<String>) -> ConfirmModal {
        ConfirmModal {
            title: title.to_string(),
            lines,
            focus: ConfirmFocus::Cancel,
            scroll: 0,
        }
    }

    /// The Confirm button.
    fn yes(&self) -> Outcome {
        Outcome::Close(ModalResult::Bool(true))
    }

    /// The Cancel button.
    fn no(&self) -> Outcome {
        Outcome::Close(ModalResult::Bool(false))
    }

    /// The `y` binding.
    fn action_confirm(&self) -> Outcome {
        self.yes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteFocus {
    Text,
    Continue,
    Cancel,
}

#[derive(Clone, Debug)]
pub struct NoteModal {
    pub target: String,
    pub reply: bool,
    pub can_toggle: bool,
    pub public: bool,
    pub text: LineInput,
    pub focus: NoteFocus,
    /// Ctrl+O asks for recipient/status options (`ticket_options=True`).
    pub ticket_options: bool,
    /// `PlanningTextScreen`: a planning editor, never a private/public note.
    pub planning: bool,
}

pub const PLANNING_VISIBILITY: &str = "Text/HTML saved verbatim; no attachment changes";

impl NoteModal {
    pub fn new(target: &str, reply: bool, can_toggle: bool) -> NoteModal {
        NoteModal {
            target: target.into(),
            reply,
            can_toggle: can_toggle && !reply,
            public: reply,
            text: LineInput::multiline(),
            focus: NoteFocus::Text,
            ticket_options: false,
            planning: false,
        }
    }

    pub fn with_ticket_options(mut self, on: bool) -> NoteModal {
        self.ticket_options = on;
        self
    }

    /// `PlanningTextScreen(target, initial)`; `title`, `hint` and
    /// `visibility` give its composed text.
    /// Port of fsv.tui.modals.PlanningTextScreen.__init__ and fsv.tui.modals.PlanningTextScreen.compose.
    pub fn planning(target: &str, initial: &str) -> NoteModal {
        let mut n = NoteModal::new(target, false, false);
        n.planning = true;
        n.text.set_value(initial);
        n
    }

    pub fn title(&self) -> String {
        if self.planning {
            format!("Edit planning text: {}", escape(&self.target))
        } else {
            let what = if self.reply { "reply" } else { "note" };
            format!("[bold]Add {what}[/]  [dim]{}[/]", escape(&self.target))
        }
    }

    pub fn hint(&self) -> String {
        if self.planning {
            return "Ctrl+S review · Esc cancel".into();
        }
        let mut hint = "Ctrl+S continue · Ctrl+T toggle · Esc cancel".to_string();
        if self.ticket_options {
            hint.push_str(" · Ctrl+O recipients/status options");
        }
        hint
    }

    pub fn visibility(&self) -> &'static str {
        if self.planning {
            PLANNING_VISIBILITY
        } else if self.reply {
            "[b]Visibility: PUBLIC reply[/] [dim](sent to the requester)[/]"
        } else if self.can_toggle {
            if self.public {
                "[b]Visibility: PUBLIC[/] [dim](Ctrl+T: private)[/]"
            } else {
                "[b]Visibility: Private[/] [dim](Ctrl+T: public)[/]"
            }
        } else {
            "[b]Visibility: Private[/] [dim](use r for a public reply)[/]"
        }
    }

    /// `(visibility markup, preview body)`; the body is `None` when empty.
    pub fn refresh(&self) -> (&'static str, Option<String>) {
        let body = self.text.value().trim();
        (
            self.visibility(),
            (!body.is_empty()).then(|| body.to_string()),
        )
    }

    /// An edit in the text area.
    fn changed(&mut self, ev: &KeyEvent) {
        self.text.handle(ev);
    }

    fn action_toggle_public(&mut self) {
        if self.can_toggle {
            self.public = !self.public;
        }
    }

    /// Ctrl+O: hand the body to the ticket options menu.
    fn action_options(&self) -> Outcome {
        if !self.ticket_options {
            return Outcome::Stay;
        }
        let body = self.text.value().trim();
        if body.is_empty() {
            Outcome::Warn("Note is empty".into())
        } else {
            Outcome::Close(ModalResult::NoteOptions(body.to_string(), self.public))
        }
    }

    /// Ctrl+S: `(body, public)`, or a warning when the body is blank.
    fn action_submit(&self) -> Outcome {
        let body = self.text.value().trim();
        if body.is_empty() {
            Outcome::Warn("Note is empty".into())
        } else {
            Outcome::Close(ModalResult::Note(body.to_string(), self.public))
        }
    }

    /// The Continue button.
    fn submit(&self) -> Outcome {
        self.action_submit()
    }

    /// The Cancel button.
    fn cancel(&self) -> Outcome {
        Outcome::Close(ModalResult::Cancelled)
    }
}

/// `ViewPickerScreen` and `ChoiceScreen`: type to filter, ↑/↓, Enter.
#[derive(Clone, Debug)]
pub struct PickerModal {
    pub title: String,
    pub input: LineInput,
    /// `(value, name, is_current, custom)`.
    pub options: Vec<(Value, String, bool, bool)>,
    pub shown: Vec<usize>,
    pub cursor: OptionCursor,
    pub show_kind: bool,
}

fn fold(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl PickerModal {
    /// `ViewPickerScreen(views, current, title)`; `options` are `(value, name, is_current, system)`.
    pub fn views(title: &str, options: Vec<(Value, String, bool, bool)>) -> PickerModal {
        let options = options
            .into_iter()
            .map(|(v, n, cur, system)| (v, n, cur, !system))
            .collect();
        let mut p = PickerModal {
            title: title.into(),
            input: LineInput::new("type to filter views"),
            options,
            shown: Vec::new(),
            cursor: OptionCursor::default(),
            show_kind: true,
        };
        p.fill();
        p
    }

    /// `ChoiceScreen.__init__`: `ChoiceScreen(choices, title)`.
    pub fn choices(title: &str, choices: Vec<(Value, String)>) -> PickerModal {
        let options = choices
            .into_iter()
            .map(|(v, n)| (v, n, false, false))
            .collect();
        let mut p = PickerModal {
            title: title.into(),
            input: LineInput::new("type to filter choices"),
            options,
            shown: Vec::new(),
            cursor: OptionCursor::default(),
            show_kind: false,
        };
        p.fill();
        p
    }

    fn fill(&mut self) {
        let needle = fold(self.input.value());
        self.shown = (0..self.options.len())
            .filter(|&i| self.options[i].1.to_lowercase().contains(&needle))
            .collect();
        self.cursor.highlighted = if self.shown.is_empty() {
            None
        } else {
            Some(
                self.shown
                    .iter()
                    .position(|&i| self.options[i].2)
                    .unwrap_or(0),
            )
        };
    }

    /// Typing in the filter input.
    fn filter(&mut self, ev: &KeyEvent) {
        if self.input.handle(ev) == InputEvent::Changed {
            self.fill();
        }
    }

    /// Enter in the filter input picks the highlighted row.
    fn submit(&self) -> Outcome {
        match self.cursor.highlighted {
            Some(pos) if pos < self.shown.len() => self.selected(pos),
            _ => Outcome::Stay,
        }
    }

    /// ↑/↓ move the highlight, clamped; other keys belong to the input.
    fn on_key(&mut self, key: &str) -> bool {
        matches!(key, "up" | "down") && self.cursor.step(self.shown.len(), key)
    }

    /// The row at `pos` among the shown options.
    fn selected(&self, pos: usize) -> Outcome {
        self.dismiss(self.shown[pos])
    }

    /// `ChoiceScreen.dismiss`: the option's value, never its label.
    fn dismiss(&self, i: usize) -> Outcome {
        Outcome::Close(ModalResult::Choice(self.options[i].0.clone()))
    }

    pub fn row_markup(&self, i: usize) -> String {
        let (_, name, current, custom) = &self.options[i];
        let mark = if *current { "▸ " } else { "  " };
        let kind = if self.show_kind && *custom {
            "  [dim](custom)[/]"
        } else {
            ""
        };
        format!("{mark}{}{kind}", escape(name))
    }

    pub fn labels(&self) -> Vec<String> {
        self.shown
            .iter()
            .map(|&i| crate::markup::plain(&self.row_markup(i)))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct ValueModal {
    pub title: String,
    pub input: LineInput,
    /// Enter on a blank value dismisses with `""` instead of staying.
    pub allow_empty: bool,
}

impl ValueModal {
    /// Enter: the trimmed value; a blank one stays unless `allow_empty`.
    fn submitted(&self) -> Outcome {
        let t = self.input.value().trim();
        if t.is_empty() && !self.allow_empty {
            Outcome::Stay
        } else {
            Outcome::Close(ModalResult::Text(t.to_string()))
        }
    }
}

/// `FailureScreen` (title fixed) and `OutcomeScreen` (title given): a
/// scrollable message with only a Close control.
#[derive(Clone, Debug)]
pub struct FailureModal {
    pub title: String,
    pub message: String,
    pub scroll: u16,
}

impl FailureModal {
    /// Escape or the Close button.
    fn action_close(&self) -> Outcome {
        Outcome::Close(ModalResult::Cancelled)
    }
}

/// `HelpScreen.on_key`: any key closes the help.
fn dismiss_help() -> Outcome {
    Outcome::Close(ModalResult::Cancelled)
}

pub const FAILURE_TITLE: &str = "Write failed — inspect before retrying";

#[derive(Clone, Debug)]
pub enum Modal {
    Help,
    Status(StatusModal),
    Confirm(ConfirmModal),
    Note(NoteModal),
    Picker(PickerModal),
    Value(ValueModal),
    Failure(FailureModal),
    Outcome(FailureModal),
}

impl Modal {
    pub fn value(title: &str, value: &str) -> Modal {
        Modal::value_with(title, value, false)
    }

    pub fn value_with(title: &str, value: &str, allow_empty: bool) -> Modal {
        let mut input = LineInput::new("");
        input.set_value(value);
        Modal::Value(ValueModal {
            title: title.into(),
            input,
            allow_empty,
        })
    }

    pub fn failure(message: &str) -> Modal {
        Modal::Failure(FailureModal {
            title: FAILURE_TITLE.into(),
            message: message.into(),
            scroll: 0,
        })
    }

    pub fn outcome(title: &str, message: &str) -> Modal {
        Modal::Outcome(FailureModal {
            title: title.into(),
            message: message.into(),
            scroll: 0,
        })
    }

    pub fn confirm(title: &str, lines: Vec<String>) -> Modal {
        Modal::Confirm(ConfirmModal::new(title, lines))
    }

    pub fn choices(title: &str, choices: Vec<(Value, String)>) -> Modal {
        Modal::Picker(PickerModal::choices(title, choices))
    }

    pub fn handle(&mut self, ev: &KeyEvent) -> Outcome {
        let key = keys::name(ev);
        match self {
            Modal::Help => dismiss_help(),
            Modal::Status(s) => match key.as_str() {
                "escape" => Outcome::Close(ModalResult::Cancelled),
                "enter" => s.selected(),
                k => {
                    s.cursor.step(s.choices.len(), k);
                    Outcome::Stay
                }
            },
            Modal::Confirm(c) => match key.as_str() {
                "escape" => c.no(),
                "y" => c.action_confirm(),
                "enter" | "space" if c.focus == ConfirmFocus::Confirm => c.yes(),
                "enter" | "space" => c.no(),
                "tab" | "shift+tab" | "left" | "right" => {
                    c.focus = if c.focus == ConfirmFocus::Confirm {
                        ConfirmFocus::Cancel
                    } else {
                        ConfirmFocus::Confirm
                    };
                    Outcome::Stay
                }
                "down" | "j" => {
                    c.scroll = c.scroll.saturating_add(1);
                    Outcome::Stay
                }
                "up" | "k" => {
                    c.scroll = c.scroll.saturating_sub(1);
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
            Modal::Note(n) => match key.as_str() {
                "escape" => n.cancel(),
                "ctrl+t" => {
                    n.action_toggle_public();
                    Outcome::Stay
                }
                "ctrl+s" => n.action_submit(),
                "ctrl+o" => n.action_options(),
                "tab" => {
                    n.focus = match n.focus {
                        NoteFocus::Text => NoteFocus::Continue,
                        NoteFocus::Continue => NoteFocus::Cancel,
                        NoteFocus::Cancel => NoteFocus::Text,
                    };
                    Outcome::Stay
                }
                "shift+tab" => {
                    n.focus = match n.focus {
                        NoteFocus::Text => NoteFocus::Cancel,
                        NoteFocus::Continue => NoteFocus::Text,
                        NoteFocus::Cancel => NoteFocus::Continue,
                    };
                    Outcome::Stay
                }
                "enter" | "space" if n.focus == NoteFocus::Continue => n.submit(),
                "enter" | "space" if n.focus == NoteFocus::Cancel => n.cancel(),
                _ => {
                    if n.focus == NoteFocus::Text {
                        n.changed(ev);
                    }
                    Outcome::Stay
                }
            },
            Modal::Picker(p) => match key.as_str() {
                "escape" => Outcome::Close(ModalResult::Cancelled),
                "enter" => p.submit(),
                k => {
                    if !p.on_key(k) {
                        p.filter(ev);
                    }
                    Outcome::Stay
                }
            },
            Modal::Value(v) => match v.input.handle(ev) {
                InputEvent::Submitted => v.submitted(),
                InputEvent::Ignored if key == "escape" => Outcome::Close(ModalResult::Cancelled),
                _ => Outcome::Stay,
            },
            Modal::Failure(fm) | Modal::Outcome(fm) => match key.as_str() {
                "escape" | "enter" | "space" => fm.action_close(),
                "down" | "j" => {
                    fm.scroll = fm.scroll.saturating_add(1);
                    Outcome::Stay
                }
                "up" | "k" => {
                    fm.scroll = fm.scroll.saturating_sub(1);
                    Outcome::Stay
                }
                _ => Outcome::Stay,
            },
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        match self {
            Modal::Help => {
                let text = to_text(HELP_TEXT);
                let inner_w = dialog_rect(area, 76, 1).width.saturating_sub(6).max(1) as usize;
                let rows: usize = text
                    .lines
                    .iter()
                    .map(|l| l.width().div_ceil(inner_w).max(1))
                    .sum();
                let rect = dialog_rect(area, 76, rows as u16 + 4);
                frame(f, rect);
                f.render_widget(
                    Paragraph::new(text)
                        .wrap(Wrap { trim: false })
                        .block(dialog_block()),
                    rect,
                );
            }
            Modal::Status(s) => {
                let title = format!(
                    "[bold]Change status[/]  [dim]current: {}[/]",
                    escape(&s.current)
                );
                let opts: Vec<Line> = s
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(i, (_, l))| {
                        let st = if s.cursor.highlighted == Some(i) {
                            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        Line::from(Span::styled(l.clone(), st))
                    })
                    .collect();
                let list_h = (opts.len() as u16).clamp(1, 16);
                let rect = dialog_rect(area, 64, list_h + 8);
                frame(f, rect);
                let inner = dialog_block().inner(rect);
                f.render_widget(dialog_block(), rect);
                let [t, _, l, _, h] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(list_h),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(inner);
                f.render_widget(Paragraph::new(to_line(&title)), t);
                let skip = s
                    .cursor
                    .highlighted
                    .unwrap_or(0)
                    .saturating_sub(list_h as usize - 1);
                f.render_widget(
                    Paragraph::new(Text::from(opts.into_iter().skip(skip).collect::<Vec<_>>()))
                        .style(Style::default().bg(PANEL)),
                    l,
                );
                f.render_widget(
                    Paragraph::new(to_line("[dim]Enter select · Esc cancel[/]")),
                    h,
                );
            }
            Modal::Confirm(c) => {
                let body = plain_text(&c.lines.join("\n"));
                // The body's VerticalScroll is 1fr: the dialog always grows to max-height.
                let rect = dialog_rect(area, 64, area.height);
                frame(f, rect);
                let inner = dialog_block().inner(rect);
                f.render_widget(dialog_block(), rect);
                let [t, _, b, _, btn, _, h] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(inner);
                f.render_widget(
                    Paragraph::new(to_line(&format!("[bold]{}[/]", escape(&c.title)))),
                    t,
                );
                f.render_widget(
                    Paragraph::new(body)
                        .wrap(Wrap { trim: false })
                        .scroll((c.scroll, 0)),
                    b,
                );
                buttons(
                    f,
                    btn,
                    &[
                        ("Confirm", WARNING, c.focus == ConfirmFocus::Confirm),
                        ("Cancel", PRIMARY, c.focus == ConfirmFocus::Cancel),
                    ],
                );
                f.render_widget(Paragraph::new(to_line("[dim]y confirm · Esc cancel[/]")), h);
            }
            Modal::Note(n) => {
                let rect = dialog_rect(area, 64, 24);
                frame(f, rect);
                let inner = dialog_block().inner(rect);
                f.render_widget(dialog_block(), rect);
                let [t, _, ta, vis, prev, btn, h] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(8),
                    Constraint::Length(2),
                    Constraint::Min(2),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .areas(inner);
                f.render_widget(Paragraph::new(to_line(&n.title())), t);
                let border = if n.focus == NoteFocus::Text {
                    PRIMARY
                } else {
                    Color::DarkGray
                };
                let block = Block::default()
                    .borders(Borders::TOP | Borders::BOTTOM)
                    .border_type(BorderType::Thick)
                    .border_style(Style::default().fg(border));
                let text_area = block.inner(ta);
                f.render_widget(
                    Paragraph::new(plain_text(n.text.value()))
                        .wrap(Wrap { trim: false })
                        .block(block),
                    ta,
                );
                if n.focus == NoteFocus::Text {
                    let before: String = n.text.value().chars().take(n.text.cursor()).collect();
                    let row = before.matches('\n').count() as u16;
                    let col = unicode_width::UnicodeWidthStr::width(
                        before.rsplit('\n').next().unwrap_or(""),
                    ) as u16;
                    f.set_cursor_position((
                        text_area.x + col.min(text_area.width.saturating_sub(1)),
                        text_area.y + row.min(text_area.height.saturating_sub(1)),
                    ));
                }
                let (visibility, body) = n.refresh();
                f.render_widget(
                    Paragraph::new(vec![Line::default(), to_line(visibility)]),
                    vis,
                );
                if !n.planning {
                    let mut preview = vec![Span::styled(
                        "Preview: ",
                        Style::default().add_modifier(Modifier::BOLD),
                    )];
                    preview.push(match body {
                        None => {
                            Span::styled("(empty)", Style::default().add_modifier(Modifier::DIM))
                        }
                        Some(b) => Span::raw(b),
                    });
                    f.render_widget(
                        Paragraph::new(vec![Line::default(), Line::from(preview)])
                            .wrap(Wrap { trim: false }),
                        prev,
                    );
                }
                let submit = if n.planning { "Review" } else { "Continue" };
                buttons(
                    f,
                    btn,
                    &[
                        (submit, PRIMARY, n.focus == NoteFocus::Continue),
                        ("Cancel", PRIMARY, n.focus == NoteFocus::Cancel),
                    ],
                );
                f.render_widget(Paragraph::new(to_line(&format!("[dim]{}[/]", n.hint()))), h);
            }
            Modal::Picker(p) => {
                let list_h = (p.shown.len() as u16).clamp(1, 18);
                let rect = dialog_rect(area, 64, list_h + 10);
                frame(f, rect);
                let inner = dialog_block().inner(rect);
                f.render_widget(dialog_block(), rect);
                let [t, _, inp, _, l, _, h] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(list_h),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(inner);
                f.render_widget(
                    Paragraph::new(to_line(&format!("[bold]{}[/]", escape(&p.title)))),
                    t,
                );
                input_line(f, inp, &p.input, true);
                let skip = p
                    .cursor
                    .highlighted
                    .unwrap_or(0)
                    .saturating_sub(list_h as usize - 1);
                let rows: Vec<Line> = p
                    .shown
                    .iter()
                    .enumerate()
                    .skip(skip)
                    .map(|(pos, &i)| {
                        let line = to_line(&p.row_markup(i));
                        if p.cursor.highlighted == Some(pos) {
                            line.patch_style(Style::default().add_modifier(Modifier::REVERSED))
                        } else {
                            line
                        }
                    })
                    .collect();
                f.render_widget(Paragraph::new(rows).style(Style::default().bg(PANEL)), l);
                f.render_widget(
                    Paragraph::new(to_line("[dim]Enter open · ↑/↓ move · Esc cancel[/]")),
                    h,
                );
            }
            Modal::Value(v) => {
                let rect = dialog_rect(area, 64, 9);
                frame(f, rect);
                let inner = dialog_block().inner(rect);
                f.render_widget(dialog_block(), rect);
                let [t, _, inp, _, h] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .areas(inner);
                f.render_widget(Paragraph::new(Line::from(v.title.clone())), t);
                input_line(f, inp, &v.input, true);
                f.render_widget(Paragraph::new(Line::from("Enter review · Esc cancel")), h);
            }
            Modal::Failure(fm) | Modal::Outcome(fm) => {
                let body = plain_text(&fm.message);
                let body_h =
                    (body.lines.len() as u16).clamp(1, area.height.saturating_sub(12).max(3));
                let rect = dialog_rect(area, 64, body_h + 9);
                frame(f, rect);
                let inner = dialog_block().inner(rect);
                f.render_widget(dialog_block(), rect);
                let [t, _, b, btn] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .areas(inner);
                f.render_widget(Paragraph::new(Line::from(fm.title.clone())), t);
                f.render_widget(
                    Paragraph::new(body)
                        .wrap(Wrap { trim: false })
                        .scroll((fm.scroll, 0)),
                    b,
                );
                buttons(f, btn, &[("Close", PRIMARY, false)]);
            }
        }
    }
}

fn dialog_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(ACCENT))
        .padding(ratatui::widgets::Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(SURFACE))
}

/// `width` columns (≤ 90%), `height` rows (≤ 90%), centered.
fn dialog_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width * 9 / 10).max(10.min(area.width));
    let h = height.min(area.height * 9 / 10).max(3.min(area.height));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn frame(f: &mut Frame, rect: Rect) {
    f.render_widget(Clear, rect);
}

fn buttons(f: &mut Frame, area: Rect, items: &[(&str, Color, bool)]) {
    let mut x = area.x;
    for (label, color, focused) in items {
        let w = (label.chars().count() as u16 + 4)
            .max(14)
            .min(area.width.saturating_sub(x - area.x));
        if w == 0 {
            break;
        }
        let rect = Rect {
            x,
            y: area.y,
            width: w,
            height: area.height,
        };
        let style = if *focused {
            Style::default()
                .bg(*color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(PANEL).add_modifier(Modifier::BOLD)
        };
        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(*color));
        f.render_widget(
            Paragraph::new(Line::from(label.to_string()).centered())
                .style(style)
                .block(block),
            rect,
        );
        x += w + 1;
    }
}

pub fn input_line(f: &mut Frame, area: Rect, input: &LineInput, focused: bool) {
    let (shown, col) = input.window(area.width as usize);
    if input.value().is_empty() && !input.placeholder.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                input.placeholder.clone(),
                Style::default().add_modifier(Modifier::DIM),
            )),
            area,
        );
    } else {
        f.render_widget(Paragraph::new(Line::from(shown.clone())), area);
    }
    if focused {
        let prefix: String = shown.chars().take(col).collect();
        let x = unicode_width::UnicodeWidthStr::width(prefix.as_str()) as u16;
        f.set_cursor_position((area.x + x.min(area.width.saturating_sub(1)), area.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn press(m: &mut Modal, k: &str) -> Outcome {
        m.handle(&keys::parse(k))
    }

    #[test]
    fn picker_filters_by_substring() {
        let mut m = Modal::Picker(PickerModal::views(
            "Tickets views",
            vec![
                (json!(0), "All tickets".into(), false, true),
                (json!(1), "My team queue".into(), true, false),
                (json!(2), "Open".into(), false, true),
            ],
        ));
        if let Modal::Picker(p) = &m {
            assert_eq!(
                p.labels(),
                vec!["  All tickets", "▸ My team queue  (custom)", "  Open"]
            );
            assert_eq!(p.cursor.highlighted, Some(1));
        }
        for c in ["t", "e", "a", "m"] {
            press(&mut m, c);
        }
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Choice(json!(1)))
        );
    }

    #[test]
    fn confirm_defaults_to_cancel() {
        let mut m = Modal::confirm("Confirm", vec!["a".into()]);
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Bool(false))
        );
        assert_eq!(press(&mut m, "y"), Outcome::Close(ModalResult::Bool(true)));
    }

    #[test]
    fn note_toggle_and_empty() {
        let mut m = Modal::Note(NoteModal::new("INC-1", false, true));
        assert_eq!(
            press(&mut m, "ctrl+s"),
            Outcome::Warn("Note is empty".into())
        );
        for c in ["q", "o", "y"] {
            press(&mut m, c);
        }
        press(&mut m, "ctrl+t");
        assert_eq!(
            press(&mut m, "ctrl+s"),
            Outcome::Close(ModalResult::Note("qoy".into(), true))
        );
        let mut r = Modal::Note(NoteModal::new("INC-1", true, true));
        press(&mut r, "ctrl+t");
        if let Modal::Note(n) = &r {
            assert!(n.public && !n.can_toggle);
        }
    }

    #[test]
    fn status_screen_highlights_current_and_selects() {
        let choices = vec![
            (json!(1), "Open".to_string()),
            (json!(2), "Pending".to_string()),
            (json!(3), "Resolved".to_string()),
        ];
        let mut m = Modal::Status(StatusModal::new(choices.clone(), "Pending"));
        if let Modal::Status(s) = &m {
            assert_eq!(s.cursor.highlighted, Some(1));
        }
        press(&mut m, "down");
        press(&mut m, "down");
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Status(json!(3), "Resolved".into()))
        );
        let mut unknown = Modal::Status(StatusModal::new(choices, "Gone"));
        if let Modal::Status(s) = &unknown {
            assert_eq!(s.cursor.highlighted, Some(0));
        }
        assert_eq!(
            press(&mut unknown, "escape"),
            Outcome::Close(ModalResult::Cancelled)
        );
        let mut empty = Modal::Status(StatusModal::new(Vec::new(), "Open"));
        assert_eq!(press(&mut empty, "enter"), Outcome::Stay);
    }

    #[test]
    fn confirm_escape_cancels_and_buttons_toggle() {
        let mut m = Modal::confirm("Confirm", vec!["a".into()]);
        assert_eq!(
            press(&mut m, "escape"),
            Outcome::Close(ModalResult::Bool(false))
        );
        let mut m = Modal::confirm("Confirm", vec!["a".into()]);
        press(&mut m, "tab");
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Bool(true))
        );
        let mut m = Modal::confirm("Confirm", vec!["a".into()]);
        assert_eq!(press(&mut m, "n"), Outcome::Stay);
    }

    #[test]
    fn note_cancel_buttons_and_visibility_text() {
        let mut m = Modal::Note(NoteModal::new("CHN-1", false, true));
        assert_eq!(
            press(&mut m, "escape"),
            Outcome::Close(ModalResult::Cancelled)
        );
        let fixed = NoteModal::new("CHN-1", false, false);
        assert!(fixed.visibility().contains("use r for a public reply"));
        assert!(
            NoteModal::new("INC-1", true, true)
                .visibility()
                .contains("PUBLIC reply")
        );
        let mut m = Modal::Note(NoteModal::new("CHN-1", false, true));
        press(&mut m, "x");
        press(&mut m, "tab");
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Note("x".into(), false))
        );
        let mut m = Modal::Note(NoteModal::new("CHN-1", false, true));
        press(&mut m, "shift+tab");
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Cancelled)
        );
        let mut m = Modal::Note(NoteModal::new("CHN-1", false, true));
        for k in ["a", "enter", "b"] {
            press(&mut m, k);
        }
        assert_eq!(
            press(&mut m, "ctrl+s"),
            Outcome::Close(ModalResult::Note("a\nb".into(), false))
        );
    }

    #[test]
    fn view_picker_arrows_clamp_and_empty_filter_stays() {
        let mut m = Modal::Picker(PickerModal::views(
            "Views",
            vec![
                (json!(0), "All".into(), false, true),
                (json!(1), "Mine".into(), false, true),
            ],
        ));
        press(&mut m, "up");
        press(&mut m, "down");
        press(&mut m, "down");
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Choice(json!(1)))
        );
        for c in ["z", "z"] {
            press(&mut m, c);
        }
        assert_eq!(press(&mut m, "enter"), Outcome::Stay);
        assert_eq!(
            press(&mut m, "escape"),
            Outcome::Close(ModalResult::Cancelled)
        );
    }

    #[test]
    fn choice_screen_returns_value_not_label() {
        let mut m = Modal::choices(
            "Pick",
            vec![
                (json!({"id": 9}), "Row nine".into()),
                (json!("skip"), "Skip".into()),
            ],
        );
        if let Modal::Picker(p) = &m {
            assert_eq!(p.labels(), vec!["  Row nine", "  Skip"]);
        }
        for c in ["s", "k"] {
            press(&mut m, c);
        }
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Choice(json!("skip")))
        );
    }

    #[test]
    fn value_screen_trims_and_blocks_empty() {
        let mut m = Modal::value("Title", "  x  ");
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Text("x".into()))
        );
        let mut m = Modal::value("Title", "   ");
        assert_eq!(press(&mut m, "enter"), Outcome::Stay);
        assert_eq!(
            press(&mut m, "escape"),
            Outcome::Close(ModalResult::Cancelled)
        );
        let mut m = Modal::value_with("Remark", "", true);
        assert_eq!(
            press(&mut m, "enter"),
            Outcome::Close(ModalResult::Text(String::new()))
        );
    }

    #[test]
    fn failure_and_outcome_screens_close_and_scroll() {
        let mut m = Modal::failure("boom\nline 2");
        press(&mut m, "j");
        if let Modal::Failure(f) = &m {
            assert_eq!((f.title.as_str(), f.scroll), (FAILURE_TITLE, 1));
        }
        assert_eq!(press(&mut m, "x"), Outcome::Stay);
        assert_eq!(
            press(&mut m, "escape"),
            Outcome::Close(ModalResult::Cancelled)
        );
        let mut o = Modal::outcome("Approval result — do not resend", "{}");
        if let Modal::Outcome(f) = &o {
            assert_eq!(f.title, "Approval result — do not resend");
        }
        assert_eq!(
            press(&mut o, "enter"),
            Outcome::Close(ModalResult::Cancelled)
        );
    }

    #[test]
    fn help_screen_any_key_closes() {
        for k in ["x", "escape", "question_mark", "q"] {
            assert_eq!(
                press(&mut Modal::Help, k),
                Outcome::Close(ModalResult::Cancelled)
            );
        }
        assert_eq!(dismiss_help(), Outcome::Close(ModalResult::Cancelled));
    }

    // One test per `modals.py` handler, calling the Rust method it maps to.

    #[test]
    fn status_selected_dismisses_with_id_and_label() {
        let s = StatusModal::new(
            vec![(json!(1), "Open".into()), (json!(2), "Pending".into())],
            "Pending",
        );
        assert_eq!(
            s.selected(),
            Outcome::Close(ModalResult::Status(json!(2), "Pending".into()))
        );
        assert_eq!(StatusModal::new(Vec::new(), "x").selected(), Outcome::Stay);
    }

    #[test]
    fn confirm_yes_no_and_action_confirm() {
        let c = ConfirmModal::new("T", vec![]);
        assert_eq!(c.yes(), Outcome::Close(ModalResult::Bool(true)));
        assert_eq!(c.no(), Outcome::Close(ModalResult::Bool(false)));
        assert_eq!(c.action_confirm(), Outcome::Close(ModalResult::Bool(true)));
    }

    #[test]
    fn note_refresh_changed_toggle_submit_cancel() {
        let mut n = NoteModal::new("CHN-1", false, true);
        assert_eq!(
            n.refresh(),
            ("[b]Visibility: Private[/] [dim](Ctrl+T: public)[/]", None)
        );
        n.changed(&keys::parse("h"));
        n.changed(&keys::parse("i"));
        n.action_toggle_public();
        assert_eq!(
            n.refresh(),
            (
                "[b]Visibility: PUBLIC[/] [dim](Ctrl+T: private)[/]",
                Some("hi".into())
            )
        );
        assert_eq!(
            n.action_submit(),
            Outcome::Close(ModalResult::Note("hi".into(), true))
        );
        assert_eq!(n.submit(), n.action_submit());
        assert_eq!(n.cancel(), Outcome::Close(ModalResult::Cancelled));
        let mut fixed = NoteModal::new("CHN-1", false, false);
        fixed.action_toggle_public();
        assert!(!fixed.public);
        assert_eq!(fixed.action_submit(), Outcome::Warn("Note is empty".into()));
    }

    /// `NoteScreen.action_options` and the `ticket_options` hint.
    #[test]
    fn note_action_options_only_for_ticket_notes_with_text() {
        let mut n = NoteModal::new("INC-1", false, true);
        n.changed(&keys::parse("x"));
        assert_eq!(n.action_options(), Outcome::Stay);
        assert_eq!(n.hint(), "Ctrl+S continue · Ctrl+T toggle · Esc cancel");
        let mut t = NoteModal::new("INC-1", false, true).with_ticket_options(true);
        assert_eq!(
            t.hint(),
            "Ctrl+S continue · Ctrl+T toggle · Esc cancel · Ctrl+O recipients/status options"
        );
        assert_eq!(t.action_options(), Outcome::Warn("Note is empty".into()));
        t.changed(&keys::parse("x"));
        t.action_toggle_public();
        let mut m = Modal::Note(t);
        assert_eq!(
            m.handle(&keys::parse("ctrl+o")),
            Outcome::Close(ModalResult::NoteOptions("x".into(), true))
        );
    }

    /// `PlanningTextScreen.__init__` / `compose`: initial text, fixed
    /// visibility line, never public, and Ctrl+S returns the stripped text.
    #[test]
    fn planning_text_screen_init_and_compose() {
        let mut p = NoteModal::planning("acme_plan", "  old\ntext ");
        assert_eq!(p.title(), "Edit planning text: acme_plan");
        assert_eq!(p.hint(), "Ctrl+S review · Esc cancel");
        assert_eq!(p.visibility(), PLANNING_VISIBILITY);
        assert_eq!(p.text.value(), "  old\ntext ");
        assert!(!p.reply && !p.public && !p.can_toggle && !p.ticket_options);
        p.action_toggle_public();
        assert!(!p.public);
        assert_eq!(p.action_options(), Outcome::Stay);
        assert_eq!(
            p.action_submit(),
            Outcome::Close(ModalResult::Note("old\ntext".into(), false))
        );
        assert_eq!(
            NoteModal::planning("[x]", "").title(),
            "Edit planning text: \\[x]"
        );
    }

    #[test]
    fn picker_fill_filter_on_key_submit_selected_dismiss() {
        let mut p = PickerModal::views(
            "Views",
            vec![
                (json!(0), "All  tickets".into(), false, true),
                (json!(1), "My open".into(), true, false),
                (json!(2), "Open urgent".into(), false, true),
            ],
        );
        assert_eq!(p.cursor.highlighted, Some(1));
        assert_eq!(
            p.labels(),
            vec!["  All  tickets", "▸ My open  (custom)", "  Open urgent"]
        );
        p.input.set_value("  OPEN ");
        p.fill();
        assert_eq!(p.shown, vec![1, 2]);
        p.filter(&keys::parse("x"));
        assert!(p.shown.is_empty());
        assert_eq!(p.submit(), Outcome::Stay);
        assert!(!p.on_key("down"));
        p.filter(&keys::parse("backspace"));
        assert_eq!(p.shown, vec![1, 2]);
        assert!(p.on_key("down"));
        assert!(p.on_key("down"));
        assert_eq!(p.cursor.highlighted, Some(1));
        assert!(!p.on_key("pagedown"));
        assert_eq!(p.submit(), Outcome::Close(ModalResult::Choice(json!(2))));
        assert_eq!(p.selected(0), Outcome::Close(ModalResult::Choice(json!(1))));
        assert_eq!(p.dismiss(0), Outcome::Close(ModalResult::Choice(json!(0))));
    }

    #[test]
    fn value_submitted_and_failure_action_close() {
        if let Modal::Value(v) = Modal::value("T", " a ") {
            assert_eq!(v.submitted(), Outcome::Close(ModalResult::Text("a".into())));
        }
        if let Modal::Value(v) = Modal::value_with("T", " ", false) {
            assert_eq!(v.submitted(), Outcome::Stay);
        }
        if let Modal::Failure(f) = Modal::failure("x") {
            assert_eq!(f.action_close(), Outcome::Close(ModalResult::Cancelled));
        }
    }
}
