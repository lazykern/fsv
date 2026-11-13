//! `changes|problems time update` and problem `time write update` against
//! canned routes (Python 2092e44, `tests/golden/cmp.py --emit`).

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
