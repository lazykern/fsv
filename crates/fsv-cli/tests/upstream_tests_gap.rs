//! Python CLI tests with no Rust counterpart yet: legacy flat-approval
//! refusals and CAB expansion, rule warnings in a ticket-create review, and
//! the stale-schema warning of a problem update dry run.

mod common;
use common::{Env, fixture, run_fixture};
use serde_json::{Value, json};

/// `approval_model_legacy` with approval-chains answering `status`.
fn legacy_tenant(status: u16) -> Env {
    let mut routes = fixture("approval_model_legacy");
    for r in &mut routes {
        if r["path"] == "/api/_/changes/123/approval-chains" {
            r["status"] = json!(status);
        }
    }
    Env::new(routes)
}

/// Python tests/test_approval_model.py::test_change_cancel_cli_refuses_even_on_dry_run
#[test]
fn change_cancel_cli_refuses_even_on_dry_run() {
    for status in [400, 404] {
        let r = legacy_tenant(status).run(&[
            "changes",
            "approval-cancel",
            "CHN-123",
            "710000000001",
            "--dry-run",
        ]);
        assert_eq!(r.code, 2, "{status}: {}", r.err);
        assert!(
            r.err.contains("legacy flat approvals: not yet supported"),
            "{status}: {}",
            r.err
        );
        assert!(r.writes().is_empty(), "{status}");
    }
}

/// Python tests/test_approval_model.py::test_change_groups_listing_names_model_and_flat_listing_works
#[test]
fn change_groups_listing_names_model_and_flat_listing_works() {
    for status in [400, 404] {
        let env = legacy_tenant(status);
        let r = env.run(&["changes", "approvals", "CHN-123", "--groups", "--json"]);
        assert_ne!(r.code, 0, "{status}");
        assert!(
            format!("{}{}", r.out, r.err).contains("legacy flat approvals"),
            "{status}: {}",
            r.err
        );
        let r = env.run(&["changes", "approvals", "CHN-123", "--json"]);
        assert_eq!(r.code, 0, "{status}: {}", r.err);
        assert_eq!(r.json()[0]["member_id"], json!(9000003001i64));
    }
}

/// Python tests/test_legacy_approvals.py::test_change_cab_request_expands_members_client_side
#[test]
fn change_cab_request_expands_members_client_side() {
    let r = run_fixture(
        "legacy_approvals_cab",
        &[
            "changes",
            "approval-request",
            "CHN-41",
            "--cab",
            "Globex CAB",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let review = r.json();
    assert_eq!(review["driver"], "legacy");
    assert!(
        review["provenance"]
            .as_str()
            .unwrap()
            .contains("not yet live-proven")
    );
    assert_eq!(review["method"], "POST");
    assert_eq!(review["path"], "/api/_/changes/41/approvals");
    assert_eq!(
        review["payload"],
        json!({"approval_type": 1, "notified_to": [9000003101i64, 9000003102i64]})
    );
    assert!(r.writes().is_empty());
}

/// Python tests/test_legacy_approvals.py::test_cancel_and_decisions_refuse_on_legacy
#[test]
fn cancel_and_decisions_refuse_on_legacy() {
    let cases: [&[&str]; 3] = [
        &[
            "changes",
            "approval-cancel",
            "CHN-41",
            "710000000001",
            "--dry-run",
        ],
        &[
            "sr",
            "approvals",
            "approve",
            "SR-41",
            "710000000001",
            "--dry-run",
        ],
        &[
            "sr",
            "approvals",
            "cancel",
            "SR-41",
            "710000000001",
            "--dry-run",
        ],
    ];
    for args in cases {
        let r = run_fixture("legacy_approvals_cab", args, None);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        assert!(
            r.err.contains("legacy flat approvals"),
            "{args:?}: {}",
            r.err
        );
        assert!(r.writes().is_empty(), "{args:?}");
    }
}

/// Python tests/test_rule_write_gate.py::test_dry_run_json_lists_rule_warnings
#[test]
fn dry_run_json_lists_rule_warnings() {
    let mut routes = fixture("ticket_create");
    for r in &mut routes {
        if r["path"] == "/api/_/business_rules" {
            r["json"]["business_rules"] = json!([{
                "id": 9000006010i64, "name": "Acme rule 9000006010", "workspace_id": 2,
                "module_type": "ticket", "item_id": null, "deleted": false, "user_type": "both",
                "form_type": "both", "state": "active", "rule_type": "client_side", "description": "",
                "condition_data": {"all": null}, "action_data": {"action": [{"name": ["tags"], "property": "hide"}]},
                "is_reversible": false, "is_validation": false, "updated_at": "2030-01-03T00:00:00Z"}]);
        }
    }
    let r = Env::new(routes).run(&[
        "tickets",
        "create",
        "--requester",
        "ann@acme.com",
        "--set",
        "Subject=Service unavailable",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.json().to_string().contains("9000006010"), "{}", r.out);
    assert!(r.writes().is_empty());
}

/// `test_problem_write_fixes.SCHEMA`, optionally with a new server choice.
fn problem_routes(new_choice: bool) -> Vec<Value> {
    let mut resolve_by = json!([{"id": 9000002001i64, "value": "Change Management"}, {"id": 9000002002i64, "value": "Cancel"}]);
    if new_choice {
        resolve_by
            .as_array_mut()
            .unwrap()
            .push(json!({"id": 9000002003i64, "value": "Irrelevant"}));
    }
    vec![
        common::get(
            "problem_form_fields",
            json!({"problem_fields": [
                {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": true,
                 "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]},
                {"name": "acme_resolve_by", "label": "Resolve By", "field_type": "custom_dropdown",
                 "default_field": false, "choices": resolve_by},
            ]}),
        ),
        common::get(
            "problems/7",
            json!({"problem": {"id": 7, "deleted": false}}),
        ),
    ]
}

/// Python tests/test_problem_write_fixes.py::test_dry_run_reports_stale_schema
#[test]
fn dry_run_reports_stale_schema() {
    let args = [
        "problems",
        "update",
        "PRB-7",
        "--priority",
        "High",
        "--dry-run",
        "--json",
    ];
    let mut env = Env::new(problem_routes(false));
    let r = env.run(&args);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.json().get("warnings").is_none(), "{}", r.out);
    env.routes = problem_routes(true);
    let r = env.run(&args);
    assert_eq!(r.code, 0, "{}", r.err);
    let warnings = r.json()["warnings"].clone();
    assert!(
        warnings
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("--refresh")),
        "{}",
        r.out
    );
    assert!(r.err.contains("--refresh"), "{}", r.err);
    assert!(r.writes().is_empty());
}
