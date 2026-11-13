//! Status batch CLI cases: problem update plan hash (Python 0208fdb).

mod common;
use common::{Env, get, route};
use serde_json::{Value, json};

fn csrf() -> Value {
    json!({"method": "GET", "path": "/a/dashboard",
           "text": "<html><head><meta name=\"csrf-token\" content=\"tok\"></head></html>"})
}

/// `test_problem_write_fixes.SCHEMA` and a readable problem 7.
fn problem_update_routes(subject: &str) -> Vec<Value> {
    vec![
        csrf(),
        get(
            "problem_form_fields",
            json!({"problem_fields": [
                {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": true,
                 "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]},
                {"name": "acme_resolve_by", "label": "Resolve By", "field_type": "custom_dropdown",
                 "default_field": false, "choices": [{"id": 9000002001i64, "value": "Change Management"},
                                                     {"id": 9000002002i64, "value": "Cancel"}]},
            ]}),
        ),
        get(
            "problems/7",
            json!({"problem": {"id": 7, "deleted": false, "subject": subject}}),
        ),
        route(
            "PUT",
            "/api/_/problems/7",
            200,
            json!({"problem": {"id": 7, "priority": 3}}),
        ),
    ]
}

/// Python `_problem_update_hash(PROBLEMS, 7, {"priority": 3}, row)` on
/// acme.freshservice.com for subject "Acme outage" (0208fdb).
const PLAN: &str = "19e0acf527f1540c258c405a57bde7fc9878063fea0f014e6ea7af833cb8ded0";

/// Python tests/test_problem_write_fixes.py::test_problem_update_dry_run_prints_a_plan_hash_bound_on_send
#[test]
fn problem_update_dry_run_prints_a_plan_hash_bound_on_send() {
    let args = ["problems", "update", "PRB-7", "--priority", "High"];
    let mut env = Env::new(problem_update_routes("Acme outage"));
    let run = |env: &Env, extra: &[&str]| env.run(&[&args[..], extra].concat());
    let r = run(&env, &["--dry-run", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let doc = r.json();
    assert_eq!(doc["plan_hash"], json!(PLAN));
    assert_eq!(
        r.out,
        format!(
            "{{\n  \"dry_run\": true,\n  \"action\": \"update\",\n  \"id\": \"PRB-7\",\n  \"body\": {{\n    \"priority\": 3\n  }},\n  \"plan_hash\": \"{PLAN}\"\n}}\n"
        )
    );
    assert!(r.writes().is_empty());
    let r = run(&env, &["--dry-run"]);
    assert!(r.out.contains(PLAN), "{}", r.out);
    let zeros = "0".repeat(64);
    let r = run(&env, &["--yes", "--expect-plan", &zeros, "--json"]);
    assert_ne!(r.code, 0);
    assert!(
        r.err.contains("PRB-7 or the update changed since review (or the problem could not be read); review again with --dry-run. Nothing was sent"),
        "{}",
        r.err
    );
    assert!(r.writes().is_empty());
    env.routes = problem_update_routes("Globex outage");
    let r = run(&env, &["--yes", "--expect-plan", PLAN, "--json"]);
    assert_ne!(r.code, 0);
    assert!(r.writes().is_empty());
    env.routes = problem_update_routes("Acme outage");
    let r = run(&env, &["--yes", "--expect-plan", PLAN, "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/7"]);
    assert_eq!(r.writes()[0].2, json!({"priority": 3}));
}

#[test]
fn ticket_update_expect_plan_still_requires_reviewed() {
    let env = Env::new(vec![]);
    let r = env.run(&[
        "tickets",
        "update",
        "5",
        "--priority",
        "High",
        "--expect-plan",
        "abc",
    ]);
    assert_eq!(r.code, 2);
    assert!(
        r.err.contains("--expect-plan requires --reviewed"),
        "{}",
        r.err
    );
}
