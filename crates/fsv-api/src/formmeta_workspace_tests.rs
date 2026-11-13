//! Per-record workspace and the default-form assumption
//! (`tests/test_shifted_tenant.py`, fb2b13c).

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;
use crate::service::testing::FnApi;

const PROBLEM_STATUSES: &str = r#"[{"id": 1, "value": "Neu"}, {"id": 21, "value": "Prüfung"},
    {"id": 22, "value": "Wip"}, {"id": 3, "value": "Erledigt"}]"#;

fn problem_fields(workspace: i64) -> Vec<Value> {
    let extra = if workspace == 5 {
        "ws5_only"
    } else {
        "ws9_only"
    };
    let statuses: Value = serde_json::from_str(PROBLEM_STATUSES).unwrap();
    vec![
        json!({"id": 601, "name": "subject", "label": "Subject", "field_type": "default_subject",
               "position": 1, "workspace_id": workspace}),
        json!({"id": 602, "name": "status", "label": "Status", "field_type": "default_status",
               "position": 2, "choices": statuses, "workspace_id": workspace}),
        json!({"id": 603, "name": extra, "label": extra, "field_type": "custom_text", "position": 3,
               "workspace_id": workspace, "default_field": false}),
    ]
}

#[derive(Default)]
struct Shifted {
    row: Value,
    rule_paths: Vec<String>,
}

fn shifted() -> (FnApi, Arc<Mutex<Shifted>>) {
    let state = Arc::new(Mutex::new(Shifted {
        row: json!({"id": 77, "subject": "Globex problem", "status": 21,
                    "planned_start_date": null, "planned_end_date": null}),
        ..Shifted::default()
    }));
    let s = Arc::clone(&state);
    let api = FnApi::new(move |call| {
        let mut s = s.lock().unwrap();
        let path = call.path.as_str();
        assert_eq!(call.method, "GET", "{path}");
        if path == "problem_form_fields" {
            return Ok(json!({"problem_fields": problem_fields(5)}));
        }
        if path.ends_with("business_rules") {
            s.rule_paths.push(path.to_string());
            return Ok(json!({"business_rules": []}));
        }
        if path == "problems/77" {
            return Ok(json!({"problem": s.row.clone()}));
        }
        panic!("{path}");
    });
    (api, state)
}

/// `test_record_workspace_beats_the_global_override_and_is_reviewed`.
#[test]
fn record_workspace_beats_the_global_override_and_is_reviewed() {
    let (c, s) = shifted();
    let store = MemStore {
        workspace: json!(5),
        ..MemStore::default()
    };
    s.lock().unwrap().row["workspace_id"] = json!(9);
    let plan = crate::problem_status::build_plan(&c, &store, "PRB-77", "Wip", None).unwrap();
    {
        let s = s.lock().unwrap();
        assert!(!s.rule_paths.is_empty());
        assert!(
            s.rule_paths.iter().all(|p| p.starts_with("ws/9/")),
            "{:?}",
            s.rule_paths
        );
    }
    let review = plan.review();
    let notes: Vec<String> = review
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        notes.iter().any(|n| n.contains("record is in workspace 9")),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.contains("forms.workspace_id=5 does not apply")),
        "{notes:?}"
    );
    // a record without its own workspace still follows the override
    {
        let mut s = s.lock().unwrap();
        s.row.as_object_mut().unwrap().remove("workspace_id");
        s.rule_paths.clear();
    }
    crate::problem_status::build_plan(&c, &store, "PRB-77", "Wip", None).unwrap();
    let s = s.lock().unwrap();
    assert!(!s.rule_paths.is_empty());
    assert!(
        s.rule_paths.iter().all(|p| p.starts_with("ws/5/")),
        "{:?}",
        s.rule_paths
    );
}

/// `test_form_spanning_workspaces_makes_the_default_form_explicit`.
#[test]
fn form_spanning_workspaces_makes_the_default_form_explicit() {
    let mut fields = problem_fields(5);
    let mut other = problem_fields(9)[2].clone();
    other["id"] = json!(604);
    fields.push(other);
    let meta = FormMeta::new(
        "problem",
        "edit_form",
        json!({"problem_fields": fields}),
        json!({"business_rules": []}),
        json!(5),
        0.0,
        "fixture",
        json!(77),
    )
    .unwrap();
    assert_eq!(meta.field_workspaces(), [json!(5), json!(9)]);
    assert!(
        meta.form_notes()
            .iter()
            .any(|n| n.contains("more than one form exists"))
    );
    let single = FormMeta::new(
        "problem",
        "edit_form",
        json!({"problem_fields": problem_fields(5)}),
        json!({"business_rules": []}),
        json!(5),
        0.0,
        "fixture",
        json!(77),
    )
    .unwrap()
    .with_workspaces(json!(5), Value::Null);
    assert!(single.form_notes().is_empty());
}

/// The exact note texts (`FormMeta.form_notes`).
#[test]
fn form_note_texts_match_python() {
    let mut fields = problem_fields(5);
    let mut other = problem_fields(9)[2].clone();
    other["id"] = json!(604);
    fields.push(other);
    let meta = FormMeta::new(
        "problem",
        "edit_form",
        json!({"problem_fields": fields}),
        json!({"business_rules": []}),
        json!(5),
        0.0,
        "fixture",
        json!(77),
    )
    .unwrap()
    .with_workspaces(json!(7), json!("3"));
    assert_eq!(
        meta.form_notes(),
        [
            "the problem form metadata spans workspaces 5, 9: more than one form exists and fsv \
             checks the single default problem form; verify the fields on the web form",
            "the record is in workspace 7 but the default problem form's fields belong to \
             workspace 5, 9; business rules come from workspace 7, the field list is the \
             default form's",
            "forms.workspace_id=3 does not apply to this record: it carries its own workspace 7",
        ]
    );
}

#[test]
fn record_workspace_of_matches_python() {
    for (ws, want) in [
        (json!(9), json!(9)),
        (json!("9"), json!(9)),
        (json!("09"), json!(9)),
        (json!(0), Value::Null),
        (json!(-3), Value::Null),
        (json!("٣"), Value::Null),
        (json!(""), Value::Null),
        (json!(9.0), Value::Null),
        (json!(true), Value::Null),
        (Value::Null, Value::Null),
    ] {
        assert_eq!(
            record_workspace_of(&json!({"workspace_id": ws})),
            want,
            "{ws}"
        );
    }
    assert_eq!(record_workspace_of(&json!([1])), Value::Null);
}

/// `PropertyPlan.review()` shows `form_notes`; they are not in the hash.
#[test]
fn ticket_property_form_notes_are_display_only() {
    let base = crate::properties::PropertyPlan {
        ticket_id: 123,
        tenant: "acme.example.com".into(),
        body_json: r#"{"tags": ["acme"]}"#.into(),
        before_json: r#"{"tags": []}"#.into(),
        record_hash: "abc".into(),
    };
    let plain = crate::properties::AnsweredPropertyPlan::from(base);
    assert!(!plain.review().contains_key("form_notes"));
    let noted = crate::properties::AnsweredPropertyPlan {
        form_notes_json: r#"["the record is in workspace 9"]"#.into(),
        ..plain.clone()
    };
    assert_eq!(noted.hash(), plain.hash());
    assert_eq!(
        noted.review()["form_notes"],
        json!(["the record is in workspace 9"])
    );
}
