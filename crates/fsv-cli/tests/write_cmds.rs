//! Write commands against canned routes. Expected output was captured from
//! the Python fsv run on the same routes (`tests/golden/cmp.py --emit`).

mod common;
use common::run_fixture;

#[test]
fn update_bad_status() {
    let r = run_fixture(
        "ticket_update",
        &["tickets", "update", "5", "--status", "bogus"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: unknown status: 'bogus'; choices: Open, Resolved, Closed
"#
    );
}

#[test]
fn update_set_fields_dry_run() {
    let r = run_fixture(
        "ticket_update",
        &[
            "tickets",
            "update",
            "5",
            "--set",
            "env=uat",
            "--set",
            "approved=yes",
            "--group",
            "dba",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "SR-5",
  "body": {
    "group_id": 9,
    "custom_fields": {
      "cf_env": "UAT",
      "cf_ok": true
    }
  },
  "requires_confirmation": [
    "reassign group (group_id=9)"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn update_ambiguous_group() {
    let r = run_fixture(
        "ticket_update",
        &["tickets", "update", "5", "--group", "ops", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "SR-5",
  "body": {
    "group_id": 7
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn update_status_yes() {
    let r = run_fixture(
        "ticket_update",
        &["tickets", "update", "5", "--status", "4", "--yes"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"updated SR-5
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/5"]);
}

#[test]
fn update_priority_json() {
    let r = run_fixture(
        "ticket_update",
        &[
            "tickets",
            "update",
            "5",
            "--priority",
            "low",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 5,
  "status": 4,
  "human_display_id": "SR-5"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/5"]);
}

#[test]
fn update_unknown_set_field() {
    let r = run_fixture(
        "ticket_update",
        &["tickets", "update", "5", "--set", "nope=1"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: field not found: 'nope'; run `fsv changes fields`
"#
    );
}

#[test]
fn update_nothing() {
    let r = run_fixture("ticket_update", &["tickets", "update", "5"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: nothing to update; pass at least one field (--status, --priority, --agent, --group, --set FIELD=VALUE)
"#
    );
}

#[test]
fn update_props_mixed() {
    let r = run_fixture(
        "ticket_update",
        &["tickets", "update", "5", "--tag", "a", "--status", "2"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket read did not include: tags
"#
    );
}

#[test]
fn update_problem_api_error() {
    let r = run_fixture(
        "ticket_update",
        &[
            "problems",
            "update",
            "5",
            "--priority",
            "2",
            "--set",
            "cf_env=Prod",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 404 listing problem form fields: no route
"#
    );
}

#[test]
fn task_delete_yes() {
    let r = run_fixture(
        "downloads",
        &["changes", "tasks-delete", "5", "3", "--yes"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: complete task inventory unavailable
"#
    );
}

#[test]
fn task_delete_needs_yes() {
    let r = run_fixture("downloads", &["changes", "tasks-delete", "5", "3"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: complete task inventory unavailable
"#
    );
}

#[test]
fn task_add_status_deleted() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "tasks-add",
            "5",
            "--title",
            "Do it",
            "--status",
            "Old",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Old requires an exact runtime task-status metadata ID; no ID inferred and no POST attempted
"#
    );
}

#[test]
fn task_add_empty_title() {
    let r = run_fixture(
        "review",
        &["changes", "tasks-add", "5", "--title", "", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"task title must be nonempty text without control characters
"#
    );
}

#[test]
fn task_add_read_only() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "tasks-add",
            "5",
            "--title",
            "Do it",
            "--read-only",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read-only mode: no task was sent
"#
    );
}

#[test]
fn task_add_deactivated_owner() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "tasks-add",
            "5",
            "--title",
            "Do it",
            "--owner",
            "bob@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"owner lookup failed: agent must match exactly one active entry with a user ID; 0 exact active candidates; use an explicit user ID if unavailable
"#
    );
}

#[test]
fn approval_request_dry_run() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-request",
            "5",
            "--name",
            "CAB",
            "--message",
            "pls\nok",
            "--approver",
            "31",
            "--approver",
            "ann@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"duplicate approver; specify each agent exactly once
"#
    );
}

#[test]
fn approval_request_deactivated() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-request",
            "5",
            "--name",
            "CAB",
            "--message",
            "hi",
            "--approver",
            "bob@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"approver email must resolve to exactly one active agent ID; no POST attempted; use an explicit agent ID if lookup is unavailable
"#
    );
}

#[test]
fn approval_request_duplicate() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-request",
            "5",
            "--name",
            "CAB",
            "--message",
            "hi",
            "--approver",
            "31",
            "--approver",
            "31",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"duplicate approver; specify each agent exactly once
"#
    );
}

#[test]
fn approval_request_bad_rule() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-request",
            "5",
            "--name",
            "CAB",
            "--message",
            "hi",
            "--approver",
            "31",
            "--rule",
            "bad",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"rule must be everyone, anyone, majority or first-responder
"#
    );
}

#[test]
fn approval_request_no_input() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-request",
            "5",
            "--name",
            "CAB",
            "--message",
            "hi",
            "--approver",
            "31",
            "--no-input",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_approval_request",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/approval-groups",
  "payload": {
    "approval_groups": [
      {
        "name": "CAB",
        "approval_type": 1,
        "email_content": "<p dir=\"ltr\">hi</p>",
        "notified_to": [
          31
        ]
      }
    ]
  },
  "recipients": [
    31
  ],
  "approval_rule": "Everyone",
  "message_text": "hi",
  "plan_hash": "c920ebf2e43cd003d49aeff9cb60ab8811aa6820896ef977e0d8bebca81758a1",
  "warning": "Approval emails are irreversible; cancel/revoke cannot recall them. This requests approval; it does not approve the change. Tenant workflows may also notify people.",
  "recipient_sources": [],
  "verification": "Complete before/after approval record IDs, exact new recipients, requested status and exact selected rule ID. When complete chains identify exactly one new matching group, verify its exact name and ID too.",
  "contract": "Per-member approvals and named approval-chain groups observed; POST response not trusted."
}
sending requires --yes and --expect-plan in no-input mode
"#
    );
}

#[test]
fn approval_request_send_unverified() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-request",
            "5",
            "--name",
            "CAB",
            "--message",
            "hi",
            "--approver",
            "31",
            "--yes",
            "--expect-plan",
            "c920ebf2e43cd003d49aeff9cb60ab8811aa6820896ef977e0d8bebca81758a1",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_approval_request",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/approval-groups",
  "payload": {
    "approval_groups": [
      {
        "name": "CAB",
        "approval_type": 1,
        "email_content": "<p dir=\"ltr\">hi</p>",
        "notified_to": [
          31
        ]
      }
    ]
  },
  "recipients": [
    31
  ],
  "approval_rule": "Everyone",
  "message_text": "hi",
  "plan_hash": "c920ebf2e43cd003d49aeff9cb60ab8811aa6820896ef977e0d8bebca81758a1",
  "warning": "Approval emails are irreversible; cancel/revoke cannot recall them. This requests approval; it does not approve the change. Tenant workflows may also notify people.",
  "recipient_sources": [],
  "verification": "Complete before/after approval record IDs, exact new recipients, requested status and exact selected rule ID. When complete chains identify exactly one new matching group, verify its exact name and ID too.",
  "contract": "Per-member approvals and named approval-chain groups observed; POST response not trusted."
}
approval request outcome unknown; inspect the change and journal before any retry; do not resubmit
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/changes/5/approval-groups"]);
}

#[test]
fn approval_respond_both() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-respond",
            "5",
            "70",
            "--approve",
            "--reject",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"choose exactly one of --approve or --reject
"#
    );
}

#[test]
fn approval_reject_needs_remark() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-respond",
            "5",
            "70",
            "--reject",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"reject requires a nonempty --remark
"#
    );
}

#[test]
fn approval_approve_dry_run() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-respond",
            "5",
            "70",
            "--approve",
            "--remark",
            "ok",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_approval_respond",
  "operation": "approve",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "PUT",
  "path": "/api/_/changes/5/approvals/70",
  "payload": {
    "approval_status": 1,
    "remark": "ok"
  },
  "body_present": true,
  "plan_hash": "63aa95e626fb54e0d3a8da12d869adb7b273a76df0476cbc2689249f03092baf",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Approve accepted live (requested=0, approved=1); reject captured aborted (read-back mapping unobserved).",
  "verification": "Complete approvals list; exact record ID and changed raw status. Named decision checked if exposed; no numeric mapping inferred.",
  "approval_id": 70
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn approval_respond_read_only() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-respond",
            "5",
            "70",
            "--approve",
            "--read-only",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read-only mode: no approval action was sent
"#
    );
}

#[test]
fn approval_respond_send_unknown() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-respond",
            "5",
            "70",
            "--approve",
            "--remark",
            "ok",
            "--yes",
            "--expect-plan",
            "63aa95e626fb54e0d3a8da12d869adb7b273a76df0476cbc2689249f03092baf",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_approval_respond",
  "operation": "approve",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "PUT",
  "path": "/api/_/changes/5/approvals/70",
  "payload": {
    "approval_status": 1,
    "remark": "ok"
  },
  "body_present": true,
  "plan_hash": "63aa95e626fb54e0d3a8da12d869adb7b273a76df0476cbc2689249f03092baf",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Approve accepted live (requested=0, approved=1); reject captured aborted (read-back mapping unobserved).",
  "verification": "Complete approvals list; exact record ID and changed raw status. Named decision checked if exposed; no numeric mapping inferred.",
  "approval_id": 70
}
approval action outcome unknown; inspect the change and journal before any retry; do not resubmit
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/5/approvals/70"]);
}

#[test]
fn approval_cancel_dry_run() {
    let r = run_fixture(
        "review",
        &["changes", "approval-cancel", "5", "3", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_approval_cancel",
  "operation": "cancel",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "PUT",
  "path": "/api/_/changes/5/approval-groups/3/cancel",
  "payload": null,
  "body_present": false,
  "plan_hash": "c9df12f50a2da842b04a3ac0bd10c85133cffd76a4b013c1adf3ada3957bee71",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Bodyless JSON-media-type cancel accepted live; chains exposed cancelled status {id:3,name:cancelled}.",
  "verification": "Complete approval chains: exact group disappears or all original approvals change away from requested; report raw statuses.",
  "group_id": 3,
  "send_enabled": true,
  "group": {
    "id": 3,
    "name": "CAB",
    "approval_status": 0,
    "approvals": [
      {
        "id": 70,
        "member_id": 31,
        "status": {
          "id": 0,
          "name": "requested"
        }
      }
    ]
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn approval_cancel_hash_mismatch() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-cancel",
            "5",
            "4",
            "--no-input",
            "--yes",
            "--expect-plan",
            "x",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"approval action review hash does not match; review the exact action again
"#
    );
}

#[test]
fn approval_cancel_no_requested() {
    let r = run_fixture("review", &["changes", "approval-cancel", "5", "4"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"approval group has no requested approval records; no cancel PUT attempted
"#
    );
}

#[test]
fn approval_cancel_missing_group() {
    let r = run_fixture("review", &["changes", "approval-cancel", "5", "9"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"approval group missing from complete chains inventory; no cancel PUT attempted
"#
    );
}

#[test]
fn approval_cancel_confirm_eof() {
    let r = run_fixture("review", &["changes", "approval-cancel", "5", "3"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Send this exact approval action (notifies people; may move workflow; cancel irreversible)? [y/N]: "#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "change_approval_cancel",
  "operation": "cancel",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "PUT",
  "path": "/api/_/changes/5/approval-groups/3/cancel",
  "payload": null,
  "body_present": false,
  "plan_hash": "c9df12f50a2da842b04a3ac0bd10c85133cffd76a4b013c1adf3ada3957bee71",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Bodyless JSON-media-type cancel accepted live; chains exposed cancelled status {id:3,name:cancelled}.",
  "verification": "Complete approval chains: exact group disappears or all original approvals change away from requested; report raw statuses.",
  "group_id": 3,
  "send_enabled": true,
  "group": {
    "id": 3,
    "name": "CAB",
    "approval_status": 0,
    "approvals": [
      {
        "id": 70,
        "member_id": 31,
        "status": {
          "id": 0,
          "name": "requested"
        }
      }
    ]
  }
}
Aborted.
"#
    );
}

#[test]
fn approval_cancel_send() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "approval-cancel",
            "5",
            "3",
            "--yes",
            "--expect-plan",
            "c9df12f50a2da842b04a3ac0bd10c85133cffd76a4b013c1adf3ada3957bee71",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_approval_cancel",
  "operation": "cancel",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "PUT",
  "path": "/api/_/changes/5/approval-groups/3/cancel",
  "payload": null,
  "body_present": false,
  "plan_hash": "c9df12f50a2da842b04a3ac0bd10c85133cffd76a4b013c1adf3ada3957bee71",
  "warning": "Approval actions notify people; approval/rejection may move the change workflow. Cancellation is irreversible. Review the exact tenant and identifiers.",
  "contract": "Bodyless JSON-media-type cancel accepted live; chains exposed cancelled status {id:3,name:cancelled}.",
  "verification": "Complete approval chains: exact group disappears or all original approvals change away from requested; report raw statuses.",
  "group_id": 3,
  "send_enabled": true,
  "group": {
    "id": 3,
    "name": "CAB",
    "approval_status": 0,
    "approvals": [
      {
        "id": 70,
        "member_id": 31,
        "status": {
          "id": 0,
          "name": "requested"
        }
      }
    ]
  }
}
approval action outcome unknown; inspect the change and journal before any retry; do not resubmit
"#
    );
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/changes/5/approval-groups/3/cancel"]
    );
}

#[test]
fn task_update_nothing() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "tasks-update", "5", "3"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"provide captured task fields; unsupported fields cannot be sent
"#
    );
}

#[test]
fn task_update_title_dry_run() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--title",
            "New",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_status_dry_run() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--status",
            "completed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_many_dry_run() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--status",
            "2",
            "--group",
            "DBA",
            "--agent",
            "131",
            "--system",
            "sap",
            "--due-date",
            "2026-01-01",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--edit/--due-date are not captured task-update contracts; use explicit planned dates and reviewed flags
"#
    );
}

#[test]
fn task_update_bad_system() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--system",
            "nope",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_bad_status() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--status",
            "bogus",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_ambiguous_agent() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--agent",
            "ann",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_needs_yes() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "tasks-update", "5", "3", "--status", "Completed"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_status_yes() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--status",
            "Completed",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_json() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--title",
            "New",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete task inventory unavailable
"#
    );
}

#[test]
fn task_update_edit_no_input() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "tasks-update", "5", "3", "--edit", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--edit/--due-date are not captured task-update contracts; use explicit planned dates and reviewed flags
"#
    );
}

#[test]
fn clone_dry_run() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "clone", "7", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "subject": "Patch",
  "status": 1,
  "priority": 2,
  "description": "<p>x</p>",
  "cf_env": "prod"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn clone_tasks_needs_yes() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "clone", "7", "--no-edit", "--with-tasks"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: clone of CHN-7: --with-tasks: creates tasks that can notify their groups and trigger tenant workflows. Pass --yes to confirm.
"#
    );
}

#[test]
fn clone_editor_no_input() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "clone", "7", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: editor disabled by --no-input; use --dry-run/--dry or provide non-interactive flags
"#
    );
}

#[test]
fn update_due_by_dry_run() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "update",
            "5",
            "--due-by",
            "2026-06-09T18:00:00+07:00",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "set_due_by",
  "id": "CHN-5",
  "body": {
    "due_by": "2026-06-09T18:00:00+07:00"
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn update_due_by() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "update",
            "5",
            "--due-by",
            "2026-06-09T18:00:00+07:00",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"updated #5 resolution due → 2026-06-09T18:00:00+07:00
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PATCH /api/_/changes/5"]);
}

#[test]
fn update_planning_dry_run() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "update",
            "5",
            "--planning",
            "Rollout plan",
            "--description",
            "hi",
            "--file",
            "x.pdf",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update_planning_field",
  "id": "CHN-5",
  "body": {
    "planning_field": "rollout_plan",
    "description": "hi",
    "files": [
      "x.pdf"
    ],
    "duplicate": "prompt"
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn update_planning_unknown() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "update", "5", "--planning", "nope", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: planning field not found: 'nope'; choices: Rollout plan, Backout plan
"#
    );
}

#[test]
fn update_attach_dry_run() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "update",
            "5",
            "--attach",
            "a.txt",
            "--dry-run",
            "--no-input",
            "--duplicate",
            "skip",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "CHN-5",
  "body": {
    "attachments": {
      "preserve_existing": true,
      "upload_files": [
        "a.txt"
      ],
      "duplicate": "skip"
    }
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn update_attach_prompt_no_input() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "update", "5", "--attach", "a.txt", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --duplicate prompt disabled by --no-input; use --duplicate skip|replace|append
"#
    );
}

#[test]
fn update_two_modes() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "update", "5", "--due-by", "x", "--planning", "y"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: choose one update mode: --due-by, --planning, --edit, or field/attachment flags
"#
    );
}

#[test]
fn update_nothing_no_input() {
    let r = run_fixture(
        "change_helpers",
        &["changes", "update", "5", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: nothing to update; use --set, --from, --attach, --planning, --edit, or pass --status/--priority/--agent/--group (in a terminal, no flags opens the edit form)
"#
    );
}

#[test]
fn clone_dry_run_side_effects() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "clone",
            "7",
            "--dry-run",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "subject": "Patch",
  "status": 1,
  "priority": 2,
  "description": "<p>x</p>",
  "cf_env": "prod"
}
"#
    );
    assert_eq!(
        r.err,
        r#"side effects if run without --dry-run:
  - 2 task(s) created (1 with an owner; owners/past dates dropped)
  - tenant workflows may add their own tasks to the new change
  - 2 asset(s) associated
  - 1 planning field(s) copied, 1 attachment(s) re-uploaded
"#
    );
}

#[test]
fn clone_with_tasks_step_failed() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "clone",
            "7",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(
        r.out,
        r#"created CHN-8  https://acme.freshservice.com/a/changes/8
"#
    );
    assert_eq!(
        r.err,
        r#"  cloned 2 task(s)
  note: task 't1': owner not copied
  note: task 't2': past due_date not copied
  note: tenant workflows may have added their own tasks; compare with the source before relying on the copy
step failed: assets: HTTP 500 updating associate: nope
error: CHN-8 WAS created but 1 step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on CHN-8 directly.
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/changes",
            "POST /api/_/changes/8/tasks",
            "POST /api/_/changes/8/tasks",
            "PUT /api/_/changes/8/assets/associate"
        ]
    );
}

#[test]
fn clone_json_step_failed() {
    let r = run_fixture(
        "change_helpers",
        &[
            "changes",
            "clone",
            "7",
            "--no-edit",
            "--with-assets",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 8,
  "human_display_id": "CHN-8",
  "fsv_clone_steps": {
    "assets": "failed: HTTP 500 updating associate: nope"
  }
}
"#
    );
    assert_eq!(
        r.err,
        r#"created CHN-8
step failed: assets: HTTP 500 updating associate: nope
error: CHN-8 WAS created but 1 step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on CHN-8 directly.
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/changes",
            "PUT /api/_/changes/8/assets/associate"
        ]
    );
}

#[test]
fn associations_add_dry_run() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "add",
            "6",
            "SR-100",
            "101",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_ticket_association",
  "tenant": "acme.freshservice.com",
  "change_id": 6,
  "ticket_ids": [
    100,
    101
  ],
  "method": "PUT",
  "path": "/api/_/changes/6/tickets/associate",
  "payload": {
    "item_ids": [
      100,
      101
    ],
    "module_type": "ticket"
  },
  "params": null,
  "plan_hash": "f1f0aa0b927f4458a489fa58fd938954e09f2c12ec4d795ac7a5f7b1c05a5547",
  "warning": "Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn associations_add_subject_dry_run() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "add",
            "6",
            "disk full",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_ticket_association",
  "tenant": "acme.freshservice.com",
  "change_id": 6,
  "ticket_ids": [
    100
  ],
  "method": "PUT",
  "path": "/api/_/changes/6/tickets/associate",
  "payload": {
    "item_ids": [
      100
    ],
    "module_type": "ticket"
  },
  "params": null,
  "plan_hash": "feb8984cef848da9017d51abf14ee3b152733cfa31239f78bb0a3efc948e6bcd",
  "warning": "Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn associations_add_ambiguous() {
    let r = run_fixture(
        "links",
        &["changes", "associations", "add", "6", "disk", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: multiple ticket matches for 'disk': SR-100, INC-101
"#
    );
}

#[test]
fn associations_add_read_only() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "add",
            "6",
            "SR-100",
            "--read-only",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: read-only mode: no association request was sent
"#
    );
}

#[test]
fn associations_add_no_input() {
    let r = run_fixture(
        "links",
        &["changes", "associations", "add", "6", "SR-100"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_ticket_association",
  "tenant": "acme.freshservice.com",
  "change_id": 6,
  "ticket_ids": [
    100
  ],
  "method": "PUT",
  "path": "/api/_/changes/6/tickets/associate",
  "payload": {
    "item_ids": [
      100
    ],
    "module_type": "ticket"
  },
  "params": null,
  "plan_hash": "feb8984cef848da9017d51abf14ee3b152733cfa31239f78bb0a3efc948e6bcd",
  "warning": "Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit."
}
error: sending requires --yes and --expect-plan in no-input mode
"#
    );
}

#[test]
fn associations_add_send() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "add",
            "6",
            "SR-100",
            "--yes",
            "--expect-plan",
            "feb8984cef848da9017d51abf14ee3b152733cfa31239f78bb0a3efc948e6bcd",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_ticket_association",
  "tenant": "acme.freshservice.com",
  "change_id": 6,
  "ticket_ids": [
    100
  ],
  "method": "PUT",
  "path": "/api/_/changes/6/tickets/associate",
  "payload": {
    "item_ids": [
      100
    ],
    "module_type": "ticket"
  },
  "params": null,
  "plan_hash": "feb8984cef848da9017d51abf14ee3b152733cfa31239f78bb0a3efc948e6bcd",
  "warning": "Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit."
}
error: association outcome unknown/unverified; inspect the change and journal; do not resubmit
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/6/tickets/associate"]);
}

#[test]
fn associations_remove_two() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "remove",
            "5",
            "SR-100",
            "INC-1",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: unlink accepts one ticket per reviewed request; review each removal separately
"#
    );
}

#[test]
fn associations_remove_subject_dry_run() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "remove",
            "5",
            "disk full",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_ticket_association",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "ticket_ids": [
    100
  ],
  "method": "DELETE",
  "path": "/api/_/changes/5/tickets/100",
  "payload": {
    "type": "unlink"
  },
  "params": {
    "module_type": "ticket"
  },
  "plan_hash": "f380f90a5980d60d3b25e238fecdb3e810630e5cc13d8a0ada20eff9f1ca18e6",
  "warning": "Initiating-ticket relationship only. Tenant workflows may notify people. One attempt; never automatically resubmit."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn associations_remove_unknown() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "remove",
            "5",
            "nope",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: ticket not associated on change #5: 'nope'
"#
    );
}

#[test]
fn associations_remove_incomplete() {
    let r = run_fixture(
        "links",
        &[
            "changes",
            "associations",
            "remove",
            "6",
            "SR-1",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: ticket not associated on change #6: 'SR-1'
"#
    );
}

#[test]
fn associations_pick_needs_tty() {
    let r = run_fixture("links", &["changes", "associations", "pick", "5"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: interactive picker requires TTY
"#
    );
}

#[test]
fn reply_cc_formatted_rejected() {
    let r = run_fixture(
        "ticket_conv",
        &[
            "tickets",
            "reply",
            "5",
            "hi",
            "--cc",
            "Ann <ann@acme.com>",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"CC must be a plain email address
"#
    );
}

#[test]
fn reply_cc_deleted_user() {
    let r = run_fixture(
        "ticket_conv",
        &[
            "tickets",
            "reply",
            "5",
            "hi",
            "--cc",
            "bob@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"each CC email must resolve to exactly one non-deleted user with a valid name and email
"#
    );
}

#[test]
fn note_set_status_read_only() {
    let r = run_fixture(
        "ticket_conv",
        &[
            "tickets",
            "add-note",
            "5",
            "--body",
            "hi",
            "--set-status",
            "pending",
            "--read-only",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read-only mode: no note was sent
"#
    );
}

#[test]
fn props_read_missing_tags() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "7", "--tag", "a"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket read did not include: tags
"#
    );
}

#[test]
fn props_tag_whitespace() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "7", "--tag", " a"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"tags must be nonempty text without surrounding whitespace or control characters
"#
    );
}

#[test]
fn props_planned_invalid() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "7", "--planned-start", "nope"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planned dates require ISO-8601 date/time with explicit offset, whole seconds (no host timezone inferred)
"#
    );
}

#[test]
fn props_planned_empty() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "7", "--planned-start", ""],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planned dates cannot be empty
"#
    );
}

#[test]
fn props_wrong_target() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "8", "--tag", "a"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket read did not identify the exact target; no update attempted
"#
    );
}

#[test]
fn props_invalid_record() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "10", "--tag", "a"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket read returned an invalid record; no update attempted
"#
    );
}

#[test]
fn props_planned_missing() {
    let r = run_fixture(
        "ticket_props",
        &[
            "tickets",
            "update",
            "7",
            "--planned-start",
            "2026-01-01T10:00:00+07:00",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket read did not include: planned_start_date
"#
    );
}

#[test]
fn props_not_ticket() {
    let r = run_fixture(
        "ticket_props",
        &["problems", "update", "7", "--tag", "a"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--tag and planned dates are supported only for tickets
"#
    );
}

#[test]
fn props_nonpositive_id() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "0", "--tag", "a"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket ID must be positive
"#
    );
}

#[test]
fn props_form_unavailable() {
    let r = run_fixture(
        "ticket_props",
        &["tickets", "update", "11", "--tag", "a"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"could not evaluate ticket edit form and rules; no update attempted
"#
    );
}

#[test]
fn ticket_create_send() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
            "--yes",
            "--expect-plan",
            "971853ece554e8e5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"created incident 777
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets"]);
}

#[test]
fn ticket_create_send_json() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
            "--yes",
            "--expect-plan",
            "971853ece554e8e5",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{"created": {"id": 777, "display_id": 777, "subject": "Hello"}, "plan_hash": "971853ece554e8e5bfcebeccb06dd09cca29d059881bdea7f8f408157c2eab08"}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets"]);
}

#[test]
fn ticket_create_needs_expect() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: non-interactive create requires --expect-plan HASH
"#
    );
}

#[test]
fn ticket_create_needs_yes() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: review with --dry-run, then use --yes --expect-plan HASH
"#
    );
}

#[test]
fn ticket_create_plan_changed() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
            "--yes",
            "--expect-plan",
            "deadbeef",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: plan changed: expected deadbeef, computed 971853ece554e8e5bfcebeccb06dd09cca29d059881bdea7f8f408157c2eab08. Review again with --dry-run.
"#
    );
}

#[test]
fn ticket_create_no_requester() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--yes",
            "--expect-plan",
            "971853ece554e8e5",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: cannot create: requester email is required; use --requester or the draft requester key
"#
    );
}

#[test]
fn ticket_create_dry_payload() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        mask_fetched(&r.out),
        r#"──────── Review: New Incident ────────
Requester  Ann Lee <ann@acme.com>
Subject    Hello                 
Priority   Medium                
Will notify / create work for
  Creating an incident may notify its requester and assignees and start workflows.
Warnings
  field 'orphan_note' belongs to a section that no choice reveals; hidden
form metadata as of unknown (fetched NOW, server, fingerprint a2982a2f46005417)
plan 971853ece554e8e5bfcebeccb06dd09cca29d059881bdea7f8f408157c2eab08
Payload (POST /api/_/tickets)
{
  "subject": "Hello",
  "priority": 2,
  "custom_fields": {},
  "email": "Ann Lee <ann@acme.com>",
  "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0="
}
plan_hash: 971853ece554e8e5bfcebeccb06dd09cca29d059881bdea7f8f408157c2eab08
Send with --yes --expect-plan 971853ece554e8e5
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_create_template() {
    let r = run_fixture("ticket_create", &["tickets", "create", "--dry-run"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"# fsv incident draft: fill in, then use tickets create --dry-run
# Fill in values after the colons. Lines starting with # are ignored.
# Fields appear and disappear with your answers, like the web form; after
# saving, new required fields are added and the file reopens if needed.
# Full choice lists: fsv form fields changes --choices LABEL
# Save and close to continue; close without changes to abort.

# REQUIRED · email or exact name of the requester
requester:
answers:
  # optional · text
  Subject:
  # optional · nested
  # choices: Cannot log in | Slow | Other
  Symptom:
  # optional · dropdown
  # choices: Low | Medium
  Priority: Low
  # optional · dropdown
  # choices: Access | Delivery
  Request Item:
  # optional · dropdown
  # choices: Yes | No
  Attachment Check:
  # optional · lookup (name or email)
  Owner:

# optional · Description · Markdown (or use description_html for raw HTML)
description_md: |
  

# Files to attach after the change is created (uploaded only after you confirm)
attachments: []
# planning:            # planning field label -> files
#   Rollout Plan: [rollout.xlsx]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_create_missing_draft() {
    let r = run_fixture(
        "ticket_create",
        &[
            "tickets",
            "create",
            "--from",
            "/nonexistent.yaml",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: cannot read /nonexistent.yaml: No such file or directory
"#
    );
}

#[test]
fn change_create_send() {
    let r = run_fixture(
        "change_create",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--yes",
            "--expect-plan",
            "10df27a1e78a8382",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"created CHN-900  https://acme.freshservice.com/a/changes/900
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes"]);
}

#[test]
fn change_create_send_json() {
    let r = run_fixture(
        "change_create",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--yes",
            "--expect-plan",
            "10df27a1e78a8382",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "created": {
    "id": 900,
    "display_id": 900,
    "subject": "Hello"
  },
  "plan_hash": "10df27a1e78a8382e879aa47ee0592d8e855fe2ac2cfb41591e477cb13263692",
  "steps": [
    {
      "step": "create",
      "ok": true,
      "id": 900
    }
  ],
  "uploaded": []
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes"]);
}

#[test]
fn change_create_plan_changed() {
    let r = run_fixture(
        "change_create",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--yes",
            "--expect-plan",
            "0000",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: plan changed: expected 0000, computed 10df27a1e78a8382e879aa47ee0592d8e855fe2ac2cfb41591e477cb13263692. Review again with --dry-run.
"#
    );
}

/// `create_command`: `has_input = bool(from_file or sets or preset or ...)`,
/// so empty `--from`/`--preset` are no input.
#[test]
fn change_create_empty_from_is_no_input() {
    let plain = run_fixture("change_create", &["changes", "create", "--dry-run"], None);
    for flag in ["--from", "--preset"] {
        let r = run_fixture(
            "change_create",
            &["changes", "create", flag, "", "--dry-run"],
            None,
        );
        assert_eq!((r.code, &r.out), (0, &plain.out), "{flag}: {}", r.err);
        let r = run_fixture("change_create", &["changes", "create", flag, ""], None);
        assert_eq!(r.code, 2, "{flag}: {}", r.err);
        assert!(r.err.contains("nothing to create"), "{flag}: {}", r.err);
    }
}

#[test]
fn change_create_template() {
    let r = run_fixture("change_create", &["changes", "create", "--dry-run"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"# fsv change draft: fill in, then `fsv changes create --from FILE --dry-run`
# Fill in values after the colons. Lines starting with # are ignored.
# Fields appear and disappear with your answers, like the web form; after
# saving, new required fields are added and the file reopens if needed.
# Full choice lists: fsv form fields changes --choices LABEL
# Save and close to continue; close without changes to abort.

# REQUIRED · email or exact name of the requester
requester:
answers:
  # NOTE: Read the change guide first.
  # optional · lookup (name or email) · The user this change is for, if not the requester (shown in fsv only: not sent yet).
  Request for:
  # optional · department
  # choices: Engineering | Finance
  Department:
  # REQUIRED · dropdown
  # choices: Platform | Payments | Labs
  Team:
  # REQUIRED · lookup (name or email)
  Group:
  # REQUIRED · lookup (name or email)
  Agent:
  # REQUIRED · dropdown
  # choices: Normal | Standard | Emergency
  Change Type: Normal
  # REQUIRED · multi-select (list)
  # choices: Billing | Search | Reports, Exports
  Services:
  # REQUIRED · dropdown · Only the cloud team performs this change
  # choices: Yes | No
  Cloud Only:
  # REQUIRED · dropdown
  # choices: Live | Staging
  Stage:
  # REQUIRED · text
  # e.g. Service: objective
  Subject:
  # optional · yes/no
  Needs Security Review:
  # REQUIRED · date-time (YYYY-MM-DD HH:MM, optional +HH:MM)
  Planned Start Date:
  # REQUIRED · date-time (YYYY-MM-DD HH:MM, optional +HH:MM)
  Planned End Date:
  # REQUIRED · dropdown
  # choices: Yes | No
  Downtime:
  # optional · text
  # e.g. 22:00 - 22:30
  Downtime Window:
  # REQUIRED · dropdown
  # choices: N/A | 01/12/2030 | 08/12/2030
  Sign-off Date:
  # optional · number
  Estimated Hours:
  # REQUIRED · dropdown
  # choices: Low | Medium | High
  #   Low — Sandbox only
  #   Medium — Internal tools
  #   High — Customer facing
  Impact: Low
  # REQUIRED · dropdown
  # choices: Low | Medium | High | Urgent
  #   Low — Nice to have
  #   Medium — Planned work
  #   High — Revenue at risk
  Priority: Low
  # REQUIRED · dropdown
  # choices: Low | Medium | High | Very High
  Risk: Low
  # optional · lookup (name or email)
  Maintenance Window:

# REQUIRED · Description · Markdown (or use description_html for raw HTML)
description_md: |
  

# Files to attach after the change is created (uploaded only after you confirm)
attachments: []
# planning:            # planning field label -> files
#   Rollout Plan: [rollout.xlsx]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_update_edit_needs_tty() {
    let r = run_fixture(
        "change_create",
        &["changes", "update", "12", "--edit", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --edit needs a terminal (no TTY or --no-input); use --from FILE or --set
"#
    );
}

#[test]
fn change_update_dry() {
    let r = run_fixture(
        "change_create",
        &[
            "changes",
            "update",
            "12",
            "--set",
            "Subject=Renamed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        mask_fetched(&r.out),
        r#"──────── Review: Update change 12 ────────
Messages
  [message] Tell the payments on-call before you start
Rule notes
  Sign-off Date: choices limited to N/A by rule 'Emergency'
  Department: read-only (disable by rule 'Labs team' (id 14))
  Subject: read-only (disable by rule 'Edit: lock subject' (id 19))
Problems (must be fixed before sending)
  Subject is disabled by a business rule (disable by rule 'Edit: lock subject' (id 19))
  Requester is required
  Group is required
  Agent is required
  Services is required
  Cloud Only is required
  Description is required
  Emergency Reason is required
  Needs Security Review is required
  Planned Start Date is required
  Planned End Date is required
  Impact is required
  Priority is required
  Risk is required
  Status is required
form metadata as of unknown (fetched NOW, server, fingerprint 9f57a1ae09548281)
No payload yet — fix the problems above
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_update_status_dry() {
    let r = run_fixture(
        "change_create",
        &["changes", "update", "12", "--status", "Closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        mask_fetched(&r.out),
        r#"──────── Review: Update change 12 ────────
Messages
  [message] Tell the payments on-call before you start
Rule notes
  Sign-off Date: choices limited to N/A by rule 'Emergency'
  Department: read-only (disable by rule 'Labs team' (id 14))
  Subject: read-only (disable by rule 'Edit: lock subject' (id 19))
Problems (must be fixed before sending)
  Status: 'Closed' is not a valid choice; choose one of: Open, Planning, Approval
  Requester is required
  Group is required
  Agent is required
  Services is required
  Cloud Only is required
  Description is required
  Emergency Reason is required
  Needs Security Review is required
  Planned Start Date is required
  Planned End Date is required
  Impact is required
  Priority is required
  Risk is required
form metadata as of unknown (fetched NOW, server, fingerprint 9f57a1ae09548281)
No payload yet — fix the problems above
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_update_status_reads_lifecycle() {
    let r = run_fixture(
        "change_create",
        &[
            "changes",
            "update",
            "13",
            "--status",
            "Approval",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.err, "");
    assert_eq!(
        mask_fetched(&r.out),
        r#"──────── Review: Update change 13 ────────
Field   Old      New     
Status  Open  →  Approval
Messages
  [message] Tell the payments on-call before you start
Rule notes
  Sign-off Date: choices limited to N/A by rule 'Emergency'
  Department: read-only (disable by rule 'Labs team' (id 14) on change of status)
  Subject: read-only (disable by rule 'Edit: lock subject' (id 19) on change of status)
Requires confirmation
  change status -> Approval
Problems (must be fixed before sending)
  Requester is required
  Group is required
  Agent is required
  Services is required
  Cloud Only is required
  Description is required
  Emergency Reason is required
  Needs Security Review is required
  Planned Start Date is required
  Planned End Date is required
  Impact is required
  Priority is required
  Risk is required
  Open → Approval is not an allowed lifecycle transition (allowed from Open: Planning)
form metadata as of unknown (fetched NOW, server, fingerprint 9f57a1ae09548281)
No payload yet — fix the problems above
"#
    );
}

#[test]
fn change_update_status_planning_is_allowed() {
    let r = run_fixture(
        "change_create",
        &[
            "changes",
            "update",
            "13",
            "--status",
            "Planning",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.err, "");
    let out = mask_fetched(&r.out);
    assert!(
        !out.contains("not an allowed lifecycle transition"),
        "{out}"
    );
    assert_eq!(
        out,
        r#"──────── Review: Update change 13 ────────
Field   Old      New     
Status  Open  →  Planning
Messages
  [message] Tell the payments on-call before you start
Rule notes
  Sign-off Date: choices limited to N/A by rule 'Emergency'
  Department: read-only (disable by rule 'Labs team' (id 14) on change of status)
  Subject: read-only (disable by rule 'Edit: lock subject' (id 19) on change of status)
Requires confirmation
  change status -> Planning
Problems (must be fixed before sending)
  Requester is required
  Group is required
  Agent is required
  Services is required
  Cloud Only is required
  Description is required
  Emergency Reason is required
  Needs Security Review is required
  Planned Start Date is required
  Planned End Date is required
  Impact is required
  Priority is required
  Risk is required
form metadata as of unknown (fetched NOW, server, fingerprint 9f57a1ae09548281)
No payload yet — fix the problems above
"#
    );
}

/// The review's `fetched <now>` stamp.
fn mask_fetched(out: &str) -> String {
    match out.find("(fetched ") {
        Some(i) => format!("{}NOW{}", &out[..i + 9], &out[i + 34..]),
        None => out.to_string(),
    }
}
#[test]
fn sr_create_send() {
    let r = run_fixture(
        "sr_create",
        &[
            "sr",
            "create",
            "77",
            "--requester",
            "ann@acme.com",
            "--set",
            "Title=T",
            "--set",
            "Details=D",
            "--set",
            "Needed by=2030-01-02",
            "--set",
            "1. Expected benefit=Lower cost",
            "--set",
            "1.2 Cost target=5",
            "--set",
            "Priority=Low - cosmetic, reports",
            "--set",
            concat!(
                "Overview deck (.pptx)=",
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/deck.pptx"
            ),
            "--attach",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/notes.txt"
            ),
            "--yes",
            "--expect-plan",
            "5e0581d3ec26d93d",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"  ok uploaded deck.pptx -> 901 (Overview deck (.pptx))
  ok uploaded notes.txt -> 901 (Other attachments)
placed SR-5001  https://acme.freshservice.com/a/tickets/5001
"#
    );
    assert_eq!(
        r.err,
        r#"  uploading deck.pptx... ok (4 B, server name "x" differs)
  uploading notes.txt... ok (5 B, server name "x" differs)
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/attachments",
            "POST /api/_/attachments",
            "POST /api/_/catalog/service-items/77/place-request"
        ]
    );
}

#[test]
fn sr_create_send_json() {
    let r = run_fixture(
        "sr_create",
        &[
            "sr",
            "create",
            "77",
            "--requester",
            "ann@acme.com",
            "--set",
            "Title=T",
            "--set",
            "Details=D",
            "--set",
            "Needed by=2030-01-02",
            "--set",
            "1. Expected benefit=Lower cost",
            "--set",
            "1.2 Cost target=5",
            "--set",
            "Priority=Low - cosmetic, reports",
            "--set",
            concat!(
                "Overview deck (.pptx)=",
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/deck.pptx"
            ),
            "--attach",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/notes.txt"
            ),
            "--yes",
            "--expect-plan",
            "5e0581d3ec26d93d",
            "-o",
            "json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "created": {
    "id": 5001,
    "display": "SR-5001"
  },
  "result": {
    "service_request": {
      "id": 5001,
      "display_id": 5001,
      "human_display_id": "SR-5001"
    }
  },
  "plan_hash": "5e0581d3ec26d93df27b1323a4704c3d88408b3ffe759d5cbad55d7b2332cda2",
  "uploaded": [
    {
      "id": 901,
      "name": "deck.pptx",
      "target": "Overview deck (.pptx)"
    },
    {
      "id": 901,
      "name": "notes.txt",
      "target": "Other attachments"
    }
  ],
  "steps": [
    {
      "step": "upload",
      "ok": true,
      "ids": [
        901,
        901
      ]
    },
    {
      "step": "place-request",
      "ok": true,
      "id": 5001
    }
  ],
  "verification": {
    "verified": false,
    "fields": {}
  }
}
"#
    );
    assert_eq!(
        r.err,
        r#"  uploading deck.pptx... ok (4 B, server name "x" differs)
  uploading notes.txt... ok (5 B, server name "x" differs)
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/attachments",
            "POST /api/_/attachments",
            "POST /api/_/catalog/service-items/77/place-request"
        ]
    );
}

#[test]
fn sr_create_plan_changed() {
    let r = run_fixture(
        "sr_create",
        &[
            "sr",
            "create",
            "77",
            "--requester",
            "ann@acme.com",
            "--set",
            "Title=T",
            "--set",
            "Details=D",
            "--set",
            "Needed by=2030-01-02",
            "--set",
            "1. Expected benefit=Lower cost",
            "--set",
            "1.2 Cost target=5",
            "--set",
            "Priority=Low - cosmetic, reports",
            "--set",
            concat!(
                "Overview deck (.pptx)=",
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/deck.pptx"
            ),
            "--attach",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/notes.txt"
            ),
            "--yes",
            "--expect-plan",
            "0000",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: plan changed: expected 0000, computed 5e0581d3ec26d93df27b1323a4704c3d88408b3ffe759d5cbad55d7b2332cda2. Review again with --dry-run.
"#
    );
}

#[test]
fn sr_create_bad_cc() {
    let r = run_fixture(
        "sr_create",
        &[
            "sr",
            "create",
            "77",
            "--requester",
            "ann@acme.com",
            "--set",
            "Title=T",
            "--set",
            "Details=D",
            "--set",
            "Needed by=2030-01-02",
            "--set",
            "1. Expected benefit=Lower cost",
            "--set",
            "1.2 Cost target=5",
            "--set",
            "Priority=Low - cosmetic, reports",
            "--set",
            concat!(
                "Overview deck (.pptx)=",
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/deck.pptx"
            ),
            "--attach",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/notes.txt"
            ),
            "--cc",
            "bad",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: CC must be a plain email address
"#
    );
}

#[test]
fn sr_create_dry_payload() {
    let r = run_fixture(
        "sr_create",
        &[
            "sr",
            "create",
            "77",
            "--requester",
            "ann@acme.com",
            "--set",
            "Title=T",
            "--set",
            "Details=D",
            "--set",
            "Needed by=2030-01-02",
            "--set",
            "1. Expected benefit=Lower cost",
            "--set",
            "1.2 Cost target=5",
            "--set",
            "Priority=Low - cosmetic, reports",
            "--set",
            concat!(
                "Overview deck (.pptx)=",
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/deck.pptx"
            ),
            "--attach",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/files/sr/notes.txt"
            ),
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        mask_fetched(&r.out),
        r#"──────── Review: New service request: Feature Request ────────
Catalog item             Feature Request (#77) · Engineering
Requester                Ann Lee <ann@acme.com>             
Subject (item template)  T                                  
Title                    T                                  
Details                  D                                  
Needed by                02 Jan 2030                        
1. Expected benefit      Lower cost                         
1.2 Cost target          5                                  
Priority                 Low - cosmetic, reports            
Overview deck (.pptx)    deck.pptx                          
Will notify / create work for
  This sends a real service request to the catalog item's fulfilment team.
  Requester: Ann Lee (approvals route via Ann Lee’s manager)
Attachments (uploaded after you confirm)
  Overview deck (.pptx): deck.pptx  4 B  sha256:d830325906c3d540
  Other attachments: notes.txt  5 B  sha256:ab5aa97074c454a0
Warnings
  Separate requested-for is available via --requested-for; requester remains unchanged.
form metadata as of 2030-01-15T10:00:00+09:00 (fetched NOW, server, fingerprint 1b3febc07fed14c1)
plan 5e0581d3ec26d93df27b1323a4704c3d88408b3ffe759d5cbad55d7b2332cda2
⚠ This sends a real service request to the catalog item's fulfilment team.
Payload (POST /api/_/catalog/service-items/77/place-request; <upload N: name> becomes the attachment id after upload)
{
  "email": "ann@acme.com",
  "custom_fields": {
    "title_100": "T",
    "paragraph_101": "D",
    "date_103": "2030-01-02",
    "goal": "Lower cost",
    "goal_2_kpi": "5",
    "priority": "Low - cosmetic, reports",
    "overview_deck": [
      "<upload 1: deck.pptx>"
    ]
  },
  "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyI0NDAwMDAxIjp7Im9wdGlvbmFsIjoiW10iLCJkaXNhYmxlZCI6IltdIiwiaGlkZGVuIjoiW10ifX0=",
  "attachments": [
    "<upload 2: notes.txt>"
  ]
}
plan_hash: 5e0581d3ec26d93df27b1323a4704c3d88408b3ffe759d5cbad55d7b2332cda2
send exactly this with: --yes --expect-plan 5e0581d3ec26d93d
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_create_template() {
    let r = run_fixture("sr_create", &["sr", "create", "77", "--dry-run"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"# New service request: Feature Request (catalog item 77)
# Fill in values after the colons. Lines starting with # are ignored.
# Fields appear with your answers, like the web form; after saving, new
# required fields are added and the file reopens if needed.
# Full item form: fsv sr fields 77
# Save and close to continue; close without changes to abort.
# This sends a real service request to the catalog item's fulfilment team.
# Separate requested-for is available via --requested-for; requester remains unchanged.

# REQUIRED · email or exact name of the requester (approvals route via their manager)
requester:
# Optional · exact email or name of the person receiving this request
requested_for:
answers:
  # NOTE: Attach the overview deck.
  # REQUIRED · text
  # e.g. Short name
  Title:
  # REQUIRED · textarea
  Details:
  # optional · textarea
  Affected system (if known):
  # REQUIRED · date (YYYY-MM-DD)
  Needed by:
  # REQUIRED · dropdown
  # choices: More revenue  (Total USD) | Lower cost | Other
  #   More revenue  (Total USD) → adds: 1.1 Revenue target
  #   Lower cost → adds: 1.2 Cost target
  1. Expected benefit:
  # REQUIRED · dropdown
  # choices: Critical - blocks customers | Medium - planned work | Low - cosmetic, reports
  Priority:
  # REQUIRED · file(s): paths relative to this draft · types .pptx · at most 1 file(s)
  Overview deck (.pptx): []

# REQUIRED · general attachments for the request (uploaded only after you confirm)
attachments: []
"#
    );
    assert_eq!(
        r.err,
        r#"fill this in, then: fsv sr create 77 --from FILE --dry-run
"#
    );
}

#[test]
fn sr_create_edit_needs_tty() {
    let r = run_fixture(
        "sr_create",
        &["sr", "create", "77", "--edit", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --edit needs a terminal (no TTY or --no-input); use --from FILE
"#
    );
}

#[test]
fn sr_create_missing_draft() {
    let r = run_fixture(
        "sr_create",
        &["sr", "create", "77", "--from", "/nope.yaml", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: cannot read /nope.yaml: No such file or directory
"#
    );
}

#[test]
fn sr_create_unknown_item() {
    let r = run_fixture("sr_create", &["sr", "create", "999", "--dry-run"], None);
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 404 fetching service-item: no route
"#
    );
}

#[test]
fn task_add_dry_run() {
    let r = run_fixture(
        "review",
        &["changes", "tasks-add", "5", "--title", "Do it", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_task_create",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/tasks",
  "payload": {
    "title": "Do it",
    "status": 1,
    "notify_before": 0
  },
  "owner": null,
  "status_label": "Open",
  "reminder": "Never",
  "note_text": null,
  "plan_hash": "e573da33f86de86b3d13fb3b6324dc267cbc30cc32b2b3ff5aba4f7651944855",
  "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
  "contract": "Accepted native create/task envelope. Runtime required_for_create custom fields and dates validated; blank optional fields omitted."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn task_add_send_unverified() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "tasks-add",
            "5",
            "--title",
            "Do it",
            "--yes",
            "--expect-plan",
            "e573da33f86de86b3d13fb3b6324dc267cbc30cc32b2b3ff5aba4f7651944855",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Task 12 submitted on change 5; exact-ID/state read-back: False
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "change_task_create",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/tasks",
  "payload": {
    "title": "Do it",
    "status": 1,
    "notify_before": 0
  },
  "owner": null,
  "status_label": "Open",
  "reminder": "Never",
  "note_text": null,
  "plan_hash": "e573da33f86de86b3d13fb3b6324dc267cbc30cc32b2b3ff5aba4f7651944855",
  "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
  "contract": "Accepted native create/task envelope. Runtime required_for_create custom fields and dates validated; blank optional fields omitted."
}
Returned task ID/state not verified in task read-back; do not resend.
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/changes/5/tasks"]);
}

#[test]
fn task_add_no_input() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "tasks-add",
            "5",
            "--title",
            "Do it",
            "--no-input",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_task_create",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/tasks",
  "payload": {
    "title": "Do it",
    "status": 1,
    "notify_before": 0
  },
  "owner": null,
  "status_label": "Open",
  "reminder": "Never",
  "note_text": null,
  "plan_hash": "e573da33f86de86b3d13fb3b6324dc267cbc30cc32b2b3ff5aba4f7651944855",
  "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
  "contract": "Accepted native create/task envelope. Runtime required_for_create custom fields and dates validated; blank optional fields omitted."
}
sending requires --yes and --expect-plan in no-input mode
"#
    );
}

#[test]
fn task_add_full_dry_run() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "tasks-add",
            "5",
            "--title",
            "Do it",
            "--owner",
            "ann@acme.com",
            "--group",
            "Ops",
            "--status",
            "in progress",
            "--note",
            "a<b",
            "--planned-start",
            "2026-01-01T10:00:00+07:00",
            "--planned-end",
            "2026-01-01T12:00:00+07:00",
            "--planned-effort",
            "1h30m",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_task_create",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/tasks",
  "payload": {
    "title": "Do it",
    "status": 2,
    "notify_before": 0,
    "description": "<div><p>a&lt;b</p></div>",
    "group_id": 4,
    "planned_start_date": "2026-01-01T10:00:00+07:00",
    "planned_end_date": "2026-01-01T12:00:00+07:00",
    "planned_effort": "1h 30m",
    "owner_id": 131
  },
  "owner": 131,
  "status_label": "in progress",
  "reminder": "Never",
  "note_text": "a<b",
  "plan_hash": "abc4854f90323e85658952e202991baba9c2065c1018232780807e0fe9eac346",
  "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
  "contract": "Accepted native create/task envelope. Runtime required_for_create custom fields and dates validated; blank optional fields omitted."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn task_add_hash_mismatch() {
    let r = run_fixture(
        "review",
        &[
            "changes",
            "tasks-add",
            "5",
            "--title",
            "Do it",
            "--yes",
            "--expect-plan",
            "x",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_task_create",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/tasks",
  "payload": {
    "title": "Do it",
    "status": 1,
    "notify_before": 0
  },
  "owner": null,
  "status_label": "Open",
  "reminder": "Never",
  "note_text": null,
  "plan_hash": "e573da33f86de86b3d13fb3b6324dc267cbc30cc32b2b3ff5aba4f7651944855",
  "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
  "contract": "Accepted native create/task envelope. Runtime required_for_create custom fields and dates validated; blank optional fields omitted."
}
task review hash does not match; review the exact task again
"#
    );
}

#[test]
fn task_add_confirm_eof() {
    let r = run_fixture(
        "review",
        &["changes", "tasks-add", "5", "--title", "Do it"],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, r#"Create this exact reviewed task? [y/N]: "#);
    assert_eq!(
        r.err,
        r#"{
  "action": "change_task_create",
  "tenant": "acme.freshservice.com",
  "change_id": 5,
  "method": "POST",
  "path": "/api/_/changes/5/tasks",
  "payload": {
    "title": "Do it",
    "status": 1,
    "notify_before": 0
  },
  "owner": null,
  "status_label": "Open",
  "reminder": "Never",
  "note_text": null,
  "plan_hash": "e573da33f86de86b3d13fb3b6324dc267cbc30cc32b2b3ff5aba4f7651944855",
  "warning": "Task creation/assignment may notify people or trigger tenant workflows. Never is a reminder setting, not a no-notification guarantee.",
  "contract": "Accepted native create/task envelope. Runtime required_for_create custom fields and dates validated; blank optional fields omitted."
}
Aborted.
"#
    );
}
