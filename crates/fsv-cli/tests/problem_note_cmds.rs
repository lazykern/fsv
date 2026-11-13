//! Legacy problem add-note sends the captured body only (Python e80a922).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: e80a92200bdd87666359ec6c21ab437351d05209

mod common;
use common::run_fixture;

#[test]
fn problem_note_sends_body_only() {
    let r = run_fixture(
        "problem_add_note",
        &["problems", "add-note", "PRB-12", "Acme evidence", "--yes"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added note 9000000077 (visibility: tenant default; not controllable on problem notes)
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/12/notes"]);
}

#[test]
fn problem_note_dry_run_states_visibility() {
    let r = run_fixture(
        "problem_add_note",
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
  "dry_run": true,
  "action": "add_note",
  "id": "PRB-12",
  "body": {
    "body": "Acme evidence"
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
fn problem_note_public_refused() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "problems", "add-note", "PRB-12", "Acme", "--public", "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --public: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies
"#
    );
}

#[test]
fn problem_note_public_dry_run_refused() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme",
            "--public",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --public: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies
"#
    );
}

#[test]
fn change_note_keeps_visibility_key() {
    let r = run_fixture(
        "problem_add_note",
        &["changes", "add-note", "CHN-3", "Acme change note"],
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
fn change_note_dry_run() {
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
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_public_note_dry_run() {
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
  "requires_confirmation": [
    "public note: visible to the requester and may send notifications"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn note_bodies_match_the_captured_shapes() {
    let r = run_fixture(
        "problem_add_note",
        &["problems", "add-note", "PRB-12", "Acme evidence", "--yes"],
        None,
    );
    assert_eq!(
        r.writes()[0].2,
        serde_json::json!({"body": "Acme evidence"})
    );
    let r = run_fixture(
        "problem_add_note",
        &["changes", "add-note", "CHN-3", "Acme change note"],
        None,
    );
    assert_eq!(
        r.writes()[0].2,
        serde_json::json!({"body": "Acme change note", "private": true})
    );
}
