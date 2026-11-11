//! The form screen driven by keys, a manual clock and inline lookups
//! (no terminal): ports of the Textual pilot tests.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{FakeApi, FakeBackend, engine, init, rules};
use crossterm::event::{KeyCode, KeyEvent};
use fsv_forms::Value;
use fsv_forms::pydt::DateTimeValue;
use fsv_formui::kit::draw::buffer_text;
use fsv_formui::kit::{Theme, WorkersMode, parse_key};
use fsv_formui::session::FormSession;
use fsv_formui::{Action, FormKind, FormMode, FormScreen};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

struct Ui {
    api: Arc<FakeApi>,
    backend: Arc<FakeBackend>,
    s: FormScreen,
}

fn ui_with(tz: &str, backend: FakeBackend) -> Ui {
    let api = Arc::new(FakeApi::default());
    let backend = Arc::new(backend);
    let mut i = init(
        FormKind::Change,
        FormMode::New,
        api.clone(),
        backend.clone(),
    );
    i.rules = Some(engine(&i.spec, &rules(Some(tz))));
    let session = FormSession::new(i).expect("session");
    let s = FormScreen::with_workers(session, Theme::textual_dark(), WorkersMode::Inline);
    Ui { api, backend, s }
}

fn ui() -> Ui {
    ui_with("Tokyo", FakeBackend::default())
}

impl Ui {
    fn key(&mut self, name: &str) {
        self.s.handle_key(parse_key(name));
    }

    fn typ(&mut self, text: &str) {
        for ch in text.chars() {
            self.s.handle_key(KeyEvent::from(KeyCode::Char(ch)));
        }
    }

    fn wait(&mut self, secs: f64) {
        let t = self.s.now() + Duration::from_secs_f64(secs);
        self.s.tick(t);
    }

    fn draw(&mut self, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        term.draw(|f| {
            let area = f.area();
            self.s.render(area, f.buffer_mut());
        })
        .expect("draw");
        buffer_text(term.backend().buffer())
    }

    fn date_text(&mut self, key: &str) -> String {
        let ed = self
            .s
            .editor_mut(key)
            .and_then(|e| e.as_date())
            .expect("date editor");
        ed.input.value()
    }

    fn auto_calls(&self) -> Vec<String> {
        self.api
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("AUTO"))
            .collect()
    }
}

#[test]
fn opens_on_the_requester_with_its_help() {
    let mut u = ui();
    assert_eq!(u.s.focused_key(), Some("requester"));
    assert!(u.s.help_text().starts_with("Requester * required"));
    assert!(
        u.s.status
            .contains("required field(s) missing: Requester, Subject")
    );
    let screen = u.draw(120, 40);
    assert!(screen.contains("New Change"));
    assert!(screen.contains("Requester *"));
    assert!(
        u.s.help_text()
            .ends_with("Type to search · ↑↓ move · Enter picks")
    );
    assert!(
        u.draw(200, 40)
            .contains("Type to search · ↑↓ move · Enter picks")
    );
    assert!(screen.contains("^s Review"));
}

#[test]
fn requester_search_waits_for_the_pause_and_calls_once() {
    let mut u = ui();
    u.typ("jan");
    u.wait(0.2);
    assert!(u.auto_calls().is_empty());
    u.wait(0.1);
    assert_eq!(u.auto_calls().len(), 1, "{:?}", u.api.calls());
    assert!(u.auto_calls()[0].ends_with(" jan"));
    let combo =
        u.s.editor_mut("requester")
            .and_then(|e| e.as_combo())
            .expect("combo");
    assert_eq!(
        combo.searches, 1,
        "typing restarted the pause of the empty search on focus"
    );
    assert_eq!(
        combo
            .options
            .iter()
            .map(|o| o.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Jane Doe <jane.doe@example.com>",
            "Jane Doerr <jane.doerr@example.com>"
        ]
    );
}

#[test]
fn threaded_lookups_arrive_on_a_later_tick() {
    let api = Arc::new(FakeApi::default());
    let session = FormSession::new(init(
        FormKind::Change,
        FormMode::New,
        api.clone(),
        Arc::new(FakeBackend::default()),
    ))
    .expect("session");
    let mut s = FormScreen::new(session, Theme::textual_dark());
    for ch in "doerr".chars() {
        s.handle_key(KeyEvent::from(KeyCode::Char(ch)));
    }
    s.tick(Duration::from_millis(300));
    for i in 0..500u64 {
        s.tick(Duration::from_millis(300 + i));
        if !s.busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    s.tick(Duration::from_millis(1000));
    let combo = s
        .editor_mut("requester")
        .and_then(|e| e.as_combo())
        .expect("combo");
    assert_eq!(combo.options.len(), 1);
    assert_eq!(combo.options_query.as_deref(), Some("doerr"));
}

#[test]
fn enter_never_picks_stale_results_and_late_answers_are_dropped() {
    let mut u = ui();
    u.typ("jane");
    u.wait(0.3);
    u.typ("x");
    {
        let combo =
            u.s.editor_mut("requester")
                .and_then(|e| e.as_combo())
                .expect("combo");
        assert!(combo.stale());
        assert!(combo.options.is_empty());
        // an answer for the text typed before is ignored
        let old = combo.options.clone();
        combo.found(None, "jane", old, "");
        assert!(combo.options_query.is_none());
    }
    u.key("enter");
    assert_eq!(u.s.session.view("requester").value, Value::Null);
    let combo =
        u.s.editor_mut("requester")
            .and_then(|e| e.as_combo())
            .expect("combo");
    assert_eq!(
        combo.note,
        "still searching… (Enter again when the results are shown)"
    );
    u.key("backspace");
    u.wait(0.3);
    u.key("enter");
    let v = u.s.session.view("requester");
    assert_eq!(v.text, "Jane Doe <jane.doe@example.com>");
    assert!(matches!(v.value, Value::Lookup(_)));
}

#[test]
fn a_rejected_date_blocks_the_review_and_is_kept() {
    let mut u = ui();
    u.s.focus_key("planned_start_date");
    u.typ("2031-02-30 10:00");
    u.key("tab");
    let err =
        u.s.session
            .errors_by_key
            .get("planned_start_date")
            .cloned()
            .unwrap_or_default();
    assert_eq!(err, "'2031-02-30 10:00' is not a real date");
    assert_eq!(u.s.session.view("planned_start_date").value, Value::Null);
    u.key("ctrl+s");
    assert_eq!(u.s.layer(), Some("review"));
    let r = u.s.review().expect("review");
    assert!(r.submit_disabled());
    assert!(
        r.plan
            .errors
            .iter()
            .any(|e| e.contains("is not a real date")),
        "{:?}",
        r.plan.errors
    );
    assert!(
        u.draw(120, 40)
            .contains("Problems (must be fixed before sending)")
    );
    u.key("escape");
    assert_eq!(u.s.layer(), None);
    assert_eq!(u.date_text("planned_start_date"), "2031-02-30 10:00");
    // fixing it clears the error
    u.s.focus_key("planned_start_date");
    for _ in 0..16 {
        u.key("backspace");
    }
    u.typ("2031-03-02 10:00");
    u.key("enter");
    assert!(!u.s.session.errors_by_key.contains_key("planned_start_date"));
    assert!(matches!(
        u.s.session.view("planned_start_date").value,
        Value::DateTime(_)
    ));
}

#[test]
fn a_daylight_saving_gap_is_refused() {
    let mut u = ui_with("Berlin", FakeBackend::default());
    u.s.focus_key("planned_start_date");
    u.typ("2031-03-30 02:30");
    u.key("enter");
    let err =
        u.s.session
            .errors_by_key
            .get("planned_start_date")
            .cloned()
            .unwrap_or_default();
    assert!(
        err.starts_with(
            "2031-03-30 02:30 does not exist in Europe/Berlin (the clocks skip ahead then)"
        ),
        "{err}"
    );
    assert_eq!(u.s.session.view("planned_start_date").value, Value::Null);
}

#[test]
fn the_calendar_keeps_the_instant() {
    let mut u = ui_with("Berlin", FakeBackend::default());
    let local = jiff::civil::date(2031, 3, 2).at(18, 0, 0, 0);
    let v = Value::DateTime(DateTimeValue::new(local, jiff::tz::Offset::constant(7)));
    let instant = match &v {
        Value::DateTime(d) => d.timestamp(),
        _ => unreachable!(),
    };
    u.s.session.set("planned_start_date", v);
    u.s.refresh_rows();
    u.s.focus_key("planned_start_date");
    u.key("f2");
    assert_eq!(u.s.layer(), Some("calendar"));
    {
        let p = u.s.date_picker().expect("picker");
        assert_eq!(p.grid.cursor, jiff::civil::date(2031, 3, 2));
        assert_eq!(p.time.value(), "12:00");
    }
    assert!(u.draw(120, 40).contains("Mo Tu We Th Fr Sa Su"));
    u.key("enter"); // the day: on to the time
    u.key("enter"); // the time: OK
    assert_eq!(u.s.layer(), None);
    match u.s.session.view("planned_start_date").value {
        Value::DateTime(d) => {
            assert_eq!(d.timestamp(), instant);
            assert_eq!(d.offset, jiff::tz::Offset::constant(1));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(u.date_text("planned_start_date"), "2031-03-02 12:00");
}

#[test]
fn the_calendar_refuses_a_skipped_time() {
    let mut u = ui_with("Berlin", FakeBackend::default());
    u.s.focus_key("planned_start_date");
    u.key("f2");
    let p = u.s.date_picker().expect("picker");
    p.grid.cursor = jiff::civil::date(2031, 3, 30);
    p.time.set_value("02:30");
    u.key("tab"); // to the time
    u.key("enter");
    assert_eq!(u.s.layer(), Some("calendar"));
    let p = u.s.date_picker().expect("picker");
    assert!(
        p.preview.contains("does not exist in Europe/Berlin"),
        "{}",
        p.preview
    );
    u.key("escape");
    assert_eq!(u.s.layer(), None);
    assert_eq!(u.s.session.view("planned_start_date").value, Value::Null);
}

#[test]
fn save_path_checks_and_errors_stay_in_the_dialog() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut u = ui_with(
        "Tokyo",
        FakeBackend {
            refuse_save: true,
            ..FakeBackend::default()
        },
    );
    u.key("f4");
    assert_eq!(u.s.layer(), Some("save"));
    let check = |u: &mut Ui, path: &str| -> String {
        u.s.save_dialog().expect("dialog").input.set_value(path);
        u.key("enter");
        u.s.save_dialog().expect("still open").note.clone()
    };
    assert_eq!(check(&mut u, "  "), "type a file path");
    let d = dir.path().display().to_string();
    assert_eq!(
        check(&mut u, &d),
        format!("{d} is a folder: add a file name")
    );
    assert_eq!(
        check(&mut u, &format!("{d}/nope/x.yaml")),
        format!("cannot save: the folder {d}/nope does not exist")
    );
    std::fs::write(dir.path().join("old.yaml"), "x").expect("write");
    let old = format!("{d}/old.yaml");
    assert_eq!(
        check(&mut u, &old),
        format!("{old} exists: press Save again to overwrite")
    );
    assert_eq!(
        check(&mut u, &old),
        format!("cannot save to {old}: Permission denied")
    );
    assert!(u.draw(120, 40).contains("Permission denied"));
    u.key("escape");
    assert_eq!(u.s.layer(), None);
    assert_eq!(u.s.done(), None);
}

#[test]
fn save_draft_finishes_with_the_path() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut u = ui();
    u.s.focus_key("subject");
    u.typ("Billing: rotate keys");
    u.key("f4");
    let path = dir.path().join("d.yaml");
    u.s.save_dialog()
        .expect("dialog")
        .input
        .set_value(&path.to_string_lossy());
    u.key("enter");
    assert_eq!(u.s.done(), Some(Action::SaveDraft));
    assert!(path.exists());
    assert_eq!(u.backend.saved.lock().expect("lock").len(), 1);
    let r = u.s.into_result();
    assert_eq!(r.saved_path.as_deref(), Some(path.as_path()));
    assert_eq!(
        r.session.draft.answers.get("subject").map(|v| v.py_str()),
        Some("Billing: rotate keys".into())
    );
}

#[test]
fn escape_closes_the_list_then_cancels_an_untouched_form() {
    let mut u = ui();
    assert!(
        u.s.editor_mut("requester")
            .and_then(|e| e.as_combo())
            .expect("combo")
            .is_open
    );
    u.key("escape");
    assert!(
        !u.s.editor_mut("requester")
            .and_then(|e| e.as_combo())
            .expect("combo")
            .is_open
    );
    assert_eq!(u.s.done(), None);
    u.key("escape");
    assert_eq!(u.s.done(), Some(Action::Cancel));
    assert!(u.s.into_result().plan.is_none());
}

#[test]
fn escape_after_typing_asks_first() {
    let mut u = ui();
    u.s.focus_key("subject");
    u.typ("hi");
    u.key("escape");
    assert_eq!(u.s.layer(), Some("confirm"));
    assert!(
        u.draw(100, 30)
            .contains("Discard this form? Nothing has been sent.")
    );
    u.key("enter"); // "Keep editing" has focus
    assert_eq!(u.s.layer(), None);
    assert_eq!(u.s.done(), None);
    u.key("escape");
    u.key("y");
    assert_eq!(u.s.done(), Some(Action::Cancel));
}

#[test]
fn text_commits_after_the_pause() {
    let mut u = ui();
    u.s.focus_key("subject");
    u.typ("Mail");
    u.wait(0.2);
    assert_eq!(u.s.session.view("subject").value, Value::Null);
    u.wait(0.2);
    assert_eq!(u.s.session.view("subject").value, Value::str("Mail"));
}

#[test]
fn rule_notes_show_and_clear() {
    let mut u = ui();
    u.s.focus_key("cloud_only");
    u.typ("No");
    u.key("enter");
    assert_eq!(u.s.session.view("cloud_only").text, "No");
    assert_eq!(
        u.s.notes,
        vec![
            "* Staging Start, Staging End now required because Cloud Only = No",
            "Stage: choices limited to Staging",
            "Stage = Staging (the only choice rule Cloud only: no allows)",
        ]
    );
    assert!(u.s.session.view("staging_start").required);
    assert_eq!(u.s.session.view("stage").text, "Staging");
    assert!(
        u.s.toast_texts()
            .iter()
            .any(|t| t.starts_with("* Staging Start"))
    );
    u.wait(8.1);
    assert!(u.s.notes.is_empty());
}

#[test]
fn review_submit_and_copy_as_command() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut u = ui();
    u.key("ctrl+s");
    assert_eq!(u.s.layer(), Some("review"));
    let screen = u.draw(120, 40);
    assert!(screen.contains("Review: New Change"));
    assert!(screen.contains("Create"));
    u.key("f6");
    let path = dir.path().join("c.yaml");
    {
        let d = u.s.save_dialog().expect("dialog");
        assert_eq!(d.title, "Save the draft for the command to");
        d.input.set_value(&path.to_string_lossy());
    }
    u.key("enter");
    let t = u.s.text_dialog().expect("command");
    assert!(
        t.text.starts_with("fsv changes create --from "),
        "{}",
        t.text
    );
    assert!(
        t.text.ends_with("--yes --expect-plan 0123456789abcdef"),
        "{}",
        t.text
    );
    u.key("enter"); // Copy
    assert!(
        u.s.take_clipboard()
            .is_some_and(|c| c.starts_with("fsv changes create"))
    );
    u.key("escape");
    assert_eq!(u.s.layer(), Some("review"));
    u.key("ctrl+s");
    assert_eq!(u.s.done(), Some(Action::Submit));
    let r = u.s.into_result();
    assert_eq!(r.plan.expect("plan").hash, "0123456789abcdef0123456789");
}

#[test]
fn narrow_and_tiny_terminals_draw() {
    let mut u = ui();
    let narrow = u.draw(80, 30);
    assert!(narrow.contains("Requester"));
    assert!(u.draw(20, 5).contains("terminal too small"));
    u.draw(1, 1);
    u.draw(40, 12);
    u.s.focus_key("planned_start_date");
    u.key("f2");
    u.draw(40, 12);
    u.key("escape");
    u.key("f4");
    u.draw(40, 12);
    u.key("escape");
    u.s.focus_key("subject");
    u.typ("x");
    u.key("escape");
    assert_eq!(u.s.layer(), Some("confirm"));
    u.draw(40, 12);
    u.key("escape");
    u.key("ctrl+s");
    u.draw(40, 12);
    u.key("f6");
    u.draw(40, 12);
}

fn combo<'a>(u: &'a mut Ui, key: &str) -> &'a mut fsv_formui::widgets::ComboBox {
    u.s.editor_mut(key)
        .and_then(|e| e.as_combo())
        .expect("combo")
}

fn labels(u: &mut Ui, key: &str) -> Vec<String> {
    combo(u, key)
        .options
        .iter()
        .map(|o| o.label.clone())
        .collect()
}

fn preview(u: &mut Ui, key: &str) -> String {
    let ed = u.s.editor_mut(key).and_then(|e| e.as_date()).expect("date");
    if ed.preview.error.is_empty() {
        ed.preview.preview.clone()
    } else {
        ed.preview.error.clone()
    }
}

/// `test_type_to_filter_picks_a_choice`.
#[test]
fn type_to_filter_picks_a_choice() {
    let mut u = ui();
    u.s.focus_key("change_type");
    assert!(combo(&mut u, "change_type").is_open);
    assert_eq!(
        labels(&mut u, "change_type"),
        ["Normal", "Standard", "Emergency"]
    );
    u.typ("emer");
    assert_eq!(labels(&mut u, "change_type"), ["Emergency"]);
    u.key("enter");
    let c = combo(&mut u, "change_type");
    assert!(!c.is_open);
    assert_eq!(c.input.value(), "Emergency");
    assert_eq!(u.s.session.view("change_type").text, "Emergency");
}

/// `test_arrows_and_multi_select_checklist`.
#[test]
fn multi_select_toggles_and_stays_open() {
    let mut u = ui();
    u.s.focus_key("services");
    u.typ("s");
    u.key("enter");
    u.typ("bill");
    u.key("enter");
    assert!(combo(&mut u, "services").is_open);
    assert!(u.draw(120, 60).contains("✓ Search, Billing"));
    u.typ("bill");
    u.key("enter");
    assert_eq!(u.s.session.view("services").text, "Search");
    assert_eq!(
        u.s.session.view("services").value,
        Value::List(vec![Value::str("Search")])
    );
}

/// `test_answer_reveals_fields_live_with_a_note`.
#[test]
fn an_answer_reveals_fields_live_with_a_note() {
    let mut u = ui();
    u.s.focus_key("cloud_only");
    u.typ("yes");
    u.key("enter");
    assert!(!u.s.session.view("staging_start").visible);
    assert!(!u.s.session.view("staging_end").visible);
    for _ in 0..3 {
        u.key("backspace");
    }
    u.typ("no");
    u.key("enter");
    let v = u.s.session.view("staging_start");
    assert!(v.visible && v.required);
    assert!(u.draw(120, 60).contains("Staging Start *"));
    assert!(u.s.notes.iter().any(|n| n
        == "+ Staging Start (required), Staging End (required) appeared because Cloud Only = No"));
}

/// `test_restricted_dropdown_only_offers_the_allowed_choice`.
#[test]
fn a_restricted_dropdown_only_offers_the_allowed_choice() {
    let mut u = ui();
    u.s.focus_key("cloud_only");
    u.typ("no");
    u.key("enter");
    u.s.focus_key("stage");
    u.typ("l");
    assert!(labels(&mut u, "stage").is_empty());
    assert!(combo(&mut u, "stage").note.contains("no choice matches"));
    assert!(
        u.s.session
            .view("stage")
            .note
            .contains("choices limited by rule Cloud only: no")
    );
    let help = u.s.help_text();
    assert!(help.contains("Choices") && help.contains("Staging") && !help.contains("Live"));
    assert_eq!(u.s.session.view("stage").value, Value::str("Staging"));
}

/// `test_typed_date_previews_and_commits`.
#[test]
fn a_typed_date_previews_and_commits() {
    let mut u = ui();
    u.s.focus_key("planned_start_date");
    u.typ("2030-11-26 19:00");
    assert_eq!(
        preview(&mut u, "planned_start_date"),
        "Tue 26 Nov 2030 19:00 (+09:00 from your Freshservice profile tz)"
    );
    u.key("enter");
    u.s.focus_key("planned_end_date");
    u.typ("2030-11-26");
    u.key("tab");
    assert!(preview(&mut u, "planned_end_date").contains("add a time"));
    match u.s.session.view("planned_start_date").value {
        Value::DateTime(d) => {
            assert_eq!(d.local, jiff::civil::date(2030, 11, 26).at(19, 0, 0, 0));
            assert_eq!(d.offset, jiff::tz::Offset::constant(9));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(u.s.session.view("planned_end_date").value, Value::Null);
}

/// `test_calendar_picks_day_and_time`.
#[test]
fn the_calendar_picks_a_day_and_a_time() {
    let mut u = ui();
    u.s.focus_key("planned_start_date");
    u.key("f2");
    assert_eq!(u.s.layer(), Some("calendar"));
    let today = u.s.date_picker().expect("picker").grid.today;
    u.key("right");
    u.key("enter");
    for _ in 0..5 {
        u.key("backspace");
    }
    u.typ("19:00");
    u.key("enter");
    assert_eq!(u.s.layer(), None);
    match u.s.session.view("planned_start_date").value {
        Value::DateTime(d) => {
            assert_eq!(d.local, today.tomorrow().unwrap().at(19, 0, 0, 0));
            assert_eq!(d.offset, jiff::tz::Offset::constant(9));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(u.s.focused_key(), Some("planned_start_date"));
}

/// The time suggestions, OK and Cancel of `DatePickerScreen`.
#[test]
fn the_calendar_time_suggestions_ok_and_cancel() {
    let mut u = ui();
    u.s.focus_key("planned_start_date");
    u.key("f2");
    u.key("tab"); // the time
    u.key("tab"); // the suggestions
    u.key("down");
    u.key("enter");
    {
        let p = u.s.date_picker().expect("picker");
        assert_eq!(p.time.value(), "10:00");
        assert_eq!(p.focus, fsv_formui::screen::dialogs::PickerFocus::Ok);
        assert!(p.preview.contains("10:00"), "{}", p.preview);
    }
    u.key("enter"); // OK
    assert_eq!(u.s.layer(), None);
    let picked = u.date_text("planned_start_date");
    assert!(picked.ends_with(" 10:00"), "{picked}");
    u.key("f2");
    for _ in 0..4 {
        u.key("tab"); // time, suggestions, OK, Cancel
    }
    u.key("enter");
    assert_eq!(u.s.layer(), None);
    assert_eq!(u.date_text("planned_start_date"), picked);
}

/// `test_required_fields_block_submit`.
#[test]
fn required_fields_block_submit() {
    let mut u = ui();
    assert!(
        u.s.status
            .contains("required field(s) missing: Requester, Subject")
    );
    // the Acme plan builder reports the missing fields as errors
    u.backend
        .errors
        .lock()
        .expect("lock")
        .push("Requester is required".into());
    u.s.focus_key("subject");
    u.key("ctrl+s");
    assert_eq!(u.s.layer(), Some("review"));
    assert!(u.s.review().expect("review").submit_disabled());
    u.key("escape");
    assert_eq!(u.s.layer(), None);
    assert_eq!(u.s.focused_key(), Some("requester"));
    assert_eq!(u.s.session.view("subject").error, "required");
}

/// The Review & submit / Save draft / Cancel buttons and F1.
#[test]
fn form_buttons_and_the_help_toggle() {
    use fsv_formui::screen::{Focus, FormButton};
    let mut u = ui();
    let help = "Type to search · ↑↓ move · Enter picks";
    assert!(u.draw(200, 40).contains(help));
    u.key("f1");
    assert!(u.s.hide_help);
    assert!(!u.draw(200, 40).contains(help));
    u.s.action_toggle_help();
    assert!(u.draw(200, 40).contains(help));

    u.s.set_focus(Some(Focus::Button(FormButton::Save)));
    u.key("enter");
    assert_eq!(u.s.layer(), Some("save"));
    assert_eq!(u.s.save_dialog().expect("dialog").title, "Save draft to");
    u.key("tab");
    u.key("tab"); // Cancel
    u.key("enter");
    assert_eq!(u.s.layer(), None);

    u.s.set_focus(Some(Focus::Button(FormButton::Review)));
    u.key("enter");
    assert_eq!(u.s.layer(), Some("review"));
    u.key("escape");

    u.s.set_focus(Some(Focus::Button(FormButton::Cancel)));
    u.key(" ");
    assert_eq!(u.s.done(), Some(Action::Cancel));
}

/// `ConfirmDialog`: arrows move between the buttons; Discard finishes.
#[test]
fn the_discard_question_takes_arrows() {
    let mut u = ui();
    u.s.focus_key("subject");
    u.typ("hi");
    u.key("escape");
    assert_eq!(u.s.layer(), Some("confirm"));
    u.key("left");
    u.key("enter");
    assert_eq!(u.s.done(), Some(Action::Cancel));
}

/// F7 payload, refused Submit / Copy as command, F4 and Cancel from the review.
#[test]
fn review_keys_and_buttons() {
    let mut u = ui();
    u.s.focus_key("subject");
    u.typ("Billing: rotate keys");
    u.key("ctrl+s");
    u.key("f7");
    assert!(u.s.review().expect("review").show_payload);
    let screen = u.draw(120, 60);
    assert!(screen.contains("Payload"), "{screen}");
    assert!(screen.contains("\"subject\": \"Billing: rotate keys\""));
    u.key("f7");
    assert!(!u.s.review().expect("review").show_payload);
    assert!(!u.draw(120, 60).contains("Payload"));

    // Tab to Cancel: Submit → Back → Save → Copy as command → Cancel
    for _ in 0..4 {
        u.key("tab");
    }
    assert_eq!(
        u.s.review().expect("review").focus,
        fsv_formui::screen::review::ReviewFocus::Cancel
    );
    u.key("enter");
    assert_eq!(u.s.done(), Some(Action::Cancel));

    let mut u = ui();
    u.backend
        .errors
        .lock()
        .expect("lock")
        .push("Subject is required".into());
    u.key("ctrl+s");
    u.key("ctrl+s");
    u.key("f6");
    let toasts = u.s.toast_texts();
    assert!(
        toasts
            .iter()
            .any(|t| t == "fix the problems first (Back to edit)"),
        "{toasts:?}"
    );
    assert!(
        toasts.iter().any(|t| t == "no complete plan yet"),
        "{toasts:?}"
    );
    assert_eq!(u.s.done(), None);

    let dir = tempfile::tempdir().expect("tmp");
    let mut u = ui();
    u.key("ctrl+s");
    u.key("f4");
    assert_eq!(u.s.save_dialog().expect("dialog").title, "Save draft to");
    let path = dir.path().join("r.yaml");
    u.s.save_dialog()
        .expect("dialog")
        .input
        .set_value(&path.to_string_lossy());
    u.key("enter");
    assert_eq!(u.s.done(), Some(Action::SaveDraft));
    assert_eq!(
        u.s.into_result().saved_path.as_deref(),
        Some(path.as_path())
    );
}

/// `TextDialog` Close goes back to the review.
#[test]
fn the_command_dialog_closes_back_to_the_review() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut u = ui();
    u.key("ctrl+s");
    u.key("f6");
    let path = dir.path().join("c.yaml");
    u.s.save_dialog()
        .expect("dialog")
        .input
        .set_value(&path.to_string_lossy());
    u.key("enter");
    assert!(u.s.text_dialog().is_some());
    u.key("tab"); // Close
    u.key("enter");
    assert_eq!(u.s.layer(), Some("review"));
    assert!(u.s.take_clipboard().is_none());
}

/// A backend whose plan has one blocking rule message.
struct Blocking;

impl fsv_formui::FormBackend for Blocking {
    fn build_plan(&self, req: &fsv_formui::PlanRequest<'_>) -> Result<fsv_formui::Plan, String> {
        let mut review = fsv_core::review::Review::new("New Change");
        review.messages.push(fsv_core::review::ReviewMessage {
            text: "Freeze window".into(),
            blocking: true,
            acknowledged: req.ack_all,
            rule: serde_json::Value::Null,
        });
        review.plan_hash = "0123456789abcdef".into();
        Ok(fsv_formui::Plan {
            errors: vec![],
            hash: "0123456789abcdef".into(),
            review,
            payload: None,
            detail: None,
        })
    }

    fn save_draft(
        &self,
        _req: &fsv_formui::PlanRequest<'_>,
        _plan: &fsv_formui::Plan,
        _path: &std::path::Path,
    ) -> std::io::Result<()> {
        Ok(())
    }

    fn send(
        &self,
        _req: &fsv_formui::PlanRequest<'_>,
        _plan: &fsv_formui::Plan,
        _allow: bool,
    ) -> Result<serde_json::Value, fsv_core::Error> {
        Ok(serde_json::Value::Null)
    }
}

/// `ReviewScreen._ack`: ticking the box rebuilds the plan and enables Submit.
#[test]
fn acknowledging_a_blocking_message_enables_submit() {
    let api = Arc::new(FakeApi::default());
    let mut i = init(
        FormKind::Change,
        FormMode::New,
        api,
        Arc::new(FakeBackend::default()),
    );
    i.backend = Arc::new(Blocking);
    let session = FormSession::new(i).expect("session");
    let mut s = FormScreen::with_workers(session, Theme::textual_dark(), WorkersMode::Inline);
    s.handle_key(parse_key("ctrl+s"));
    let r = s.review().expect("review");
    assert!(r.has_ack && r.submit_disabled());
    assert_eq!(
        r.banner(&s.session),
        ["A blocking rule message needs your acknowledgement (tick the box below)."]
    );
    s.handle_key(parse_key("shift+tab")); // Back → the box
    s.handle_key(parse_key("space"));
    assert!(s.session.ack);
    let r = s.review().expect("review");
    assert!(!r.submit_disabled());
    assert!(r.banner(&s.session).is_empty());
    s.handle_key(parse_key("ctrl+s"));
    assert_eq!(s.done(), Some(Action::Submit));
}

/// `FormScreen.show_help` / `show_notes` and the actions behind the keys.
#[test]
fn form_screen_help_notes_and_actions() {
    let mut u = ui();
    u.s.show_help("subject", false);
    assert!(
        u.s.help_text()
            .starts_with("Subject * required\ne.g. Service: objective"),
        "{}",
        u.s.help_text()
    );
    let notes: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
    u.s.show_notes(&notes);
    assert_eq!(u.s.notes, ["b", "c", "d", "e"]);
    assert!(u.s.toast_texts().iter().any(|t| t == "a\nb\nc\nd\ne"));
    u.s.show_notes(&[]);
    assert_eq!(u.s.notes.len(), 4);

    u.s.action_save();
    assert_eq!(u.s.layer(), Some("save"));
    u.key("escape");
    u.s.focus_key("subject");
    u.typ("x");
    u.s.action_review();
    assert_eq!(u.s.layer(), Some("review"));
    assert_eq!(
        u.s.session.view("subject").value,
        Value::str("x"),
        "committed first"
    );
    u.key("escape");
    u.s.action_cancel();
    assert_eq!(u.s.layer(), Some("confirm"));
}

/// Textual `Input` selects its text on focus: typing replaces it.
#[test]
fn focusing_a_field_selects_its_text() {
    let mut u = ui();
    u.s.session.set("subject", Value::str("Hello"));
    u.s.refresh_rows();
    u.s.focus_key("subject");
    u.typ("x");
    u.key("enter");
    assert_eq!(u.s.session.view("subject").value, Value::str("x"));
}
