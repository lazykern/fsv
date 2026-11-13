//! Change relationship read refusals (Python 618593b), ticket kind proof
//! from the record's own fields (e12c5ea) and ticket-side problem, change
//! and child-ticket associations (7ea60aa). Routes are synthetic; unmatched
//! requests answer 404.

mod common;
use common::{Env, get};
use serde_json::{Value, json};

fn change_routes() -> Vec<Value> {
    vec![
        get(
            "changes/123",
            json!({"change": {"id": 123, "subject": "Acme change"}}),
        ),
        get(
            "changes/123/releases",
            json!({"releases": [], "meta": {"has_next": false}}),
        ),
        get(
            "changes/123/problems",
            json!({"problems": [], "meta": {"has_next": false}}),
        ),
        get(
            "changes/123/tickets",
            json!({"tickets": [], "meta": {"has_next": false}}),
        ),
        get("releases/71", json!({"release": {"id": 71}})),
    ]
}

/// Python tests/test_change_relationships.py::test_missing_ids_are_one_line_refusals
#[test]
fn change_relationship_missing_ids_are_one_line_refusals() {
    for (kind, missing, named) in [
        (
            "release",
            "/api/_/releases/62",
            "release 62 not found (HTTP 404); nothing was sent",
        ),
        (
            "problem",
            "/api/_/problems/62",
            "problem 62 not found (HTTP 404); nothing was sent",
        ),
        (
            "caused-incident",
            "/api/_/tickets/62",
            "incident 62 not found (HTTP 404); nothing was sent",
        ),
        (
            "release",
            "/api/_/changes/123",
            "change 123 not found (HTTP 404); nothing was sent",
        ),
        (
            "problem",
            "/api/_/changes/123/problems",
            "mutation inventory unavailable",
        ),
    ] {
        let routes: Vec<Value> = change_routes()
            .into_iter()
            .filter(|r| r["path"] != missing)
            .collect();
        let r = Env::new(routes).run(&[
            "changes",
            "relationships",
            "write",
            "CHN-123",
            kind,
            "add",
            "--item",
            "62",
            "--dry-run",
            "--json",
        ]);
        assert_eq!(r.code, 2, "{kind} {missing}: {}", r.err);
        assert!(r.err.contains(named), "{kind} {missing}: {}", r.err);
        assert!(!r.err.contains("Traceback"));
        assert_eq!(r.err.trim().lines().count(), 1, "{}", r.err);
        assert_eq!(r.out, "");
        assert!(r.writes().is_empty());
    }
}

#[test]
fn change_relationship_list_api_failure_is_one_line() {
    let r =
        Env::new(change_routes()).run(&["changes", "relationships", "list", "CHN-123", "project"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(
        r.err,
        "the project relationships of CHN-123 not found (HTTP 404)\n"
    );
}
