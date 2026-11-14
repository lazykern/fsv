//! `tests/test_shifted_tenant.py` status-semantics cases (Python b6b741f).

use serde_json::json;

use super::*;

/// Built-in problem statuses keep their platform internal ids (1 Open,
/// 3 Closed) under renamed/translated labels.
fn problem_statuses() -> Vec<Value> {
    vec![
        json!({"id": 1, "value": "Neu"}),
        json!({"id": 21, "value": "Prüfung"}),
        json!({"id": 22, "value": "Wip"}),
        json!({"id": 3, "value": "Erledigt"}),
    ]
}

/// Renamed built-ins (2-5) plus shifted custom statuses.
pub(crate) fn ticket_statuses() -> Vec<Value> {
    vec![
        json!({"id": 2, "value": "Offen"}),
        json!({"id": 3, "value": "Wartend"}),
        json!({"id": 12, "value": "Abgebrochen"}),
        json!({"id": 4, "value": "Gelöst"}),
        json!({"id": 5, "value": "Erledigt"}),
        json!({"id": 13, "value": "In Arbeit"}),
    ]
}

/// A metadata shape without the built-in internal ids.
pub(crate) fn unprovable_statuses() -> Vec<Value> {
    vec![
        json!({"id": 10, "value": "New"}),
        json!({"id": 11, "value": "Wip"}),
        json!({"id": 12, "value": "Fixed"}),
        json!({"id": 13, "value": "Done"}),
    ]
}

fn refs(v: &[Value]) -> Vec<&Value> {
    v.iter().collect()
}

/// `test_safety_and_problem_semantics_use_ids_not_words`.
#[test]
fn safety_and_problem_semantics_use_ids_not_words() {
    let t = ticket_statuses();
    let u = unprovable_statuses();
    let p = problem_statuses();
    assert_eq!(
        status_closes("tickets", Some(&json!(4)), &refs(&t)),
        Some(true)
    );
    assert_eq!(
        status_closes("tickets", Some(&json!(12)), &refs(&t)),
        Some(false)
    );
    assert_eq!(status_closes("tickets", Some(&json!(12)), &refs(&u)), None);
    assert_eq!(status_closes("changes", Some(&json!(4)), &refs(&t)), None);
    assert_eq!(
        closes(&refs(&p), "problem", Some(&json!(3))),
        Ok(Some(true))
    );
    assert_eq!(
        closes(&refs(&p), "problem", Some(&json!(22))),
        Ok(Some(false))
    );
    let lone = vec![json!({"id": 21, "value": "Closed"})];
    assert_eq!(closes(&refs(&lone), "problem", Some(&json!(21))), Ok(None));
}

#[test]
fn select_find_and_role_of() {
    let t = ticket_statuses();
    let pool = refs(&t);
    assert_eq!(
        select(&pool, "ticket", Wanted::Role(Role::Resolved)).unwrap()["id"],
        4
    );
    assert_eq!(select(&pool, "ticket", "Wartend".into()).unwrap()["id"], 3);
    assert_eq!(
        select(&pool, "change", Wanted::Label("x")).unwrap_err().0,
        "no status semantics for 'change'"
    );
    assert_eq!(select(&pool, "ticket", "closed".into()).unwrap()["id"], 5);
    assert_eq!(
        select(&pool, "ticket", "Bogus".into()).unwrap_err().0,
        "status 'Bogus' is not a runtime status label; choices: Offen, Wartend, Abgebrochen, Gelöst, Erledigt, In Arbeit"
    );
    let dup = vec![
        json!({"id": 4, "value": "A"}),
        json!({"id": 9, "value": "a"}),
    ];
    assert_eq!(
        select(&refs(&dup), "ticket", "a".into()).unwrap_err().0,
        "status 'a' matches several runtime statuses; use a unique label"
    );
    let u = unprovable_statuses();
    assert_eq!(
        find(&refs(&u), "ticket", Role::Closed).unwrap_err().0,
        "the closed status is not identifiable from the runtime status metadata (the built-in internal id is missing); pass the tenant's own status label explicitly"
    );
    let twice = vec![
        json!({"id": 5, "value": "A"}),
        json!({"id": "5", "value": "B"}),
    ];
    assert!(
        find(&refs(&twice), "ticket", Role::Closed)
            .unwrap_err()
            .0
            .contains("duplicated")
    );
    assert_eq!(
        find(&pool, "problem", Role::Pending).unwrap_err().0,
        "problem has no built-in 'pending' status"
    );
    assert_eq!(
        role_of(&pool, "ticket", Some(&json!("3"))),
        Ok(Some("pending"))
    );
    assert_eq!(role_of(&pool, "ticket", Some(&json!(12))), Ok(None));
    let hidden = vec![json!({"id": 5, "value": "Closed", "visible": false})];
    assert!(select(&refs(&hidden), "ticket", Wanted::Role(Role::Closed)).is_err());
}

/// `_label` of a choice with no text (Python 31a9103): a form choice with
/// no id shows `None`; a non-dict JSON choice shows nothing.
#[test]
fn label_of_empty_choices_matches_python() {
    let blank = fsv_forms::Choice {
        value: String::new(),
        label: String::new(),
        id: None,
        display_id: None,
        description: String::new(),
        visible: true,
        children: Vec::new(),
    };
    assert_eq!(
        select(&[&blank], "ticket", "Nope".into()).unwrap_err().0,
        "status 'Nope' is not a runtime status label; choices: None"
    );
    let text = json!("x");
    assert_eq!(
        select(&[&text], "ticket", "Nope".into()).unwrap_err().0,
        "status 'Nope' is not a runtime status label"
    );
}
