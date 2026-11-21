//! `changes update --planning` on the reviewed contract (Python 9056d44,
//! `tests/test_change_planning_reviewed.py`): plan hash over the file bytes,
//! `--yes` needs the full `--expect-plan`, uploads then the planning save,
//! readback, exit 4 when unverified.

mod common;
use common::{Env, Run, get, route};
use serde_json::{Value, json};

const OLD_ROW: fn() -> Value = || {
    json!({"name": "acme_plan", "description": "<div>Acme rollout</div>",
           "attachments": [{"id": 501, "name": "old.txt", "size": 9}]})
};

fn schema() -> Value {
    json!({"change_fields": [
        {"id": 901, "name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "default_field": false}],
        "attachment_settings": {"max_attachment_size_bytes": 4096}})
}

fn planning(rows: Value) -> Value {
    json!({"method": "GET", "path": "/api/_/changes/123/planning-fields",
           "json": {"change_planning_fields": rows}})
}

fn once(mut r: Value) -> Value {
    r["once"] = json!(true);
    r
}

fn env(readback: Value) -> Env {
    let routes = vec![
        json!({"method": "GET", "path": "/a/dashboard",
               "text": "<html><head><meta name=\"csrf-token\" content=\"tok\"></head></html>"}),
        get("change_form_fields", schema()),
        get(
            "changes/123",
            json!({"change": {"id": 123, "updated_at": "2026-01-01T00:00:00Z"}}),
        ),
        once(planning(json!([OLD_ROW()]))),
        once(planning(json!([OLD_ROW()]))),
        planning(readback),
        route(
            "POST",
            "/api/_/attachments",
            200,
            json!({"id": 701, "name": "plan.txt", "size": "13"}),
        ),
        route(
            "PUT",
            "/api/_/changes/123/planning-fields/acme_plan",
            200,
            json!({}),
        ),
    ];
    let env = Env::new(routes);
    let work = env.home().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("plan.txt"), b"Acme plan v2\n").unwrap();
    env
}

fn args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut v = vec![
        "changes",
        "update",
        "CHN-123",
        "--planning",
        "Rollout Plan",
        "--file",
        "plan.txt",
    ];
    v.extend_from_slice(extra);
    v
}

fn run(env: &mut Env, extra: &[&str]) -> Run {
    env.cwd = Some(env.home().join("work"));
    env.stdin = Some(String::new());
    env.run(&args(extra))
}

fn verified_row() -> Value {
    json!([{"name": "acme_plan", "description": "<div>Acme v2</div>",
            "attachments": [{"id": 501, "name": "old.txt", "size": 9},
                            {"id": 701, "name": "plan.txt", "size": "13"}]}])
}

#[test]
fn dry_run_prints_the_plan_hash_and_sends_nothing() {
    let mut e = env(verified_row());
    let r = run(
        &mut e,
        &["--description", "<div>Acme v2</div>", "--dry-run"],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let out = r.json();
    assert_eq!(out["status"], "dry_run");
    assert_eq!(out["action"], "change_planning_update");
    assert_eq!(out["section"], "acme_plan");
    assert_eq!(out["steps"][0]["step"], "upload");
    assert_eq!(
        out["steps"][1]["path"],
        "/api/_/changes/123/planning-fields/acme_plan"
    );
    assert_eq!(out["attachment_size_limit"]["max_bytes"], 4096);
    assert!(out["plan_hash"].as_str().unwrap().len() == 64);
    assert!(r.write_paths().is_empty());
}

#[test]
fn yes_alone_refuses_and_sends_nothing() {
    let mut e = env(verified_row());
    let r = run(&mut e, &["--yes"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.err.contains("--expect-plan"), "{}", r.err);
    assert!(r.write_paths().is_empty());
}

#[test]
fn reviewed_send_uploads_then_saves_and_reads_back() {
    let mut e = env(verified_row());
    let review = run(
        &mut e,
        &["--description", "<div>Acme v2</div>", "--dry-run"],
    );
    let hash = review.json()["plan_hash"].as_str().unwrap().to_string();
    let mut e = env(verified_row());
    let r = run(
        &mut e,
        &[
            "--description",
            "<div>Acme v2</div>",
            "--yes",
            "--expect-plan",
            &hash,
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let out = r.json();
    assert_eq!(out["verified"], true, "{out}");
    assert_eq!(out["description"], "matched");
    assert_eq!(out["uploaded_ids"], json!([701]));
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/attachments",
            "PUT /api/_/changes/123/planning-fields/acme_plan"
        ]
    );
    let w = r.writes();
    assert_eq!(
        w[1].2,
        json!({"description": "<div>Acme v2</div>", "attachments": [501, 701]})
    );
}

#[test]
fn unverified_readback_exits_4_and_names_the_mismatch() {
    let mut e = env(
        json!([{"name": "acme_plan", "description": "<div>Acme rollout</div>",
                            "attachments": [{"id": 501, "name": "old.txt", "size": 9}]}]),
    );
    let review = run(
        &mut e,
        &["--description", "<div>Acme v2</div>", "--dry-run"],
    );
    let hash = review.json()["plan_hash"].as_str().unwrap().to_string();
    let mut e = env(
        json!([{"name": "acme_plan", "description": "<div>Acme rollout</div>",
                            "attachments": [{"id": 501, "name": "old.txt", "size": 9}]}]),
    );
    let r = run(
        &mut e,
        &[
            "--description",
            "<div>Acme v2</div>",
            "--yes",
            "--expect-plan",
            &hash,
        ],
    );
    assert_eq!(r.code, 4, "{}", r.err);
    let out = r.json();
    assert_eq!(out["verified"], false);
    assert_eq!(out["description"], "mismatched");
}

#[test]
fn a_skipped_duplicate_is_nothing_to_change() {
    let mut e = env(verified_row());
    let work = e.home().join("work");
    std::fs::write(work.join("old.txt"), b"Acme old\n").unwrap();
    e.cwd = Some(work);
    e.stdin = Some(String::new());
    let r = e.run(&[
        "changes",
        "update",
        "CHN-123",
        "--planning",
        "Rollout Plan",
        "--file",
        "old.txt",
        "--duplicate",
        "skip",
        "--dry-run",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("nothing to change"), "{}", r.out);
    assert!(!r.out.contains("plan_hash"));
    let r = e.run(&[
        "changes",
        "update",
        "CHN-123",
        "--planning",
        "Rollout Plan",
        "--file",
        "old.txt",
        "--duplicate",
        "skip",
        "--yes",
        "--expect-plan",
        &"0".repeat(64),
    ]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.write_paths().is_empty());
}
