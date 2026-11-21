//! Python commits 8a3d2c6 and neighbours: `tickets update` is a reviewed write
//! by default (`tests/test_ticket_update_reviewed_default.py`).

mod common;
use common::Env;
use serde_json::{Value, json};

const NOTICE: &str = "warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default";

fn routes() -> Vec<Value> {
    serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/routes/ticket_reviewed.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

fn review(env: &Env, args: &[&str]) -> Value {
    let mut full = vec!["tickets", "update"];
    full.extend_from_slice(args);
    full.push("--dry-run");
    let r = env.run(&full);
    assert_eq!(r.code, 0, "{}", r.err);
    serde_json::from_str(&r.out).unwrap()
}

#[test]
fn default_dry_run_shows_plan_hash_and_record_state_without_reviewed() {
    for target in ["INC-5", "5"] {
        let env = Env::new(routes());
        let plan = review(&env, &[target, "--priority", "High"]);
        assert_eq!(plan["action"], json!("ticket_properties"));
        assert_eq!(plan["plan_hash"].as_str().unwrap().len(), 64);
        assert_eq!(
            plan["record_state"],
            json!({"sends": 0, "server_version": null})
        );
        assert_eq!(plan["payload"]["priority"], json!(73));
        assert!(
            env.run(&[
                "tickets",
                "update",
                target,
                "--priority",
                "High",
                "--dry-run"
            ])
            .writes()
            .is_empty()
        );
        let alias = review(&env, &[target, "--priority", "High", "--reviewed"]);
        assert_eq!(alias["plan_hash"], plan["plan_hash"]);
    }
}

#[test]
fn send_verifies_and_replay_is_refused_with_exit_2() {
    let env = Env::new(routes());
    let digest = review(&env, &["5", "--priority", "High"])["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let send = |env: &Env| {
        env.run(&[
            "tickets",
            "update",
            "5",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            &digest,
        ])
    };
    let first = send(&env);
    assert_eq!(first.code, 0, "{}", first.err);
    let result: Value = serde_json::from_str(&first.out).unwrap();
    assert_eq!(result["verified"], json!(true));
    assert_eq!(result["plan_hash"], json!(digest));
    assert_eq!(result["fields"]["priority"], json!("matched"));
    assert_eq!(first.write_paths(), ["PUT /api/_/tickets/5"]);
    let after = review(&env, &["5", "--priority", "High"]);
    assert_eq!(after["record_state"]["sends"], json!(1));
    assert_ne!(after["plan_hash"], json!(digest));
    let replay = send(&env);
    assert_eq!(replay.code, 2, "{}", replay.err);
    assert!(replay.err.contains("was already sent"), "{}", replay.err);
    assert!(replay.write_paths().is_empty());
}

#[test]
fn yes_without_the_full_hash_sends_nothing() {
    let env = Env::new(routes());
    let digest = review(&env, &["5", "--priority", "High"])["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let r = env.run(&["tickets", "update", "5", "--priority", "High", "--yes"]);
    assert_eq!(r.code, 2);
    let r = env.run(&[
        "tickets",
        "update",
        "5",
        "--priority",
        "High",
        "--yes",
        "--expect-plan",
        &digest[..16],
    ]);
    assert_eq!(r.code, 2);
    assert!(r.write_paths().is_empty());
}

#[test]
fn ack_works_without_reviewed_and_no_review_has_no_ack() {
    let env = Env::new(routes());
    let plan = review(&env, &["5", "--priority", "High", "--ack"]);
    assert_eq!(plan["action"], json!("ticket_properties"));
    let r = env.run(&[
        "tickets",
        "update",
        "--no-review",
        "5",
        "--priority",
        "High",
        "--ack",
        "--dry-run",
    ]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("drop --no-review"), "{}", r.err);
}

#[test]
fn no_review_prints_one_notice_and_keeps_stdout_pure() {
    let env = Env::new(routes());
    let dry = env.run(&[
        "tickets",
        "update",
        "--no-review",
        "5",
        "--priority",
        "High",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "{}", dry.err);
    let body: Value = serde_json::from_str(&dry.out).unwrap();
    assert_eq!(body["body"], json!({"priority": 73}));
    let notices: Vec<&str> = dry.err.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(notices, [NOTICE]);
    assert!(!dry.out.contains("deprecated"));
}

#[test]
fn no_review_refuses_reviewed_only_options() {
    let env = Env::new(routes());
    for extra in [
        vec!["--ack"],
        vec!["--tag", "acme"],
        vec!["--planned-start", "2030-01-02T09:00:00Z"],
    ] {
        let mut args = vec![
            "tickets",
            "update",
            "--no-review",
            "INC-5",
            "--priority",
            "High",
        ];
        args.extend(extra);
        args.push("--dry-run");
        let r = env.run(&args);
        assert_eq!(r.code, 2, "{}", r.err);
        assert!(r.err.contains("drop --no-review"), "{}", r.err);
        assert!(r.write_paths().is_empty());
    }
}

fn with_extra_choice(mut routes: Vec<Value>) -> Vec<Value> {
    for r in &mut routes {
        if r["path"] == json!("/api/_/ticket_form_fields") {
            for f in r["json"]["ticket_fields"].as_array_mut().unwrap() {
                if f["name"] == json!("priority") {
                    f["choices"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"id": 74, "value": "Urgent Acme"}));
                }
            }
        }
    }
    routes
}

fn form_gets(r: &common::Run) -> usize {
    r.calls()
        .iter()
        .filter(|(m, p)| {
            m == "GET"
                && p.split('?')
                    .next()
                    .unwrap()
                    .ends_with("/ticket_form_fields")
        })
        .count()
}

/// `test_ticket_update_schema_cache.py::test_dry_run_inside_ttl_sends_no_form_fields_get`.
#[test]
fn dry_run_inside_ttl_sends_no_form_fields_get() {
    let env = Env::new(routes());
    let args = [
        "tickets",
        "update",
        "5",
        "--priority",
        "Medium",
        "--no-input",
        "--dry-run",
    ];
    let first = env.run(&args);
    assert_eq!(first.code, 0, "{}", first.err);
    assert_eq!(form_gets(&first), 1);
    let second = env.run(&args);
    assert_eq!(second.code, 0, "{}", second.err);
    assert_eq!(form_gets(&second), 0);
    assert_eq!(first.json()["plan_hash"], second.json()["plan_hash"]);
}

/// `test_send_rebuild_refetches_and_verifies`.
#[test]
fn send_rebuild_refetches_the_form() {
    let env = Env::new(routes());
    let digest = review(&env, &["5", "--priority", "High"])["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let sent = env.run(&[
        "tickets",
        "update",
        "5",
        "--priority",
        "High",
        "--yes",
        "--expect-plan",
        &digest,
    ]);
    assert_eq!(sent.code, 0, "{}", sent.err);
    assert_eq!(form_gets(&sent), 1);
    assert_eq!(sent.write_paths(), ["PUT /api/_/tickets/5"]);
}

/// `test_form_changed_since_review_is_refused_at_send`.
#[test]
fn form_changed_since_review_is_refused_at_send() {
    let mut env = Env::new(routes());
    let digest = review(&env, &["5", "--priority", "Medium"])["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    env.routes = with_extra_choice(routes());
    let sent = env.run(&[
        "tickets",
        "update",
        "5",
        "--priority",
        "Medium",
        "--yes",
        "--expect-plan",
        &digest,
    ]);
    assert_eq!(sent.code, 2, "{}", sent.err);
    assert!(sent.err.contains("changed since review"), "{}", sent.err);
    assert!(sent.write_paths().is_empty());
}

/// `test_value_not_in_the_cached_form_refetches_once`.
#[test]
fn value_not_in_the_cached_form_refetches_once() {
    let mut env = Env::new(routes());
    review(&env, &["5", "--priority", "Medium"]);
    env.routes = with_extra_choice(routes());
    let r = env.run(&[
        "tickets",
        "update",
        "5",
        "--priority",
        "Urgent Acme",
        "--dry-run",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.err.contains("refetching the ticket form once"),
        "{}",
        r.err
    );
    assert_eq!(form_gets(&r), 1);
    let bad = env.run(&["tickets", "update", "5", "--priority", "Nope", "--dry-run"]);
    assert_eq!(bad.code, 2, "{}", bad.err);
    assert_eq!(form_gets(&bad), 1);
    assert!(bad.write_paths().is_empty());
}

// problems update: form fields reviewed by default (b04e117,
// `tests/test_problem_update_reviewed_default.py`).

fn problem_form() -> Value {
    json!({"problem_fields": [
        {"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1},
        {"id": 502, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
         "choices": [{"id": 31, "value": "Open"}]},
        {"id": 503, "name": "priority", "label": "Priority", "field_type": "default_priority", "position": 3,
         "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]},
        {"id": 504, "name": "description", "label": "Description", "field_type": "default_description", "position": 4},
        {"id": 505, "name": "acme_code", "label": "Account code", "field_type": "custom_text", "position": 5,
         "default_field": false},
    ]})
}

fn problem(priority: i64, code: &str, description: &str, updated_at: &str) -> Value {
    json!({"problem": {"id": 123, "subject": "Acme problem", "description": description, "status": 31,
        "priority": priority, "custom_fields": {"acme_code": code}, "deleted": false,
        "updated_at": updated_at}})
}

fn problem_routes(after: Value, put_status: u16) -> Vec<Value> {
    let get = |once: bool, body: Value| {
        let mut r = json!({"method": "GET", "path": "/api/_/problems/123", "json": body});
        if once {
            r["once"] = json!(true);
        }
        r
    };
    let before = problem(1, "A1", "<div>Acme outage</div>", "2030-01-01T00:00:00Z");
    vec![
        get(true, before.clone()),
        get(true, before),
        get(false, after),
        json!({"method": "GET", "path": "/api/_/problem_form_fields", "json": problem_form()}),
        json!({"method": "GET", "path": "/api/_/business_rules", "json": {"business_rules": []}}),
        json!({"method": "PUT", "path": "/api/_/problems/123", "status": put_status,
               "json": if put_status == 200 { json!({"problem": {"id": 123}}) } else { json!({"message": "synthetic outage"}) }}),
        json!({"method": "GET", "path": "/a/dashboard",
               "text": "<html><head><meta name=\"csrf-token\" content=\"tok\"></head></html>"}),
    ]
}

fn applied() -> Value {
    problem(
        3,
        "B2",
        "<div>Acme root cause</div>",
        "2030-01-01T00:00:01Z",
    )
}

fn problem_review(env: &Env, args: &[&str]) -> Value {
    let mut full = vec!["problems", "update", "PRB-123"];
    full.extend_from_slice(args);
    full.push("--dry-run");
    let r = env.run(&full);
    assert_eq!(r.code, 0, "{}", r.err);
    serde_json::from_str(&r.out).unwrap()
}

#[test]
fn problem_dry_run_reviews_only_changed_keys_with_record_state() {
    let env = Env::new(problem_routes(applied(), 200));
    let plan = problem_review(
        &env,
        &[
            "--priority",
            "High",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Account code=B2",
        ],
    );
    assert_eq!(plan["action"], json!("problem_update"));
    assert_eq!(plan["plan_hash"].as_str().unwrap().len(), 64);
    assert_eq!(
        plan["payload"],
        json!({"priority": 3, "custom_fields": {"acme_code": "B2"}})
    );
    assert_eq!(
        plan["before"],
        json!({"priority": 1, "custom_fields": {"acme_code": "A1"}})
    );
    assert_eq!(
        plan["record_state"],
        json!({"sends": 0, "server_version": "2030-01-01T00:00:00Z"})
    );
    assert_eq!(plan["path"], json!("/api/_/problems/123"));
    let alias = problem_review(&env, &["--priority", "High", "--reviewed"]);
    let plain = problem_review(&env, &["--priority", "High"]);
    assert_eq!(alias["plan_hash"], plain["plan_hash"]);
}

#[test]
fn problem_send_verifies_each_changed_key_and_replay_exits_2() {
    let env = Env::new(problem_routes(applied(), 200));
    let args = [
        "--priority",
        "High",
        "--set",
        "Account code=B2",
        "--set",
        "Description=Acme root cause",
    ];
    let digest = problem_review(&env, &args)["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let mut full = vec!["problems", "update", "PRB-123"];
    full.extend_from_slice(&args);
    full.extend(["--yes", "--expect-plan", digest.as_str(), "--json"]);
    let sent = env.run(&full);
    assert_eq!(sent.code, 0, "{} {}", sent.err, sent.out);
    let result: Value = serde_json::from_str(&sent.out).unwrap();
    assert_eq!(result["verified"], json!(true));
    assert_eq!(result["plan_hash"], json!(digest));
    assert_eq!(
        result["fields"],
        json!({"priority": "matched", "custom_fields.acme_code": "matched", "description": "matched"})
    );
    let writes = sent.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].2,
        json!({"priority": 3, "custom_fields": {"acme_code": "B2"}, "description": "Acme root cause"})
    );
    let replay = env.run(&full);
    assert_eq!(replay.code, 2, "{}", replay.err);
    assert!(replay.err.contains("already sent"), "{}", replay.err);
    assert!(replay.write_paths().is_empty());
}

#[test]
fn problem_ambiguous_outcome_is_not_replayable() {
    let mut env = Env::new(problem_routes(applied(), 503));
    let digest = problem_review(&env, &["--priority", "High"])["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let send = |env: &Env| {
        env.run(&[
            "problems",
            "update",
            "PRB-123",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            &digest,
        ])
    };
    let first = send(&env);
    assert_eq!(first.code, 2, "{}", first.err);
    assert!(first.err.contains("outcome unknown"), "{}", first.err);
    env.routes = problem_routes(applied(), 200);
    assert_eq!(
        problem_review(&env, &["--priority", "High"])["plan_hash"],
        json!(digest)
    );
    let second = send(&env);
    assert_eq!(second.code, 2, "{}", second.err);
    assert!(second.err.contains("unknown"), "{}", second.err);
    assert!(second.write_paths().is_empty());
}

#[test]
fn problem_unverified_readback_exits_4() {
    let unchanged = problem(1, "A1", "<div>Acme outage</div>", "2030-01-01T00:00:00Z");
    let env = Env::new(problem_routes(unchanged, 200));
    let digest = problem_review(&env, &["--priority", "High"])["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let sent = env.run(&[
        "problems",
        "update",
        "PRB-123",
        "--priority",
        "High",
        "--yes",
        "--expect-plan",
        &digest,
        "--json",
    ]);
    assert_eq!(sent.code, 4, "{}", sent.err);
    let result: Value = serde_json::from_str(&sent.out).unwrap();
    assert_eq!(result["fields"], json!({"priority": "mismatched"}));
    assert_eq!(
        problem_review(&env, &["--priority", "High"])["plan_hash"],
        json!(digest)
    );
}

#[test]
fn problem_no_op_says_nothing_to_change() {
    let env = Env::new(problem_routes(applied(), 200));
    let r = env.run(&[
        "problems",
        "update",
        "PRB-123",
        "--priority",
        "Low",
        "--set",
        "Account code=A1",
        "--dry-run",
        "--json",
    ]);
    let out: Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(out, json!({"status": "nothing_to_change", "target": 123}));
    let r = env.run(&[
        "problems",
        "update",
        "PRB-123",
        "--priority",
        "Low",
        "--dry-run",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("nothing to change"), "{}", r.out);
    assert!(!r.out.contains("plan_hash"));
    let r = env.run(&[
        "problems",
        "update",
        "PRB-123",
        "--priority",
        "Low",
        "--yes",
        "--expect-plan",
        &"a".repeat(64),
    ]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.err.contains("does not exist"), "{}", r.err);
    assert!(r.write_paths().is_empty());
}

#[test]
fn problem_status_and_other_refusals() {
    let env = Env::new(problem_routes(applied(), 200));
    for (extra, needle) in [
        (vec!["--status", "Open", "--dry-run"], "problems status"),
        (vec!["--tag", "acme", "--dry-run"], "only for tickets"),
        (vec!["--priority", "High", "--ack", "--dry-run"], "ticket"),
        (
            vec!["--priority", "High", "--yes"],
            "--yes requires the full --expect-plan",
        ),
    ] {
        let mut args = vec!["problems", "update", "PRB-123"];
        args.extend(extra);
        let r = env.run(&args);
        assert_eq!(r.code, 2, "{}", r.err);
        assert!(r.err.contains(needle), "{needle}: {}", r.err);
        assert!(r.write_paths().is_empty());
    }
}

#[test]
fn problem_no_review_prints_one_notice_and_sends_body_only() {
    let env = Env::new(problem_routes(applied(), 200));
    let dry = env.run(&[
        "problems",
        "update",
        "--no-review",
        "PRB-123",
        "--priority",
        "High",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "{}", dry.err);
    let body: Value = serde_json::from_str(&dry.out).unwrap();
    assert_eq!(body["body"], json!({"priority": 3}));
    let notices: Vec<&str> = dry.err.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(notices.len(), 1, "{}", dry.err);
    assert!(notices[0].contains("problems update --no-review is deprecated"));
    let sent = env.run(&[
        "problems",
        "update",
        "--no-review",
        "PRB-123",
        "--priority",
        "High",
        "--status",
        "Open",
        "--yes",
    ]);
    assert_eq!(sent.code, 0, "{}", sent.err);
    assert!(
        sent.err.contains("--no-review is deprecated"),
        "{}",
        sent.err
    );
    assert_eq!(sent.writes()[0].2, json!({"status": 31, "priority": 3}));
    let r = env.run(&[
        "problems",
        "update",
        "--no-review",
        "PRB-123",
        "--planned-effort",
        "1h",
        "--dry-run",
    ]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("drop --no-review"), "{}", r.err);
}
