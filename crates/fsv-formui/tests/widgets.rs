//! The field editors driven one by one (keys, timers, syncs) without the
//! screen: what each reports to the form and when.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{FakeApi, FakeBackend, acme, engine, init, rules};
use fsv_forms::Value;
use fsv_forms::pydt::DateTimeValue;
use fsv_formui::files::human_size;
use fsv_formui::kit::draw::row_text;
use fsv_formui::kit::{Theme, Timers, parse_key};
use fsv_formui::session::{ATTACHMENTS, FormItem, FormSession, ItemView};
use fsv_formui::widgets::{
    COMBO_CLOSE, CheckEditor, ComboBox, DateEditor, DescriptionEditor, EdCx, EdMsg, Editor,
    FieldRow, FileEditor, ParagraphEditor, TextEditor, TimerId, safe_id,
};
use fsv_formui::{FormKind, FormMode};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

struct H {
    s: FormSession,
    timers: Timers<TimerId>,
    out: Vec<EdMsg>,
}

impl H {
    fn new() -> H {
        H::with(acme().1)
    }

    fn with(s: FormSession) -> H {
        H {
            s,
            timers: Timers::default(),
            out: vec![],
        }
    }

    fn cx<R>(&mut self, f: impl FnOnce(&mut EdCx<'_>) -> R) -> R {
        let mut cx = EdCx {
            session: &self.s,
            now: Duration::ZERO,
            timers: &mut self.timers,
            out: &mut self.out,
        };
        f(&mut cx)
    }

    fn take(&mut self) -> Vec<EdMsg> {
        std::mem::take(&mut self.out)
    }

    fn item(&self, key: &str) -> FormItem {
        self.s.item(key).expect("item").clone()
    }

    fn view(&self, key: &str, text: &str, value: Value) -> ItemView {
        let mut v = self.s.view(key);
        v.text = text.into();
        v.value = value;
        v
    }

    fn keys(&mut self, ed: &mut dyn Editor, slot: usize, keys: &[&str]) {
        for k in keys {
            self.cx(|cx| ed.handle_key(&parse_key(k), slot, cx));
        }
    }

    fn typ(&mut self, ed: &mut dyn Editor, text: &str) {
        for ch in text.chars() {
            self.cx(|cx| ed.handle_key(&parse_key(&ch.to_string()), 0, cx));
        }
    }
}

fn edited(key: &str, v: Value) -> EdMsg {
    EdMsg::Edited(key.into(), v)
}

fn draw(ed: &mut dyn Editor, w: u16, h: u16) -> Vec<String> {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    ed.render(area, &mut buf, Some(0), &Theme::textual_dark());
    (0..h).map(|y| row_text(&buf, y)).collect()
}

/// `TextEditor`: commits after the pause, on Enter (then the next field)
/// and on blur; only a change is reported; focus selects the text.
#[test]
fn text_editor_commits_changes_only() {
    let mut h = H::new();
    let mut ed = TextEditor::new(&h.item("subject"), false);
    assert_eq!(ed.input.placeholder, "Service: objective");
    assert!(TextEditor::new(&h.item("subject"), true).input.number);
    h.typ(&mut ed, "Mail");
    assert!(h.timers.is_set(&TimerId::Commit("subject".into())));
    assert!(ed.dirty());
    h.cx(|cx| ed.timer(&TimerId::Commit("subject".into()), true, cx));
    assert_eq!(h.take(), [edited("subject", Value::str("Mail"))]);
    assert!(!ed.dirty());
    h.cx(|cx| ed.commit(cx));
    assert!(h.take().is_empty());
    h.typ(&mut ed, "!");
    h.keys(&mut ed, 0, &["enter"]);
    assert_eq!(
        h.take(),
        [edited("subject", Value::str("Mail!")), EdMsg::FocusNext]
    );
    assert!(!h.timers.is_set(&TimerId::Commit("subject".into())));
    h.typ(&mut ed, "?");
    h.cx(|cx| ed.blur(0, cx));
    assert_eq!(h.take(), [edited("subject", Value::str("Mail!?"))]);

    // a focused input keeps what is typed; a blurred one shows the form
    let v = h.view("subject", "Other", Value::str("Other"));
    h.cx(|cx| ed.sync(&v, true, cx));
    assert_eq!(ed.input.value(), "Mail!?");
    h.cx(|cx| ed.sync(&v, false, cx));
    assert_eq!(ed.input.value(), "Other");
    assert!(!ed.dirty());
    h.cx(|cx| ed.focus(0, cx));
    h.typ(&mut ed, "x");
    assert_eq!(ed.input.value(), "x");
    h.cx(|cx| ed.paste("yz", 0, cx));
    assert_eq!(ed.input.value(), "xyz");
}

/// `ParagraphEditor`: Enter is a newline; commit on the pause or blur.
#[test]
fn paragraph_editor_commits_on_the_pause_or_blur() {
    let mut h = H::new();
    let mut ed = ParagraphEditor::new(&h.item("downtime_window"));
    h.typ(&mut ed, "a");
    h.keys(&mut ed, 0, &["enter"]);
    h.typ(&mut ed, "b");
    assert!(ed.dirty());
    assert!(h.take().is_empty());
    h.cx(|cx| ed.timer(&TimerId::Commit("downtime_window".into()), true, cx));
    assert_eq!(h.take(), [edited("downtime_window", Value::str("a\nb"))]);
    h.cx(|cx| ed.paste("c", 0, cx));
    h.cx(|cx| ed.blur(0, cx));
    assert_eq!(h.take(), [edited("downtime_window", Value::str("a\nbc"))]);
    h.cx(|cx| ed.commit(cx));
    assert!(h.take().is_empty());
    let v = h.view("downtime_window", "x\ny", Value::str("x\ny"));
    h.cx(|cx| ed.sync(&v, true, cx));
    assert_eq!(ed.area.text(), "a\nbc");
    h.cx(|cx| ed.sync(&v, false, cx));
    assert_eq!(ed.area.text(), "x\ny");
    assert!(!ed.dirty());
}

/// `DescriptionEditor`: F3 shows the Markdown preview (keys and pastes
/// do not edit it), F3 again edits.
#[test]
fn description_editor_preview_and_commit() {
    let mut h = H::new();
    let mut ed = DescriptionEditor::new(&h.item("description"));
    assert_eq!(ed.height(76), 11);
    h.keys(&mut ed, 0, &["f3"]);
    assert!(ed.preview);
    // `_(empty)_`: border, "(empty)", its margin, border, the bar
    assert_eq!(ed.height(76), 5);
    let rows = draw(&mut ed, 76, 5);
    assert_eq!(rows[1], format!("│ {:<73}│", "(empty)"));
    assert!(rows[4].starts_with("Markdown: **bold**, *italic*"));
    assert!(!h.cx(|cx| ed.handle_key(&parse_key("x"), 0, cx)));
    h.cx(|cx| ed.paste("x", 0, cx));
    assert_eq!(ed.area.text(), "");
    ed.action_toggle_preview();
    assert!(!ed.preview);
    h.typ(&mut ed, "Rotate **now**");
    assert!(ed.dirty());
    h.keys(&mut ed, 0, &["f3"]);
    assert_eq!(draw(&mut ed, 76, 5)[1], format!("│ {:<73}│", "Rotate now"));
    h.keys(&mut ed, 0, &["f3"]);
    h.cx(|cx| ed.blur(0, cx));
    assert_eq!(
        h.take(),
        [edited("description", Value::str("Rotate **now**"))]
    );
    let v = h.view("description", "Other", Value::str("Other"));
    h.cx(|cx| ed.sync(&v, false, cx));
    assert_eq!(ed.area.text(), "Other");
    h.cx(|cx| ed.commit(cx));
    assert!(h.take().is_empty() && !ed.dirty());
}

/// `CheckEditor`: Space or Enter toggles; the form value is shown.
#[test]
fn check_editor_toggles_and_syncs() {
    let mut h = H::new();
    let mut ed = CheckEditor::new(&h.item("legacy_flag"));
    h.keys(&mut ed, 0, &["space"]);
    assert_eq!(h.take(), [edited("legacy_flag", Value::Bool(true))]);
    h.keys(&mut ed, 0, &["enter"]);
    assert_eq!(h.take(), [edited("legacy_flag", Value::Bool(false))]);
    let v = h.view("legacy_flag", "Yes", Value::Bool(true));
    h.cx(|cx| ed.sync(&v, false, cx));
    assert!(ed.value);
    assert_eq!(draw(&mut ed, 20, 1)[0].trim_end(), "▐X▌ Yes");
    let mut v = h.view("legacy_flag", "", Value::Null);
    v.disabled = true;
    h.cx(|cx| ed.sync(&v, false, cx));
    assert!(!ed.value && ed.disabled());
    assert!(!h.cx(|cx| ed.handle_key(&parse_key("space"), 0, cx)));
}

/// `ComboBox` over choices: focus opens the full list with the value
/// selected, typing filters, Enter picks and closes, Esc closes.
#[test]
fn combo_box_filters_and_picks_choices() {
    let mut h = H::new();
    let mut c = ComboBox::new(&h.item("change_type"));
    assert_eq!(c.input.placeholder, "type to filter, Enter picks");
    let v = h.view("change_type", "Normal", Value::str("Normal"));
    h.cx(|cx| c.sync(&v, false, cx));
    assert_eq!(
        (c.input.value(), c.shown_text.as_str()),
        ("Normal".into(), "Normal")
    );
    assert!(!c.dirty());
    h.cx(|cx| c.focus(0, cx));
    assert!(c.is_open);
    assert_eq!(h.take(), [EdMsg::Opened("change_type".into())]);
    assert_eq!(c.input.selection(), Some((0, 6)));
    assert_eq!(c.options.len(), 3);
    h.typ(&mut c, "st");
    assert_eq!(
        c.options
            .iter()
            .map(|o| o.label.as_str())
            .collect::<Vec<_>>(),
        ["Standard"]
    );
    assert!(c.dirty());
    h.keys(&mut c, 0, &["enter"]);
    assert_eq!(h.take(), [edited("change_type", Value::str("Standard"))]);
    assert!(!c.is_open && c.input.value() == "Standard");

    // Down opens a closed list; Esc closes it, then is not used
    h.keys(&mut c, 0, &["down"]);
    assert!(c.is_open);
    h.take();
    h.keys(&mut c, 0, &["down", "down", "down"]);
    assert_eq!(c.highlighted, Some(2));
    h.keys(&mut c, 0, &["up"]);
    assert_eq!(c.highlighted, Some(1));
    assert!(h.cx(|cx| c.handle_key(&parse_key("escape"), 0, cx)));
    assert!(!c.is_open);
    assert!(!h.cx(|cx| c.handle_key(&parse_key("escape"), 0, cx)));

    // an emptied box clears the value
    h.cx(|cx| c.open(cx));
    h.keys(&mut c, 0, &["ctrl+u"]);
    h.keys(&mut c, 0, &["enter"]);
    assert_eq!(h.take().last(), Some(&edited("change_type", Value::Null)));
    assert_eq!(c.shown_text, "");

    // blur closes after the pause unless focus came back
    h.cx(|cx| c.open(cx));
    h.cx(|cx| c.blur(0, cx));
    assert!(h.timers.is_set(&TimerId::ComboClose("change_type".into())));
    assert_eq!(h.timers.next_deadline(), Some(COMBO_CLOSE));
    h.cx(|cx| c.timer(&TimerId::ComboClose("change_type".into()), true, cx));
    assert!(c.is_open);
    h.typ(&mut c, "zz");
    h.cx(|cx| c.timer(&TimerId::ComboClose("change_type".into()), false, cx));
    assert!(!c.is_open);
    assert_eq!(c.input.value(), "", "the typed text goes back to the value");
}

/// `ComboBox.show_options` / `_note`: the highlighted choice's
/// description, then the note.
#[test]
fn combo_box_notes() {
    let mut h = H::new();
    let mut c = ComboBox::new(&h.item("priority"));
    h.cx(|cx| c.open(cx));
    let mut opts = c.options.clone();
    opts[0].detail = "Can wait".into();
    c.show_options(opts, "3 more");
    assert_eq!(c.note, "Can wait · 3 more");
    c.show_options(vec![], "no choice matches");
    assert_eq!(
        (c.highlighted, c.note.as_str()),
        (None, "no choice matches")
    );
    c.close(true);
    assert!(c.note.is_empty() && c.options.is_empty());
}

/// `ComboBox` over people: searches after the pause, groups and agents at
/// once; answers for other text are dropped.
#[test]
fn combo_box_searches_remote_lookups() {
    let mut h = H::new();
    let mut c = ComboBox::new(&h.item("requester"));
    assert!(c.remote);
    h.cx(|cx| c.focus(0, cx));
    h.take();
    assert!(h.timers.is_set(&TimerId::Search("requester".into())));
    h.cx(|cx| c.timer(&TimerId::Search("requester".into()), true, cx));
    assert_eq!(
        h.take(),
        [EdMsg::Search {
            key: "requester".into(),
            query: String::new(),
            generation: 1
        }]
    );
    assert_eq!((c.searches, c.note.as_str()), (1, "searching…"));
    h.typ(&mut c, "jan");
    assert!(c.stale());
    assert_eq!(c.note, "searching…");
    let (sugg, _) = h.s.suggestions("requester", "jan").expect("people");
    c.found(Some(0), "jan", sugg.clone(), "");
    assert!(c.options.is_empty(), "an older search");
    c.found(Some(1), "ja", sugg.clone(), "");
    assert!(c.options.is_empty(), "other text");
    c.found(None, "jan", sugg, "");
    assert!(!c.stale());
    assert_eq!(c.options[0].label, "Jane Doe <jane.doe@example.com>");
    h.keys(&mut c, 0, &["enter"]);
    let out = h.take();
    assert!(matches!(&out[..], [EdMsg::Edited(k, Value::Lookup(_))] if k == "requester"));

    let mut g = ComboBox::new(&h.item("group"));
    h.cx(|cx| g.open(cx));
    let out = h.take();
    assert!(
        matches!(&out[..], [EdMsg::Opened(_), EdMsg::Search { query, .. }] if query.is_empty())
    );
}

/// A multi-select toggles with Enter and stays open; the chosen line.
#[test]
fn combo_box_multi_select() {
    let mut h = H::new();
    let mut c = ComboBox::new(&h.item("services"));
    assert_eq!(c.input.placeholder, "type to filter, Enter toggles");
    h.cx(|cx| c.focus(0, cx));
    let rows = c.height(40);
    assert_eq!(draw(&mut c, 40, rows)[0].trim(), "nothing selected");
    h.take();
    h.typ(&mut c, "bill");
    h.keys(&mut c, 0, &["enter"]);
    assert_eq!(
        h.take(),
        [edited("services", Value::List(vec![Value::str("Billing")]))]
    );
    assert!(c.is_open && c.input.value().is_empty());
    let v = h.view(
        "services",
        "Billing",
        Value::List(vec![Value::str("Billing")]),
    );
    h.cx(|cx| c.sync(&v, true, cx));
    let rows = c.height(40);
    assert_eq!(draw(&mut c, 40, rows)[0].trim(), "✓ Billing");
}

/// `DateEditor`: a live preview, unreadable text kept with its error
/// (withdrawn when it is put back), F2 / the button ask for the calendar.
#[test]
fn date_editor_previews_keeps_errors_and_opens_the_calendar() {
    let mut h = H::new();
    let mut d = DateEditor::new(&h.item("planned_start_date"), &h.s);
    assert_eq!(d.key(), "planned_start_date");
    assert_eq!(d.slots(), 2);
    assert_eq!(
        d.parse("2030-11-26 19:00", &h.s).preview,
        "Tue 26 Nov 2030 19:00 (+09:00 from your Freshservice profile tz)"
    );
    h.typ(&mut d, "2031-02-30 10:00");
    assert!(d.dirty());
    h.keys(&mut d, 0, &["enter"]);
    let err = "'2031-02-30 10:00' is not a real date";
    assert_eq!(
        h.take(),
        [EdMsg::Invalid("planned_start_date".into(), err.into())]
    );
    assert_eq!(d.invalid.as_deref(), Some("2031-02-30 10:00"));
    // a re-sync does not wipe the text being fixed
    let v = h.view("planned_start_date", "", Value::Null);
    h.cx(|cx| d.sync(&v, false, cx));
    assert_eq!(d.input.value(), "2031-02-30 10:00");
    h.keys(&mut d, 0, &["ctrl+u"]);
    h.cx(|cx| d.blur(0, cx));
    assert_eq!(
        h.take(),
        [EdMsg::Invalid("planned_start_date".into(), String::new())]
    );
    h.typ(&mut d, "2030-11-26 19:00");
    h.cx(|cx| d.commit(cx));
    let out = h.take();
    assert!(matches!(&out[..], [EdMsg::Edited(_, Value::DateTime(_))]));
    assert!(!d.dirty());
    h.cx(|cx| d.focus(0, cx));
    assert_eq!(d.input.selection(), Some((0, 16)));

    h.keys(&mut d, 0, &["f2"]);
    h.keys(&mut d, 1, &["enter"]);
    h.cx(|cx| d.action_calendar(cx));
    assert_eq!(
        h.take(),
        vec![EdMsg::OpenCalendar("planned_start_date".into()); 3]
    );
    let mut v = h.view("planned_start_date", "", Value::Null);
    v.disabled = true;
    h.cx(|cx| d.sync(&v, false, cx));
    assert!(!h.cx(|cx| d.handle_key(&parse_key("enter"), 1, cx)));
}

/// `DateEditor.set_value` from the calendar: the text, then the value.
/// An ambiguous wall time keeps its offset (`when._no_dst_doubt`).
#[test]
fn date_editor_set_value_writes_the_calendar_pick() {
    let api = Arc::new(FakeApi::default());
    let mut i = init(
        FormKind::Change,
        FormMode::New,
        api,
        Arc::new(FakeBackend::default()),
    );
    i.rules = Some(engine(&i.spec, &rules(Some("Berlin"))));
    let mut h = H::with(FormSession::new(i).expect("session"));
    let mut d = DateEditor::new(&h.item("planned_start_date"), &h.s);
    for (local, offset, text) in [
        (
            jiff::civil::date(2031, 10, 26).at(2, 30, 0, 0),
            1,
            "2031-10-26 02:30 +01:00",
        ),
        (
            jiff::civil::date(2031, 7, 1).at(9, 15, 0, 0),
            2,
            "2031-07-01 09:15",
        ),
    ] {
        let v = Value::DateTime(DateTimeValue::new(
            local,
            jiff::tz::Offset::constant(offset),
        ));
        h.cx(|cx| d.set_value(v.clone(), cx));
        assert_eq!(d.input.value(), text);
        assert!(!d.dirty());
        assert_eq!(h.take(), [edited("planned_start_date", v)]);
    }
}

/// `FileEditor`: completes paths (→ accepts), lists the chosen files and
/// commits the typed text.
#[test]
fn file_editor_completes_and_lists() {
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::write(dir.path().join("report.pdf"), "abc").expect("write");
    let mut h = H::new();
    let mut f = FileEditor::new(&h.item(ATTACHMENTS));
    assert!(
        f.input
            .placeholder
            .starts_with("path/to/file.pdf; another.png (→ completes)")
    );
    let d = dir.path().display().to_string();
    h.typ(&mut f, &format!("{d}/rep"));
    assert_eq!(f.input.suggestion, Some(format!("{d}/report.pdf")));
    h.keys(&mut f, 0, &["right"]);
    assert_eq!(f.input.value(), format!("{d}/report.pdf"));
    assert!(f.dirty());
    h.keys(&mut f, 0, &["enter"]);
    assert_eq!(
        h.take(),
        [edited(ATTACHMENTS, Value::str(format!("{d}/report.pdf")))]
    );
    let files = Value::List(vec![
        Value::str(format!("{d}/report.pdf")),
        Value::str(format!("{d}/gone.png")),
    ]);
    let v = h.view(ATTACHMENTS, &format!("{d}/report.pdf; {d}/gone.png"), files);
    h.cx(|cx| f.sync(&v, false, cx));
    assert!(!f.dirty());
    assert_eq!(f.height(80), 6);
    let rows = draw(&mut f, 80, 6);
    assert_eq!(
        rows[3].trim_end(),
        format!(" ✓ report.pdf  {}", human_size(3))
    );
    assert_eq!(rows[3].trim_end(), " ✓ report.pdf  3 B");
    assert_eq!(rows[4].trim_end(), " ✗ gone.png  missing");
    assert_eq!(
        rows[5].trim_end(),
        " uploaded only after you confirm the review"
    );
    h.cx(|cx| f.focus(0, cx));
    h.cx(|cx| f.paste(";", 0, cx));
    h.cx(|cx| f.blur(0, cx));
    assert_eq!(h.take(), [edited(ATTACHMENTS, Value::str(";"))]);
    h.cx(|cx| f.commit(cx));
    assert!(h.take().is_empty());
}

/// `FieldRow`: the key, syncing the view into the editor, focusability.
#[test]
fn field_row_syncs_its_editor() {
    let mut h = H::new();
    let mut row = FieldRow::new(&h.item("subject"), &h.s);
    assert_eq!(row.key(), "subject");
    h.s.set("subject", Value::str("Billing: rotate keys"));
    h.cx(|cx| row.sync(false, cx));
    assert!(row.focusable());
    assert_eq!(row.view.text, "Billing: rotate keys");
    let ed = row.editor.as_mut().and_then(|e| e.as_combo());
    assert!(ed.is_none());
    let mut staging = FieldRow::new(&h.item("staging_start"), &h.s);
    h.cx(|cx| staging.sync(false, cx));
    assert!(staging.focusable());
    h.s.set("cloud_only", Value::str("Yes"));
    h.cx(|cx| staging.sync(false, cx));
    assert!(!staging.focusable(), "hidden when Cloud Only = Yes");
    assert_eq!(
        safe_id("planning:Go-Live Checklist"),
        "planning_Go-Live_Checklist"
    );
}
