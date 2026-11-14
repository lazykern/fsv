//! `tests/test_shifted_tenant.py` status-semantics call sites (Python
//! b6b741f): renamed/translated built-in statuses keep their meaning in
//! lifecycle, merge, --set-status and problem status; metadata without the
//! built-in ids is refused.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::formmeta::MemStore;
use crate::service::testing::{Call, FnApi, form_field};
use crate::status_semantics::{Role, Wanted};

fn ticket_statuses() -> Value {
    json!([{"id": 2, "value": "Offen"}, {"id": 3, "value": "Wartend"}, {"id": 12, "value": "Abgebrochen"},
           {"id": 4, "value": "Gelöst"}, {"id": 5, "value": "Erledigt"}, {"id": 13, "value": "In Arbeit"}])
}

fn unprovable_statuses() -> Value {
    json!([{"id": 10, "value": "New"}, {"id": 11, "value": "Wip"}, {"id": 12, "value": "Fixed"},
           {"id": 13, "value": "Done"}])
}

struct World {
    statuses: Value,
    row: Value,
}

fn globex(statuses: Value) -> (FnApi, Arc<Mutex<World>>) {
    let w = Arc::new(Mutex::new(World {
        statuses,
        row: json!({"id": 123, "status": 2, "group_id": null, "responder_id": 9000001234_i64,
                    "workspace_id": 9, "resolution_notes": null,
                    "custom_fields": {"summary": "Globex fix"}}),
    }));
    let state = w.clone();
    let mut c = FnApi::new(move |call: &Call| {
        let w = state.lock().unwrap();
        match call.path.as_str() {
            "ticket_form_fields" => Ok(json!({"ticket_fields": [
                form_field(9000007101, "status", "Zustand", "default_status", 1,
                           json!({"choices": w.statuses})),
                form_field(9000007102, "summary", "Lösung", "custom_text", 2,
                           json!({"required_for_closure": true})),
            ]})),
            p if p.ends_with("business_rules") => Ok(json!({"business_rules": []})),
            "tickets/123" => Ok(json!({"ticket": w.row})),
            other => panic!("unexpected {other}"),
        }
    });
    c.domain = "globex.example.com".into();
    (c, w)
}

fn store() -> MemStore {
    MemStore {
        workspace: json!(9),
        ..MemStore::default()
    }
}

fn plan(
    c: &FnApi,
    status: Wanted<'_>,
    note: Option<&str>,
) -> fsv_core::Result<crate::ticket_lifecycle::LifecyclePlan> {
    crate::ticket_lifecycle::build_plan(c, &store(), "123", status, note, None, None)
}

/// `test_renamed_builtin_ticket_statuses_keep_their_meaning`.
#[test]
fn renamed_builtin_ticket_statuses_keep_their_meaning() {
    let (c, w) = globex(ticket_statuses());
    let resolved = plan(&c, Wanted::Role(Role::Resolved), Some("Globex fix")).unwrap();
    assert_eq!(resolved.payload()["status"], 4);
    assert_eq!(resolved.status, "Gelöst");
    assert!(resolved.note_html.is_some());
    w.lock().unwrap().row["resolution_notes"] = json!({"notes_html": "<p>Globex fix</p>"});
    assert_eq!(
        plan(&c, Wanted::Role(Role::Closed), None)
            .unwrap()
            .payload()["status"],
        5
    );
    assert_eq!(
        plan(&c, Wanted::Role(Role::Open), None).unwrap().payload()["status"],
        2
    );
    // a custom status is not closing: no resolution note or closure field is demanded
    w.lock().unwrap().row["custom_fields"]["summary"] = Value::Null;
    assert_eq!(
        plan(&c, "Abgebrochen".into(), None).unwrap().payload()["status"],
        12
    );
    // the translated Closed still enforces closure fields
    let e = plan(&c, "Erledigt".into(), None).unwrap_err().to_string();
    assert!(e.contains("Lösung"), "{e}");
}

/// `test_ticket_statuses_without_builtin_ids_are_refused`.
#[test]
fn ticket_statuses_without_builtin_ids_are_refused() {
    let (c, _) = globex(unprovable_statuses());
    let e = plan(&c, Wanted::Role(Role::Resolved), Some("Globex fix"))
        .unwrap_err()
        .to_string();
    assert!(e.contains("not identifiable"), "{e}");
    let e = plan(&c, "Fixed".into(), None).unwrap_err().to_string();
    assert!(e.contains("cannot tell"), "{e}");
}

/// `test_merge_and_conversation_resolve_builtin_ids_under_translated_labels`.
#[test]
fn merge_and_conversation_resolve_builtin_ids_under_translated_labels() {
    let (c, w) = globex(ticket_statuses());
    assert_eq!(crate::ticket_merge::closed(&c, &store(), 123).unwrap().0, 5);
    let (label, role) = crate::conversations::status_choice(&c, &store(), 123, "Wartend").unwrap();
    assert_eq!((label.as_str(), role), ("Wartend", Role::Pending));
    let e = crate::conversations::status_choice(&c, &store(), 123, "Abgebrochen")
        .unwrap_err()
        .to_string();
    assert!(e.contains("not a supported status"), "{e}");
    w.lock().unwrap().statuses = unprovable_statuses();
    let e = crate::ticket_merge::closed(&c, &store(), 123)
        .unwrap_err()
        .to_string();
    assert!(e.contains("Closed status"), "{e}");
}

/// `test_problem_closed_label_translated_still_runs_closure_checks`.
#[test]
fn problem_closed_label_translated_still_runs_closure_checks() {
    let mut c = FnApi::new(|call: &Call| match call.path.as_str() {
        "problem_form_fields" => Ok(json!({"problem_fields": [
            form_field(501, "subject", "Subject", "default_subject", 1, json!({})),
            form_field(502, "status", "Status", "default_status", 2, json!({"choices": [
                {"id": 1, "value": "Neu"}, {"id": 21, "value": "Prüfung"}, {"id": 22, "value": "Wip"},
                {"id": 3, "value": "Erledigt"}]})),
        ]})),
        p if p.ends_with("business_rules") => Ok(json!({"business_rules": []})),
        "problems/77" => Ok(
            json!({"problem": {"id": 77, "subject": "Globex problem", "status": 1,
                                                "planned_start_date": null, "planned_end_date": null}}),
        ),
        "problems/77/planning-fields" => Ok(json!({"problem_planning_fields": []})),
        "problems/77/solutions" => Ok(json!({"articles": [], "has_more": false})),
        _ => Err(fsv_core::ApiError::new(404, json!({"error": "unexpected"})).into()),
    });
    c.domain = "globex.example.com".into();
    let store = MemStore {
        workspace: json!(5),
        ..MemStore::default()
    };
    let e = crate::problem_status::build_plan(&c, &store, "PRB-77", "Erledigt", None)
        .unwrap_err()
        .to_string();
    // closing was recognised by id 3: the closure checks ran
    assert!(!e.contains("closes the problem"), "{e}");
    assert!(e.starts_with("violated_closure_rules: missing"), "{e}");
}
