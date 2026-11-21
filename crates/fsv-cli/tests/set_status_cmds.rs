//! reply/add-note --set-status refusals list the valid choices (Python e15e02d).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: e15e02dfa5a0b56cc71c08ecd35f174b10a993a9

mod common;
use common::run_fixture;

#[test]
fn reply_set_status_invalid_choice_lists_choices() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "reply",
            "124",
            "Hello",
            "--set-status",
            "Bogus",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set-status 'Bogus' is not a supported status; choices: Pending, Resolved, Closed (or the tenant's own label of one of these): status 'Bogus' is not a runtime status label; choices: Open, Pending, Resolved, Closed; nothing was sent
"#
    );
}

#[test]
fn note_set_status_invalid_choice_lists_choices() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "add-note",
            "124",
            "--body",
            "Private",
            "--set-status",
            "Bogus",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set-status 'Bogus' is not a supported status; choices: Pending, Resolved, Closed (or the tenant's own label of one of these): status 'Bogus' is not a runtime status label; choices: Open, Pending, Resolved, Closed; nothing was sent
"#
    );
}

#[test]
fn reply_set_status_label_is_stripped() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "reply",
            "124",
            "Hello",
            "--set-status",
            " pending ",
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
    "body": "<div>Hello</div>",
    "cc_emails": [],
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "full_text": "<div>Hello</div>",
    "helpdesk_ticket": {
      "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
      "status": 3
    },
    "translate_reply": false
  },
  "status_label": " pending ",
  "plan_hash": "89b0abfa75def9566d1edac0684a0ab70baf8e6d8cbb0138626cb1a91873e757",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration.",
  "record_state": {
    "sends": 0
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn note_set_status_closed_label_is_stripped() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "add-note",
            "124",
            "--body",
            "Private",
            "--set-status",
            " Closed",
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
    "body": "<div>Private</div>",
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
  "status_label": " Closed",
  "plan_hash": "e2f7a0aee34b1904214a70b7c9d3922e2e886cebbdad5265716545b2d3820551",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration. Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them, and the server or a workflow may still reject or revert the transition.",
  "record_state": {
    "sends": 0
  },
  "acknowledged_rule_messages": [
    "Only the requester closes Acme tickets"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}
