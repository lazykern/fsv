//! `changes update --attach` (attach-only) on the reviewed contract (Python
//! 8e51091, `tests/test_change_attach_reviewed.py`).

mod common;
use common::{Env, Run, get, route};
use serde_json::{Value, json};

fn change(attachments: Value) -> Value {
    get(
        "changes/123",
        json!({"change": {"id": 123, "updated_at": "2026-01-01T00:00:00Z", "attachments": attachments}}),
    )
}

fn once(mut r: Value) -> Value {
    r["once"] = json!(true);
    r
}

fn env(readback: Value) -> Env {
    let old = json!([{"id": 501, "name": "old.txt", "size": 9}]);
    let routes = vec![
        json!({"method": "GET", "path": "/a/dashboard",
               "text": "<html><head><meta name=\"csrf-token\" content=\"tok\"></head><script>window.MAX_ATTACHMENT_LIMIT = 1;</script></html>"}),
        once(change(old.clone())),
        once(change(old)),
        change(readback),
        route(
            "POST",
            "/api/_/attachments",
            200,
            json!({"id": 701, "name": "plan.txt", "size": "10"}),
        ),
        route(
            "PUT",
            "/api/_/changes/123",
            200,
            json!({"change": {"id": 123}}),
        ),
    ];
    let env = Env::new(routes);
    let work = env.home().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("plan.txt"), b"Acme plan\n").unwrap();
    env
}

fn go(env: &mut Env, extra: &[&str]) -> Run {
    env.cwd = Some(env.home().join("work"));
    env.stdin = Some(String::new());
    let mut args = vec!["changes", "update", "CHN-123", "--attach", "plan.txt"];
    args.extend_from_slice(extra);
    env.run(&args)
}

fn verified() -> Value {
    json!([{"id": 501, "name": "old.txt", "size": 9}, {"id": 701, "name": "plan.txt", "size": "10"}])
}

#[test]
fn dry_run_prints_the_plan_hash_and_sends_nothing() {
    let mut e = env(verified());
    let r = go(&mut e, &["--dry-run"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let out = r.json();
    assert_eq!(out["status"], "dry_run");
    assert_eq!(out["action"], "change_attachments_update");
    assert_eq!(out["steps"][1]["path"], "/api/_/changes/123");
    assert_eq!(out["attachment_size_limit"]["max_bytes"], 1048576);
    assert_eq!(out["kept_attachment_ids"], json!([501]));
    assert!(r.write_paths().is_empty());
}

#[test]
fn yes_alone_refuses_and_sends_nothing() {
    let mut e = env(verified());
    let r = go(&mut e, &["--yes"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.err.contains("--expect-plan"), "{}", r.err);
    assert!(r.write_paths().is_empty());
}

#[test]
fn reviewed_send_uploads_then_attaches_and_reads_back() {
    let mut e = env(verified());
    let hash = go(&mut e, &["--dry-run"]).json()["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let mut e = env(verified());
    let r = go(&mut e, &["--yes", "--expect-plan", &hash]);
    assert_eq!(r.code, 0, "{}", r.err);
    let out = r.json();
    assert_eq!(out["verified"], true, "{out}");
    assert_eq!(out["uploaded_ids"], json!([701]));
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/attachments", "PUT /api/_/changes/123"]
    );
    assert_eq!(r.writes()[1].2, json!({"attachments": [501, 701]}));
    let again = go(&mut e, &["--yes", "--expect-plan", &hash]);
    assert_eq!(again.code, 2, "{}", again.err);
    assert!(again.write_paths().is_empty());
}

#[test]
fn unverified_readback_exits_4() {
    let old = json!([{"id": 501, "name": "old.txt", "size": 9}]);
    let mut e = env(old.clone());
    let hash = go(&mut e, &["--dry-run"]).json()["plan_hash"]
        .as_str()
        .unwrap()
        .to_string();
    let mut e = env(old);
    let r = go(&mut e, &["--yes", "--expect-plan", &hash]);
    assert_eq!(r.code, 4, "{}{}", r.out, r.err);
    assert_eq!(r.json()["verified"], false);
}

#[test]
fn a_skipped_duplicate_is_nothing_to_change() {
    let mut e = env(verified());
    let work = e.home().join("work");
    std::fs::write(work.join("old.txt"), b"Acme old\n").unwrap();
    e.cwd = Some(work);
    e.stdin = Some(String::new());
    let r = e.run(&[
        "changes",
        "update",
        "CHN-123",
        "--attach",
        "old.txt",
        "--duplicate",
        "skip",
        "--dry-run",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out
            .contains("nothing to change on change 123 (old.txt already attached)"),
        "{}",
        r.out
    );
    let r = e.run(&[
        "changes",
        "update",
        "CHN-123",
        "--attach",
        "old.txt",
        "--duplicate",
        "skip",
        "--yes",
        "--expect-plan",
        &"0".repeat(64),
    ]);
    assert_eq!(r.code, 2, "{}", r.err);
}

#[test]
fn backup_replaced_is_refused_on_this_path() {
    let mut e = env(verified());
    let r = go(
        &mut e,
        &["--duplicate", "replace", "--backup-replaced", "--dry-run"],
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(
        r.err
            .contains("--backup-replaced is not on the reviewed attachment path"),
        "{}",
        r.err
    );
}

#[test]
fn max_bytes_is_checked_before_any_upload_and_names_its_source() {
    let mut e = env(verified());
    let r = go(&mut e, &["--max-bytes", "5", "--dry-run"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(
        r.err.contains("--attach files total 10 bytes and exceed the 5-byte limit (--max-bytes override); nothing was uploaded"),
        "{}",
        r.err
    );
    assert!(r.write_paths().is_empty());
    let r = go(&mut e, &["--max-bytes", "100", "--dry-run"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.json()["attachment_size_limit"]["source"],
        "--max-bytes override"
    );
}
