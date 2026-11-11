//! Text, number, paragraph, description, checkbox and file editors.

use crossterm::event::{KeyCode, KeyEvent};
use fsv_forms::Value;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap};

use super::{DEBOUNCE_TEXT, EdCx, EdMsg, Editor, TimerId};
use crate::files::{complete_path, human_size};
use crate::kit::{InputEvent, LineInput, TextArea, Theme};
use crate::session::{FormItem, ItemView};

/// One-line text (or number).
pub struct TextEditor {
    key: String,
    pub input: LineInput,
    committed: String,
}

impl TextEditor {
    pub fn new(item: &FormItem, number: bool) -> Self {
        let ph = if !item.placeholder.is_empty() {
            item.placeholder.clone()
        } else if number {
            "a number".into()
        } else {
            String::new()
        };
        let mut input = LineInput::new(ph);
        input.number = number;
        TextEditor {
            key: item.key.clone(),
            input,
            committed: String::new(),
        }
    }

    fn changed(&mut self, cx: &mut EdCx<'_>) {
        cx.timer(TimerId::Commit(self.key.clone()), DEBOUNCE_TEXT);
    }
}

impl Editor for TextEditor {
    fn disabled(&self) -> bool {
        self.input.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, _slot: usize, cx: &mut EdCx<'_>) -> bool {
        match self.input.handle_key(k) {
            InputEvent::Changed => {
                self.changed(cx);
                true
            }
            InputEvent::Submitted => {
                self.commit(cx);
                cx.emit(EdMsg::FocusNext);
                true
            }
            InputEvent::Handled => true,
            InputEvent::Ignored => false,
        }
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if self.input.paste(text) == InputEvent::Changed {
            self.changed(cx);
        }
    }

    fn blur(&mut self, _slot: usize, cx: &mut EdCx<'_>) {
        self.commit(cx);
    }

    fn timer(&mut self, id: &TimerId, _focused: bool, cx: &mut EdCx<'_>) {
        if matches!(id, TimerId::Commit(_)) {
            self.commit(cx);
        }
    }

    fn sync(&mut self, view: &ItemView, focused: bool, _cx: &mut EdCx<'_>) {
        self.input.disabled = view.disabled;
        if !focused && self.input.value() != view.text {
            self.committed = view.text.clone();
            self.input.set_value(&view.text);
        }
    }

    fn dirty(&self) -> bool {
        self.input.value() != self.committed
    }

    fn commit(&mut self, cx: &mut EdCx<'_>) {
        cx.timers.cancel(&TimerId::Commit(self.key.clone()));
        let v = self.input.value();
        if v != self.committed {
            self.committed = v.clone();
            cx.emit(EdMsg::Edited(self.key.clone(), Value::Str(v)));
        }
    }

    fn height(&self, _width: u16) -> u16 {
        3
    }

    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        self.input.render(area, buf, focus.is_some(), theme)
    }
}

/// Multi-line plain text (Enter is a newline; commit on pause or blur).
pub struct ParagraphEditor {
    key: String,
    pub area: TextArea,
    committed: String,
}

impl ParagraphEditor {
    pub fn new(item: &FormItem) -> Self {
        ParagraphEditor {
            key: item.key.clone(),
            area: TextArea::default(),
            committed: String::new(),
        }
    }
}

fn area_key(area: &mut TextArea, key: &str, k: &KeyEvent, cx: &mut EdCx<'_>) -> bool {
    match area.handle_key(k) {
        InputEvent::Changed => {
            cx.timer(TimerId::Commit(key.to_string()), DEBOUNCE_TEXT);
            true
        }
        InputEvent::Handled | InputEvent::Submitted => true,
        InputEvent::Ignored => false,
    }
}

fn area_commit(area: &TextArea, key: &str, committed: &mut String, cx: &mut EdCx<'_>) {
    cx.timers.cancel(&TimerId::Commit(key.to_string()));
    let text = area.text();
    if text != *committed {
        *committed = text.clone();
        cx.emit(EdMsg::Edited(key.to_string(), Value::Str(text)));
    }
}

impl Editor for ParagraphEditor {
    fn disabled(&self) -> bool {
        self.area.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, _slot: usize, cx: &mut EdCx<'_>) -> bool {
        area_key(&mut self.area, &self.key, k, cx)
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if !self.area.disabled {
            self.area.insert(text);
            cx.timer(TimerId::Commit(self.key.clone()), DEBOUNCE_TEXT);
        }
    }

    fn blur(&mut self, _slot: usize, cx: &mut EdCx<'_>) {
        self.commit(cx);
    }

    fn timer(&mut self, id: &TimerId, _focused: bool, cx: &mut EdCx<'_>) {
        if matches!(id, TimerId::Commit(_)) {
            self.commit(cx);
        }
    }

    fn sync(&mut self, view: &ItemView, focused: bool, _cx: &mut EdCx<'_>) {
        self.area.disabled = view.disabled;
        if !focused && self.area.text() != view.text {
            self.committed = view.text.clone();
            self.area.load_text(&view.text);
        }
    }

    fn dirty(&self) -> bool {
        self.area.text() != self.committed
    }

    fn commit(&mut self, cx: &mut EdCx<'_>) {
        area_commit(&self.area, &self.key, &mut self.committed, cx);
    }

    fn height(&self, _width: u16) -> u16 {
        5
    }

    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        self.area.render(area, buf, focus.is_some(), theme)
    }
}

pub const DESCRIPTION_BAR: &str =
    "Markdown: **bold**, *italic*, - lists, [link](https://…) · F3 preview";

/// Markdown text area with a preview (F3); sent as HTML.
pub struct DescriptionEditor {
    key: String,
    pub area: TextArea,
    pub preview: bool,
    committed: String,
}

impl DescriptionEditor {
    pub fn new(item: &FormItem) -> Self {
        DescriptionEditor {
            key: item.key.clone(),
            area: TextArea::default(),
            preview: false,
            committed: String::new(),
        }
    }

    fn preview_lines(&self) -> Vec<Line<'static>> {
        let text = self.area.text();
        let text = if text.is_empty() {
            "_(empty)_".to_string()
        } else {
            text
        };
        text.lines().map(|l| Line::from(l.to_string())).collect()
    }
}

impl Editor for DescriptionEditor {
    fn disabled(&self) -> bool {
        self.area.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, _slot: usize, cx: &mut EdCx<'_>) -> bool {
        if k.code == KeyCode::F(3) {
            // focus stays here in preview mode so F3 toggles back
            self.preview = !self.preview;
            return true;
        }
        if self.preview {
            return false;
        }
        area_key(&mut self.area, &self.key, k, cx)
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if !self.preview && !self.area.disabled {
            self.area.insert(text);
            cx.timer(TimerId::Commit(self.key.clone()), DEBOUNCE_TEXT);
        }
    }

    fn blur(&mut self, _slot: usize, cx: &mut EdCx<'_>) {
        self.commit(cx);
    }

    fn timer(&mut self, id: &TimerId, _focused: bool, cx: &mut EdCx<'_>) {
        if matches!(id, TimerId::Commit(_)) {
            self.commit(cx);
        }
    }

    fn sync(&mut self, view: &ItemView, focused: bool, _cx: &mut EdCx<'_>) {
        if !focused && self.area.text() != view.text {
            self.committed = view.text.clone();
            self.area.load_text(&view.text);
        }
    }

    fn dirty(&self) -> bool {
        self.area.text() != self.committed
    }

    fn commit(&mut self, cx: &mut EdCx<'_>) {
        area_commit(&self.area, &self.key, &mut self.committed, cx);
    }

    fn height(&self, _width: u16) -> u16 {
        if self.preview {
            (self.preview_lines().len() as u16 + 2).min(14) + 1
        } else {
            10 + 1
        }
    }

    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        let body = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
        let cursor = if self.preview {
            Paragraph::new(self.preview_lines())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(theme.primary)),
                )
                .render(body, buf);
            None
        } else {
            self.area.render(body, buf, focus.is_some(), theme)
        };
        buf.set_stringn(
            area.x,
            area.bottom().saturating_sub(1),
            DESCRIPTION_BAR,
            usize::from(area.width),
            Style::default().fg(theme.text_muted),
        );
        cursor
    }

    fn hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("f3", "Preview")]
    }
}

/// `▐X▌ Yes`; Enter/Space toggles.
pub struct CheckEditor {
    key: String,
    pub value: bool,
    pub disabled: bool,
}

impl CheckEditor {
    pub fn new(item: &FormItem) -> Self {
        CheckEditor {
            key: item.key.clone(),
            value: false,
            disabled: false,
        }
    }
}

impl Editor for CheckEditor {
    fn disabled(&self) -> bool {
        self.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, _slot: usize, cx: &mut EdCx<'_>) -> bool {
        if self.disabled || !matches!(k.code, KeyCode::Enter | KeyCode::Char(' ')) {
            return false;
        }
        self.value = !self.value;
        cx.emit(EdMsg::Edited(self.key.clone(), Value::Bool(self.value)));
        true
    }

    fn sync(&mut self, view: &ItemView, _focused: bool, _cx: &mut EdCx<'_>) {
        self.disabled = view.disabled;
        self.value = matches!(view.value, Value::Bool(true));
    }

    fn height(&self, _width: u16) -> u16 {
        1
    }

    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        let mark = if self.value { "X" } else { " " };
        let mut style = Style::default().fg(theme.foreground);
        if focus.is_some() {
            style = style.add_modifier(Modifier::REVERSED);
        }
        if self.disabled {
            style = style.add_modifier(Modifier::DIM);
        }
        buf.set_stringn(
            area.x,
            area.y,
            format!("▐{mark}▌ Yes"),
            usize::from(area.width),
            style,
        );
        None
    }
}

/// Paths separated by `;`, with file-system completion (→ accepts).
pub struct FileEditor {
    key: String,
    pub input: LineInput,
    committed: String,
    files: Vec<String>,
}

impl FileEditor {
    pub fn new(item: &FormItem) -> Self {
        let mut rules = vec![];
        if !item.allowed_types.is_empty() {
            rules.push(item.allowed_types.join(", "));
        }
        if let Some(n) = item.max_files.filter(|n| *n != 0) {
            rules.push(format!("max {n}"));
        }
        let mut ph = "path/to/file.pdf; another.png (→ completes)".to_string();
        if !rules.is_empty() {
            ph.push_str(&format!("  [{}]", rules.join(" · ")));
        }
        FileEditor {
            key: item.key.clone(),
            input: LineInput::new(ph),
            committed: String::new(),
            files: vec![],
        }
    }

    fn list_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .files
            .iter()
            .map(|p| {
                let name = std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match std::fs::metadata(p) {
                    Ok(m) => format!("✓ {name}  {}", human_size(m.len())),
                    Err(_) => format!("✗ {name}  missing"),
                }
            })
            .collect();
        if !lines.is_empty() {
            lines.push("uploaded only after you confirm the review".into());
        }
        lines
    }
}

impl Editor for FileEditor {
    fn disabled(&self) -> bool {
        self.input.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, _slot: usize, cx: &mut EdCx<'_>) -> bool {
        match self.input.handle_key(k) {
            InputEvent::Submitted => {
                self.commit(cx);
                true
            }
            InputEvent::Changed => {
                self.input.suggestion = complete_path(&self.input.value(), &cx.session.paths);
                true
            }
            InputEvent::Handled => true,
            InputEvent::Ignored => false,
        }
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if self.input.paste(text) == InputEvent::Changed {
            self.input.suggestion = complete_path(&self.input.value(), &cx.session.paths);
        }
    }

    fn blur(&mut self, _slot: usize, cx: &mut EdCx<'_>) {
        self.commit(cx);
    }

    fn sync(&mut self, view: &ItemView, focused: bool, _cx: &mut EdCx<'_>) {
        self.input.disabled = view.disabled;
        if !focused && self.input.value() != view.text {
            self.committed = view.text.clone();
            self.input.set_value(&view.text);
        }
        self.files = view
            .value
            .as_list()
            .map(|l| {
                l.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
    }

    fn dirty(&self) -> bool {
        self.input.value() != self.committed
    }

    fn commit(&mut self, cx: &mut EdCx<'_>) {
        let v = self.input.value();
        if v != self.committed {
            self.committed = v.clone();
            cx.emit(EdMsg::Edited(self.key.clone(), Value::Str(v)));
        }
    }

    fn height(&self, _width: u16) -> u16 {
        3 + self.list_lines().len() as u16
    }

    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focus: Option<usize>,
        theme: &Theme,
    ) -> Option<Position> {
        let cursor = self.input.render(
            Rect::new(area.x, area.y, area.width, 3),
            buf,
            focus.is_some(),
            theme,
        );
        let lines = self.list_lines();
        let n = lines.len();
        for (i, l) in lines.into_iter().enumerate() {
            let mut style = Style::default().fg(theme.text_muted);
            if i + 1 == n {
                style = style.add_modifier(Modifier::DIM);
            }
            buf.set_stringn(
                area.x + 1,
                area.y + 3 + i as u16,
                l,
                usize::from(area.width.saturating_sub(2)),
                style,
            );
        }
        cursor
    }
}
