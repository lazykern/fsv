//! `changes|problems time update`, problem `time write update` and the
//! non-billable create default against canned routes (Python 2092e44,
//! 68ea4a4; `tests/golden/cmp.py --emit`).

mod common;
use common::run_fixture;

#[test]
fn problems_time_write_update_dry_run() {
    let r = run_fixture(
        "module_time",
        &[
            "problems",
            "time",
            "write",
            "63",
            "update",
            "--entry",
            "75",
            "--note",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problems_time_update",
  "method": "PUT",
  "path": "/api/_/problems/63/time_entries/75",
  "payload": {
    "note": "x",
    "time_spent": "01:30"
  },
  "plan_hash": "9deb72ebbbd2f8fc7bc15fbcb72e57ba3c482ec75d3d99935dd0168ba888b8b1",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_time_update_shortcut_dry_run() {
    let r = run_fixture(
        "module_time",
        &[
            "problems",
            "time",
            "update",
            "PRB-63",
            "75",
            "--spent",
            "00:45",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problems_time_update",
  "method": "PUT",
  "path": "/api/_/problems/63/time_entries/75",
  "payload": {
    "note": "Acme work",
    "time_spent": "00:45"
  },
  "plan_hash": "d311ef34c634b4d2106d8164b0c13fb6111cf960e412fec62fe62b72b47c3cec",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_time_update_shortcut_dry_run() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "update",
            "61",
            "72",
            "--note",
            "Edited",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "changes_time_update",
  "method": "PUT",
  "path": "/api/_/changes/61/time_entries/72",
  "payload": {
    "note": "Edited",
    "time_spent": "01:30"
  },
  "plan_hash": "dbe8529905fe34c05b519b5f7816d14207b0791d82346958537c73f250ad1d69",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_time_update_needs_review() {
    let r = run_fixture(
        "module_time",
        &[
            "problems", "time", "update", "63", "75", "--spent", "00:45", "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"review with --dry-run, then pass --yes and the full --expect-plan hash
"#
    );
}

#[test]
fn changes_time_create_no_billable_dry_run() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "60",
            "create",
            "--spent",
            "00:45",
            "--executed-at",
            "2030-01-02T09:00:00+07:00",
            "--note",
            "Acme deploy",
            "--no-billable",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "changes_time_create",
  "method": "POST",
  "path": "/api/_/changes/60/time_entries",
  "payload": {
    "agent_id": 9,
    "billable": false,
    "executed_at": "2030-01-02T09:00:00+07:00",
    "note": "Acme deploy",
    "time_spent": "00:45",
    "timer_running": false
  },
  "plan_hash": "28d4b788bf14df27996bfa30b3d6f25be7d848e5ca9610032205d6e15e2e7ce1",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_time_create_billable_dry_run() {
    let r = run_fixture(
        "module_time",
        &[
            "problems",
            "time",
            "write",
            "PRB-63",
            "create",
            "--spent",
            "00:01",
            "--executed-at",
            "2026-01-01T09:00:00Z",
            "--billable",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problems_time_create",
  "method": "POST",
  "path": "/api/_/problems/63/time_entries",
  "payload": {
    "agent_id": 9,
    "billable": true,
    "executed_at": "2026-01-01T09:00:00+00:00",
    "note": "",
    "time_spent": "00:01",
    "timer_running": false
  },
  "plan_hash": "1b1646e83f299bde1735e42bee820c753399b4d1eae9e6924145e0e8d197a5f0",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}
