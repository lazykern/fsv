//! Text, number, paragraph, description, checkbox and file editors.

use crossterm::event::{KeyCode, KeyEvent};
use fsv_forms::Value;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use super::markdown::markdown_lines;
use super::{DEBOUNCE_TEXT, EdCx, EdMsg, Editor, TimerId};
use crate::files::{complete_path, human_size};
use crate::kit::draw::draw_lines;
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

    /// `TextEditor._changed`: commit after the pause.
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

    fn focus(&mut self, _slot: usize, _cx: &mut EdCx<'_>) {
        self.input.select_all();
    }

    /// `TextEditor.on_blur`: leaving the field commits it.
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

    /// `ParagraphEditor._changed`: commit after the pause.
    fn changed(&mut self, cx: &mut EdCx<'_>) {
        cx.timer(TimerId::Commit(self.key.clone()), DEBOUNCE_TEXT);
    }
}

/// A key for a text area: `(used, text changed)`.
fn area_key(area: &mut TextArea, k: &KeyEvent) -> (bool, bool) {
    match area.handle_key(k) {
        InputEvent::Changed => (true, true),
        InputEvent::Handled | InputEvent::Submitted => (true, false),
        InputEvent::Ignored => (false, false),
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
        let (used, changed) = area_key(&mut self.area, k);
        if changed {
            self.changed(cx);
        }
        used
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if !self.area.disabled {
            self.area.insert(text);
            self.changed(cx);
        }
    }

    /// `ParagraphEditor.on_blur`: leaving the field commits it.
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
    /// Port of fsv.formui.widgets.DescriptionEditor.area
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

    /// The Markdown preview inside the border and the side padding.
    fn preview_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        markdown_lines(&self.area.text(), width.saturating_sub(4), theme)
    }

    /// `DescriptionEditor.action_toggle_preview` (F3).
    pub fn action_toggle_preview(&mut self) {
        self.preview = !self.preview;
    }

    /// `DescriptionEditor._changed`: commit after the pause.
    fn changed(&mut self, cx: &mut EdCx<'_>) {
        cx.timer(TimerId::Commit(self.key.clone()), DEBOUNCE_TEXT);
    }
}

impl Editor for DescriptionEditor {
    fn disabled(&self) -> bool {
        self.area.disabled
    }

    fn handle_key(&mut self, k: &KeyEvent, _slot: usize, cx: &mut EdCx<'_>) -> bool {
        if k.code == KeyCode::F(3) {
            // focus stays here in preview mode so F3 toggles back
            self.action_toggle_preview();
            return true;
        }
        if self.preview {
            return false;
        }
        let (used, changed) = area_key(&mut self.area, k);
        if changed {
            self.changed(cx);
        }
        used
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if !self.preview && !self.area.disabled {
            self.area.insert(text);
            self.changed(cx);
        }
    }

    /// `DescriptionEditor.on_descendant_blur`: leaving the area commits it.
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

    fn height(&self, width: u16) -> u16 {
        if self.preview {
            let rows = self.preview_lines(width, &Theme::textual_dark()).len();
            (rows as u16 + 2).min(14) + 1
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
            let frame = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.primary));
            let inner = frame.inner(body);
            frame.render(body, buf);
            let text = Rect::new(
                inner.x + 1,
                inner.y,
                inner.width.saturating_sub(2),
                inner.height,
            );
            draw_lines(text, buf, &self.preview_lines(body.width, theme), 0);
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

    /// `CheckEditor._changed`: a toggle by the person is a committed value.
    /// [`Editor::sync`] sets the box without reporting it.
    fn changed(&mut self, value: bool, cx: &mut EdCx<'_>) {
        self.value = value;
        cx.emit(EdMsg::Edited(self.key.clone(), Value::Bool(value)));
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
        self.changed(!self.value, cx);
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
    /// Port of fsv.formui.widgets.FileEditor.input
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

    /// `PathSuggester.get_suggestion`: the completion → accepts.
    fn suggest(&mut self, cx: &EdCx<'_>) {
        self.input.suggestion = complete_path(&self.input.value(), &cx.session.paths);
    }

    /// `FileEditor._enter`: Enter commits the paths.
    fn enter(&mut self, cx: &mut EdCx<'_>) {
        self.commit(cx);
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
                self.enter(cx);
                true
            }
            InputEvent::Changed => {
                self.suggest(cx);
                true
            }
            InputEvent::Handled => true,
            InputEvent::Ignored => false,
        }
    }

    fn paste(&mut self, text: &str, _slot: usize, cx: &mut EdCx<'_>) {
        if self.input.paste(text) == InputEvent::Changed {
            self.suggest(cx);
        }
    }

    fn focus(&mut self, _slot: usize, _cx: &mut EdCx<'_>) {
        self.input.select_all();
    }

    /// `FileEditor.on_descendant_blur`: leaving the input commits it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::ATTACHMENTS;
    use crate::widgets::TestCx;

    fn commit_timer(key: &str) -> TimerId {
        TimerId::Commit(key.into())
    }

    /// `TextEditor._changed`, `ParagraphEditor._changed`,
    /// `DescriptionEditor._changed`: a change (re)starts the 0.3 s commit
    /// timer and reports nothing yet.
    #[test]
    fn changed_restarts_the_commit_timer() {
        let mut h = TestCx::new();
        let mut t = TextEditor::new(&h.item("subject"), false);
        h.cx(|cx| t.changed(cx));
        let mut p = ParagraphEditor::new(&h.item("downtime_window"));
        h.cx(|cx| p.changed(cx));
        let mut d = DescriptionEditor::new(&h.item("description"));
        h.cx(|cx| d.changed(cx));
        for key in ["subject", "downtime_window", "description"] {
            assert!(h.timers.is_set(&commit_timer(key)), "{key}");
        }
        assert_eq!(
            h.timers
                .due(DEBOUNCE_TEXT - std::time::Duration::from_millis(1)),
            vec![]
        );
        assert_eq!(h.timers.due(DEBOUNCE_TEXT).len(), 3);
        assert!(h.take().is_empty());
    }

    /// `CheckEditor._changed`: a toggle is reported; a sync from the form
    /// (Python's `_syncing`) is not.
    #[test]
    fn check_changed_reports_toggles_not_syncs() {
        let mut h = TestCx::new();
        let mut c = CheckEditor::new(&h.item("legacy_flag"));
        h.cx(|cx| c.changed(true, cx));
        assert!(c.value);
        assert_eq!(
            h.take(),
            [EdMsg::Edited("legacy_flag".into(), Value::Bool(true))]
        );
        let mut v = h.s.view("legacy_flag");
        v.value = Value::Bool(false);
        h.cx(|cx| c.sync(&v, false, cx));
        assert!(!c.value);
        assert!(h.take().is_empty());
    }

    /// `PathSuggester.get_suggestion` (`complete_path`) and
    /// `FileEditor._enter`.
    #[test]
    fn file_suggest_and_enter() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("report.pdf"), "abc").expect("write");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let d = dir.path().display().to_string();
        let mut h = TestCx::new();
        let mut f = FileEditor::new(&h.item(ATTACHMENTS));
        for (typed, want) in [
            (format!("{d}/rep"), Some(format!("{d}/report.pdf"))),
            (format!("a.pdf; {d}/su"), Some(format!("a.pdf; {d}/sub/"))),
            (format!("{d}/report.pdf"), None),
            (format!("{d}/zz"), None),
            ("a.pdf; ".to_string(), None),
        ] {
            f.input.set_value(&typed);
            h.cx(|cx| f.suggest(cx));
            assert_eq!(f.input.suggestion, want, "{typed}");
        }
        f.input.set_value("x.pdf");
        h.cx(|cx| f.enter(cx));
        assert_eq!(
            h.take(),
            [EdMsg::Edited(ATTACHMENTS.into(), Value::str("x.pdf"))]
        );
        h.cx(|cx| f.enter(cx));
        assert!(h.take().is_empty());
    }
}
