//! reply/add-note --set-status Resolved/Closed --ack (Python 1c9dca7).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: 1c9dca726d789c06203c7ea8af25e86f304d7861

mod common;
use common::run_fixture;

#[test]
fn note_closed_refused() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "add-note",
            "124",
            "--body",
            "Acme",
            "--set-status",
            "Closed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket form validation failed: blocked by rule (acknowledge to continue): Only the requester closes Acme tickets; pass --ack to acknowledge the blocking rule message(s) after reading them
"#
    );
}

#[test]
fn note_closed_ack_review() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "add-note",
            "124",
            "--body",
            "Acme",
            "--set-status",
            "Closed",
            "--ack",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_note",
  "method": "POST",
  "path": "/api/_/tickets/124/notes",
  "payload": {
    "body": "<div>Acme</div>",
    "conversation_from_summary": false,
    "helpdesk_ticket": {
      "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
      "group_id": null,
      "responder_id": 9000001234,
      "status": 5
    },
    "notify_emails": [],
    "private": true
  },
  "status_label": "Closed",
  "plan_hash": "9ea6c0f621941396d071ee589de2c49bf27ef6fcbccb2a5f3c7d4d57f423138b",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration. Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them, and the server or a workflow may still reject or revert the transition.",
  "acknowledged_rule_messages": [
    "Only the requester closes Acme tickets"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn reply_closed_refused() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "reply",
            "124",
            "Acme",
            "--set-status",
            "Closed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket form validation failed: blocked by rule (acknowledge to continue): Only the requester closes Acme tickets; pass --ack to acknowledge the blocking rule message(s) after reading them
"#
    );
}

#[test]
fn reply_closed_ack_review() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "reply",
            "124",
            "Acme",
            "--set-status",
            "Closed",
            "--ack",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_reply",
  "method": "POST",
  "path": "/api/_/tickets/124/reply",
  "payload": {
    "body": "<div>Acme</div>",
    "cc_emails": [],
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "full_text": "<div>Acme</div>",
    "helpdesk_ticket": {
      "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
      "group_id": null,
      "responder_id": 9000001234,
      "status": 5
    },
    "translate_reply": false
  },
  "status_label": "Closed",
  "plan_hash": "d08f33615ff89817a8544323b8744b52d33e69349f7684a015dfc0ee03105d3c",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration. Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them, and the server or a workflow may still reject or revert the transition.",
  "acknowledged_rule_messages": [
    "Only the requester closes Acme tickets"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn reply_ack_alone() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &["tickets", "reply", "124", "Acme", "--ack", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--ack acknowledges blocking rule messages of a --set-status Resolved/Closed transition; pass it together with --set-status
"#
    );
}

#[test]
fn note_ack_alone() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "add-note",
            "124",
            "--body",
            "Acme",
            "--ack",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--ack acknowledges blocking rule messages of a --set-status Resolved/Closed transition; pass it together with --set-status
"#
    );
}

#[test]
fn reply_pending_ack() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "reply",
            "124",
            "Acme",
            "--set-status",
            "Pending",
            "--ack",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--ack applies only with --set-status Resolved or Closed; the captured Pending wire has no blocking-rule acknowledgement
"#
    );
}

#[test]
fn note_pending_ack() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "add-note",
            "124",
            "--body",
            "Acme",
            "--set-status",
            "Pending",
            "--ack",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--ack applies only with --set-status Resolved or Closed; the captured Pending wire has no blocking-rule acknowledgement
"#
    );
}
