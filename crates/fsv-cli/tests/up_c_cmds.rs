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
        .filter(|(m, p)| m == "GET" && p.split('?').next().unwrap().ends_with("/ticket_form_fields"))
        .count()
}

/// `test_ticket_update_schema_cache.py::test_dry_run_inside_ttl_sends_no_form_fields_get`.
#[test]
fn dry_run_inside_ttl_sends_no_form_fields_get() {
    let env = Env::new(routes());
    let args = ["tickets", "update", "5", "--priority", "Medium", "--no-input", "--dry-run"];
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
        "tickets", "update", "5", "--priority", "High", "--yes", "--expect-plan", &digest,
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
        "tickets", "update", "5", "--priority", "Medium", "--yes", "--expect-plan", &digest,
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
        "tickets", "update", "5", "--priority", "Urgent Acme", "--dry-run",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.err.contains("refetching the ticket form once"), "{}", r.err);
    assert_eq!(form_gets(&r), 1);
    let bad = env.run(&["tickets", "update", "5", "--priority", "Nope", "--dry-run"]);
    assert_eq!(bad.code, 2, "{}", bad.err);
    assert_eq!(form_gets(&bad), 1);
    assert!(bad.write_paths().is_empty());
}
