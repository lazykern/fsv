// Python commit: 01a88d41e82448c1585f0e9f68e9e03e9b471834
//! Upstream commands ported by the up-b batch: reviewed problem notes,
//! change note hashes, plain reply, legacy ticket update hash, record-state
//! tokens and replay refusals. Emitted by cmp.py from the Python oracle named
//! above (31a9103 plus the up-b commits).

mod common;
#[allow(unused_imports)]
use common::{Opts, run_files, run_fixture, run_steps, run_with};

#[test]
fn problem_note_reviewed_dry_run() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme evidence",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "dry_run": true,
  "action": "problem_note",
  "method": "POST",
  "path": "/api/_/problems/12/notes",
  "payload": {
    "body": "Acme evidence"
  },
  "plan_hash": "877fc29c38d1e3bfeb2dccbef2b3222ac2841135186649381a11e0cf446d8c50",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "id": "PRB-12",
  "body": {
    "body": "Acme evidence"
  },
  "record_state": {
    "sends": 0
  },
  "warnings": [
    "visibility: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_note_reviewed_send_json() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme evidence",
            "--yes",
            "--expect-plan",
            "877fc29c38d1e3bfeb2dccbef2b3222ac2841135186649381a11e0cf446d8c50",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "877fc29c38d1e3bfeb2dccbef2b3222ac2841135186649381a11e0cf446d8c50",
  "verified": true,
  "note_id": 9000000077,
  "readback": {
    "id": 9000000077,
    "body": "<div>Acme evidence</div>"
  },
  "identity_verified": true,
  "readback_complete": true,
  "fields": {
    "body": "matched"
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/12/notes"]);
}

#[test]
fn problem_note_reviewed_send_human() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme evidence",
            "--yes",
            "--expect-plan",
            "877fc29c38d1e3bfeb2dccbef2b3222ac2841135186649381a11e0cf446d8c50",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added note 9000000077 on problem 12 (visibility: tenant default; not controllable on problem notes); verified: true
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/12/notes"]);
}

#[test]
fn problem_note_yes_needs_expect_plan() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-12", "Acme", "--yes"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--yes requires the full --expect-plan from a prior --dry-run review
"#
    );
}

#[test]
fn problem_note_wrong_hash() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme",
            "--yes",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"note review hash does not match; review the exact body again
"#
    );
}

#[test]
fn problem_note_no_input() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-12", "Acme", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"sending requires --yes and --expect-plan in no-input mode
"#
    );
}

#[test]
fn problem_note_public_refused() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems", "add-note", "PRB-12", "Acme", "--public", "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--public: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies
"#
    );
}

#[test]
fn problem_note_empty_body() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-12", "  ", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"note body must be nonempty text without NUL characters
"#
    );
}

#[test]
fn problem_note_bad_id() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-0", "x", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"problem ID must be positive
"#
    );
}

#[test]
fn problem_note_prompt_declined() {
    let r = run_with(
        "problem_note_reviewed",
        &[],
        &["problems", "add-note", "PRB-12", "Acme"],
        Some("n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Add this exact reviewed note? [y/N]: Cancelled; no note was sent.
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "problem_note",
  "method": "POST",
  "path": "/api/_/problems/12/notes",
  "payload": {
    "body": "Acme"
  },
  "plan_hash": "7c6a4c9eb87ad2bcd791a5fab236dc2bef142217be64c740076509a351b4d041",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "id": "PRB-12",
  "body": {
    "body": "Acme"
  },
  "record_state": {
    "sends": 0
  },
  "warnings": [
    "visibility: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies"
  ]
}
"#
    );
}

#[test]
fn problem_note_prompt_accepted() {
    let r = run_with(
        "problem_note_reviewed",
        &[],
        &["problems", "add-note", "PRB-12", "Acme evidence"],
        Some("y"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Add this exact reviewed note? [y/N]: added note 9000000077 on problem 12 (visibility: tenant default; not controllable on problem notes); verified: true
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "problem_note",
  "method": "POST",
  "path": "/api/_/problems/12/notes",
  "payload": {
    "body": "Acme evidence"
  },
  "plan_hash": "877fc29c38d1e3bfeb2dccbef2b3222ac2841135186649381a11e0cf446d8c50",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "id": "PRB-12",
  "body": {
    "body": "Acme evidence"
  },
  "record_state": {
    "sends": 0
  },
  "warnings": [
    "visibility: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies"
  ]
}
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/problems/12/notes"]);
}
#[test]
fn change_note_dry_run_hash() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "add_note",
  "id": "CHN-3",
  "body": {
    "body": "Acme change note",
    "private": true
  },
  "plan_hash": "8bc603b9b1eb949c82f5c7dc44b8c6020d04ee493c346c1c328484f3bd338a9c"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_public_note_dry_run_hash() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--public",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "add_note",
  "id": "CHN-3",
  "body": {
    "body": "Acme change note",
    "private": false
  },
  "plan_hash": "dffb44720a319259485eb90f0e8bbffaec5c60ec03ac48e96afe08f90e04da17",
  "requires_confirmation": [
    "public note: visible to the requester and may send notifications"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_note_expect_plan_sends() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--expect-plan",
            "8bc603b9b1eb949c82f5c7dc44b8c6020d04ee493c346c1c328484f3bd338a9c",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added private note 9000000078
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes/3/notes"]);
}

#[test]
fn change_note_expect_plan_uppercase_ok() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--expect-plan",
            "8BC603B9B1EB949C82F5C7DC44B8C6020D04EE493C346C1C328484F3BD338A9C",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added private note 9000000078
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes/3/notes"]);
}

#[test]
fn change_note_expect_plan_edited_body() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note (edited)",
            "--expect-plan",
            "8bc603b9b1eb949c82f5c7dc44b8c6020d04ee493c346c1c328484f3bd338a9c",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3 does not match the reviewed plan (body, visibility or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn change_note_expect_plan_visibility() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--public",
            "--yes",
            "--expect-plan",
            "8bc603b9b1eb949c82f5c7dc44b8c6020d04ee493c346c1c328484f3bd338a9c",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3 does not match the reviewed plan (body, visibility or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn change_note_expect_plan_prefix() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--expect-plan",
            "8bc603b9b1eb949c",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3 does not match the reviewed plan (body, visibility or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn change_note_no_expect_plan_private() {
    let r = run_fixture(
        "problem_add_note",
        &["changes", "add-note", "CHN-3", "private words"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added private note 9000000078
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes/3/notes"]);
}

#[test]
fn change_note_public_needs_yes() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "public words",
            "--public",
            "--no-input",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3: public note: visible to the requester and may send notifications. Pass --yes to confirm.
"#
    );
}

#[test]
fn change_note_help_documents_expect_plan() {
    let r = run_fixture("problem_add_note", &["changes", "add-note", "-h"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("--expect-plan <TEXT>"));
    assert!(!r.out.contains("unhashed"));
}

#[test]
fn problem_note_help_lists_the_reviewed_flags() {
    let r = run_fixture("problem_add_note", &["problems", "add-note", "-h"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    for flag in [
        "--expect-plan",
        "--json",
        "--dry-run",
        "--yes",
        "--no-input",
        "--public",
    ] {
        assert!(r.out.contains(flag), "{flag}: {}", r.out);
    }
}
