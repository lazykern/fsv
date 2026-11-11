//! One test per `render.py` def; expected values come from running
//! `fsv.render` on the same fixtures.

use jiff::tz::{self, TimeZone};
use serde_json::{Value, json};

use fsv_core::output;
use fsv_core::resource::{CHANGES, TICKETS};

use super::*;

fn schema() -> Value {
    json!({"fields": [
        {"name": "status", "choices": [{"id": 2, "value": "Open"}, {"id": 3, "value": "Pending"}]},
        {"name": "change_type", "choices": [{"id": 1, "value": "Minor"}]},
        {"name": "cf_pick", "label": "Pick", "field_type": "custom_dropdown"},
        {"name": "cf_note", "label": "Note", "field_type": "custom_paragraph"},
        {"name": "cf_when", "label": "When", "field_type": "custom_date"},
        {"name": "cf_n", "label": "N", "field_type": "custom_number"},
        {"name": "cf_ok", "label": "OK", "field_type": "custom_checkbox"},
        {"name": "cf_x", "label": "X", "field_type": "custom_url"}
    ]})
}

fn items() -> Vec<Value> {
    vec![
        json!({"id": 9, "status": 2, "priority": 3, "subject": "Disk", "type": "Incident", "requester": {"name": "Ann"}}),
        json!({"id": 10, "status": 7, "priority": null, "subject": null, "requester": "bob"}),
    ]
}

macro_rules! compact {
    ($v:expr) => {
        serde_json::to_string(&$v).unwrap()
    };
}

#[test]
fn custom_field_category_by_field_type() {
    let s = schema();
    let got: Vec<&str> = [
        "cf_pick", "cf_note", "cf_when", "cf_n", "cf_ok", "cf_x", "missing",
    ]
    .iter()
    .map(|n| custom_field_category(n, &s))
    .collect();
    assert_eq!(
        got,
        [
            "choices", "text", "dates", "numbers", "booleans", "other", "other"
        ]
    );
}

#[test]
fn strip_internal_drops_bookkeeping_keys() {
    let got = output::strip_internal(
        &json!({"id": 1, "_resource": 1, "x": [{"_agent": 2, "y": 3}], "_group": 4}),
    );
    assert_eq!(compact!(got), r#"{"id":1,"x":[{"y":3}]}"#);
}

#[test]
fn fmt_value_lists_dicts_bools() {
    let got: Vec<String> = [
        json!(["a", {"name": "B"}]),
        json!({"k": 1, "j": null}),
        json!(true),
        json!(false),
        Value::Null,
        json!(3),
    ]
    .iter()
    .map(output::fmt_value)
    .collect();
    assert_eq!(got, ["a, B", "k=1, j=-", "Yes", "No", "-", "3"]);
}

#[test]
fn fmt_dt_blank_garbage_date_and_local_time() {
    let got: Vec<String> = [
        Value::Null,
        json!(""),
        json!("garbage"),
        json!("2030-11-26"),
    ]
    .iter()
    .map(output::fmt_dt)
    .collect();
    assert_eq!(got, ["-", "-", "garbage", "2030-11-26"]);
    let tz7 = TimeZone::fixed(tz::offset(7));
    assert_eq!(
        output::fmt_dt_in(&json!("2030-11-26T12:00:00Z"), &tz7),
        "2030-11-26 19:00"
    );
}

#[test]
fn emit_json_and_emit_delimited_text() {
    let rows = list_rows(&items(), &TICKETS, Some(&schema()));
    let mut buf = Vec::new();
    output::write_delimited(&mut buf, &rows, ',').unwrap();
    output::write_delimited(&mut buf, &[], ',').unwrap();
    let mut text = String::from_utf8(buf).unwrap();
    text += &output::json_text(&json!([{"id": 1, "_resource": 2, "x": {"_agent": "a", "y": 1}}]));
    assert_eq!(
        text,
        "id,status,status_id,priority,subject,requester,type\r\nINC-9,Open,2,High,Disk,Ann,Incident\r\nINC-10,7,7,-,,,\r\n[\n  {\n    \"id\": 1,\n    \"x\": {\n      \"y\": 1\n    }\n  }\n]\n"
    );
}

#[test]
fn strip_html_tags_and_entities() {
    assert_eq!(output::strip_html(None), "");
    assert_eq!(
        output::strip_html(Some("<p>a &amp; b</p><br>c ")),
        "a & b  c"
    );
}

#[test]
fn resolve_status_name_precedence() {
    let s = schema();
    let got: Vec<String> = [
        json!({"status_name": "Named", "status": 2}),
        json!({"change_status": {"name": "CS"}, "status": 2}),
        json!({"ticket_status": {"name": "TS"}}),
        json!({"problem_status": {"name": "PS"}}),
        json!({"status": 3}),
    ]
    .iter()
    .map(|x| resolve_status_name(x, x.get("status").unwrap_or(&Value::Null), &s))
    .collect();
    assert_eq!(got, ["Named", "CS", "TS", "PS", "Pending"]);
}

#[test]
fn list_rows_tickets_and_changes() {
    let rows = list_rows(&items(), &TICKETS, Some(&schema()));
    assert_eq!(
        compact!(rows),
        r#"[{"id":"INC-9","status":"Open","status_id":2,"priority":"High","subject":"Disk","requester":"Ann","type":"Incident"},{"id":"INC-10","status":"7","status_id":7,"priority":"-","subject":"","requester":"","type":""}]"#
    );
    let rows = list_rows(
        &[json!({"display_id": 4, "status": 2, "change_type": 1})],
        &CHANGES,
        Some(&schema()),
    );
    assert_eq!(
        compact!(rows),
        r#"[{"id":"CHN-4","status":"Open","status_id":2,"priority":"-","subject":"","requester":"","type":"Minor"}]"#
    );
}

#[test]
fn list_table_columns_and_cells() {
    let t = list_table(&items(), &TICKETS, Some(&schema()));
    let headers: Vec<&str> = t.columns.iter().map(|c| c.header.as_str()).collect();
    assert_eq!(
        headers,
        ["ID", "Status", "Type", "Pri", "Subject", "Requester"]
    );
    let styles: Vec<&str> = t.columns.iter().map(|c| c.style.as_str()).collect();
    assert_eq!(styles, ["cyan", "green", "", "", "", "dim"]);
    assert!(t.columns[0].no_wrap);
    assert_eq!(
        t.rows,
        [
            ["INC-9", "Open (2)", "Incident", "High", "Disk", "Ann"],
            ["INC-10", "7 (7)", "-", "-", "", "-"]
        ]
    );
    let t = list_table(
        &[json!({"display_id": 4, "status": 2, "change_type": 9})],
        &CHANGES,
        Some(&schema()),
    );
    assert_eq!(t.rows, [["CHN-4", "Open (2)", "9", "-", "", "-"]]);
}

#[test]
fn state_label_names_then_choices() {
    let s = schema();
    let got: Vec<String> = [
        json!({"name": "Plan"}),
        json!({"display_name": "Disp"}),
        json!({"id": 2}),
        json!({"status": 3}),
        json!(3),
        json!(99),
        json!({"id": 0, "status": 0}),
    ]
    .iter()
    .map(|v| state_label(v, Some(&s)))
    .collect();
    assert_eq!(
        got,
        ["Plan", "Disp", "Open", "Pending", "Pending", "99", "0"]
    );
}

#[test]
fn change_flow_bolds_current() {
    let s = json!({"fields": [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 3, "value": "Done"}]}]});
    assert_eq!(
        change_flow(
            &json!({"state_traversal": [1, {"id": 2, "name": "Plan [x]"}, 3], "status": 2}),
            Some(&s)
        ),
        "Open → [bold]Plan \\[x][/] → Done"
    );
    assert_eq!(
        change_flow(
            &json!({"state_traversal": [1, 3], "status": 9, "status_name": "Done"}),
            Some(&s)
        ),
        "Open → [bold]Done[/]"
    );
    assert_eq!(change_flow(&json!({}), Some(&json!({}))), "");
}

#[test]
fn party_display_name_id_fallbacks() {
    let got = [
        party_display(Some(&json!({"name": "Ann", "id": 3})), None, None),
        party_display(
            Some(&json!({"email": "a@x", "id": null})),
            Some(&json!(5)),
            None,
        ),
        party_display(Some(&json!({"value": "V"})), None, None),
        party_display(None, Some(&json!(8)), Some(&json!("Ops"))),
        party_display(None, None, Some(&json!("Ops"))),
        party_display(None, Some(&json!(0)), None),
        party_display(Some(&json!({})), None, None),
        party_display(None, Some(&json!(12)), None),
    ];
    assert_eq!(
        got,
        ["Ann (3)", "a@x (5)", "V", "Ops (8)", "Ops", "-", "-", "12"]
    );
}

#[test]
fn field_value_text_shapes() {
    let got: Vec<String> = [
        json!(["a", {"label": "L"}]),
        json!([]),
        json!({"email": "e@x"}),
        json!({"k": "ü"}),
        json!("  "),
        json!(" t "),
        json!(0),
        json!(true),
    ]
    .iter()
    .map(field_value_text)
    .collect();
    assert_eq!(
        got,
        ["a, L", "-", "e@x", "{\"k\": \"ü\"}", "-", "t", "0", "True"]
    );
}

/// `render.strip_html` is `fsv_core::output::strip_html`; expected values
/// from `/tmp/fsv_port/views_oracle.py`.
#[test]
fn strip_html_matches_python_render() {
    let fx: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/views_expected.json")).unwrap();
    for row in fx["strip_html"].as_array().unwrap() {
        assert_eq!(
            output::strip_html(row[0].as_str()),
            row[1].as_str().unwrap(),
            "{row}"
        );
    }
}

/// `_requested_item_field_defs` is `field_defs`.
#[test]
fn requested_item_field_defs_matches_python() {
    let fx: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/views_expected.json")).unwrap();
    let defs = &fx["field_defs"];
    let names: Vec<&str> = field_defs(&defs["input"])
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(json!(names), defs["names"]);
    let solo = json!({"name": "solo"});
    let names: Vec<&str> = field_defs(&solo)
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(json!(names), defs["solo"]);
}

#[test]
fn requested_item_field_defs_depth_first() {
    let f = json!({"name": "a", "nested_fields": [{"name": "b", "nested_fields": [{"name": "c"}]}, {"name": "d"}]});
    let names: Vec<&str> = field_defs(&f)
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a", "b", "c", "d"]);
}

#[test]
fn requested_item_rows_active_section_and_nested() {
    let item = json!({
        "custom_fields": {"action": "Create group", "category": "Banking", "system": "AAD (Azure AD)", "group": "rg-demo", "details": "because"},
        "item": {"custom_fields": [{"name": "action", "label": "Action", "field_type": "custom_dropdown", "sections": [
            {"name": "Create group", "fields": [
                {"name": "category", "label": "Category", "field_type": "nested_field",
                 "nested_fields": [{"name": "system", "label": "System", "nested_fields": []}]},
                {"name": "group", "label": "Group", "nested_fields": []},
                {"name": "details", "label": "Details", "nested_fields": []}]}]}]}
    });
    let want: Vec<(String, String)> = [
        ("Action", "Create group"),
        ("Category", "Banking"),
        ("System", "AAD (Azure AD)"),
        ("Group", "rg-demo"),
        ("Details", "because"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    assert_eq!(requested_item_rows(&item), want);
}

#[test]
fn requested_items_panel_text() {
    let items = [
        json!({"stage": {"name": "Requested"}, "custom_fields": {"action": "Create group", "group": "rg-demo"},
               "item": {"name": "Access Request", "description": "Need access", "custom_fields": [
                   {"name": "action", "label": "Action", "sections": [
                       {"name": "Create group", "fields": [{"name": "group", "label": "Group", "nested_fields": []}]}]}]}}),
        json!({"stage": "Done", "item": {}}),
    ];
    assert_eq!(
        blocks_text(&requested_items_panel(&items), 60),
        "\n───────────── requested_item 1  Access Request ─────────────\nstage     Requested\n\n──────────────── requested_item description ────────────────\nNeed access\n\n────────────────── requested_item fields ───────────────────\nAction  Create group\nGroup  rg-demo\n\n──────────── requested_item 2  requested item 2 ────────────\nstage     Done\n"
    );
}

#[test]
fn detail_panel_ticket_text() {
    let item = json!({
        "id": 4, "subject": "Disk full", "status": 2, "priority": 3, "impact": 1, "type": "Incident",
        "department": {"name": "IT"}, "created_at": "2026-01-02", "updated_at": "2026-01-03",
        "planned_start_date": "2026-02-01", "requester": {"name": "Ann", "id": 3},
        "responder_id": 7, "owner_name": "Bob", "group": {"name": "Ops"},
        "description": "<p>Root &amp; cause</p>",
        "custom_fields": {"cf_x": "u", "cf_pick": ["a", "b"], "cf_ok": true, "cf_n": 0, "cf_note": "long"},
        "planning_fields": {"cf_plan": {"description_text": "x".repeat(210)}, "cf_skip": {"description_text": ""}}
    });
    // Rich soft-wraps the 200-character planning line at 60 columns; the
    // block renderer leaves wrapping to the console, so compare it unwrapped.
    let want = String::from(
        "───────────────────── INC-4  Disk full ─────────────────────\nstatus    Open  (id=2)\ntype      Incident\ndepartment IT\npriority  High\nimpact    Low\nplanned   2026-02-01  →  -\ncreated   2026-01-02  updated 2026-01-03\nrequester Ann (3)  agent Bob (7)  group Ops\n\n─────────────────────── description ────────────────────────\nRoot & cause\n\n────────────────────── custom_fields ───────────────────────\n  choices\n    Pick: a, b\n\n  text\n    Note: long\n\n  booleans\n    OK: Yes\n\n  other\n    X: u\n\n───────────────────── planning_fields ──────────────────────\n  cf_plan: ",
    ) + &"x".repeat(200)
        + "\n";
    assert_eq!(
        blocks_text(&detail_panel(&item, &TICKETS, Some(&schema()), None), 60),
        want
    );
}
