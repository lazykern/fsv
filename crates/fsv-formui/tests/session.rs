//! `FormSession` on the Acme change form (ports of
//! `test_formui_units.py` session tests, with strings from the Python run).

mod common;

use std::sync::Arc;

use common::{FakeApi, FakeBackend, acme, init, jane};
use fsv_forms::{LookupValue, Value};
use fsv_formui::session::{
    ATTACHMENTS, FormSession, REQUESTER, default_requester, rules_warnings, shell_quote,
};
use fsv_formui::{FormKind, FormMode, WidgetKind};
use serde_json::json;

#[test]
fn items_follow_web_order_and_skip_status_on_create() {
    let (_a, s) = acme();
    let keys: Vec<(&str, WidgetKind)> =
        s.items.iter().map(|i| (i.key.as_str(), i.widget)).collect();
    assert_eq!(keys[0], ("requester", WidgetKind::Lookup));
    assert!(keys.contains(&("services", WidgetKind::Multi)));
    assert!(keys.contains(&("legacy_flag", WidgetKind::Checkbox)));
    assert!(keys.contains(&("description", WidgetKind::Richtext)));
    assert!(keys.contains(&("planned_start_date", WidgetKind::Datetime)));
    assert!(!keys.iter().any(|(k, _)| *k == "status"));
    let n = keys.len();
    assert_eq!(keys[n - 2], (ATTACHMENTS, WidgetKind::File));
    assert_eq!(
        keys[n - 1],
        ("planning:Go-Live Checklist", WidgetKind::File)
    );
    assert_eq!(s.tz_source, "Freshservice user time zone (Tokyo)");
}

#[test]
fn answer_reveals_and_notes() {
    let (_a, mut s) = acme();
    let yes = s.set("cloud_only", Value::str("Yes"));
    assert_eq!(
        yes.notes,
        vec![
            "- Staging Start, Staging End hidden because Cloud Only = Yes",
            "Stage: choices limited to Live",
            "Stage = Live (the only choice rule Cloud only: yes allows)",
        ]
    );
    assert!(!s.view("staging_start").visible);
    let no = s.set("cloud_only", Value::str("No"));
    assert!(
        no.notes.contains(
            &"+ Staging Start (required), Staging End (required) appeared because Cloud Only = No"
                .to_string()
        )
    );
    let v = s.view("staging_start");
    assert!(v.visible && v.required);
}

#[test]
fn restricted_dropdown_offers_only_allowed_and_preselects() {
    let (_a, mut s) = acme();
    let out = s.set("cloud_only", Value::str("Yes"));
    let (sugg, _) = s.suggestions("stage", "").expect("local");
    assert_eq!(
        sugg.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(),
        vec!["Live"]
    );
    assert!(sugg[0].selected);
    assert_eq!(
        s.state.evaluated.values.get("stage"),
        Some(&Value::str("Live"))
    );
    assert!(
        out.notes
            .iter()
            .any(|n| n == "Stage = Live (the only choice rule Cloud only: yes allows)")
    );
    assert!(
        s.view("stage")
            .note
            .contains("choices limited by rule Cloud only: yes")
    );
    assert!(
        s.set("stage", Value::str("Staging"))
            .error
            .contains("is not offered now")
    );
}

#[test]
fn effects_explain_what_each_choice_does() {
    let (_a, s) = acme();
    let eff = s.effects("cloud_only");
    assert!(
        eff.iter()
            .any(|e| e.starts_with("Picking No will") && e.contains("limit Stage to Staging"))
    );
    assert!(
        eff.iter()
            .any(|e| e.starts_with("Picking Yes will hide Staging Start, Staging End"))
    );
    assert_eq!(
        s.effects("priority"),
        vec![
            "Picking Urgent will show the message \"Urgent changes need the duty lead's approval\""
        ]
    );
    assert_eq!(s.effects("cloud_only"), eff, "cached answer is the same");
}

#[test]
fn problems_list_required_and_requester() {
    let (_a, mut s) = acme();
    assert_eq!(
        s.problems(),
        vec![
            "Requester is required",
            "Subject is required",
            "Cloud Only is required",
            "Description is required",
            "Planned Start Date is required",
            "Planned End Date is required",
        ]
    );
    s.set("requester", Value::Lookup(jane()));
    assert!(!s.problems().contains(&"Requester is required".to_string()));
    assert_eq!(s.view("subject").error, "");
    s.show_all_errors = true;
    assert_eq!(s.view("subject").error, "required");
    assert_eq!(s.draft.requester.as_deref(), Some("jane.doe@example.com"));
}

#[test]
fn bad_typed_value_is_an_inline_error() {
    let (_a, mut s) = acme();
    let out = s.set(
        "services",
        Value::list([Value::str("Billing"), Value::str("Nope")]),
    );
    assert!(out.error.contains("not a valid choice"));
    assert!(s.view("services").error.contains("not a valid choice"));
    assert!(s.problems().iter().any(|p| p.starts_with("Services:")));
    s.set("services", Value::list([Value::str("Billing")]));
    assert_eq!(s.view("services").error, "");
    let (sugg, _) = s.suggestions("services", "bil").expect("local");
    assert!(sugg[0].selected && sugg[0].label == "Billing");
}

#[test]
fn blocking_message_note_and_help() {
    let (_a, mut s) = acme();
    let out = s.set("priority", Value::str("Urgent"));
    assert_eq!(
        out.notes,
        vec![
            "! Urgent changes need the duty lead's approval (blocking: acknowledge it on the review)"
        ]
    );
    let h = s.help_for("priority");
    assert_eq!(h.choices[0], ("Low".to_string(), "Can wait".to_string()));
    assert!(h.required);
    let h = s.help_for("planned_start_date");
    assert_eq!(
        h.why,
        vec!["time zone: Freshservice user time zone (Tokyo)"]
    );
}

#[test]
fn requester_must_be_picked_and_people_search_is_remote() {
    let (a, mut s) = acme();
    assert_eq!(
        s.set("requester", Value::str("jane")).error,
        "pick a person from the list"
    );
    assert_eq!(s.view("requester").error, "pick a person from the list");
    let (none, note) = s.suggestions("requester", "  ").expect("blank");
    assert!(none.is_empty() && note == "type part of a name or email");
    assert!(a.api.calls().is_empty(), "a blank query makes no call");
    let (rows, _) = s.suggestions("requester", "jane").expect("rows");
    assert_eq!(rows[0].label, "Jane Doe <jane.doe@example.com>");
    assert_eq!(a.api.calls(), vec!["AUTO requesters jane"]);
    let (groups, _) = s.suggestions("group", "").expect("groups");
    assert_eq!(groups[0].label, "Platform Ops");
    s.suggestions("group", "plat").expect("groups");
    assert_eq!(
        a.api
            .calls()
            .iter()
            .filter(|c| c.contains("bootstrap"))
            .count(),
        1
    );
}

#[test]
fn search_unavailable_offers_the_typed_address() {
    let api = Arc::new(FakeApi {
        deny_people: true,
        ..FakeApi::default()
    });
    let s = FormSession::new(init(
        FormKind::Sr,
        FormMode::New,
        api,
        Arc::new(FakeBackend::default()),
    ))
    .expect("session");
    assert_eq!(s.items[0].key, REQUESTER);
    let (rows, note) = s
        .suggestions(REQUESTER, "jörg@example.com")
        .expect("fallback");
    assert_eq!(rows[0].label, "use jörg@example.com");
    assert_eq!(rows[0].detail, "typed address (search unavailable)");
    assert_eq!(
        note,
        "requester search is not available for your role (HTTP 403); type the exact email"
    );
}

#[test]
fn sr_requester_needs_an_email() {
    let api = Arc::new(FakeApi::default());
    let mut s = FormSession::new(init(
        FormKind::Sr,
        FormMode::New,
        api,
        Arc::new(FakeBackend::default()),
    ))
    .expect("session");
    let out = s.set(
        REQUESTER,
        Value::Lookup(LookupValue::new(1, "No Mail", None)),
    );
    assert_eq!(out.error, "No Mail has no email address");
    assert!(s.requester.is_none());
}

#[test]
fn invalid_input_blocks_the_plan_and_clears_the_hash() {
    let (_a, mut s) = acme();
    s.set("subject", Value::str("Billing: rotate"));
    let ok = s.build_plan(None).expect("plan");
    assert!(ok.ok() && !ok.hash.is_empty());
    s.mark_invalid("planned_start_date", "'nonsense' is not a date");
    let blocked = s.build_plan(None).expect("plan");
    assert!(!blocked.ok() && blocked.hash.is_empty() && blocked.review.plan_hash.is_empty());
    assert_eq!(
        blocked.errors,
        vec!["Planned Start Date: 'nonsense' is not a date"]
    );
    assert_eq!(blocked.review.errors, blocked.errors);
    s.mark_invalid("planned_start_date", "");
    assert!(s.input_errors().is_empty());
    assert!(s.build_plan(None).expect("plan").ok());
}

#[test]
fn hidden_fields_do_not_block_with_their_input_errors() {
    let (_a, mut s) = acme();
    s.set("cloud_only", Value::str("No"));
    s.mark_invalid("staging_start", "bad date");
    assert_eq!(s.input_errors(), vec!["Staging Start: bad date"]);
    s.set("cloud_only", Value::str("Yes"));
    assert!(s.input_errors().is_empty());
}

#[test]
fn description_markdown_becomes_html() {
    let (_a, mut s) = acme();
    s.set("description", Value::str("**hi**"));
    assert_eq!(
        s.draft.description_html.as_deref(),
        Some("<p><strong>hi</strong></p>")
    );
    let it = s.item("description").expect("item").clone();
    assert_eq!(s.text_for(&it, &Value::str("<p>x</p>")), "**hi**");
}

#[test]
fn edit_mode_records_cleared_answers_and_keeps_description() {
    let api = Arc::new(FakeApi::default());
    let mut i = init(
        FormKind::Change,
        FormMode::Edit,
        api,
        Arc::new(FakeBackend::default()),
    );
    i.existing
        .insert("subject".into(), Value::str("Billing: rotate keys"));
    i.existing
        .insert("description".into(), Value::str("<p>Rotate keys.</p>"));
    i.target = Some(json!(501));
    let mut s = FormSession::new(i).expect("session");
    assert!(
        s.items.iter().any(|it| it.key == "status"),
        "status is editable on edit"
    );
    s.set("subject", Value::Null);
    assert_eq!(s.draft.answers.get("subject"), Some(&Value::Null));
    s.set("description", Value::str("Rotate keys."));
    assert_eq!(
        s.draft.description_html, None,
        "unchanged text keeps the record's HTML"
    );
    s.set("description", Value::str(""));
    assert_eq!(s.draft.description_html.as_deref(), Some(""));
    assert!(s.default_draft_path().ends_with("change-501-draft.yaml"));
    let plan = fsv_formui::Plan {
        hash: "0123456789abcdef0123".into(),
        ..Default::default()
    };
    assert_eq!(
        s.command_for("my draft.yaml", &plan),
        "fsv changes update 501 --from 'my draft.yaml' --yes --expect-plan 0123456789abcdef"
    );
}

#[test]
fn files_are_checked_before_they_are_kept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let f = dir.path().join("plan.pdf");
    std::fs::write(&f, b"x").expect("write");
    let (_a, mut s) = acme();
    let out = s.set(
        ATTACHMENTS,
        Value::str(dir.path().join("nope.pdf").to_string_lossy().into_owned()),
    );
    assert_eq!(out.error, "nope.pdf: file not found");
    assert!(
        s.problems()
            .contains(&"Attachments: nope.pdf: file not found".to_string())
    );
    s.set(ATTACHMENTS, Value::str(f.to_string_lossy().into_owned()));
    assert_eq!(s.draft.attachments, vec![f.to_string_lossy().into_owned()]);
    s.set(
        "planning:Go-Live Checklist",
        Value::str(f.to_string_lossy().into_owned()),
    );
    assert_eq!(s.draft.planning["Go-Live Checklist"].files.len(), 1);
    s.set("planning:Go-Live Checklist", Value::str(""));
    assert!(s.draft.planning.is_empty());
}

#[test]
fn dry_run_never_sends() {
    let api = Arc::new(FakeApi::default());
    let backend = Arc::new(FakeBackend::default());
    let mut i = init(FormKind::Change, FormMode::New, api, backend.clone());
    i.dry_run = true;
    let mut s = FormSession::new(i).expect("session");
    let plan = s.build_plan(None).expect("plan");
    let e = s.send(&plan, false).expect_err("dry run");
    assert_eq!(e.exit_code(), 2);
    assert_eq!(e.to_string(), "dry run: nothing is sent");
    assert!(backend.sent.lock().expect("lock").is_empty());
}

#[test]
fn profile_time_zone_is_used_without_rules_user() {
    let api = Arc::new(FakeApi {
        profile_tz: Some("Bangkok".into()),
        ..FakeApi::default()
    });
    let mut i = init(
        FormKind::Change,
        FormMode::New,
        api.clone(),
        Arc::new(FakeBackend::default()),
    );
    let spec = i.spec.clone();
    i.rules = Some(common::engine(&spec, &common::rules(None)));
    let s = FormSession::new(i).expect("session");
    assert_eq!(s.tz_source, "Freshservice user time zone (Bangkok)");
    assert_eq!(api.calls(), vec!["ME"]);
}

#[test]
fn load_notes_put_the_requester_note_first() {
    let api = Arc::new(FakeApi::default());
    let mut i = init(
        FormKind::Change,
        FormMode::New,
        api,
        Arc::new(FakeBackend::default()),
    );
    i.requester = Some(jane());
    i.requester_note = "Requester jane.doe@example.com from config defaults.requester".into();
    i.rules_warnings = rules_warnings(Some(&json!({"unavailable": "HTTP 404"})));
    let s = FormSession::new(i).expect("session");
    assert_eq!(
        s.load_notes[0],
        "Requester jane.doe@example.com from config defaults.requester"
    );
    assert!(s.load_notes[1].starts_with("business rules could not be loaded (HTTP 404); only"));
    assert_eq!(s.view("requester").text, "Jane Doe <jane.doe@example.com>");
}

#[test]
fn default_requester_from_config() {
    let cfg = json!({"defaults": {"requester": " jane.doe@example.com "}});
    let cfg = cfg.as_object().expect("object");
    let (who, note) = default_requester(cfg, "requester", |_| Ok(jane()));
    assert_eq!(who, Some(jane()));
    assert_eq!(
        note,
        "Requester jane.doe@example.com from config defaults.requester"
    );
    let (who, note) = default_requester(cfg, "requester", |_| Err("no person\n matches".into()));
    assert!(who.is_none());
    assert_eq!(
        note,
        "config defaults.requester = 'jane.doe@example.com' not used: no person matches"
    );
    assert_eq!(
        default_requester(cfg, "sr_requester", |_| Ok(jane())),
        (None, String::new())
    );
}

#[test]
fn shell_quote_matches_shlex() {
    assert_eq!(shell_quote("draft.yaml"), "draft.yaml");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_quote("it's"), "'it'\"'\"'s'");
}

#[test]
fn kind_aliases() {
    assert_eq!(FormKind::parse("incident"), Ok(FormKind::Ticket));
    assert_eq!(FormKind::parse("service_request"), Ok(FormKind::Sr));
    assert_eq!(
        FormKind::parse("task"),
        Err("unknown form kind 'task'; expected change, ticket or sr".to_string())
    );
}
