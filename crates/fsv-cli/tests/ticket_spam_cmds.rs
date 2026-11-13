//! Single-ticket spam and unspam (`ticket_spam_cli`).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: 9309b1bc45d42b8a90213e84b5c2679a6b357267

mod common;
use common::run_fixture;

#[test]
fn tickets_spam_dry_run() {
    let r = run_fixture(
        "ticket_spam",
        &["tickets", "spam", "INC-32", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_spam",
  "method": "PUT",
  "path": "/api/_/tickets/32/spam",
  "payload": {},
  "plan_hash": "159c07580a9b9c6d48255cfc60c2ec7a0e5a4a6cd33b18e199fd1a3c2dbbfece",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_spam_needs_confirm() {
    let r = run_fixture(
        "ticket_spam",
        &[
            "tickets",
            "spam",
            "32",
            "--yes",
            "--expect-plan",
            "159c07580a9b9c6d48255cfc60c2ec7a0e5a4a6cd33b18e199fd1a3c2dbbfece",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"spam hides the ticket from every view; pass --confirm-destructive after review
"#
    );
}

#[test]
fn tickets_spam_wrong_hash() {
    let r = run_fixture(
        "ticket_spam",
        &[
            "tickets",
            "spam",
            "32",
            "--yes",
            "--expect-plan",
            "abc",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"review with --dry-run, then --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn tickets_spam_send() {
    let r = run_fixture(
        "ticket_spam",
        &[
            "tickets",
            "spam",
            "32",
            "--yes",
            "--expect-plan",
            "159c07580a9b9c6d48255cfc60c2ec7a0e5a4a6cd33b18e199fd1a3c2dbbfece",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "159c07580a9b9c6d48255cfc60c2ec7a0e5a4a6cd33b18e199fd1a3c2dbbfece",
  "verified": true,
  "ticket_id": 32,
  "spam": true
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/32/spam"]);
}

#[test]
fn tickets_spam_send_unverified() {
    let r = run_fixture(
        "ticket_spam",
        &[
            "tickets",
            "spam",
            "36",
            "--yes",
            "--expect-plan",
            "e9ea55ed431b681408ac7ec6e6bf1754aef36a311f89ea6c4a2b0666cf8958cb",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e9ea55ed431b681408ac7ec6e6bf1754aef36a311f89ea6c4a2b0666cf8958cb",
  "verified": false,
  "ticket_id": 36,
  "spam": false,
  "warning": "Readback was not fully verified; do not resend."
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/36/spam"]);
}

#[test]
fn tickets_spam_rejected() {
    let r = run_fixture(
        "ticket_spam",
        &[
            "tickets",
            "spam",
            "37",
            "--yes",
            "--expect-plan",
            "57dbc1cff8ef95049553b7620ec3f39c62b4f32b584f25565d8256a13b449a10",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"write rejected (HTTP 422): synthetic; no retry
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/37/spam"]);
}

#[test]
fn tickets_spam_already() {
    let r = run_fixture("ticket_spam", &["tickets", "spam", "33", "--dry-run"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 33 is already marked as spam; nothing was sent
"#
    );
}

#[test]
fn tickets_spam_in_trash() {
    let r = run_fixture("ticket_spam", &["tickets", "spam", "34", "--dry-run"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 34 is in the trash; restore it first; nothing was sent
"#
    );
}

#[test]
fn tickets_spam_flag_unavailable() {
    let r = run_fixture("ticket_spam", &["tickets", "spam", "35", "--dry-run"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 35: spam flag unavailable; nothing was sent
"#
    );
}

#[test]
fn tickets_spam_not_found() {
    let r = run_fixture("ticket_spam", &["tickets", "spam", "99", "--dry-run"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 99 not found (HTTP 404); nothing was sent
"#
    );
}

#[test]
fn tickets_spam_read_failed() {
    let r = run_fixture("ticket_spam", &["tickets", "spam", "38", "--dry-run"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read failed (HTTP 500); nothing was sent
"#
    );
}

#[test]
fn tickets_unspam_dry_run() {
    let r = run_fixture(
        "ticket_spam",
        &["tickets", "unspam", "33", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_unspam",
  "method": "PUT",
  "path": "/api/_/tickets/33/unspam",
  "payload": null,
  "plan_hash": "d3c1e160c6dce39231a4b892a5e4c7694604e534746f7a13ce44f1e2551f0885",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_unspam_not_marked() {
    let r = run_fixture(
        "ticket_spam",
        &["tickets", "unspam", "32", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 32 is not marked as spam; nothing was sent
"#
    );
}

#[test]
fn tickets_unspam_send() {
    let r = run_fixture(
        "ticket_spam",
        &[
            "tickets",
            "unspam",
            "33",
            "--yes",
            "--expect-plan",
            "d3c1e160c6dce39231a4b892a5e4c7694604e534746f7a13ce44f1e2551f0885",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d3c1e160c6dce39231a4b892a5e4c7694604e534746f7a13ce44f1e2551f0885",
  "verified": true,
  "ticket_id": 33,
  "spam": false
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/33/unspam"]);
}
