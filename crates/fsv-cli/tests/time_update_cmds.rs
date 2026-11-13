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
  "plan_hash": "b8585304864383253259663b85791ce7726d1ec04d03e87d926f6882a75d721d",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "556f54acb4309c8784fc30ebb6ea60f57e871a0554be88b3d70fe5a64c156752",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "857c013dc55082b91c8ad75822efd73f088515663b4e908ca96266460f086d62",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "bf99aee3382ce2e1ad8d921e971a72863b42cdd4f9d216df0e9f37db9eaedfc7",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "e876f9c0122e499484eb2af019f44bb32bac1479dd7c6bd8694d55d6aac4849a",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
}
"#
    );
    assert_eq!(r.err, r#""#);
}
