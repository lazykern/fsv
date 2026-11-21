//! Upstream commands ported from Python: watchers, ticket time,
//! planning-text, mark-known-error, bulk-priority, reply --bcc/--sender,
//! forward, update --reviewed with --set answers, the SR send-failure report,
//! the change asset picker, canned replies and scenarios, private ticket
//! views, list columns and exports, problem create and child writes,
//! Click's missing-parameter errors, change clones that fail part-way, and
//! ticket status transitions, merges, assignment and pick-up, closing
//! replies and notes (`--set-status`), the add-note send report, note
//! edit/delete, problem incident links and status, reviewed ticket tasks,
//! strict `--where`, attachments (uploads, before-create, delete), forward
//! `--conversation`, integrated assets, SR approvals, solutions, ticket bulk
//! actions (filtered with `--max-records`), change relationships, reviewed
//! change task update/delete, change and problem time entries, problem
//! task writes and SR requested items.
//! Expected output was captured from
//! Python on the same routes (`tests/golden/emit_upstream.py`); one-shot
//! routes serve the record before the write and the changed record after it.
//! Python commit: 84ffb0ff098009e3580c7ab9154175f8338e1b1f

mod common;
use common::{run_files, run_fixture};

#[test]
fn tickets_watchers_list() {
    let r = run_fixture("watchers", &["tickets", "watchers", "list", "INC-5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{"watcher_ids": [3]}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_watchers_list_json() {
    let r = run_fixture(
        "watchers",
        &["changes", "watchers", "list", "CHN-12", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{"watcher_ids": [4, 9]}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn watchers_list_duplicate_ids() {
    let r = run_fixture("watchers", &["tickets", "watchers", "list", "6"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv tickets watchers list [OPTIONS] ID
Try 'fsv tickets watchers list -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Invalid value: watcher membership contains duplicate identities                                                                                                                                      │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn watchers_list_incomplete() {
    let r = run_fixture(
        "watchers",
        &["tickets", "watchers", "list", "7", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv tickets watchers list [OPTIONS] ID
Try 'fsv tickets watchers list -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Invalid value: complete watcher membership is unavailable                                                                                                                                            │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn watchers_list_bad_id() {
    let r = run_fixture("watchers", &["tickets", "watchers", "list", "CHN-5"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv tickets watchers list [OPTIONS] ID
Try 'fsv tickets watchers list -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Invalid value: prefix CHN not valid for tickets (expect INC or SR)                                                                                                                                   │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn tickets_watch_dry_run() {
    let r = run_fixture(
        "watchers",
        &["tickets", "watchers", "set", "5", "--watch", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "tickets_self_watch",
  "method": "POST",
  "path": "/api/_/tickets/5/watch",
  "payload": {
    "user_id": 9
  },
  "plan_hash": "a825fa859b245a376620af57e169d3a3fb9d284ac137ee1af6e4890e950e4b32",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_watch_needs_hash() {
    let r = run_fixture(
        "watchers",
        &["tickets", "watchers", "set", "5", "--watch", "--yes"],
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
fn tickets_watch_send() {
    let r = run_fixture(
        "watchers",
        &[
            "tickets",
            "watchers",
            "set",
            "5",
            "--watch",
            "--yes",
            "--output=json",
            "--json",
            "--expect-plan",
            "a825fa859b245a376620af57e169d3a3fb9d284ac137ee1af6e4890e950e4b32",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "a825fa859b245a376620af57e169d3a3fb9d284ac137ee1af6e4890e950e4b32",
  "verified": true,
  "watcher_ids": [
    3,
    9
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/5/watch"]);
}

#[test]
fn changes_unwatch_dry_run() {
    let r = run_fixture(
        "watchers",
        &[
            "changes",
            "watchers",
            "set",
            "CHN-12",
            "--watch",
            "--unwatch",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "changes_self_unwatch",
  "method": "DELETE",
  "path": "/api/_/changes/12/unwatch?user_id=9",
  "payload": null,
  "plan_hash": "507f42c4b08ff712034f8c7542e100bc32915906167c8ac205784990694bbc6e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_unwatch_send() {
    let r = run_fixture(
        "watchers",
        &[
            "changes",
            "watchers",
            "set",
            "12",
            "--unwatch",
            "--yes",
            "--expect-plan",
            "507f42c4b08ff712034f8c7542e100bc32915906167c8ac205784990694bbc6e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "507f42c4b08ff712034f8c7542e100bc32915906167c8ac205784990694bbc6e",
  "verified": true,
  "watcher_ids": [
    4
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/changes/12/unwatch"]);
}

#[test]
fn tickets_watch_already_watching() {
    let r = run_fixture(
        "watchers",
        &[
            "tickets",
            "watchers",
            "set",
            "8",
            "--watch",
            "--yes",
            "--expect-plan",
            "01bb7814043fe1fc794c2fbbd712761e5e471d6e5dbc31239ddbcf52b532e312",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "already_in_requested_state",
  "verified": true,
  "plan_hash": "01bb7814043fe1fc794c2fbbd712761e5e471d6e5dbc31239ddbcf52b532e312",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn time_list() {
    let r = run_fixture("ticket_time", &["tickets", "time", "list", "INC-5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 21,
    "agent_id": 9,
    "billable": false,
    "timer_running": false,
    "time_spent": "01:30:00",
    "note": "Globex review",
    "executed_at": "2030-01-01T20:00:00Z"
  },
  {
    "id": 22,
    "agent_id": 4,
    "billable": false,
    "timer_running": false,
    "time_spent": "01:30:00",
    "note": "Globex review",
    "executed_at": "2030-01-01T20:00:00Z"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn time_create_dry_run() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "5",
            "create",
            "--spent",
            "01:15",
            "--executed-at",
            "2030-01-02T03:00:00+07:00",
            "--note",
            "Acme work",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_time_create",
  "method": "POST",
  "path": "/api/_/tickets/5/time_entries",
  "payload": {
    "agent_id": 9,
    "billable": false,
    "executed_at": "2030-01-02T03:00:00+07:00",
    "note": "Acme work",
    "time_spent": "01:15",
    "timer_running": false
  },
  "plan_hash": "9116412581801138360bd9688d73a90b35ab0f84b818cbea47959ced366aee41",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn time_create_send() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "5",
            "create",
            "--spent",
            "01:15",
            "--executed-at",
            "2030-01-02T03:00:00+07:00",
            "--note",
            "Acme work",
            "--yes",
            "--expect-plan",
            "9116412581801138360bd9688d73a90b35ab0f84b818cbea47959ced366aee41",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "9116412581801138360bd9688d73a90b35ab0f84b818cbea47959ced366aee41",
  "verified": true,
  "entry_id": 31,
  "fields": {
    "agent_id": "matched",
    "billable": "matched",
    "executed_at": "matched",
    "note": "matched",
    "time_spent": "matched",
    "timer_running": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/5/time_entries"]);
}

#[test]
fn time_create_needs_executed_at() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "5",
            "create",
            "--spent",
            "01:15",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"create requires --spent and --executed-at, without an entry ID
"#
    );
}

#[test]
fn time_create_bad_duration() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "5",
            "create",
            "--spent",
            "1:5",
            "--executed-at",
            "2030-01-02T03:00:00+07:00",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"manual time must be a positive HH:MM duration
"#
    );
}

#[test]
fn time_create_naive_timestamp() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "5",
            "create",
            "--spent",
            "00:30",
            "--executed-at",
            "2030-01-02T03:00:00",
            "--dry-run",
        ],
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
fn time_unsupported_action() {
    let r = run_fixture(
        "ticket_time",
        &["tickets", "time", "write", "5", "start", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unsupported time-entry action
"#
    );
}

#[test]
fn time_update_note_keeps_minutes() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "6",
            "update",
            "--entry",
            "41",
            "--note",
            "Globex",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_time_update",
  "method": "PUT",
  "path": "/api/_/tickets/6/time_entries/41",
  "payload": {
    "note": "Globex",
    "time_spent": "02:05"
  },
  "plan_hash": "915b26ef33d43d8e2ef0570d97bcc6aec8d2c4ede3f63d1c94b0c674c9f28cf9",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn time_update_send() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "6",
            "update",
            "--entry",
            "41",
            "--note",
            "Globex",
            "--yes",
            "--json",
            "--expect-plan",
            "915b26ef33d43d8e2ef0570d97bcc6aec8d2c4ede3f63d1c94b0c674c9f28cf9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "915b26ef33d43d8e2ef0570d97bcc6aec8d2c4ede3f63d1c94b0c674c9f28cf9",
  "verified": true,
  "entry_id": 41,
  "fields": {
    "note": "matched",
    "time_spent": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/6/time_entries/41"]);
}

#[test]
fn time_update_sub_minute_refused() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "6",
            "update",
            "--entry",
            "42",
            "--note",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"entry has sub-minute precision; provide --spent explicitly
"#
    );
}

#[test]
fn time_update_other_agent_refused() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "6",
            "update",
            "--entry",
            "43",
            "--spent",
            "00:20",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact self-owned nonbillable manual entry is unavailable
"#
    );
}

#[test]
fn time_update_billable_refused() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "6",
            "delete",
            "--entry",
            "44",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact self-owned nonbillable manual entry is unavailable
"#
    );
}

#[test]
fn time_delete_send() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "7",
            "delete",
            "--entry",
            "51",
            "--yes",
            "--expect-plan",
            "81da26138267add64b8ad95e701eeee07103ffd7b2a6a58a6f6ab37533431693",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "81da26138267add64b8ad95e701eeee07103ffd7b2a6a58a6f6ab37533431693",
  "verified": true,
  "entry_id": 51,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/tickets/7/time_entries/51"]);
}

#[test]
fn time_delete_wrong_hash() {
    let r = run_fixture(
        "ticket_time",
        &[
            "tickets",
            "time",
            "write",
            "7",
            "delete",
            "--entry",
            "51",
            "--yes",
            "--expect-plan",
            "abc",
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
fn change_planning_text_dry_run() {
    let r = run_fixture(
        "planning",
        &[
            "changes",
            "planning-text",
            "CHN-12",
            "--section",
            "Rollout Plan",
            "--description",
            "<p>Globex rollout</p>",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "changes_planning_text",
  "method": "POST",
  "path": "/api/_/changes/12/planning-fields?id=acme_plan",
  "payload": {
    "description": "<p>Globex rollout</p>"
  },
  "plan_hash": "c4d4cd168f18a0df604fd88cdcb3c0140a033e0dc0e311b56763857632a318c7",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "create",
  "before": "",
  "after": "<p>Globex rollout</p>",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_planning_text_send() {
    let r = run_fixture(
        "planning",
        &[
            "changes",
            "planning-text",
            "12",
            "--section",
            "acme_plan",
            "--description",
            "<p>Globex rollout</p>",
            "--yes",
            "--expect-plan",
            "c4d4cd168f18a0df604fd88cdcb3c0140a033e0dc0e311b56763857632a318c7",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c4d4cd168f18a0df604fd88cdcb3c0140a033e0dc0e311b56763857632a318c7",
  "verified": true,
  "section": "acme_plan",
  "description": "matched",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes/12/planning-fields"]);
}

#[test]
fn change_planning_text_attachments_refused() {
    let r = run_fixture(
        "planning",
        &[
            "changes",
            "planning-text",
            "13",
            "--section",
            "Rollout Plan",
            "--description",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planning attachments are present or unavailable; text-only preservation is not proven
"#
    );
}

#[test]
fn change_planning_text_empty_refused() {
    let r = run_fixture(
        "planning",
        &[
            "changes",
            "planning-text",
            "12",
            "--section",
            "Rollout Plan",
            "--description",
            "  ",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planning description must be nonempty text/HTML; use --clear to delete
"#
    );
}

#[test]
fn change_planning_text_existing_refused() {
    let r = run_fixture(
        "planning",
        &[
            "changes",
            "planning-text",
            "14",
            "--section",
            "Rollout Plan",
            "--description",
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
  "action": "changes_planning_text",
  "method": "PUT",
  "path": "/api/_/changes/14/planning-fields/acme_plan",
  "payload": {
    "description": "x"
  },
  "plan_hash": "a066c3852753967edc288f69779650d3ef337409c7893010592b1965c9557e00",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "<p>old</p>",
  "after": "x",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_planning_text_unknown_section() {
    let r = run_fixture(
        "planning",
        &[
            "changes",
            "planning-text",
            "12",
            "--section",
            "nope",
            "--description",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planning field not found: 'nope'; choices: Rollout Plan, Backout Plan
"#
    );
}

#[test]
fn change_planning_text_api_error() {
    let r = run_fixture(
        "planning",
        &[
            "changes",
            "planning-text",
            "15",
            "--section",
            "Rollout Plan",
            "--description",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"HTTP 500 listing planning-fields: boom
"#
    );
}

#[test]
fn problem_planning_text_dry_run() {
    let r = run_fixture(
        "planning",
        &[
            "problems",
            "planning-text",
            "PRB-7",
            "--section",
            "root cause",
            "--description",
            "Acme DNS outage",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problems_planning_text",
  "method": "POST",
  "path": "/api/_/problems/7/planning-fields?id=problem_cause",
  "payload": {
    "description": "Acme DNS outage"
  },
  "plan_hash": "881b034a95a1b4d9473fef0ba3b34717b2c23970078364cde5245e8df7190775",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "create",
  "before": "",
  "after": "Acme DNS outage",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_planning_text_send() {
    let r = run_fixture(
        "planning",
        &[
            "problems",
            "planning-text",
            "7",
            "--section",
            "Root Cause",
            "--description",
            "Acme DNS outage",
            "--yes",
            "--expect-plan",
            "881b034a95a1b4d9473fef0ba3b34717b2c23970078364cde5245e8df7190775",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "881b034a95a1b4d9473fef0ba3b34717b2c23970078364cde5245e8df7190775",
  "verified": true,
  "section": "problem_cause",
  "description": "matched",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/7/planning-fields"]);
}

#[test]
fn problem_planning_text_unknown_section() {
    let r = run_fixture(
        "planning",
        &[
            "problems",
            "planning-text",
            "7",
            "--section",
            "Workaround",
            "--description",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"choose Root Cause, Impact or Symptoms; other analysis wires are not proven
"#
    );
}

#[test]
fn problem_planning_text_attachments_unknown() {
    let r = run_fixture(
        "planning",
        &[
            "problems",
            "planning-text",
            "8",
            "--section",
            "problem_impact",
            "--description",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planning attachments are present or unavailable; text-only preservation is not proven
"#
    );
}

#[test]
fn problem_planning_text_needs_yes() {
    let r = run_fixture(
        "planning",
        &[
            "problems",
            "planning-text",
            "7",
            "--section",
            "Root Cause",
            "--description",
            "x",
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
fn change_planning_text_edit_dry_run() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "123",
            "--section",
            "Rollout Plan",
            "--description",
            "Acme new",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "changes_planning_text",
  "method": "PUT",
  "path": "/api/_/changes/123/planning-fields/acme_plan",
  "payload": {
    "description": "Acme new"
  },
  "plan_hash": "c8ac68f44b8e044c17a8a59d106b566a804268250af1ce8605f7968fb7b4372b",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "<div>Acme original</div>",
  "after": "Acme new",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_planning_text_edit_needs_expect_plan() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "123",
            "--section",
            "Rollout Plan",
            "--description",
            "Acme new",
            "--yes",
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
fn change_planning_text_edit_send() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "123",
            "--section",
            "Rollout Plan",
            "--description",
            "Acme new",
            "--yes",
            "--expect-plan",
            "c8ac68f44b8e044c17a8a59d106b566a804268250af1ce8605f7968fb7b4372b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c8ac68f44b8e044c17a8a59d106b566a804268250af1ce8605f7968fb7b4372b",
  "verified": true,
  "section": "acme_plan",
  "description": "matched",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/changes/123/planning-fields/acme_plan"]
    );
}

#[test]
fn change_planning_text_clear_dry_run() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "124",
            "--section",
            "Rollout Plan",
            "--clear",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "changes_planning_text",
  "method": "DELETE",
  "path": "/api/_/changes/124/planning-fields/acme_plan",
  "payload": null,
  "plan_hash": "c274dcc4151a61572517a65c418881f5d66ea69428a2c947da430deed49a3e59",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "clear",
  "before": "<div>Acme original</div>",
  "after": "",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_planning_text_clear_needs_expect_plan() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "124",
            "--section",
            "Rollout Plan",
            "--clear",
            "--yes",
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
fn change_planning_text_clear_send() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "124",
            "--section",
            "Rollout Plan",
            "--clear",
            "--yes",
            "--expect-plan",
            "c274dcc4151a61572517a65c418881f5d66ea69428a2c947da430deed49a3e59",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c274dcc4151a61572517a65c418881f5d66ea69428a2c947da430deed49a3e59",
  "verified": true,
  "section": "acme_plan",
  "description": "matched",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["DELETE /api/_/changes/124/planning-fields/acme_plan"]
    );
}

#[test]
fn change_planning_text_existing_content_edit_dry_run() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "125",
            "--section",
            "Rollout Plan",
            "--description",
            "Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "changes_planning_text",
  "method": "PUT",
  "path": "/api/_/changes/125/planning-fields/acme_plan",
  "payload": {
    "description": "Acme"
  },
  "plan_hash": "86fe69a0aa249e70668ce06dcb076ad4992ccdb9194709fbfbd6aadf0cacd8da",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "Acme original",
  "after": "Acme",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_planning_text_existing_content_wrong_hash() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "125",
            "--section",
            "Rollout Plan",
            "--description",
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
        r#"review with --dry-run, then pass --yes and the full --expect-plan hash
"#
    );
}

#[test]
fn problem_planning_text_edit_dry_run() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "123",
            "--section",
            "Root Cause",
            "--description",
            "Acme new",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problems_planning_text",
  "method": "PUT",
  "path": "/api/_/problems/123/planning-fields/problem_cause",
  "payload": {
    "description": "Acme new"
  },
  "plan_hash": "22985d135e002d4011b6c1e12acfed092ed1a01dd142475e84a541ecec0ade88",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "<div>Acme original</div>",
  "after": "Acme new",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_planning_text_edit_needs_expect_plan() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "123",
            "--section",
            "Root Cause",
            "--description",
            "Acme new",
            "--yes",
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
fn problem_planning_text_edit_send() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "123",
            "--section",
            "Root Cause",
            "--description",
            "Acme new",
            "--yes",
            "--expect-plan",
            "22985d135e002d4011b6c1e12acfed092ed1a01dd142475e84a541ecec0ade88",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "22985d135e002d4011b6c1e12acfed092ed1a01dd142475e84a541ecec0ade88",
  "verified": true,
  "section": "problem_cause",
  "description": "matched",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/problems/123/planning-fields/problem_cause"]
    );
}

#[test]
fn problem_planning_text_clear_dry_run() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "124",
            "--section",
            "Root Cause",
            "--clear",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problems_planning_text",
  "method": "DELETE",
  "path": "/api/_/problems/124/planning-fields/problem_cause",
  "payload": null,
  "plan_hash": "0b14e151a8d4efbc608ef606a42ff892efc9da2570bfc9cddb32f33cc070772b",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "clear",
  "before": "<div>Acme original</div>",
  "after": "",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_planning_text_clear_needs_expect_plan() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "124",
            "--section",
            "Root Cause",
            "--clear",
            "--yes",
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
fn problem_planning_text_clear_send() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "124",
            "--section",
            "Root Cause",
            "--clear",
            "--yes",
            "--expect-plan",
            "0b14e151a8d4efbc608ef606a42ff892efc9da2570bfc9cddb32f33cc070772b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "0b14e151a8d4efbc608ef606a42ff892efc9da2570bfc9cddb32f33cc070772b",
  "verified": true,
  "section": "problem_cause",
  "description": "matched",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["DELETE /api/_/problems/124/planning-fields/problem_cause"]
    );
}

#[test]
fn problem_planning_text_existing_content_edit_dry_run() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "125",
            "--section",
            "Root Cause",
            "--description",
            "Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problems_planning_text",
  "method": "PUT",
  "path": "/api/_/problems/125/planning-fields/problem_cause",
  "payload": {
    "description": "Acme"
  },
  "plan_hash": "9dbeb4db9c4441abf48f8fb33a44d797b595271878ef7094bd8683c19a0ac7b2",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "Acme original",
  "after": "Acme",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_planning_text_existing_content_wrong_hash() {
    let r = run_fixture(
        "planning_edit",
        &[
            "problems",
            "planning-text",
            "125",
            "--section",
            "Root Cause",
            "--description",
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
        r#"review with --dry-run, then pass --yes and the full --expect-plan hash
"#
    );
}

#[test]
fn change_planning_text_no_operation_refused() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "123",
            "--section",
            "Rollout Plan",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planning description must be nonempty text/HTML; use --clear to delete
"#
    );
}

#[test]
fn change_planning_text_clear_and_description_refused() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "123",
            "--section",
            "Rollout Plan",
            "--dry-run",
            "--clear",
            "--description",
            "Acme",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--clear and --description are mutually exclusive
"#
    );
}

#[test]
fn change_planning_text_blank_description_refused() {
    let r = run_fixture(
        "planning_edit",
        &[
            "changes",
            "planning-text",
            "123",
            "--section",
            "Rollout Plan",
            "--dry-run",
            "--description",
            "",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planning description must be nonempty text/HTML; use --clear to delete
"#
    );
}

#[test]
fn mark_known_error_dry_run() {
    let r = run_fixture(
        "planning",
        &["problems", "mark-known-error", "PRB-7", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_known_error_mark",
  "method": "PUT",
  "path": "/api/_/problems/7",
  "payload": {
    "known_error": true
  },
  "plan_hash": "efb37468aeb2e628d21801c22b5e16665aec849849a263883e44944426debf1f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn mark_known_error_send() {
    let r = run_fixture(
        "planning",
        &[
            "problems",
            "mark-known-error",
            "7",
            "--yes",
            "-o",
            "json",
            "--expect-plan",
            "efb37468aeb2e628d21801c22b5e16665aec849849a263883e44944426debf1f",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "efb37468aeb2e628d21801c22b5e16665aec849849a263883e44944426debf1f",
  "verified": true,
  "problem_id": 7,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/7"]);
}

#[test]
fn mark_known_error_already() {
    let r = run_fixture(
        "planning",
        &[
            "problems",
            "mark-known-error",
            "9",
            "--yes",
            "--expect-plan",
            "6269951058b01081df9ecbb7c16fa2fe18544261beeea75d3123c274042c34c3",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "already_in_requested_state",
  "verified": true,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn mark_known_error_state_unavailable() {
    let r = run_fixture(
        "planning",
        &["problems", "mark-known-error", "10", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact problem identity/known-error state is unavailable
"#
    );
}

#[test]
fn reply_bcc_dry_run() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "reply",
            "5",
            "thanks",
            "--bcc",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
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
  "path": "/api/_/tickets/5/reply",
  "payload": {
    "bcc_emails": [
      "ann@acme.com"
    ],
    "body": "<div>thanks</div>",
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "from_email": "support@acme.com",
    "full_text": "<div>thanks</div>",
    "translate_reply": false
  },
  "status_label": null,
  "plan_hash": "390983463def2d14f82f2e09d3a585bda1b574ca566c695a21f6bc86d68127df",
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
fn reply_bcc_send() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "reply",
            "5",
            "thanks",
            "--bcc",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--yes",
            "--expect-plan",
            "390983463def2d14f82f2e09d3a585bda1b574ca566c695a21f6bc86d68127df",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "conversation_id": 900,
  "ticket_id": 5,
  "plan_hash": "390983463def2d14f82f2e09d3a585bda1b574ca566c695a21f6bc86d68127df",
  "verified": true,
  "fields": {
    "body": "matched",
    "bcc_emails": "matched",
    "from_email": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_reply",
  "method": "POST",
  "path": "/api/_/tickets/5/reply",
  "payload": {
    "bcc_emails": [
      "ann@acme.com"
    ],
    "body": "<div>thanks</div>",
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "from_email": "support@acme.com",
    "full_text": "<div>thanks</div>",
    "translate_reply": false
  },
  "status_label": null,
  "plan_hash": "390983463def2d14f82f2e09d3a585bda1b574ca566c695a21f6bc86d68127df",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration.",
  "record_state": {
    "sends": 0
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/5/reply"]);
}

#[test]
fn reply_bcc_needs_sender() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "reply",
            "5",
            "thanks",
            "--bcc",
            "ann@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"BCC requires a configured --sender and bare recipient emails
"#
    );
}

#[test]
fn reply_bcc_with_cc_refused() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "reply",
            "5",
            "thanks",
            "--cc",
            "ann@acme.com",
            "--bcc",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"combined BCC/CC/status wire is not proven
"#
    );
}

#[test]
fn reply_sender_not_plain() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "reply",
            "5",
            "thanks",
            "--sender",
            "Support <support@acme.com>",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"sender must be a plain configured support email address
"#
    );
}

#[test]
fn reply_bcc_not_plain() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "reply",
            "5",
            "thanks",
            "--bcc",
            "Ann <ann@acme.com>",
            "--sender",
            "support@acme.com",
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
fn forward_dry_run() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "forward",
            "INC-5",
            "see below",
            "--to",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_forward",
  "method": "POST",
  "path": "/api/_/tickets/5/forward",
  "payload": {
    "body": "<div>see below</div>",
    "conversation_from_summary": false,
    "from_email": "support@acme.com",
    "notify_emails": [
      "ann@acme.com"
    ],
    "private": true
  },
  "status_label": null,
  "plan_hash": "ca2fc452e44f19c97f2ef33bad1dbfb46d41934cc7d15b022eecb9a04545ea1e",
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
fn forward_send() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "forward",
            "5",
            "see below",
            "--to",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--json",
            "--yes",
            "--expect-plan",
            "ca2fc452e44f19c97f2ef33bad1dbfb46d41934cc7d15b022eecb9a04545ea1e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "conversation_id": 901,
  "ticket_id": 5,
  "plan_hash": "ca2fc452e44f19c97f2ef33bad1dbfb46d41934cc7d15b022eecb9a04545ea1e",
  "verified": true,
  "fields": {
    "body": "matched",
    "private": "matched",
    "notify_emails": "matched",
    "from_email": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/5/forward"]);
}

#[test]
fn forward_needs_exact_review() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "forward",
            "5",
            "see below",
            "--to",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--yes",
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
fn forward_formatted_recipient() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "forward",
            "5",
            "see below",
            "--to",
            "Ann <ann@acme.com>",
            "--sender",
            "support@acme.com",
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
fn forward_conversation_dry_run() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "forward",
            "5",
            "see below",
            "--conversation",
            "900",
            "--to",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_forward",
  "method": "POST",
  "path": "/api/_/tickets/5/forward",
  "payload": {
    "body": "<div>see below</div>",
    "conversation_from_summary": false,
    "conversation_id": 900,
    "from_email": "support@acme.com",
    "notify_emails": [
      "ann@acme.com"
    ],
    "private": true
  },
  "status_label": null,
  "plan_hash": "9824a116e379c37e5e2f01f560345a6902e60ed78e302c38b37ff86c9a171067",
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
fn forward_conversation_send() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "forward",
            "5",
            "see below",
            "--conversation",
            "900",
            "--to",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--json",
            "--yes",
            "--expect-plan",
            "9824a116e379c37e5e2f01f560345a6902e60ed78e302c38b37ff86c9a171067",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "conversation_id": 901,
  "ticket_id": 5,
  "plan_hash": "9824a116e379c37e5e2f01f560345a6902e60ed78e302c38b37ff86c9a171067",
  "verified": true,
  "fields": {
    "body": "matched",
    "private": "matched",
    "notify_emails": "matched",
    "from_email": "matched"
  },
  "source_conversation_id": 900,
  "source_link_readback": "unavailable",
  "source_link_verified": false,
  "verification_scope": "delivery fields; source link is not exposed by independent readback",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/5/forward"]);
}

#[test]
fn forward_conversation_unknown() {
    let r = run_fixture(
        "ticket_mail",
        &[
            "tickets",
            "forward",
            "5",
            "see below",
            "--conversation",
            "999",
            "--to",
            "ann@acme.com",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact forward source unavailable in complete ticket history
"#
    );
}

#[test]
fn update_reviewed_priority_dry_run() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_properties",
  "method": "PUT",
  "path": "/api/_/tickets/5",
  "before": {
    "priority": 1
  },
  "after": {
    "priority": 73
  },
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "priority": 73
  },
  "plan_hash": "f47fab1ee5b9db9cd1c0df84ecb7bf798ab3e4381897d12b9835cfb282b4f088",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn update_reviewed_custom_field_dry_run() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--set",
            "Cost Code=new",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_properties",
  "method": "PUT",
  "path": "/api/_/tickets/5",
  "before": {
    "custom_fields": {
      "cost_code": "old",
      "other": "kept"
    }
  },
  "after": {
    "custom_fields": {
      "cost_code": "new"
    }
  },
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "custom_fields": {
      "cost_code": "new"
    }
  },
  "plan_hash": "f07a0b9ed88e7796ed74a63f89ae2110dd931a91cc849609c6bd5446147abe94",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn update_reviewed_send_verifies() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "f47fab1ee5b9db9cd1c0df84ecb7bf798ab3e4381897d12b9835cfb282b4f088",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 5,
  "plan_hash": "f47fab1ee5b9db9cd1c0df84ecb7bf798ab3e4381897d12b9835cfb282b4f088",
  "verified": true,
  "fields": {
    "priority": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_properties",
  "method": "PUT",
  "path": "/api/_/tickets/5",
  "before": {
    "priority": 1
  },
  "after": {
    "priority": 73
  },
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "priority": 73
  },
  "plan_hash": "f47fab1ee5b9db9cd1c0df84ecb7bf798ab3e4381897d12b9835cfb282b4f088",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/5"]);
}

#[test]
fn update_reviewed_yes_without_hash() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_properties",
  "method": "PUT",
  "path": "/api/_/tickets/5",
  "before": {
    "priority": 1
  },
  "after": {
    "priority": 73
  },
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "priority": 73
  },
  "plan_hash": "f47fab1ee5b9db9cd1c0df84ecb7bf798ab3e4381897d12b9835cfb282b4f088",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
--yes requires the full --expect-plan from a prior review
"#
    );
}

#[test]
fn update_reviewed_wrong_hash() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "abc",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_properties",
  "method": "PUT",
  "path": "/api/_/tickets/5",
  "before": {
    "priority": 1
  },
  "after": {
    "priority": 73
  },
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "priority": 73
  },
  "plan_hash": "f47fab1ee5b9db9cd1c0df84ecb7bf798ab3e4381897d12b9835cfb282b4f088",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
review hash changed; review again
"#
    );
}

#[test]
fn update_reviewed_closure_refused() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--status",
            "Closed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"update does not resolve or close tickets (combined resolution/closure requirements are not proven here); use `fsv tickets resolve ID --note TEXT` or `fsv tickets close ID` (both check runtime closure requirements and accept --set/--ack), or the web form
"#
    );
}

#[test]
fn update_reviewed_unknown_choice() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "Urgent",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Priority: 'Urgent' is not a valid choice; choose one of: Low, Medium, High; no update attempted
"#
    );
}

#[test]
fn update_reviewed_duplicate_answer() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--set",
            "priority=Low",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"duplicate property answer: priority
"#
    );
}

#[test]
fn update_reviewed_set_without_equals() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--set",
            "priority",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set requires unique FIELD=VALUE answers
"#
    );
}

#[test]
fn update_reviewed_problems_refused() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "problems",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--dry-run",
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
fn update_expect_plan_needs_reviewed() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--priority",
            "High",
            "--expect-plan",
            "abc",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_properties",
  "method": "PUT",
  "path": "/api/_/tickets/5",
  "before": {
    "priority": 1
  },
  "after": {
    "priority": 73
  },
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "priority": 73
  },
  "plan_hash": "f47fab1ee5b9db9cd1c0df84ecb7bf798ab3e4381897d12b9835cfb282b4f088",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
review hash changed; review again
"#
    );
}

#[test]
fn bulk_priority_dry_run() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "bulk-priority",
            "5",
            "6",
            "--priority",
            "High",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_bulk_priority",
  "method": "PUT",
  "path": "/api/_/bulk_ticket_actions/update_multiple",
  "payload": {
    "ids": [
      5,
      6
    ],
    "priority": 73,
    "reply": {}
  },
  "plan_hash": "7d2b7d833ad008e3ad2ad62cefa316c1389a4589f7db2472833498b6acfccc0c",
  "warning": "Explicit tickets only. Bulk actions may notify people; partial per-ticket outcomes are possible.",
  "before": [
    {
      "ticket_id": 5,
      "priority": 1
    },
    {
      "ticket_id": 6,
      "priority": 1
    }
  ],
  "priority_label": "High",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn bulk_priority_send_verifies() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "bulk-priority",
            "5",
            "6",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "7d2b7d833ad008e3ad2ad62cefa316c1389a4589f7db2472833498b6acfccc0c",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7d2b7d833ad008e3ad2ad62cefa316c1389a4589f7db2472833498b6acfccc0c",
  "verified": true,
  "tickets": {
    "5": "matched",
    "6": "matched"
  },
  "readback_attempts": 1,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/bulk_ticket_actions/update_multiple"]
    );
}

#[test]
fn bulk_priority_unverified_exits_1() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "bulk-priority",
            "5",
            "7",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "febac7b50647c8c5c53d5c3aec573740067e500763881eb814bb27c824488039",
        ],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "febac7b50647c8c5c53d5c3aec573740067e500763881eb814bb27c824488039",
  "verified": false,
  "tickets": {
    "5": "matched",
    "7": "mismatched"
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/bulk_ticket_actions/update_multiple"]
    );
}

#[test]
fn bulk_priority_yes_without_hash() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "bulk-priority",
            "5",
            "6",
            "--priority",
            "High",
            "--yes",
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
fn bulk_priority_duplicate_ids() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "bulk-priority",
            "5",
            "INC-5",
            "--priority",
            "High",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket IDs must be positive and unique
"#
    );
}

#[test]
fn bulk_priority_bad_id() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "bulk-priority",
            "5",
            "x",
            "--priority",
            "High",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"invalid id: x
"#
    );
}

#[test]
fn bulk_priority_unknown_priority() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "bulk-priority",
            "5",
            "6",
            "--priority",
            "Urgent",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Priority: 'Urgent' is not a valid choice; choose one of: Low, Medium, High; no update attempted
"#
    );
}

#[test]
fn sr_send_failure_lists_uploads() {
    let r = run_fixture(
        "sr_fail",
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
            "Overview deck (.pptx)=/Users/phusit.somb/lab/work/fsv-rust/crates/fsv-cli/tests/fixtures/files/sr/deck.pptx",
            "--attach",
            "/Users/phusit.somb/lab/work/fsv-rust/crates/fsv-cli/tests/fixtures/files/sr/notes.txt",
            "--yes",
            "--expect-plan",
            "19860090d92948522db5bbd3d75040951dfd4d69ae5020eecbdad4e106289553",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: plan changed: expected 19860090d92948522db5bbd3d75040951dfd4d69ae5020eecbdad4e106289553, computed 4b0d598affb101dfb0f3bad2c06d3df24fe42c1b59dc18f4cf5f70ab4231c2a3. Review again with --dry-run.
"#
    );
}

#[test]
fn asset_picker_page() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "12"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "change_id": 12,
  "page": 1,
  "scope": "single search page; not a complete inventory",
  "assets": [
    {
      "display_id": 701,
      "name": "Acme Caf\u00e9 web",
      "ci_type_name": "Server",
      "inventory": {
        "name": {
          "id": 701,
          "label": "Acme Caf\u00e9 web"
        },
        "type": "Server",
        "device_pk": 701
      }
    },
    {
      "display_id": 702,
      "name": "Acme API",
      "ci_type_name": "Server",
      "inventory": {
        "name": {
          "id": 702,
          "label": "Acme API"
        },
        "type": "Server",
        "device_pk": 702
      }
    }
  ],
  "mutation": "none",
  "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_picker_display_id_target() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "CHN-12", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "change_id": 12,
  "page": 1,
  "scope": "single search page; not a complete inventory",
  "assets": [
    {
      "display_id": 701,
      "name": "Acme Caf\u00e9 web",
      "ci_type_name": "Server",
      "inventory": {
        "name": {
          "id": 701,
          "label": "Acme Caf\u00e9 web"
        },
        "type": "Server",
        "device_pk": 701
      }
    },
    {
      "display_id": 702,
      "name": "Acme API",
      "ci_type_name": "Server",
      "inventory": {
        "name": {
          "id": 702,
          "label": "Acme API"
        },
        "type": "Server",
        "device_pk": 702
      }
    }
  ],
  "mutation": "none",
  "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_picker_search_page_2() {
    let r = run_fixture(
        "asset_picker",
        &[
            "changes",
            "asset-picker",
            "12",
            "--search",
            "Globex",
            "--page",
            "2",
            "-o",
            "json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "change_id": 12,
  "page": 2,
  "scope": "single search page; not a complete inventory",
  "assets": [
    {
      "display_id": 703,
      "name": "Globex DB",
      "ci_type_name": "Server",
      "inventory": {
        "name": {
          "id": 703,
          "label": "Globex DB"
        },
        "type": "Server",
        "device_pk": 703
      }
    }
  ],
  "mutation": "none",
  "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_picker_select() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--select", "702"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "change_id": 12,
  "page": 1,
  "scope": "single search page; not a complete inventory",
  "assets": [
    {
      "display_id": 701,
      "name": "Acme Caf\u00e9 web",
      "ci_type_name": "Server",
      "inventory": {
        "name": {
          "id": 701,
          "label": "Acme Caf\u00e9 web"
        },
        "type": "Server",
        "device_pk": 701
      }
    },
    {
      "display_id": 702,
      "name": "Acme API",
      "ci_type_name": "Server",
      "inventory": {
        "name": {
          "id": 702,
          "label": "Acme API"
        },
        "type": "Server",
        "device_pk": 702
      }
    }
  ],
  "mutation": "none",
  "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker.",
  "selected": {
    "display_id": 702,
    "name": "Acme API",
    "ci_type_name": "Server",
    "inventory": {
      "name": {
        "id": 702,
        "label": "Acme API"
      },
      "type": "Server",
      "device_pk": 702
    }
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_picker_select_missing() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--select", "999"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact asset display ID is not unique on this search page
"#
    );
}

#[test]
fn asset_picker_empty_page() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "empty"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "change_id": 12,
  "page": 1,
  "scope": "single search page; not a complete inventory",
  "assets": [],
  "mutation": "none",
  "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn asset_picker_empty_select() {
    let r = run_fixture(
        "asset_picker",
        &[
            "changes",
            "asset-picker",
            "12",
            "--search",
            "empty",
            "--select",
            "1",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact asset display ID is not unique on this search page
"#
    );
}

#[test]
fn asset_picker_duplicate_display_ids() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "dup"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"invalid inventory totals or duplicate identities
"#
    );
}

#[test]
fn asset_picker_rows_not_list() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "notlist"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"integrated inventory unavailable
"#
    );
}

#[test]
fn asset_picker_row_not_object() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "notobj"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"integrated inventory identity unavailable
"#
    );
}

#[test]
fn asset_picker_bad_display_id() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "bad"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"expected a positive numeric identity
"#
    );
}

#[test]
fn asset_picker_duplicate_then_bad() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "dupbad"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"expected a positive numeric identity
"#
    );
}

#[test]
fn asset_picker_conflicting_pk() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "pk"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"conflicting inventory identities
"#
    );
}

#[test]
fn asset_picker_page_mismatch() {
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "pages"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"integrated inventory pagination unavailable
"#
    );
}

#[test]
fn asset_picker_bad_target() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "abc"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"invalid id: abc
"#
    );
}

#[test]
fn asset_picker_zero_target() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "0"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"expected a positive numeric identity
"#
    );
}

#[test]
fn asset_picker_ticket_target() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "INC-12"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"prefix INC not valid for changes (expect CHN)
"#
    );
}

#[test]
fn asset_picker_api_error_is_uncaught() {
    // Python leaves APIError uncaught (traceback, exit 1).
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--search", "boom"],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("HTTP 500 listing device: boom"), "{}", r.err);
}

#[test]
fn asset_picker_page_must_be_positive() {
    // Click's `min=1` range error; clap's text differs.
    let r = run_fixture(
        "asset_picker",
        &["changes", "asset-picker", "12", "--page", "0"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.requests.is_empty());
}

#[test]
fn canned_folders() {
    let r = run_fixture("canned", &["tickets", "canned", "folders"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "canned_response_folders": [
    {
      "id": 702,
      "name": "Acme caf\u00e9"
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn canned_list() {
    let r = run_fixture(
        "canned",
        &["tickets", "canned", "list", "--folder", "702", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "canned_responses": [
    {
      "id": 701,
      "title": "Thanks"
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn canned_show() {
    let r = run_fixture(
        "canned",
        &["tickets", "canned", "show", "701", "-o", "json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "canned_response": {
    "id": 701,
    "title": "Thanks",
    "content": "<div>Acme response</div>",
    "meta": {
      "note": 7
    }
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn canned_reply_dry_run() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "INC-5",
            "--response",
            "701",
            "--content-pointer",
            "/canned_response/content",
            "--sender",
            "support@acme.com",
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
  "path": "/api/_/tickets/5/reply",
  "payload": {
    "body": "<div>Acme response</div>",
    "cc_emails": [],
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "from_email": "support@acme.com",
    "full_text": "<div>Acme response</div>",
    "translate_reply": false
  },
  "status_label": null,
  "plan_hash": "412c1906e85fdc2200975a7392e4d3635c04c6d09a75363828b27a40e79e488f",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration.",
  "record_state": {
    "sends": 0
  },
  "source": {
    "id": 701,
    "hash": "f136a71f4682d335d1cff7e3510f7e17a3a3c0be74155646755b18ebedfe2573"
  },
  "insertion": "Literal selected HTML only; no attachments, signatures or placeholder expansion.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn canned_reply_send() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "INC-5",
            "--response",
            "701",
            "--content-pointer",
            "/canned_response/content",
            "--sender",
            "support@acme.com",
            "--yes",
            "--expect-plan",
            "412c1906e85fdc2200975a7392e4d3635c04c6d09a75363828b27a40e79e488f",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "conversation_id": 900,
  "ticket_id": 5,
  "plan_hash": "412c1906e85fdc2200975a7392e4d3635c04c6d09a75363828b27a40e79e488f",
  "verified": true,
  "fields": {
    "body": "matched",
    "cc_emails": "matched",
    "from_email": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/5/reply"]);
}

#[test]
fn canned_reply_needs_hash() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "5",
            "--response",
            "701",
            "--content-pointer",
            "/canned_response/content",
            "--sender",
            "support@acme.com",
            "--yes",
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
fn canned_reply_pointer_without_slash() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "5",
            "--response",
            "701",
            "--content-pointer",
            "canned_response/content",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"content pointer must start with /; inspect canned show first
"#
    );
}

#[test]
fn canned_reply_pointer_bad_escape() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "5",
            "--response",
            "701",
            "--content-pointer",
            "/canned_response/~2",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"invalid JSON pointer escape
"#
    );
}

#[test]
fn canned_reply_pointer_missing() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "5",
            "--response",
            "701",
            "--content-pointer",
            "/canned_response/body",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"content pointer does not identify a source field
"#
    );
}

#[test]
fn canned_reply_pointer_not_text() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "5",
            "--response",
            "701",
            "--content-pointer",
            "/canned_response/meta",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"selected canned content must be nonempty text/HTML
"#
    );
}

#[test]
fn canned_reply_placeholder_refused() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "5",
            "--response",
            "705",
            "--content-pointer",
            "/canned_response/content",
            "--sender",
            "support@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unresolved canned placeholders require the web editor
"#
    );
}

#[test]
fn canned_reply_sender_not_plain() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "canned",
            "reply",
            "5",
            "--response",
            "701",
            "--content-pointer",
            "/canned_response/content",
            "--sender",
            "Support <support@acme.com>",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"sender must be a plain configured support email address
"#
    );
}

#[test]
fn scenarios_list() {
    let r = run_fixture("canned", &["tickets", "scenarios", "list"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "scenario_automations": [
    {
      "id": 703,
      "name": "Globex"
    },
    {
      "id": 704,
      "name": "Dup A"
    },
    {
      "id": 704,
      "name": "Dup B"
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn scenarios_show() {
    let r = run_fixture("canned", &["tickets", "scenarios", "show", "703"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 703,
  "name": "Globex"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn scenarios_show_ambiguous() {
    let r = run_fixture("canned", &["tickets", "scenarios", "show", "704"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv tickets scenarios show [OPTIONS] ID_
Try 'fsv tickets scenarios show -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Invalid value: scenario identity is missing or ambiguous in this inventory                                                                                                                           │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn scenarios_show_missing() {
    let r = run_fixture("canned", &["tickets", "scenarios", "show", "999"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv tickets scenarios show [OPTIONS] ID_
Try 'fsv tickets scenarios show -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Invalid value: scenario identity is missing or ambiguous in this inventory                                                                                                                           │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn scenarios_apply_dry_run() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "scenarios",
            "apply",
            "INC-5",
            "--scenario",
            "703",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_scenario",
  "method": "PUT",
  "path": "/api/_/tickets/execute_scenario",
  "payload": {
    "ids": [
      5
    ],
    "scenario_id": 703
  },
  "plan_hash": "1f643c49b416a5925f9b19a2e9b38059243d152697a28cb065921126f1ea4497",
  "warning": "Native automation may send mail or change records. No complete side-effect verification contract is captured.",
  "definition": {
    "id": 703,
    "name": "Globex"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn scenarios_apply_send() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "scenarios",
            "apply",
            "5",
            "--scenario",
            "703",
            "--yes",
            "--expect-plan",
            "1f643c49b416a5925f9b19a2e9b38059243d152697a28cb065921126f1ea4497",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1f643c49b416a5925f9b19a2e9b38059243d152697a28cb065921126f1ea4497",
  "verified": false,
  "ticket_id": 5,
  "identity_verified": true,
  "warning": "Exact ticket was read back; scenario side effects (including mail/workflows) are unverified. Do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/execute_scenario"]);
}

#[test]
fn scenarios_apply_wrong_hash() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "scenarios",
            "apply",
            "5",
            "--scenario",
            "703",
            "--yes",
            "--expect-plan",
            "abc",
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
fn scenarios_apply_ambiguous() {
    let r = run_fixture(
        "canned",
        &[
            "tickets",
            "scenarios",
            "apply",
            "5",
            "--scenario",
            "704",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"scenario identity is missing or ambiguous in this inventory
"#
    );
}

#[test]
fn pv_show() {
    let r = run_fixture(
        "private_views",
        &["tickets", "private-views", "show", "701"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "view": {
    "id": 701,
    "user_id": 9,
    "default": false,
    "visibility": 81,
    "visibility_name": "Me",
    "name": "Acme source",
    "workspace_id": 17,
    "query_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "42"
        ]
      }
    ],
    "advanced_query_hash": null,
    "order_by": "created_at",
    "order_type": "desc"
  },
  "visibility": "private",
  "private": true,
  "workspace_id": 17
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_show_json_flag() {
    let r = run_fixture(
        "private_views",
        &["tickets", "private-views", "show", "702", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "view": {
    "id": 702,
    "user_id": 9,
    "default": false,
    "visibility": 81,
    "visibility_name": "Me",
    "name": "Globex clone",
    "workspace_id": 17,
    "query_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "42"
        ]
      }
    ],
    "advanced_query_hash": null,
    "order_by": "created_at",
    "order_type": "desc"
  },
  "visibility": "private",
  "private": true,
  "workspace_id": 17
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_create_dry_run() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "701",
            "create",
            "--name",
            "Globex",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "private_view_create",
  "method": "POST",
  "path": "/api/_/ticket_filters",
  "payload": {
    "advanced_query_hash": null,
    "name": "Globex",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "42"
        ]
      }
    ],
    "visibility": 81,
    "workspace_id": 17
  },
  "plan_hash": "5cf7a49d4f362ca8cfb420ef82d8da401e86a14aa5182b07c242fe9d5cef6bf5",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_edit_dry_run() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "703",
            "edit",
            "--name",
            "Globex renamed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "private_view_edit",
  "method": "PUT",
  "path": "/api/_/ticket_filters/703",
  "payload": {
    "advanced_query_hash": null,
    "name": "Globex renamed",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "42"
        ]
      }
    ],
    "visibility": 81,
    "workspace_id": 17
  },
  "plan_hash": "72c24fc263ceee0cf5cdb47536553af020e4df41ea586e2ee9baec6fdb9528cc",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_delete_dry_run() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "704",
            "delete",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "private_view_delete",
  "method": "DELETE",
  "path": "/api/_/ticket_filters/704",
  "payload": null,
  "plan_hash": "397f33d1d0691afdb9e0bbd8d45c48202ea5a0893bc82edecaadc76a25fc0dac",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_object_label_dry_run() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "709",
            "create",
            "--name",
            "Only",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "private_view_create",
  "method": "POST",
  "path": "/api/_/ticket_filters",
  "payload": {
    "advanced_query_hash": null,
    "name": "Only",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "42"
        ]
      }
    ],
    "visibility": 81,
    "workspace_id": 17
  },
  "plan_hash": "d8cacfaf02254cb1efcd721067a17ea443edf887c2317a0ea369a2a7d80a7b31",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_needs_yes() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "701",
            "create",
            "--name",
            "Globex",
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
fn pv_wrong_hash() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "701",
            "create",
            "--name",
            "Globex",
            "--yes",
            "--expect-plan",
            "abc",
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
fn pv_unsupported_action() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "701",
            "rename",
            "--name",
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
  "action": "private_view_edit",
  "method": "PUT",
  "path": "/api/_/ticket_filters/701",
  "payload": {
    "advanced_query_hash": null,
    "name": "x",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "42"
        ]
      }
    ],
    "visibility": 81,
    "workspace_id": 17
  },
  "plan_hash": "728aea34f382c92d845ffa026aeb19449bfc6d6d182bcd6356ad452ebdb86374",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_delete_with_name() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "704",
            "delete",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"delete does not accept a name
"#
    );
}

#[test]
fn pv_create_without_name() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "701",
            "create",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"create/rename requires a nonempty --name
"#
    );
}

#[test]
fn pv_create_blank_name() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "701",
            "create",
            "--name",
            "  ",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"create/rename requires a nonempty --name
"#
    );
}

#[test]
fn pv_refuses_default() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "705",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only self-owned, explicitly non-default private views are supported
"#
    );
}

#[test]
fn pv_refuses_other_owner() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "706",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only self-owned, explicitly non-default private views are supported
"#
    );
}

#[test]
fn pv_refuses_visibility_name() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "707",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"view visibility is 'All agents', not private; nothing was sent
"#
    );
}

#[test]
fn pv_refuses_order() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "708",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only captured created-at descending views are supported
"#
    );
}

#[test]
fn pv_refuses_advanced_query() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "710",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only the captured ordinary query-hash subset is supported
"#
    );
}

#[test]
fn pv_refuses_id_mismatch() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "711",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact saved-view identity is unavailable
"#
    );
}

#[test]
fn pv_refuses_null_visibility_name() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "713",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"GET /api/_/ticket_filters/713 returns visibility 81 (unknown) without group_ids; only Myself (3) with an empty group_ids list is treated as private (1 = All agents, 2 = Agents in groups). Nothing was sent
"#
    );
}

#[test]
fn pv_refuses_system() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "714",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only self-owned, explicitly non-default private views are supported
"#
    );
}

#[test]
fn pv_create_send() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "701",
            "create",
            "--name",
            "Globex",
            "--yes",
            "--expect-plan",
            "5cf7a49d4f362ca8cfb420ef82d8da401e86a14aa5182b07c242fe9d5cef6bf5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "5cf7a49d4f362ca8cfb420ef82d8da401e86a14aa5182b07c242fe9d5cef6bf5",
  "verified": false,
  "view_id": 702,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "mismatched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/ticket_filters"]);
}

#[test]
fn pv_edit_send() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "703",
            "edit",
            "--name",
            "Globex renamed",
            "--yes",
            "--expect-plan",
            "72c24fc263ceee0cf5cdb47536553af020e4df41ea586e2ee9baec6fdb9528cc",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "72c24fc263ceee0cf5cdb47536553af020e4df41ea586e2ee9baec6fdb9528cc",
  "verified": true,
  "view_id": 703,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "matched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/ticket_filters/703"]);
}

#[test]
fn pv_edit_send_mismatch() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "703",
            "edit",
            "--name",
            "Globex_renamed",
            "--yes",
            "--expect-plan",
            "1b8e57667447246d365618928025db3bf4e880cfce000a226d6f0d6f41683238",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1b8e57667447246d365618928025db3bf4e880cfce000a226d6f0d6f41683238",
  "verified": false,
  "view_id": 703,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "mismatched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/ticket_filters/703"]);
}

#[test]
fn pv_delete_send() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "704",
            "delete",
            "--yes",
            "--expect-plan",
            "397f33d1d0691afdb9e0bbd8d45c48202ea5a0893bc82edecaadc76a25fc0dac",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"deleting a view cannot be undone; pass --confirm-destructive with --yes --expect-plan. Nothing was sent
"#
    );
}

#[test]
fn pv_list() {
    let r = run_fixture("private_views", &["tickets", "private-views", "list"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"HTTP 404 listing ticket filters: no route
"#
    );
}

#[test]
fn pv_show_refused_identity() {
    let r = run_fixture(
        "private_views",
        &["tickets", "private-views", "show", "711"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact saved-view identity is unavailable
"#
    );
}

#[test]
fn pv_show_api_error() {
    let r = run_fixture(
        "private_views",
        &["tickets", "private-views", "show", "799"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"HTTP 404 fetching ticket filter: no route
"#
    );
}

#[test]
fn pv_write_read_failed() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "799",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read failed (HTTP 404); nothing was sent
"#
    );
}

#[test]
fn pv_rename_dry_run() {
    let r = run_fixture(
        "private_views",
        &[
            "tickets",
            "private-views",
            "write",
            "703",
            "rename",
            "--name",
            "Globex renamed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "private_view_edit",
  "method": "PUT",
  "path": "/api/_/ticket_filters/703",
  "payload": {
    "advanced_query_hash": null,
    "name": "Globex renamed",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "42"
        ]
      }
    ],
    "visibility": 81,
    "workspace_id": 17
  },
  "plan_hash": "72c24fc263ceee0cf5cdb47536553af020e4df41ea586e2ee9baec6fdb9528cc",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_native_list() {
    let r = run_fixture(
        "private_views_native",
        &["tickets", "private-views", "list"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "views": [
    {
      "id": 801,
      "name": "Acme queue",
      "editable": null
    }
  ],
  "note": "the list carries no visibility; run show ID to see whether a view is private"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_native_show() {
    let r = run_fixture(
        "private_views_native",
        &["tickets", "private-views", "show", "801"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "view": {
    "id": 801,
    "name": "Acme queue",
    "default": false,
    "visibility": 3,
    "user_id": 9,
    "group_id": null,
    "group_ids": [],
    "advanced_query_hash": null,
    "advanced": false,
    "order_by": "created_at",
    "order_type": "desc",
    "per_page": 30,
    "query_hash": [
      {
        "condition": "priority",
        "operator": "is_in",
        "value": [
          "3"
        ],
        "type": "default"
      },
      {
        "condition": "workspace_id",
        "operator": "is",
        "value": "7",
        "type": "default"
      }
    ],
    "meta": {
      "workspace_id": [
        {
          "id": 7,
          "name": "Acme IT"
        }
      ]
    }
  },
  "visibility": "Myself",
  "private": true,
  "workspace_id": 7
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_native_create_dry_run() {
    let r = run_fixture(
        "private_views_native",
        &[
            "tickets",
            "private-views",
            "write",
            "801",
            "create",
            "--name",
            "Acme mine",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "private_view_create",
  "method": "POST",
  "path": "/api/_/ticket_filters",
  "payload": {
    "advanced_query_hash": null,
    "name": "Acme mine",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": [
      {
        "condition": "priority",
        "operator": "is_in",
        "type": "default",
        "value": [
          "3"
        ]
      },
      {
        "condition": "workspace_id",
        "operator": "is",
        "type": "default",
        "value": [
          {
            "id": 7
          }
        ]
      }
    ],
    "visibility": 3,
    "workspace_id": 7
  },
  "plan_hash": "2cc50d7145cb05879b2380bb6463a8ec52e565b92d09ef049423dd302c64c8dc",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn pv_native_create_send() {
    let r = run_fixture(
        "private_views_native",
        &[
            "tickets",
            "private-views",
            "write",
            "801",
            "create",
            "--name",
            "Acme mine",
            "--yes",
            "--expect-plan",
            "2cc50d7145cb05879b2380bb6463a8ec52e565b92d09ef049423dd302c64c8dc",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2cc50d7145cb05879b2380bb6463a8ec52e565b92d09ef049423dd302c64c8dc",
  "verified": true,
  "view_id": 901,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "matched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/ticket_filters"]);
}

#[test]
fn cpv_list() {
    let r = run_fixture(
        "private_views_native",
        &["changes", "private-views", "list"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "views": [
    {
      "id": 811,
      "name": "Globex changes",
      "editable": true
    },
    {
      "id": 812,
      "name": "Globex shared",
      "editable": false
    }
  ],
  "note": "the list carries no visibility; run show ID to see whether a view is private"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cpv_list_json() {
    let r = run_fixture(
        "private_views_native",
        &["changes", "private-views", "list", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "views": [
    {
      "id": 811,
      "name": "Globex changes",
      "editable": true
    },
    {
      "id": 812,
      "name": "Globex shared",
      "editable": false
    }
  ],
  "note": "the list carries no visibility; run show ID to see whether a view is private"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cpv_show() {
    let r = run_fixture(
        "private_views_native",
        &["changes", "private-views", "show", "811"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "view": {
    "id": 811,
    "name": "Globex changes",
    "default": false,
    "visibility": 3,
    "user_id": 9,
    "group_ids": [],
    "query_hash": null,
    "order_by": "created_at",
    "order_type": "desc",
    "advanced_query_hash": {
      "all": [
        {
          "condition": "planned_start_date",
          "operator": "in_the_last",
          "value": "one_week",
          "type": "default"
        },
        {
          "condition": "workspace_id",
          "operator": "is",
          "value": 7,
          "type": "default"
        }
      ]
    }
  },
  "visibility": "Myself",
  "private": true,
  "workspace_id": 7
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cpv_show_shared() {
    let r = run_fixture(
        "private_views_native",
        &["changes", "private-views", "show", "812"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "view": {
    "id": 812,
    "name": "Globex shared",
    "default": false,
    "visibility": 2,
    "user_id": 9,
    "group_ids": [
      44
    ],
    "query_hash": null,
    "order_by": "created_at",
    "order_type": "desc",
    "advanced_query_hash": {
      "all": [
        {
          "condition": "planned_start_date",
          "operator": "in_the_last",
          "value": "one_week",
          "type": "default"
        },
        {
          "condition": "workspace_id",
          "operator": "is",
          "value": 7,
          "type": "default"
        }
      ]
    }
  },
  "private": false,
  "reason": "the view is shared with groups; only views visible to you alone are supported",
  "workspace_id": 7
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cpv_show_missing() {
    let r = run_fixture(
        "private_views_native",
        &["changes", "private-views", "show", "899"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"HTTP 404 fetching change-filter: no route
"#
    );
}

#[test]
fn cpv_create_dry_run() {
    let r = run_fixture(
        "private_views_native",
        &[
            "changes",
            "private-views",
            "write",
            "811",
            "create",
            "--name",
            "Globex mine",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_private_view_create",
  "method": "POST",
  "path": "/api/_/change-filters",
  "payload": {
    "advanced_query_hash": {
      "all": [
        {
          "condition": "planned_start_date",
          "operator": "in_the_last",
          "type": "default",
          "value": "one_week"
        },
        {
          "condition": "workspace_id",
          "operator": "is",
          "type": "default",
          "value": 7
        }
      ]
    },
    "name": "Globex mine",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": null,
    "visibility": 3,
    "workspace_id": 7
  },
  "plan_hash": "4d7a208879a3091bd65b73325edb73f9f2fbfc71c6c3d3552a47f8b71a91eeee",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cpv_create_needs_hash() {
    let r = run_fixture(
        "private_views_native",
        &[
            "changes",
            "private-views",
            "write",
            "811",
            "create",
            "--name",
            "Globex mine",
            "--yes",
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
fn cpv_create_send() {
    let r = run_fixture(
        "private_views_native",
        &[
            "changes",
            "private-views",
            "write",
            "811",
            "create",
            "--name",
            "Globex mine",
            "--yes",
            "--expect-plan",
            "4d7a208879a3091bd65b73325edb73f9f2fbfc71c6c3d3552a47f8b71a91eeee",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "4d7a208879a3091bd65b73325edb73f9f2fbfc71c6c3d3552a47f8b71a91eeee",
  "verified": true,
  "view_id": 900,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "matched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/change-filters"]);
}

#[test]
fn cpv_create_shared_refused() {
    let r = run_fixture(
        "private_views_native",
        &[
            "changes",
            "private-views",
            "write",
            "812",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"the view is shared with groups; only views visible to you alone are supported
"#
    );
}

#[test]
fn cpv_delete_not_captured() {
    let r = run_fixture(
        "private_views_native",
        &[
            "changes",
            "private-views",
            "write",
            "811",
            "delete",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_private_view_delete",
  "method": "DELETE",
  "path": "/api/_/change-filters/811",
  "payload": null,
  "plan_hash": "0ae41c0fa8ae84e7dd16f052a973cc579c44caaa9b186aa192c5979630b236e7",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cpv_rename_not_captured() {
    let r = run_fixture(
        "private_views_native",
        &[
            "changes",
            "private-views",
            "write",
            "811",
            "rename",
            "--name",
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
  "action": "change_private_view_edit",
  "method": "PUT",
  "path": "/api/_/change-filters/811",
  "payload": {
    "advanced_query_hash": {
      "all": [
        {
          "condition": "planned_start_date",
          "operator": "in_the_last",
          "type": "default",
          "value": "one_week"
        },
        {
          "condition": "workspace_id",
          "operator": "is",
          "type": "default",
          "value": 7
        }
      ]
    },
    "name": "x",
    "order_by": "created_at",
    "order_type": "desc",
    "query_hash": null,
    "visibility": 3,
    "workspace_id": 7
  },
  "plan_hash": "7d6bcb55cf40fe09d628946a4689543b603bda725d6ea1110358f82440327ea9",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn cpv_write_read_failed() {
    let r = run_fixture(
        "private_views_native",
        &[
            "changes",
            "private-views",
            "write",
            "899",
            "create",
            "--name",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read failed (HTTP 404); nothing was sent
"#
    );
}

#[test]
fn tickets_columns_list() {
    let r = run_fixture("list_prefs", &["tickets", "columns", "list"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "name": "ticket_subject",
    "label": "Acme subject",
    "position": 1,
    "selected": true,
    "frozen": false
  },
  {
    "name": "requester",
    "label": "Acme requester",
    "position": 2,
    "selected": true,
    "frozen": false
  },
  {
    "name": "status",
    "label": "Acme state",
    "position": 3,
    "selected": true,
    "frozen": false
  },
  {
    "name": "priority",
    "label": "Acme priority",
    "position": 4,
    "selected": true,
    "frozen": false
  },
  {
    "name": "display_id",
    "label": "Acme ID",
    "position": 5,
    "selected": true,
    "frozen": true
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_columns_set_dry_run() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "status",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "list_columns",
  "method": "POST",
  "path": "/api/_/tickets/headers",
  "payload": {
    "columns": {
      "display_id": 1,
      "status": 2,
      "ticket_subject": 1
    },
    "frozen_columns": [
      "display_id"
    ]
  },
  "plan_hash": "0f135ff2bd4853a16ed00f7abfb26b495cec8a3bf24536187685b00b1ce1404c",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_columns_set_send() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "status",
            "--yes",
            "--expect-plan",
            "0f135ff2bd4853a16ed00f7abfb26b495cec8a3bf24536187685b00b1ce1404c",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "0f135ff2bd4853a16ed00f7abfb26b495cec8a3bf24536187685b00b1ce1404c",
  "verified": true,
  "fields": {
    "display_id": "matched",
    "status": "matched",
    "ticket_subject": "matched"
  },
  "unverified_omissions": [],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/headers"]);
}

#[test]
fn tickets_columns_set_needs_hash() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "status",
            "--yes",
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
fn tickets_export_dry_run() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "list_export",
  "method": "POST",
  "path": "/api/_/tickets/export",
  "payload": {
    "advanced_data_hash": null,
    "custom_fields": {},
    "data_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "47"
        ]
      }
    ],
    "date_filter": 30,
    "export_fields": {
      "display_id": "Acme ID",
      "status_name": "Acme state",
      "subject": "Acme subject"
    },
    "format": "csv",
    "selected_filter_view": {
      "id": "701",
      "is_additional_filters_applied": false,
      "name": "Acme queue"
    },
    "ticket_state_filter": "created_at"
  },
  "plan_hash": "f4883d54c4c1ca579346bd2482264b8603a05f37b7e9f1ee216a5fe006a9fea2",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "recipient": {
    "mailbox": "agent@example.com",
    "note": "the export request carries no recipient field; the server delivers the export to the requesting agent's mailbox, which fsv cannot read back",
    "source": "signed-in agent profile (GET /api/v2/agents/me)"
  },
  "body_keys": [
    "advanced_data_hash",
    "custom_fields",
    "data_hash",
    "date_filter",
    "export_fields",
    "format",
    "selected_filter_view",
    "ticket_state_filter"
  ],
  "view": {
    "id": "701",
    "name": "Acme queue"
  },
  "format": "csv",
  "fields": {
    "display_id": "Acme ID",
    "status_name": "Acme state",
    "subject": "Acme subject"
  },
  "submit_only": true,
  "delivery": "emailed by the server, not downloadable here",
  "note": "Export is submit-only: the server answers 202 and emails the file to the signed-in agent's own mailbox (shown as recipient), usually within 15 minutes and up to 2 hours. fsv cannot read the mailbox, so the result is never verified; check the mailbox and do not resend.",
  "requires": "--yes --expect-plan FULL_HASH --confirm-send-to-me",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_export_send() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--yes",
            "--expect-plan",
            "f4883d54c4c1ca579346bd2482264b8603a05f37b7e9f1ee216a5fe006a9fea2",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"the server emails this export to your own mailbox; pass --confirm-send-to-me with --yes --expect-plan. Nothing was sent
"#
    );
}

#[test]
fn changes_columns_list() {
    let r = run_fixture("list_prefs", &["changes", "columns", "list"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "name": "ticket_subject",
    "label": "Acme subject",
    "position": 1,
    "selected": true,
    "frozen": false
  },
  {
    "name": "requester",
    "label": "Acme requester",
    "position": 2,
    "selected": true,
    "frozen": false
  },
  {
    "name": "status",
    "label": "Acme state",
    "position": 3,
    "selected": true,
    "frozen": false
  },
  {
    "name": "priority",
    "label": "Acme priority",
    "position": 4,
    "selected": true,
    "frozen": false
  },
  {
    "name": "display_id",
    "label": "Acme ID",
    "position": 5,
    "selected": true,
    "frozen": true
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_columns_set_dry_run() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "requester",
            "--column",
            "status",
            "--freeze",
            "requester",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "list_columns",
  "method": "POST",
  "path": "/api/_/changes/headers",
  "payload": {
    "columns": {
      "display_id": 1,
      "requester": 2,
      "status": 3,
      "ticket_subject": 1
    },
    "frozen_columns": [
      "requester",
      "display_id"
    ]
  },
  "plan_hash": "411440d7ecd346284dc9d5149022d32f9a6290caf328b48cff4884efa8da2025",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_columns_set_send() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "requester",
            "--column",
            "status",
            "--freeze",
            "requester",
            "--yes",
            "--expect-plan",
            "411440d7ecd346284dc9d5149022d32f9a6290caf328b48cff4884efa8da2025",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "411440d7ecd346284dc9d5149022d32f9a6290caf328b48cff4884efa8da2025",
  "verified": true,
  "fields": {
    "display_id": "matched",
    "requester": "matched",
    "status": "matched",
    "ticket_subject": "matched"
  },
  "unverified_omissions": [],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes/headers"]);
}

#[test]
fn changes_columns_set_needs_hash() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "requester",
            "--column",
            "status",
            "--freeze",
            "requester",
            "--yes",
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
fn changes_export_dry_run() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "list_export",
  "method": "POST",
  "path": "/api/_/changes/export",
  "payload": {
    "advanced_data_hash": null,
    "custom_fields": {},
    "data_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "47"
        ]
      }
    ],
    "date_filter": 30,
    "export_fields": {
      "display_id": "Acme ID",
      "status_name": "Acme state",
      "subject": "Acme subject"
    },
    "format": "csv",
    "state_filter": "created_at"
  },
  "plan_hash": "e2953e8a9518977c703900adb5ebbf25eb2723d7c1db656e42ec129508be9004",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "recipient": {
    "mailbox": "agent@example.com",
    "note": "the export request carries no recipient field; the server delivers the export to the requesting agent's mailbox, which fsv cannot read back",
    "source": "signed-in agent profile (GET /api/v2/agents/me)"
  },
  "body_keys": [
    "advanced_data_hash",
    "custom_fields",
    "data_hash",
    "date_filter",
    "export_fields",
    "format",
    "state_filter"
  ],
  "view": {
    "id": "701",
    "name": "Acme queue"
  },
  "format": "csv",
  "fields": {
    "display_id": "Acme ID",
    "status_name": "Acme state",
    "subject": "Acme subject"
  },
  "submit_only": true,
  "delivery": "emailed by the server, not downloadable here",
  "note": "Export is submit-only: the server answers 202 and emails the file to the signed-in agent's own mailbox (shown as recipient), usually within 15 minutes and up to 2 hours. fsv cannot read the mailbox, so the result is never verified; check the mailbox and do not resend.",
  "requires": "--yes --expect-plan FULL_HASH --confirm-send-to-me",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_export_send() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--yes",
            "--expect-plan",
            "e2953e8a9518977c703900adb5ebbf25eb2723d7c1db656e42ec129508be9004",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"the server emails this export to your own mailbox; pass --confirm-send-to-me with --yes --expect-plan. Nothing was sent
"#
    );
}

#[test]
fn problems_columns_list() {
    let r = run_fixture("list_prefs", &["problems", "columns", "list"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "name": "ticket_subject",
    "label": "Acme subject",
    "position": 1,
    "selected": true,
    "frozen": false
  },
  {
    "name": "requester",
    "label": "Acme requester",
    "position": 2,
    "selected": true,
    "frozen": false
  },
  {
    "name": "status",
    "label": "Acme state",
    "position": 3,
    "selected": true,
    "frozen": false
  },
  {
    "name": "priority",
    "label": "Acme priority",
    "position": 4,
    "selected": true,
    "frozen": false
  },
  {
    "name": "display_id",
    "label": "Acme ID",
    "position": 5,
    "selected": true,
    "frozen": true
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_columns_set_dry_run() {
    let r = run_fixture(
        "list_prefs",
        &[
            "problems",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "status",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "list_columns",
  "method": "POST",
  "path": "/api/_/problems/headers",
  "payload": {
    "columns": {
      "display_id": 1,
      "status": 2,
      "ticket_subject": 1
    },
    "frozen_columns": [
      "display_id"
    ]
  },
  "plan_hash": "4868655e22518c06b7e185538609025d3d6444039a9816f65408ba408616d0fa",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_columns_set_send() {
    let r = run_fixture(
        "list_prefs",
        &[
            "problems",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "status",
            "--yes",
            "--expect-plan",
            "4868655e22518c06b7e185538609025d3d6444039a9816f65408ba408616d0fa",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "4868655e22518c06b7e185538609025d3d6444039a9816f65408ba408616d0fa",
  "verified": false,
  "fields": {
    "display_id": "mismatched",
    "status": "mismatched",
    "ticket_subject": "matched"
  },
  "unverified_omissions": [
    "requester",
    "priority"
  ],
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/headers"]);
}

#[test]
fn problems_columns_set_needs_hash() {
    let r = run_fixture(
        "list_prefs",
        &[
            "problems",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "status",
            "--yes",
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
fn problems_export_dry_run() {
    let r = run_fixture(
        "list_prefs",
        &[
            "problems",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "list_export",
  "method": "POST",
  "path": "/api/_/problems/export",
  "payload": {
    "custom_fields": {},
    "data_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "47"
        ]
      }
    ],
    "date_filter": 30,
    "export_fields": {
      "display_id": "Acme ID",
      "status_name": "Acme state",
      "subject": "Acme subject"
    },
    "format": "csv",
    "state_filter": "created_at"
  },
  "plan_hash": "c4135df6d36dfed0ea200334023bc867f7d8aa86ecddcbf3ae775b598b899941",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "recipient": {
    "mailbox": "agent@example.com",
    "note": "the export request carries no recipient field; the server delivers the export to the requesting agent's mailbox, which fsv cannot read back",
    "source": "signed-in agent profile (GET /api/v2/agents/me)"
  },
  "body_keys": [
    "custom_fields",
    "data_hash",
    "date_filter",
    "export_fields",
    "format",
    "state_filter"
  ],
  "view": {
    "id": "701",
    "name": "Acme queue"
  },
  "format": "csv",
  "fields": {
    "display_id": "Acme ID",
    "status_name": "Acme state",
    "subject": "Acme subject"
  },
  "submit_only": true,
  "delivery": "emailed by the server, not downloadable here",
  "note": "Export is submit-only: the server answers 202 and emails the file to the signed-in agent's own mailbox (shown as recipient), usually within 15 minutes and up to 2 hours. fsv cannot read the mailbox, so the result is never verified; check the mailbox and do not resend.",
  "requires": "--yes --expect-plan FULL_HASH --confirm-send-to-me",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_export_send() {
    let r = run_fixture(
        "list_prefs",
        &[
            "problems",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--yes",
            "--expect-plan",
            "c4135df6d36dfed0ea200334023bc867f7d8aa86ecddcbf3ae775b598b899941",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"the server emails this export to your own mailbox; pass --confirm-send-to-me with --yes --expect-plan. Nothing was sent
"#
    );
}

#[test]
fn columns_set_display_id() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "display_id",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"choose unique exact runtime column names; display_id is anchored automatically
"#
    );
}

#[test]
fn columns_set_needs_subject() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "status",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"captured preference contract requires ticket_subject
"#
    );
}

#[test]
fn columns_set_duplicate() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "ticket_subject",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"choose unique exact runtime column names; display_id is anchored automatically
"#
    );
}

#[test]
fn columns_set_unknown() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "nope",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"choose unique exact runtime column names; display_id is anchored automatically
"#
    );
}

#[test]
fn columns_set_ticket_freeze() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "requester",
            "--freeze",
            "requester",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only change requester freezing plus the automatic display_id anchor is captured
"#
    );
}

#[test]
fn columns_set_freeze_unselected() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--freeze",
            "requester",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"frozen columns must be unique selected columns
"#
    );
}

#[test]
fn columns_set_freeze_other() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "columns",
            "set",
            "--column",
            "ticket_subject",
            "--column",
            "status",
            "--freeze",
            "status",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only change requester freezing plus the automatic display_id anchor is captured
"#
    );
}

#[test]
fn export_ticket_owner_refused() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "export",
            "--view",
            "701",
            "--field",
            "owner",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"select unique captured export field keys
"#
    );
}

#[test]
fn export_duplicate_field() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "export",
            "--view",
            "701",
            "--field",
            "subject",
            "--field",
            "subject",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"select unique captured export field keys
"#
    );
}

#[test]
fn export_label_unavailable() {
    let r = run_fixture(
        "list_prefs",
        &[
            "problems",
            "export",
            "--view",
            "701",
            "--field",
            "department_name",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"runtime export-field label is unavailable for department_name
"#
    );
}

#[test]
fn export_unknown_view() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "export",
            "--view",
            "Nope",
            "--field",
            "subject",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no view named 'Nope'; views: 'Acme queue', 'Acme advanced'
"#
    );
}

#[test]
fn export_advanced_view() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "export",
            "--view",
            "702",
            "--field",
            "subject",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only ordinary saved-view queries have a captured export contract
"#
    );
}

#[test]
fn export_ambiguous_view() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "export",
            "--view",
            "Acme",
            "--field",
            "subject",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"view 'Acme' is ambiguous: 'Acme queue', 'Acme advanced'
"#
    );
}

#[test]
fn tickets_export_without_dry_run() {
    let r = run_fixture(
        "list_prefs",
        &[
            "tickets",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "list_export",
  "method": "POST",
  "path": "/api/_/tickets/export",
  "payload": {
    "advanced_data_hash": null,
    "custom_fields": {},
    "data_hash": [
      {
        "condition": "status",
        "operator": "is_in",
        "type": "default",
        "value": [
          "47"
        ]
      }
    ],
    "date_filter": 30,
    "export_fields": {
      "display_id": "Acme ID",
      "status_name": "Acme state",
      "subject": "Acme subject"
    },
    "format": "csv",
    "selected_filter_view": {
      "id": "701",
      "is_additional_filters_applied": false,
      "name": "Acme queue"
    },
    "ticket_state_filter": "created_at"
  },
  "plan_hash": "f4883d54c4c1ca579346bd2482264b8603a05f37b7e9f1ee216a5fe006a9fea2",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "recipient": {
    "mailbox": "agent@example.com",
    "note": "the export request carries no recipient field; the server delivers the export to the requesting agent's mailbox, which fsv cannot read back",
    "source": "signed-in agent profile (GET /api/v2/agents/me)"
  },
  "body_keys": [
    "advanced_data_hash",
    "custom_fields",
    "data_hash",
    "date_filter",
    "export_fields",
    "format",
    "selected_filter_view",
    "ticket_state_filter"
  ],
  "view": {
    "id": "701",
    "name": "Acme queue"
  },
  "format": "csv",
  "fields": {
    "display_id": "Acme ID",
    "status_name": "Acme state",
    "subject": "Acme subject"
  },
  "submit_only": true,
  "delivery": "emailed by the server, not downloadable here",
  "note": "Export is submit-only: the server answers 202 and emails the file to the signed-in agent's own mailbox (shown as recipient), usually within 15 minutes and up to 2 hours. fsv cannot read the mailbox, so the result is never verified; check the mailbox and do not resend.",
  "requires": "--yes --expect-plan FULL_HASH --confirm-send-to-me",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_export_yes_refused() {
    let r = run_fixture(
        "list_prefs",
        &[
            "changes",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"review first, then pass --yes with the full --expect-plan hash; nothing was sent
"#
    );
}

#[test]
fn problems_export_expect_plan_refused() {
    let r = run_fixture(
        "list_prefs",
        &[
            "problems",
            "export",
            "--view",
            "Acme queue",
            "--field",
            "subject",
            "--field",
            "status_name",
            "--field",
            "display_id",
            "--expect-plan",
            "abc",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--expect-plan sends only with --yes --confirm-send-to-me; nothing was sent
"#
    );
}

#[test]
fn problem_create_dry_run() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Description=Acme details",
            "--set",
            "Status=Open",
            "--set",
            "Priority=Low",
            "--set",
            "Impact=Low",
            "--set",
            "Acme count=7",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_create",
  "method": "POST",
  "path": "/api/_/problems",
  "payload": {
    "custom_fields": {
      "acme_count": 7
    },
    "description": "<div>Acme details</div>",
    "email": "ann@acme.com",
    "impact": 43,
    "priority": 42,
    "status": 41,
    "subject": "Acme problem"
  },
  "plan_hash": "8f8f4875eb288c360bf807fe2e2af309513aa2f776748b6cf1572396928980d5",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "readback_note": "Problem reads do not expose the requester email; the readback proves the requester by identity (requester id or nested requester) instead.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_create_send() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Description=Acme details",
            "--set",
            "Status=Open",
            "--set",
            "Priority=Low",
            "--set",
            "Impact=Low",
            "--set",
            "Acme count=7",
            "--yes",
            "--expect-plan",
            "8f8f4875eb288c360bf807fe2e2af309513aa2f776748b6cf1572396928980d5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8f8f4875eb288c360bf807fe2e2af309513aa2f776748b6cf1572396928980d5",
  "verified": true,
  "problem_id": 456,
  "fields": {
    "custom_fields": "matched",
    "description": "matched",
    "email": "matched",
    "impact": "matched",
    "priority": "matched",
    "status": "matched",
    "subject": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems"]);
}

#[test]
fn problem_create_needs_hash() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Description=Acme details",
            "--set",
            "Status=Open",
            "--set",
            "Priority=Low",
            "--set",
            "Impact=Low",
            "--set",
            "Acme count=7",
            "--yes",
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
fn problem_create_requester_answer() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Description=Acme details",
            "--set",
            "Status=Open",
            "--set",
            "Priority=Low",
            "--set",
            "Impact=Low",
            "--set",
            "Acme count=7",
            "--set",
            "Requester=ann@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"requester answers are not allowed; use --requester
"#
    );
}

#[test]
fn problem_create_missing_subject() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Status=Open",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Subject is required; Description is required
"#
    );
}

#[test]
fn problem_create_unknown_label() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Description=Acme details",
            "--set",
            "Status=Open",
            "--set",
            "Priority=Low",
            "--set",
            "Impact=Low",
            "--set",
            "Acme count=7",
            "--set",
            "Nope=1",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unknown field: 'Nope'
"#
    );
}

#[test]
fn problem_create_needs_requester() {
    let r = run_fixture("problem_flows", &["problems", "create", "--dry-run"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv problems create [OPTIONS]
Try 'fsv problems create -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Missing option '--requester'.                                                                                                                                                                        │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn problem_note_dry_run() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "PRB-123",
            "note",
            "Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_note",
  "method": "POST",
  "path": "/api/_/problems/123/notes",
  "payload": {
    "body": "Acme"
  },
  "plan_hash": "f2b27cfbf9fa46d49b8b6e6019f4f8740744528783298d3cccfbb718d499fca0",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_task_dry_run() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "123",
            "task",
            "Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_task",
  "method": "POST",
  "path": "/api/_/problems/123/tasks",
  "payload": {
    "notify_before": 52,
    "owner_id": 9,
    "planned_effort": "1m",
    "status": 51,
    "title": "Acme"
  },
  "plan_hash": "4ced54eee18490c66b45e89833fd2401968316eb5ec096a47708db68e1ed7f14",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_note_send() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "PRB-123",
            "note",
            "Acme",
            "--yes",
            "--expect-plan",
            "f2b27cfbf9fa46d49b8b6e6019f4f8740744528783298d3cccfbb718d499fca0",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "f2b27cfbf9fa46d49b8b6e6019f4f8740744528783298d3cccfbb718d499fca0",
  "verified": true,
  "note_id": 456,
  "fields": {
    "body": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/123/notes"]);
}

#[test]
fn problem_task_send() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "123",
            "task",
            "Acme",
            "--yes",
            "--expect-plan",
            "4ced54eee18490c66b45e89833fd2401968316eb5ec096a47708db68e1ed7f14",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "4ced54eee18490c66b45e89833fd2401968316eb5ec096a47708db68e1ed7f14",
  "verified": true,
  "task_id": 456,
  "fields": {
    "notify_before": "matched",
    "owner_id": "matched",
    "planned_effort": "matched",
    "status": "matched",
    "title": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/123/tasks"]);
}

#[test]
fn problem_note_send_mismatch() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "123",
            "note",
            "Other",
            "--yes",
            "--expect-plan",
            "c4cbe305baf91b7a0dfaaae0c5afadcae504eca88d85f43751e4fb3c72c70420",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c4cbe305baf91b7a0dfaaae0c5afadcae504eca88d85f43751e4fb3c72c70420",
  "verified": false,
  "note_id": 456,
  "fields": {
    "body": "mismatched"
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/123/notes"]);
}

#[test]
fn problem_child_bad_kind() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "123",
            "comment",
            "Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unsupported problem child kind
"#
    );
}

#[test]
fn problem_child_blank_text() {
    let r = run_fixture(
        "problem_flows",
        &["problems", "write-child", "123", "note", "  ", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"nonempty text is required
"#
    );
}

#[test]
fn problem_child_bad_target() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "CHN-123",
            "note",
            "Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"prefix CHN not valid for problems (expect PRB)
"#
    );
}

#[test]
fn problem_child_identity_mismatch() {
    let r = run_fixture(
        "problem_flows",
        &[
            "problems",
            "write-child",
            "124",
            "note",
            "Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact problem identity is unavailable
"#
    );
}

#[test]
fn problem_child_needs_yes() {
    let r = run_fixture(
        "problem_flows",
        &["problems", "write-child", "123", "note", "Acme"],
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
fn form_fields_problems() {
    let r = run_fixture("problem_flows", &["form", "fields", "problems"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"         problem form fields (web order; state on a new form)          
┏━━━━━━━━━━━━━┳━━━━━━━━━━━━━┳━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━┓
┃ Label       ┃ Name        ┃ Type         ┃ On load           ┃ Help ┃
┡━━━━━━━━━━━━━╇━━━━━━━━━━━━━╇━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━┩
│ Subject     │ subject     │ text         │ visible, required │      │
│ Description │ description │ richtext     │ visible, required │      │
│ Requester   │ requester   │ lookup_user  │ visible, required │      │
│ Status      │ status      │ dropdown (1) │ visible, required │      │
│ Priority    │ priority    │ dropdown (1) │ visible, required │      │
│ Impact      │ impact      │ dropdown (1) │ visible, required │      │
│ Acme count  │ acme_count  │ number       │ visible           │      │
└─────────────┴─────────────┴──────────────┴───────────────────┴──────┘
choices: fsv form fields problems --choices LABEL
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_fields_problems_choices() {
    let r = run_fixture(
        "problem_flows",
        &["form", "fields", "problems", "--choices", "Status"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#" Status (status): 1 
      choices       
┏━━━━━━━━┳━━━━━━━━━┓
┃ Choice ┃ Meaning ┃
┡━━━━━━━━╇━━━━━━━━━┩
│ Open   │         │
└────────┴─────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_problems() {
    let r = run_fixture("problem_flows", &["form", "explain", "problems"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                    problem form (new)                     
┏━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━━━━━━┓
┃ Field       ┃ State             ┃ Value ┃ Why           ┃
┡━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━━━━━━┩
│ Subject     │ visible, required │       │ form metadata │
│ Description │ visible, required │       │ form metadata │
│ Requester   │ visible, required │       │ form metadata │
│ Status      │ visible, required │ Open  │ form metadata │
│ Priority    │ visible, required │ Low   │ form metadata │
│ Impact      │ visible, required │ Low   │ form metadata │
│ Acme count  │ visible           │       │ form metadata │
└─────────────┴───────────────────┴───────┴───────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_explain_problem_record() {
    let r = run_fixture(
        "problem_flows",
        &[
            "form",
            "explain",
            "problems",
            "--id",
            "PRB-123",
            "--set",
            "Status=Open",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"                    problem form (edit)                    
┏━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┳━━━━━━━┳━━━━━━━━━━━━━━━┓
┃ Field       ┃ State             ┃ Value ┃ Why           ┃
┡━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━╇━━━━━━━╇━━━━━━━━━━━━━━━┩
│ Subject     │ visible, required │ Acme  │ form metadata │
│ Description │ visible, required │       │ form metadata │
│ Requester   │ visible, required │       │ form metadata │
│ Status      │ visible, required │ Open  │ form metadata │
│ Priority    │ visible, required │       │ form metadata │
│ Impact      │ visible, required │       │ form metadata │
│ Acme count  │ visible           │       │ form metadata │
└─────────────┴───────────────────┴───────┴───────────────┘
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn form_unsupported_lists_problems() {
    let r = run_fixture("problem_flows", &["form", "dump", "nope"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: unsupported form 'nope'; supported: changes, tickets, problems
"#
    );
}

#[test]
fn canned_show_api_error_is_uncaught() {
    // Python leaves APIError uncaught (traceback, exit 1).
    let r = run_fixture("canned", &["tickets", "canned", "show", "706"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(
        r.err
            .contains("HTTP 404 fetching canned response: not found"),
        "{}",
        r.err
    );
}

#[test]
fn columns_list_api_error_is_uncaught() {
    let r = run_fixture("private_views", &["problems", "columns", "list"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(r.err.contains("HTTP 404 listing headers"), "{}", r.err);
}

#[test]
fn form_dump_problems() {
    // `taken_at` is the clock, so compare the rest of Python's snapshot.
    let r = run_fixture("problem_flows", &["form", "dump", "problems"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "fsv_form_snapshot",
            "module",
            "form_type",
            "workspace_id",
            "taken_at",
            "fields",
            "business_rules"
        ]
    );
    assert_eq!(v["module"], "problem");
    assert_eq!(v["form_type"], "new_form");
    assert_eq!(v["workspace_id"], serde_json::Value::Null);
    assert_eq!(v["fields"].as_array().unwrap().len(), 7);
}

#[test]
fn update_reviewed_multi_field_dry_run() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--set",
            "Subject=Globex",
            "--set",
            "Cost Code=new",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains(
            r#"  "after": {
    "custom_fields": {
      "cost_code": "new"
    },
    "priority": 73,
    "subject": "Globex"
  },"#
        ),
        "{}",
        r.out
    );
}

#[test]
fn json_only_output_rejects_other_formats() {
    // Click's rich usage panel is not reproduced for clap value errors.
    let r = run_fixture(
        "watchers",
        &["tickets", "watchers", "list", "5", "--output", "csv"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert!(r.requests.is_empty());
}

#[test]
fn missing_option_with_short() {
    let r = run_fixture("watchers", &["config", "defaults", "set", "changes"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv config defaults set [OPTIONS] RESOURCE
Try 'fsv config defaults set -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Missing option '--where' / '-w'.                                                                                                                                                                     │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn missing_secondary_flag() {
    let r = run_fixture("watchers", &["tickets", "watchers", "set", "5"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv tickets watchers set [OPTIONS] ID
Try 'fsv tickets watchers set -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Missing option '--watch'.                                                                                                                                                                            │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn missing_variadic_argument() {
    let r = run_fixture("watchers", &["tickets", "bulk-priority"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv tickets bulk-priority [OPTIONS] ID...
Try 'fsv tickets bulk-priority -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Missing argument 'ID...'.                                                                                                                                                                            │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn missing_argument_before_optional() {
    let r = run_fixture("watchers", &["changes", "lookup"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv changes lookup [OPTIONS] KIND [QUERY]
Try 'fsv changes lookup -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Missing argument 'KIND'.                                                                                                                                                                             │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn missing_argument_without_metavar() {
    let r = run_fixture("watchers", &["changes", "activity"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv changes activity [OPTIONS] ID_
Try 'fsv changes activity -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Missing argument 'ID_'.                                                                                                                                                                              │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn missing_second_argument() {
    let r = run_fixture("watchers", &["changes", "tasks-update", "5"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Usage: fsv changes tasks-update [OPTIONS] CHANGE_ID TASK_ID
Try 'fsv changes tasks-update -h' for help.
╭─ Error ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ Missing argument 'TASK_ID'.                                                                                                                                                                          │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
"#
    );
}

#[test]
fn clone_partial_tasks_403_keeps_new_identity() {
    let r = run_fixture(
        "clone_tasks_403",
        &[
            "changes",
            "clone",
            "CHN-123",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 456,
  "human_display_id": "CHN-456",
  "fsv_clone_steps": {
    "tasks": "failed: HTTP 403 creating tasks: synthetic denial",
    "assets": "ok",
    "planning": "ok"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"created CHN-456
step failed: tasks: HTTP 403 creating tasks: synthetic denial
  note: task 'Verify Acme': owner not copied
  note: tenant workflows may have added their own tasks; compare with the source before relying on the copy
  associated 1 asset(s)
  cloned 1 planning field(s)
error: CHN-456 WAS created but 1 step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on CHN-456 directly.
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/changes",
            "POST /api/_/changes/456/tasks",
            "PUT /api/_/changes/456/assets/associate",
            "POST /api/_/changes/456/planning-fields"
        ]
    );
}

#[test]
fn clone_partial_assets_409_keeps_new_identity() {
    let r = run_fixture(
        "clone_assets_409",
        &[
            "changes",
            "clone",
            "CHN-123",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 456,
  "human_display_id": "CHN-456",
  "fsv_clone_steps": {
    "tasks": "ok",
    "assets": "failed: HTTP 409 updating associate: synthetic denial",
    "planning": "ok"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"created CHN-456
  cloned 1 task(s)
  note: task 'Verify Acme': owner not copied
  note: tenant workflows may have added their own tasks; compare with the source before relying on the copy
step failed: assets: HTTP 409 updating associate: synthetic denial
  cloned 1 planning field(s)
error: CHN-456 WAS created but 1 step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on CHN-456 directly.
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/changes",
            "POST /api/_/changes/456/tasks",
            "PUT /api/_/changes/456/assets/associate",
            "POST /api/_/changes/456/planning-fields"
        ]
    );
}

#[test]
fn clone_partial_planning_429_keeps_new_identity() {
    let r = run_fixture(
        "clone_planning_429",
        &[
            "changes",
            "clone",
            "CHN-123",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 456,
  "human_display_id": "CHN-456",
  "fsv_clone_steps": {
    "tasks": "ok",
    "assets": "ok",
    "planning": "failed: HTTP 429 creating planning-fields: synthetic denial"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"created CHN-456
  cloned 1 task(s)
  note: task 'Verify Acme': owner not copied
  note: tenant workflows may have added their own tasks; compare with the source before relying on the copy
  associated 1 asset(s)
step failed: planning: HTTP 429 creating planning-fields: synthetic denial
error: CHN-456 WAS created but 1 step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on CHN-456 directly.
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/changes",
            "POST /api/_/changes/456/tasks",
            "PUT /api/_/changes/456/assets/associate",
            "POST /api/_/changes/456/planning-fields"
        ]
    );
}

#[test]
fn clone_partial_tasks_500_keeps_new_identity() {
    let r = run_fixture(
        "clone_tasks_500",
        &[
            "changes",
            "clone",
            "CHN-123",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 456,
  "human_display_id": "CHN-456",
  "fsv_clone_steps": {
    "tasks": "failed: HTTP 500 creating tasks: synthetic denial",
    "assets": "ok",
    "planning": "ok"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"created CHN-456
step failed: tasks: HTTP 500 creating tasks: synthetic denial
  note: task 'Verify Acme': owner not copied
  note: tenant workflows may have added their own tasks; compare with the source before relying on the copy
  associated 1 asset(s)
  cloned 1 planning field(s)
error: CHN-456 WAS created but 1 step(s) failed; do not rerun clone (it would duplicate the change). Fix the failed step(s) on CHN-456 directly.
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/changes",
            "POST /api/_/changes/456/tasks",
            "PUT /api/_/changes/456/assets/associate",
            "POST /api/_/changes/456/planning-fields"
        ]
    );
}

#[test]
fn clone_dry_run_counts_side_effects() {
    let r = run_fixture(
        "clone_ok",
        &[
            "changes",
            "clone",
            "CHN-123",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
            "--yes",
            "--json",
            "--dry-run",
            "--keep-task-owners",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "subject": "Acme maintenance",
  "status": 1,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"side effects if run without --dry-run:
  - 1 task(s) created (1 with an owner; owners/past dates kept)
  - tenant workflows may add their own tasks to the new change
  - 1 asset(s) associated
  - 1 planning field(s) copied, 0 attachment(s) re-uploaded
"#
    );
}

#[test]
fn clone_missing_returned_id() {
    let r = run_fixture(
        "clone_noid",
        &[
            "changes",
            "clone",
            "CHN-123",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"created CHN-None
warning: change created but the response had no id; see `fsv journal tail`
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/changes"]);
}

#[test]
fn clone_no_input_needs_yes() {
    let r = run_fixture(
        "clone_ok",
        &[
            "changes",
            "clone",
            "CHN-123",
            "--no-edit",
            "--with-tasks",
            "--with-assets",
            "--with-planning",
            "--json",
            "--no-input",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: clone of CHN-123: --with-tasks: creates tasks that can notify their groups and trigger tenant workflows. Pass --yes to confirm.
"#
    );
}

#[test]
fn tickets_resolve_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "resolve",
            "INC-123",
            "--note",
            "Acme fix <safe>",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_transition",
  "status_label": "Resolved",
  "steps": [
    {
      "method": "PUT",
      "path": "/api/_/tickets/123",
      "payload": {
        "resolution_notes": "<div>Acme fix &lt;safe&gt;</div>",
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0="
      }
    },
    {
      "method": "PUT",
      "path": "/api/_/tickets/123",
      "payload": {
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
        "group_id": null,
        "responder_id": 9000001234,
        "status": 4
      }
    }
  ],
  "plan_hash": "b4117210766e01e05a2ba117ca95d62bbc6e814de1122ee4e58559607227df54",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_resolve_with_answer_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "resolve",
            "123",
            "--note",
            "Acme fix",
            "--set",
            "summary=Globex fix",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_transition",
  "status_label": "Resolved",
  "steps": [
    {
      "method": "PUT",
      "path": "/api/_/tickets/123",
      "payload": {
        "resolution_notes": "<div>Acme fix</div>",
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0="
      }
    },
    {
      "method": "PUT",
      "path": "/api/_/tickets/123",
      "payload": {
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
        "custom_fields": {
          "summary": "Globex fix"
        },
        "group_id": null,
        "responder_id": 9000001234,
        "status": 4
      }
    }
  ],
  "plan_hash": "b43bb269655edfaaac2515776a565782a584830cf7155fd5fbbd7432874500b1",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_resolve_needs_expect_plan() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "resolve",
            "123",
            "--note",
            "Acme fix <safe>",
            "--yes",
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
fn tickets_resolve_wrong_hash() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "resolve",
            "123",
            "--note",
            "Acme fix <safe>",
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
        r#"review with --dry-run, then pass --yes and the full --expect-plan hash
"#
    );
}

#[test]
fn tickets_resolve_send() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "resolve",
            "INC-123",
            "--note",
            "Acme fix <safe>",
            "--yes",
            "--expect-plan",
            "b4117210766e01e05a2ba117ca95d62bbc6e814de1122ee4e58559607227df54",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 123,
  "status_label": "Resolved",
  "plan_hash": "b4117210766e01e05a2ba117ca95d62bbc6e814de1122ee4e58559607227df54",
  "verified": true,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/tickets/123", "PUT /api/_/tickets/123"]
    );
}

#[test]
fn tickets_close_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle",
        &["tickets", "close", "124", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_transition",
  "status_label": "Closed",
  "steps": [
    {
      "method": "PUT",
      "path": "/api/_/tickets/124",
      "payload": {
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
        "group_id": null,
        "responder_id": 9000001234,
        "status": 5
      }
    }
  ],
  "plan_hash": "6af55e3443a89c777fc669f581951ffef0ea523fdaf53a4e088e0a40a40bc5d4",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_close_send() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "close",
            "124",
            "--yes",
            "--expect-plan",
            "6af55e3443a89c777fc669f581951ffef0ea523fdaf53a4e088e0a40a40bc5d4",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 124,
  "status_label": "Closed",
  "plan_hash": "6af55e3443a89c777fc669f581951ffef0ea523fdaf53a4e088e0a40a40bc5d4",
  "verified": true,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/124"]);
}

#[test]
fn tickets_close_needs_resolution_note() {
    let r = run_fixture(
        "ticket_lifecycle",
        &["tickets", "close", "125", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"a persisted resolution note is required; use tickets resolve ID --note TEXT first
"#
    );
}

#[test]
fn tickets_close_required_field_missing() {
    let r = run_fixture(
        "ticket_lifecycle",
        &["tickets", "close", "126", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"required fields: Resolution Summary (summary); supply --set FIELD=VALUE
"#
    );
}

#[test]
fn tickets_close_required_field_answered() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "close",
            "126",
            "--set",
            "summary=Globex fix",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_transition",
  "status_label": "Closed",
  "steps": [
    {
      "method": "PUT",
      "path": "/api/_/tickets/126",
      "payload": {
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
        "custom_fields": {
          "summary": "Globex fix"
        },
        "group_id": null,
        "responder_id": 9000001234,
        "status": 5
      }
    }
  ],
  "plan_hash": "8196396de1ded560bc8aeab3e0f5afd6729a2aa7374e6e3e57fdb7a0fabccd84",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_reopen_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle",
        &["tickets", "reopen", "127", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_transition",
  "status_label": "Open",
  "steps": [
    {
      "method": "PUT",
      "path": "/api/_/tickets/127",
      "payload": {
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
        "group_id": null,
        "responder_id": 9000001234,
        "status": 2
      }
    }
  ],
  "plan_hash": "abee2114683281e4179b61b94d0f4fb1858f6284b2cccedc521e91a395db95b6",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_reopen_send() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "reopen",
            "127",
            "--yes",
            "--expect-plan",
            "abee2114683281e4179b61b94d0f4fb1858f6284b2cccedc521e91a395db95b6",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 127,
  "status_label": "Open",
  "plan_hash": "abee2114683281e4179b61b94d0f4fb1858f6284b2cccedc521e91a395db95b6",
  "verified": true,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/127"]);
}

#[test]
fn tickets_status_pending_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "status",
            "123",
            "pending",
            "--set",
            "reason=Waiting on Acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_transition",
  "status_label": "Pending",
  "steps": [
    {
      "method": "PUT",
      "path": "/api/_/tickets/123",
      "payload": {
        "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
        "custom_fields": {
          "reason": "Waiting on Acme"
        },
        "group_id": null,
        "responder_id": 9000001234,
        "status": 3
      }
    }
  ],
  "plan_hash": "3ed10295fb85b239f29334ab69bf597e068655711dc882397100cbb6fa49c0a6",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_status_unknown_refused() {
    let r = run_fixture(
        "ticket_lifecycle",
        &["tickets", "status", "123", "Bogus", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"status 'Bogus' is not a runtime status label; choices: Open, Pending, Resolved, Closed; nothing was sent
"#
    );
}

#[test]
fn tickets_status_set_without_value_refused() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "status",
            "123",
            "Pending",
            "--set",
            "reason",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set requires unique FIELD=VALUE answers
"#
    );
}

#[test]
fn tickets_status_set_duplicate_refused() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "status",
            "123",
            "Pending",
            "--set",
            "reason=a",
            "--set",
            "reason=b",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set requires unique FIELD=VALUE answers
"#
    );
}

#[test]
fn tickets_status_set_default_field_refused() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "status",
            "123",
            "Pending",
            "--set",
            "status=Open",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Status: --set requires unique writable custom fields
"#
    );
}

#[test]
fn tickets_status_bad_id_refused() {
    let r = run_fixture(
        "ticket_lifecycle",
        &["tickets", "status", "0", "Pending", "--dry-run"],
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
fn tickets_close_readback_mismatch() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "close",
            "128",
            "--yes",
            "--expect-plan",
            "743f1b8b0c77da04ffde4e11b2b6061771856aba75193a57e3827ed3aab2a1cf",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"transition readback mismatch (status is still 4); outcome unknown; do not resend
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/128"]);
}

#[test]
fn tickets_merge_dry_run() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "INC-123",
            "INC-124",
            "INC-125",
            "--note-primary",
            "Primary note",
            "--note-secondary",
            "Secondary note",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_merge",
  "method": "POST",
  "path": "/api/_/merge_tickets/complete_merge",
  "payload": {
    "source": {
      "is_private": true,
      "note": "Secondary note",
      "ticket_ids": [
        124,
        125
      ]
    },
    "target": {
      "is_private": true,
      "note": "Primary note",
      "ticket_id": 123
    }
  },
  "plan_hash": "e3e050d9ed762970c1ad2ec082f99d4bdf5ef7329f132baab35d60ad2b548361",
  "warning": "Irreversible: secondary tickets are closed and their conversations move to the surviving primary. Tenant workflows may notify people. Unknown outcomes must not be resent.",
  "surviving_ticket": 123,
  "merged_tickets": [
    124,
    125
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_merge_public_notes_dry_run() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "123",
            "124",
            "--note-primary",
            "Primary note",
            "--note-secondary",
            "Secondary note",
            "--public-primary",
            "--public-secondary",
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
  "action": "ticket_merge",
  "method": "POST",
  "path": "/api/_/merge_tickets/complete_merge",
  "payload": {
    "source": {
      "is_private": false,
      "note": "Secondary note",
      "ticket_ids": [
        124
      ]
    },
    "target": {
      "is_private": false,
      "note": "Primary note",
      "ticket_id": 123
    }
  },
  "plan_hash": "eff4da966b712ea58c8275e8b6e024d269d630832450b53f949613a16d3206ff",
  "warning": "Irreversible: secondary tickets are closed and their conversations move to the surviving primary. Tenant workflows may notify people. Unknown outcomes must not be resent.",
  "surviving_ticket": 123,
  "merged_tickets": [
    124
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_merge_needs_expect_plan() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "123",
            "124",
            "--note-primary",
            "Primary note",
            "--note-secondary",
            "Secondary note",
            "--yes",
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
fn tickets_merge_short_hash_refused() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "123",
            "124",
            "--note-primary",
            "Primary note",
            "--note-secondary",
            "Secondary note",
            "--yes",
            "--expect-plan",
            "0000000000000000",
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
fn tickets_merge_send() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "INC-123",
            "INC-124",
            "INC-125",
            "--note-primary",
            "Primary note",
            "--note-secondary",
            "Secondary note",
            "--yes",
            "--expect-plan",
            "e3e050d9ed762970c1ad2ec082f99d4bdf5ef7329f132baab35d60ad2b548361",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e3e050d9ed762970c1ad2ec082f99d4bdf5ef7329f132baab35d60ad2b548361",
  "verified": true,
  "surviving_ticket": 123,
  "primary_status_unchanged": true,
  "primary_note": true,
  "secondaries": {
    "124": {
      "closed": true,
      "conversations_moved": true,
      "source_note": true
    },
    "125": {
      "closed": true,
      "conversations_moved": true,
      "source_note": true
    }
  },
  "readback_attempts": 1,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/merge_tickets/complete_merge"]
    );
}

#[test]
fn tickets_merge_readback_unverified() {
    let r = run_fixture(
        "ticket_merge_unverified",
        &[
            "tickets",
            "merge",
            "INC-123",
            "INC-124",
            "INC-125",
            "--note-primary",
            "Primary note",
            "--note-secondary",
            "Secondary note",
            "--yes",
            "--expect-plan",
            "e3e050d9ed762970c1ad2ec082f99d4bdf5ef7329f132baab35d60ad2b548361",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e3e050d9ed762970c1ad2ec082f99d4bdf5ef7329f132baab35d60ad2b548361",
  "verified": false,
  "surviving_ticket": 123,
  "primary_status_unchanged": true,
  "primary_note": false,
  "secondaries": {
    "124": {
      "closed": true,
      "conversations_moved": true,
      "source_note": true
    },
    "125": {
      "closed": true,
      "conversations_moved": true,
      "source_note": true
    }
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/merge_tickets/complete_merge"]
    );
}

#[test]
fn tickets_merge_server_error_unknown() {
    let r = run_fixture(
        "ticket_merge_500",
        &[
            "tickets",
            "merge",
            "INC-123",
            "INC-124",
            "INC-125",
            "--note-primary",
            "Primary note",
            "--note-secondary",
            "Secondary note",
            "--yes",
            "--expect-plan",
            "e3e050d9ed762970c1ad2ec082f99d4bdf5ef7329f132baab35d60ad2b548361",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"write outcome unknown (HTTP 500: synthetic failure); inspect target and journal before retry
"#
    );
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/merge_tickets/complete_merge"]
    );
}

#[test]
fn tickets_merge_same_ticket_refused() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "123",
            "123",
            "--note-primary",
            "A",
            "--note-secondary",
            "B",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"primary and secondary tickets must be distinct
"#
    );
}

#[test]
fn tickets_merge_duplicate_secondary_refused() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "123",
            "124",
            "INC-124",
            "--note-primary",
            "A",
            "--note-secondary",
            "B",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"primary and secondary tickets must be distinct
"#
    );
}

#[test]
fn tickets_merge_blank_note_refused() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "123",
            "124",
            "--note-primary",
            "  ",
            "--note-secondary",
            "B",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"both merge notes must be nonempty text
"#
    );
}

#[test]
fn tickets_merge_bad_id_refused() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "0",
            "124",
            "--note-primary",
            "A",
            "--note-secondary",
            "B",
            "--dry-run",
        ],
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
fn tickets_merge_incomplete_history_refused() {
    let r = run_fixture(
        "ticket_merge",
        &[
            "tickets",
            "merge",
            "123",
            "126",
            "--note-primary",
            "A",
            "--note-secondary",
            "B",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete conversation history is required; no merge attempted
"#
    );
}

#[test]
fn tickets_assign_unassign_dry_run() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "INC-123",
            "--unassign",
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
  "action": "ticket_assign",
  "method": "PUT",
  "path": "/api/_/tickets/123",
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "responder_id": null
  },
  "plan_hash": "31e0babf19bbd9b314c970901a594c2f98db0c95938fce647c5ca0ce83751a97",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "before": [
    {
      "group_id": 81,
      "responder_id": 71,
      "ticket_id": 123
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assign_unassign_needs_expect_plan() {
    let r = run_fixture(
        "ticket_assign",
        &["tickets", "assign", "INC-123", "--unassign", "--yes"],
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
fn tickets_assign_unassign_short_hash_refused() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "INC-123",
            "--unassign",
            "--yes",
            "--expect-plan",
            "0000000000000000",
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
fn tickets_assign_unassign_send() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "INC-123",
            "--unassign",
            "--yes",
            "--expect-plan",
            "31e0babf19bbd9b314c970901a594c2f98db0c95938fce647c5ca0ce83751a97",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "31e0babf19bbd9b314c970901a594c2f98db0c95938fce647c5ca0ce83751a97",
  "verified": true,
  "tickets": {
    "123": {
      "responder_id": "matched"
    }
  },
  "readback_attempts": 1,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/123"]);
}

#[test]
fn tickets_assign_agent_in_group_dry_run() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "124",
            "--agent",
            "agent@example.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_assign",
  "method": "PUT",
  "path": "/api/_/tickets/124",
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "responder_id": 71
  },
  "plan_hash": "63f98b4a0e527acb4d2586fa889c7fd566fab2c599bd9c2006d861acd25bb7c7",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "before": [
    {
      "group_id": 81,
      "responder_id": null,
      "ticket_id": 124
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assign_agent_send() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "124",
            "--agent",
            "agent@example.com",
            "--yes",
            "--expect-plan",
            "63f98b4a0e527acb4d2586fa889c7fd566fab2c599bd9c2006d861acd25bb7c7",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "63f98b4a0e527acb4d2586fa889c7fd566fab2c599bd9c2006d861acd25bb7c7",
  "verified": true,
  "tickets": {
    "124": {
      "responder_id": "matched"
    }
  },
  "readback_attempts": 1,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/124"]);
}

#[test]
fn tickets_assign_group_and_agent_dry_run() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "123",
            "--group",
            "Acme Desk",
            "--agent",
            "Acme Agent",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_assign",
  "method": "PUT",
  "path": "/api/_/tickets/123",
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "group_id": 81,
    "responder_id": 71
  },
  "plan_hash": "51318a40737acbfd1118bdffe6cf16ee65f0e729fb71c4b5e83b16e059971416",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "before": [
    {
      "group_id": 81,
      "responder_id": 71,
      "ticket_id": 123
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assign_group_only_clears_agent_dry_run() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "123",
            "--group",
            "Globex Desk",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_assign",
  "method": "PUT",
  "path": "/api/_/tickets/123",
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "group_id": 82,
    "responder_id": null
  },
  "plan_hash": "99b890fa7d517c117bb4947afe8edee05a0df18c838645fa7d22f5b42104a6ac",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "before": [
    {
      "group_id": 81,
      "responder_id": 71,
      "ticket_id": 123
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assign_agent_not_member_refused() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "123",
            "--agent",
            "Globex Agent",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"agent is not a verified member of the group
"#
    );
}

#[test]
fn tickets_assign_unknown_agent_refused() {
    let r = run_fixture(
        "ticket_assign",
        &["tickets", "assign", "123", "--agent", "Nobody", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"agent must identify exactly one active runtime entry
"#
    );
}

#[test]
fn tickets_assign_no_option_refused() {
    let r = run_fixture(
        "ticket_assign",
        &["tickets", "assign", "123", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"assign one ticket with --agent and/or --group, or --unassign
"#
    );
}

#[test]
fn tickets_assign_unassign_with_agent_refused() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "123",
            "--unassign",
            "--agent",
            "71",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"assign one ticket with --agent and/or --group, or --unassign
"#
    );
}

#[test]
fn tickets_assign_readback_unverified() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "assign",
            "126",
            "--agent",
            "71",
            "--yes",
            "--expect-plan",
            "d1e7ac890a83230073750c9d9fac546c8e625f1aae911d408eccd74e1d91892e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d1e7ac890a83230073750c9d9fac546c8e625f1aae911d408eccd74e1d91892e",
  "verified": false,
  "tickets": {
    "126": {
      "responder_id": "mismatched"
    }
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/126"]);
}

#[test]
fn tickets_pick_up_dry_run() {
    let r = run_fixture(
        "ticket_assign",
        &["tickets", "pick-up", "124", "INC-125", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_pick_up",
  "method": "PUT",
  "path": "/api/_/tickets/pick_up",
  "payload": {
    "ids": [
      124,
      125
    ]
  },
  "plan_hash": "60811dd949803fb31a0ed4728de43304124e2357f82b839775ddc14870d49c7a",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "record_state": {
    "sends": 0,
    "server_version": [
      null,
      null
    ]
  },
  "before": [
    {
      "group_id": 81,
      "responder_id": null,
      "ticket_id": 124
    },
    {
      "group_id": null,
      "responder_id": null,
      "ticket_id": 125
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_pick_up_send() {
    let r = run_fixture(
        "ticket_assign",
        &[
            "tickets",
            "pick-up",
            "124",
            "INC-125",
            "--yes",
            "--expect-plan",
            "60811dd949803fb31a0ed4728de43304124e2357f82b839775ddc14870d49c7a",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "60811dd949803fb31a0ed4728de43304124e2357f82b839775ddc14870d49c7a",
  "verified": true,
  "tickets": {
    "124": {
      "responder_id": "matched"
    },
    "125": {
      "responder_id": "matched"
    }
  },
  "readback_attempts": 1,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/pick_up"]);
}

#[test]
fn tickets_pick_up_duplicate_refused() {
    let r = run_fixture(
        "ticket_assign",
        &["tickets", "pick-up", "124", "INC-124", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket IDs must be positive and unique
"#
    );
}

#[test]
fn reply_set_status_resolved_needs_note() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "reply",
            "123",
            "hi",
            "--set-status",
            "Resolved",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"a persisted resolution note is required; use tickets resolve ID --note TEXT first
"#
    );
}

#[test]
fn reply_set_status_closed_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "reply",
            "124",
            "hi",
            "--set-status",
            "Closed",
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
    "body": "<div>hi</div>",
    "cc_emails": [],
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "full_text": "<div>hi</div>",
    "helpdesk_ticket": {
      "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
      "group_id": null,
      "responder_id": 9000001234,
      "status": 5
    },
    "translate_reply": false
  },
  "status_label": "Closed",
  "plan_hash": "214d6636c44bbedf4051fabd724c47f83cd7ef7fd546c375aa1335d5846396b6",
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
fn reply_set_status_closed_required_field() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "reply",
            "126",
            "hi",
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
        r#"required fields: Resolution Summary (summary); supply --set FIELD=VALUE
"#
    );
}

#[test]
fn reply_set_status_unknown() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "reply",
            "123",
            "hi",
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
fn note_set_status_resolved_needs_note() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "add-note",
            "123",
            "--body",
            "hi",
            "--set-status",
            "Resolved",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"a persisted resolution note is required; use tickets resolve ID --note TEXT first
"#
    );
}

#[test]
fn note_set_status_closed_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "add-note",
            "124",
            "--body",
            "hi",
            "--set-status",
            "Closed",
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
    "body": "<div>hi</div>",
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
  "plan_hash": "d04342994da1822a2a4ba6eae2325006cb9132bc5e13d7001857e230c3d59fed",
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
fn note_set_status_closed_needs_resolution() {
    let r = run_fixture(
        "ticket_lifecycle",
        &[
            "tickets",
            "add-note",
            "125",
            "--body",
            "hi",
            "--set-status",
            "closed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"a persisted resolution note is required; use tickets resolve ID --note TEXT first
"#
    );
}

#[test]
fn reply_set_status_not_pending() {
    let r = run_fixture(
        "ticket_conv",
        &[
            "tickets",
            "reply",
            "5",
            "hi",
            "--set-status",
            "Open",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set-status 'Open' is not a supported status; choices: Pending, Resolved, Closed; nothing was sent
"#
    );
}

#[test]
fn note_set_status_not_pending() {
    let r = run_fixture(
        "ticket_conv",
        &[
            "tickets",
            "add-note",
            "5",
            "--body",
            "hi",
            "--set-status",
            "Open",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set-status 'Open' is not a supported status; choices: Pending, Resolved, Closed; nothing was sent
"#
    );
}

#[test]
fn tickets_add_note_send_verified() {
    let r = run_fixture(
        "ticket_note_send",
        &[
            "tickets",
            "add-note",
            "130",
            "--body",
            "Acme fix",
            "--yes",
            "--expect-plan",
            "8ff83d4f79c3c87f45b818fc68fd86c4327ec6b1cfbddc9487ad270156cf3aa0",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Note 601 submitted on ticket 130; exact-ID read-back: True; all fields verified: True
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_note",
  "tenant": "acme.freshservice.com",
  "ticket_id": 130,
  "visibility": "private note",
  "path": "/api/_/tickets/130/notes",
  "method": "POST",
  "payload": {
    "body": "<div>Acme fix</div>",
    "conversation_from_summary": false,
    "private": true,
    "notify_emails": []
  },
  "body_text": "Acme fix",
  "plan_hash": "8ff83d4f79c3c87f45b818fc68fd86c4327ec6b1cfbddc9487ad270156cf3aa0",
  "record_state": {
    "sends": 0
  },
  "risks": [],
  "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications.",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/130/notes"]);
}

#[test]
fn tickets_add_note_send_fields_unverified() {
    let r = run_fixture(
        "ticket_note_send",
        &[
            "tickets",
            "add-note",
            "131",
            "--body",
            "Acme fix",
            "--yes",
            "--expect-plan",
            "4ed3ceeaf6d0e449139c4ab21dc237bba1bffd898260aa9fc056cf03c2653a09",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Note 602 submitted on ticket 131; exact-ID read-back: True; all fields verified: False
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_note",
  "tenant": "acme.freshservice.com",
  "ticket_id": 131,
  "visibility": "private note",
  "path": "/api/_/tickets/131/notes",
  "method": "POST",
  "payload": {
    "body": "<div>Acme fix</div>",
    "conversation_from_summary": false,
    "private": true,
    "notify_emails": []
  },
  "body_text": "Acme fix",
  "plan_hash": "4ed3ceeaf6d0e449139c4ab21dc237bba1bffd898260aa9fc056cf03c2653a09",
  "record_state": {
    "sends": 0
  },
  "risks": [],
  "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications.",
  "config_dir": "<CONFIG_DIR>"
}
Read-back did not fully verify the note fields and history; do not resend.
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/131/notes"]);
}

#[test]
fn tickets_add_note_send_json() {
    let r = run_fixture(
        "ticket_note_send",
        &[
            "tickets",
            "add-note",
            "130",
            "--body",
            "Acme fix",
            "--json",
            "--yes",
            "--expect-plan",
            "8ff83d4f79c3c87f45b818fc68fd86c4327ec6b1cfbddc9487ad270156cf3aa0",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 130,
  "note_id": 601,
  "note": {
    "id": 601,
    "body": "<div>Acme fix</div>",
    "body_text": "<div>Acme fix</div>",
    "source": 2,
    "private": true,
    "user_id": 9000001234,
    "created_at": "2030-01-01T00:00:00Z"
  },
  "plan_hash": "8ff83d4f79c3c87f45b818fc68fd86c4327ec6b1cfbddc9487ad270156cf3aa0",
  "verified": true,
  "readback": {
    "id": 601,
    "body": "<div>Acme\n fix</div>",
    "body_text": "<div>Acme\n fix</div>",
    "source": 2,
    "private": true,
    "user_id": 9000001234,
    "created_at": "2030-01-01T00:00:00Z",
    "to_emails": []
  },
  "identity_verified": true,
  "fields_verified": true,
  "readback_complete": true,
  "fields": {
    "body": "matched",
    "private": "matched",
    "notify_emails": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "ticket_note",
  "tenant": "acme.freshservice.com",
  "ticket_id": 130,
  "visibility": "private note",
  "path": "/api/_/tickets/130/notes",
  "method": "POST",
  "payload": {
    "body": "<div>Acme fix</div>",
    "conversation_from_summary": false,
    "private": true,
    "notify_emails": []
  },
  "body_text": "Acme fix",
  "plan_hash": "8ff83d4f79c3c87f45b818fc68fd86c4327ec6b1cfbddc9487ad270156cf3aa0",
  "record_state": {
    "sends": 0
  },
  "risks": [],
  "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications.",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/130/notes"]);
}

#[test]
fn tickets_note_edit_dry_run() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-edit",
            "INC-123",
            "501",
            "--body",
            "Globex final",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_note_edit",
  "method": "PUT",
  "path": "/api/_/conversations/501",
  "payload": {
    "body": "<div>Globex final</div>",
    "conversation_from_summary": false
  },
  "plan_hash": "7f212d2e3ebfc8083cd0be6eb6b2203028b09a8360dd32756d2584f4d085bd8e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "ticket_id": 123,
  "before": {
    "id": 501,
    "body": "<div>Acme draft</div>",
    "body_text": "<div>Acme draft</div>",
    "source": 2,
    "private": true,
    "user_id": 9000001234,
    "created_at": "2030-01-01T00:00:00Z"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_note_edit_needs_expect_plan() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-edit",
            "123",
            "501",
            "--body",
            "Globex final",
            "--yes",
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
fn tickets_note_edit_send() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-edit",
            "INC-123",
            "501",
            "--body",
            "Globex final",
            "--yes",
            "--expect-plan",
            "7f212d2e3ebfc8083cd0be6eb6b2203028b09a8360dd32756d2584f4d085bd8e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7f212d2e3ebfc8083cd0be6eb6b2203028b09a8360dd32756d2584f4d085bd8e",
  "verified": true,
  "conversation_id": 501,
  "history_complete": true,
  "body": "matched",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/conversations/501"]);
}

#[test]
fn tickets_note_edit_readback_unverified() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-edit",
            "126",
            "503",
            "--body",
            "Globex final",
            "--yes",
            "--expect-plan",
            "b9a34cf3d5e44006c6e0101d3b8c7275fe97b15818e6a4ce3379e8620e2d5287",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "b9a34cf3d5e44006c6e0101d3b8c7275fe97b15818e6a4ce3379e8620e2d5287",
  "verified": false,
  "conversation_id": 503,
  "history_complete": true,
  "body": "mismatched",
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/conversations/503"]);
}

#[test]
fn tickets_note_delete_dry_run() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-delete",
            "124",
            "502",
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
  "action": "ticket_note_delete",
  "method": "DELETE",
  "path": "/api/_/conversations/502",
  "payload": null,
  "plan_hash": "9aa8b0caf5238ba35d46c39af484ea7baf44673a66076f6c7e9f576166a94648",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "ticket_id": 124,
  "before": {
    "id": 502,
    "body": "<div>Acme stale</div>",
    "body_text": "<div>Acme stale</div>",
    "source": 2,
    "private": true,
    "user_id": 9000001234,
    "created_at": "2030-01-01T00:00:00Z"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_note_delete_send() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-delete",
            "124",
            "502",
            "--yes",
            "--expect-plan",
            "9aa8b0caf5238ba35d46c39af484ea7baf44673a66076f6c7e9f576166a94648",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "9aa8b0caf5238ba35d46c39af484ea7baf44673a66076f6c7e9f576166a94648",
  "verified": true,
  "conversation_id": 502,
  "history_complete": true,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/conversations/502"]);
}

#[test]
fn tickets_note_edit_not_a_note_refused() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-edit",
            "125",
            "500",
            "--body",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"conversation is not a verified note
"#
    );
}

#[test]
fn tickets_note_edit_missing_note_refused() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-edit",
            "125",
            "599",
            "--body",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact note not found or ambiguous
"#
    );
}

#[test]
fn tickets_note_delete_bad_id_refused() {
    let r = run_fixture(
        "ticket_note_mutation",
        &["tickets", "note-delete", "124", "0", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket and conversation IDs must be positive
"#
    );
}

#[test]
fn tickets_note_edit_unknown_history_refused() {
    let r = run_fixture(
        "ticket_note_mutation",
        &[
            "tickets",
            "note-edit",
            "127",
            "501",
            "--body",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete conversation history unavailable; no write attempted
"#
    );
}

#[test]
fn problems_link_incident_dry_run() {
    let r = run_fixture(
        "problem_incidents",
        &["problems", "link-incident", "PRB-7", "INC-123", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_link_incident",
  "method": "PUT",
  "path": "/api/_/problems/7/tickets/associate",
  "payload": {
    "item_ids": [
      123
    ]
  },
  "plan_hash": "70f59b4ac3d96a5eaa5a367ee16f1af9a1a6b47f8482d0343a242be363358e3e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "problem_id": 7,
  "incident_id": "INC-123",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_link_incident_needs_expect_plan() {
    let r = run_fixture(
        "problem_incidents",
        &["problems", "link-incident", "7", "123", "--yes"],
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
fn problems_link_incident_send() {
    let r = run_fixture(
        "problem_incidents",
        &[
            "problems",
            "link-incident",
            "PRB-7",
            "INC-123",
            "--yes",
            "--expect-plan",
            "70f59b4ac3d96a5eaa5a367ee16f1af9a1a6b47f8482d0343a242be363358e3e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "70f59b4ac3d96a5eaa5a367ee16f1af9a1a6b47f8482d0343a242be363358e3e",
  "verified": true,
  "problem_id": 7,
  "incident_id": 123,
  "linked": true,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/7/tickets/associate"]);
}

#[test]
fn problems_link_incident_readback_unverified() {
    let r = run_fixture(
        "problem_incidents",
        &[
            "problems",
            "link-incident",
            "9",
            "123",
            "--yes",
            "--expect-plan",
            "6b2b2bab39b8460d05e20b9eb0ceb550d096766af8f486de3213dbe238538886",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "6b2b2bab39b8460d05e20b9eb0ceb550d096766af8f486de3213dbe238538886",
  "verified": false,
  "problem_id": 9,
  "incident_id": 123,
  "linked": false,
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/9/tickets/associate"]);
}

#[test]
fn problems_unlink_incident_dry_run() {
    let r = run_fixture(
        "problem_incidents",
        &[
            "problems",
            "unlink-incident",
            "8",
            "INC-124",
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
  "action": "problem_unlink_incident",
  "method": "DELETE",
  "path": "/api/_/problems/8/tickets/124",
  "payload": {
    "type": "unlink"
  },
  "plan_hash": "eaf62885632a511a268634126f627dcd07693e2ecfa8e414d67c0e3e9fe848aa",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "problem_id": 8,
  "incident_id": "INC-124",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_unlink_incident_send() {
    let r = run_fixture(
        "problem_incidents",
        &[
            "problems",
            "unlink-incident",
            "8",
            "INC-124",
            "--yes",
            "--expect-plan",
            "eaf62885632a511a268634126f627dcd07693e2ecfa8e414d67c0e3e9fe848aa",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "eaf62885632a511a268634126f627dcd07693e2ecfa8e414d67c0e3e9fe848aa",
  "verified": true,
  "problem_id": 8,
  "incident_id": 124,
  "linked": false,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/problems/8/tickets/124"]);
}

#[test]
fn problems_link_incident_already_linked_refused() {
    let r = run_fixture(
        "problem_incidents",
        &["problems", "link-incident", "8", "124", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"incident is already in the requested association state
"#
    );
}

#[test]
fn problems_unlink_incident_not_linked_refused() {
    let r = run_fixture(
        "problem_incidents",
        &["problems", "unlink-incident", "7", "124", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"incident is already in the requested association state
"#
    );
}

#[test]
fn problems_link_incident_sr_prefix_refused() {
    let r = run_fixture(
        "problem_incidents",
        &["problems", "link-incident", "7", "SR-125", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"incident ID must use the INC prefix, not SR
"#
    );
}

#[test]
fn problems_link_incident_not_incident_refused() {
    let r = run_fixture(
        "problem_incidents",
        &["problems", "link-incident", "7", "125", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket is not a verified incident
"#
    );
}

#[test]
fn problems_link_incident_bad_id_refused() {
    let r = run_fixture(
        "problem_incidents",
        &["problems", "link-incident", "0", "123", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"problem and incident IDs must be positive
"#
    );
}

#[test]
fn problems_status_dry_run() {
    let r = run_fixture(
        "problem_status",
        &[
            "problems",
            "status",
            "PRB-7",
            "--to",
            "Change Requested",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_status",
  "method": "PUT",
  "path": "/api/_/problems/7",
  "payload": {
    "status": 2
  },
  "plan_hash": "b2f499363bd03c95d0aac9943d6db8ad17beb8c203d2788c2765598bb7489128",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "to": "Change Requested",
  "before": {
    "custom_fields": {
      "cf_owner_note": "Acme owner"
    },
    "human_display_id": "PRB-7",
    "id": 7,
    "planned_end_date": null,
    "planned_start_date": null,
    "status": 1,
    "subject": "Acme problem 7"
  },
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_status_needs_expect_plan() {
    let r = run_fixture(
        "problem_status",
        &[
            "problems",
            "status",
            "7",
            "--to",
            "Change Requested",
            "--yes",
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
fn problems_status_send_reports_planned_dates() {
    let r = run_fixture(
        "problem_status",
        &[
            "problems",
            "status",
            "PRB-7",
            "--to",
            "Change Requested",
            "--yes",
            "--expect-plan",
            "b2f499363bd03c95d0aac9943d6db8ad17beb8c203d2788c2765598bb7489128",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "b2f499363bd03c95d0aac9943d6db8ad17beb8c203d2788c2765598bb7489128",
  "verified": true,
  "problem_id": 7,
  "readback": {
    "status": 2,
    "planned_start_date": "2030-01-02T00:00:00Z",
    "planned_end_date": "2030-01-03T00:00:00Z"
  },
  "unavailable": [],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/7"]);
}

#[test]
fn problems_status_close_dry_run() {
    let r = run_fixture(
        "problem_status",
        &["problems", "status", "8", "--to", "closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_status",
  "method": "PUT",
  "path": "/api/_/problems/8",
  "payload": {
    "status": 3
  },
  "plan_hash": "8a86188acbcd704999a7ac0b4c0dd17da2acef8e9776544fa026d3fa82d4e4da",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "to": "Closed",
  "before": {
    "custom_fields": {
      "cf_owner_note": "Acme owner"
    },
    "human_display_id": "PRB-8",
    "id": 8,
    "planned_end_date": null,
    "planned_start_date": null,
    "status": 2,
    "subject": "Acme problem 8"
  },
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "closure_check": {
    "analysis": "Root Cause, Impact and Symptoms are filled",
    "linked_solution": "linked (1)"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_status_close_send() {
    let r = run_fixture(
        "problem_status",
        &[
            "problems",
            "status",
            "8",
            "--to",
            "closed",
            "--yes",
            "--expect-plan",
            "8a86188acbcd704999a7ac0b4c0dd17da2acef8e9776544fa026d3fa82d4e4da",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8a86188acbcd704999a7ac0b4c0dd17da2acef8e9776544fa026d3fa82d4e4da",
  "verified": true,
  "problem_id": 8,
  "readback": {
    "status": 3,
    "planned_start_date": null,
    "planned_end_date": null
  },
  "unavailable": [],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/8"]);
}

#[test]
fn problems_status_close_rejected_by_server() {
    let r = run_fixture(
        "problem_status_rejected",
        &[
            "problems",
            "status",
            "8",
            "--to",
            "closed",
            "--yes",
            "--expect-plan",
            "8a86188acbcd704999a7ac0b4c0dd17da2acef8e9776544fa026d3fa82d4e4da",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"violated_closure_rules: Root Cause, Impact and Symptoms must be filled; a workaround / solution must be linked
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/8"]);
}

#[test]
fn problems_status_close_missing_analysis_refused() {
    let r = run_fixture(
        "problem_status",
        &["problems", "status", "9", "--to", "Closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"violated_closure_rules: missing Root Cause, Impact, Symptoms
"#
    );
}

#[test]
fn problems_status_close_required_field_refused() {
    let r = run_fixture(
        "problem_status",
        &["problems", "status", "10", "--to", "Closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"violated_closure_rules: missing required closure fields: Owner Note
"#
    );
}

#[test]
fn problems_status_unknown_refused() {
    let r = run_fixture(
        "problem_status",
        &["problems", "status", "7", "--to", "Bogus", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"status must match one active runtime problem-status ID or label; 'Bogus' is not one (options: Open, Change Requested, Closed)
"#
    );
}

#[test]
fn problems_status_readback_unverified() {
    let r = run_fixture(
        "problem_status",
        &[
            "problems",
            "status",
            "11",
            "--to",
            "Change Requested",
            "--yes",
            "--expect-plan",
            "75d781164d08c97a85b6606d23751e4f560a0b9b4a14353707fbb9bb7d24619d",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "75d781164d08c97a85b6606d23751e4f560a0b9b4a14353707fbb9bb7d24619d",
  "verified": false,
  "problem_id": 11,
  "readback": {
    "status": 1,
    "planned_start_date": null,
    "planned_end_date": null
  },
  "unavailable": [],
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/11"]);
}

#[test]
fn problems_update_planned_dry_run() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "PRB-71",
            "--planned-start",
            "2030-01-02T09:00:00+00:00",
            "--planned-end",
            "2030-01-03T17:00:00Z",
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
  "action": "problem_planning",
  "method": "PUT",
  "path": "/api/_/problems/71",
  "payload": {
    "planned_effort": "1h 30m",
    "planned_end_date": "2030-01-03T17:00:00+00:00",
    "planned_start_date": "2030-01-02T09:00:00+00:00"
  },
  "plan_hash": "ecb5a82c539c91c2fff9481bf54de5f29abee779960c9130ec029a05759db8d5",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "before": {
    "planned_effort": null,
    "planned_end_date": null,
    "planned_start_date": null
  },
  "after": {
    "planned_effort": "1h 30m",
    "planned_end_date": "2030-01-03T17:00:00+00:00",
    "planned_start_date": "2030-01-02T09:00:00+00:00"
  },
  "side_effects": "Tenant workflows may react to planning changes.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_update_planned_send() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "71",
            "--planned-start",
            "2030-01-02T09:00:00+00:00",
            "--planned-end",
            "2030-01-03T17:00:00Z",
            "--planned-effort",
            "01:30",
            "--yes",
            "--expect-plan",
            "ecb5a82c539c91c2fff9481bf54de5f29abee779960c9130ec029a05759db8d5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ecb5a82c539c91c2fff9481bf54de5f29abee779960c9130ec029a05759db8d5",
  "verified": true,
  "problem_id": 71,
  "fields": {
    "planned_effort": "matched",
    "planned_end_date": "matched",
    "planned_start_date": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/71"]);
}

#[test]
fn problems_update_planned_send_unverified() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "76",
            "--planned-effort",
            "2h",
            "--yes",
            "--expect-plan",
            "9548e490a0f0cc3a45621cba3cd123250ba3948806e06b667cd1ad103b83bc57",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "9548e490a0f0cc3a45621cba3cd123250ba3948806e06b667cd1ad103b83bc57",
  "verified": false,
  "problem_id": 76,
  "fields": {
    "planned_effort": "mismatched"
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/76"]);
}

#[test]
fn problems_update_planned_needs_expect_plan() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "71",
            "--planned-effort",
            "2h",
            "--yes",
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
fn problems_update_planned_with_other_fields() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "71",
            "--planned-effort",
            "2h",
            "--status",
            "Closed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: problem planned dates/effort are reviewed on their own; send other fields separately
"#
    );
}

#[test]
fn problems_update_planned_end_before_start() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "71",
            "--planned-start",
            "2030-01-03T00:00:00Z",
            "--planned-end",
            "2030-01-02T00:00:00Z",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planned end must not precede planned start
"#
    );
}

#[test]
fn problems_update_planned_deleted_refused() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "72",
            "--planned-effort",
            "2h",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"problem 72 is deleted (in trash); restore it before updating
"#
    );
}

#[test]
fn problems_update_planned_rules_unavailable() {
    let r = run_fixture(
        "problem_update_rules500",
        &[
            "problems",
            "update",
            "71",
            "--planned-effort",
            "2h",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_planning",
  "method": "PUT",
  "path": "/api/_/problems/71",
  "payload": {
    "planned_effort": "2h 0m"
  },
  "plan_hash": "c9b0020f32ba167e51c0e5893a72feb9559abd44d0c7c618d6423cddf91f23f7",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "before": {
    "planned_effort": null
  },
  "after": {
    "planned_effort": "2h 0m"
  },
  "side_effects": "Tenant workflows may react to planning changes.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warnings": [
    "rules unavailable (the workspace-less path answered HTTP 500 (server error)): conditional mandates, field locks, choice limits and validation messages of the web form were not checked; the server still enforces the static required fields and its own closure rules"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_update_planned_effort_refused() {
    let r = run_fixture(
        "problem_update",
        &[
            "tickets",
            "update",
            "5",
            "--planned-effort",
            "2h",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --planned-effort is supported only for problems
"#
    );
}

#[test]
fn problems_update_custom_field_dry_run() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "74",
            "--status",
            "Change Requested",
            "--set",
            "cf_owner_note=Acme new owner",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: problem status changes go through `fsv problems status ID --to STATUS` (a reviewed transition with closure checks); nothing sent
"#
    );
}

#[test]
fn problems_update_custom_field_send() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "74",
            "--set",
            "Owner Note=Acme new owner",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "error",
  "plan_hash": "58ac4ed671cec0ec08384728c6801f1a1737c7744539cc2bbd0d5317151e070b",
  "verified": false,
  "error": "--yes requires the full --expect-plan from a prior --dry-run review",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"--yes requires the full --expect-plan from a prior --dry-run review
"#
    );
}

#[test]
fn problems_update_deleted_refused() {
    let r = run_fixture(
        "problem_update",
        &["problems", "update", "72", "--status", "Closed", "--yes"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: problem status changes go through `fsv problems status ID --to STATUS` (a reviewed transition with closure checks); nothing sent
"#
    );
}

#[test]
fn problems_update_not_editable_405() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "73",
            "--set",
            "cf_owner_note=x",
            "--yes",
        ],
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
fn problem_create_rules_unavailable_dry_run() {
    let r = run_fixture(
        "problem_flows_rules500",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Description=Acme details",
            "--set",
            "Status=Open",
            "--set",
            "Priority=Low",
            "--set",
            "Impact=Low",
            "--set",
            "Acme count=7",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_create",
  "method": "POST",
  "path": "/api/_/problems",
  "payload": {
    "custom_fields": {
      "acme_count": 7
    },
    "description": "<div>Acme details</div>",
    "email": "ann@acme.com",
    "impact": 43,
    "priority": 42,
    "status": 41,
    "subject": "Acme problem"
  },
  "plan_hash": "2f4f1d3ff8be7f63918b108dd1a56748fa5c2b93aff196454e487cb34a2ba5da",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "warnings": [
    "rules unavailable (the workspace-less path answered HTTP 500 (server error)): conditional mandates, field locks, choice limits and validation messages of the web form were not checked; the server still enforces the static required fields and its own closure rules"
  ],
  "readback_note": "Problem reads do not expose the requester email; the readback proves the requester by identity (requester id or nested requester) instead.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_create_rules_unavailable_send() {
    let r = run_fixture(
        "problem_flows_rules500",
        &[
            "problems",
            "create",
            "--requester",
            "ann@acme.com",
            "--set",
            "Subject=Acme problem",
            "--set",
            "Description=Acme details",
            "--set",
            "Status=Open",
            "--set",
            "Priority=Low",
            "--set",
            "Impact=Low",
            "--set",
            "Acme count=7",
            "--yes",
            "--expect-plan",
            "2f4f1d3ff8be7f63918b108dd1a56748fa5c2b93aff196454e487cb34a2ba5da",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2f4f1d3ff8be7f63918b108dd1a56748fa5c2b93aff196454e487cb34a2ba5da",
  "verified": true,
  "problem_id": 456,
  "fields": {
    "custom_fields": "matched",
    "description": "matched",
    "email": "matched",
    "impact": "matched",
    "priority": "matched",
    "status": "matched",
    "subject": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems"]);
}

#[test]
fn problems_status_dry_run_rules_unavailable() {
    let r = run_fixture(
        "problem_status_rules500",
        &[
            "problems",
            "status",
            "PRB-7",
            "--to",
            "Change Requested",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_status",
  "method": "PUT",
  "path": "/api/_/problems/7",
  "payload": {
    "status": 2
  },
  "plan_hash": "e6750646b441c58cb20d420edad81fee5edd87167d0050ef0c4aeb4c98afd40d",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "to": "Change Requested",
  "before": {
    "custom_fields": {
      "cf_owner_note": "Acme owner"
    },
    "human_display_id": "PRB-7",
    "id": 7,
    "planned_end_date": null,
    "planned_start_date": null,
    "status": 1,
    "subject": "Acme problem 7"
  },
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "warnings": [
    "rules unavailable (the workspace-less path answered HTTP 500 (server error)): conditional mandates, field locks, choice limits and validation messages of the web form were not checked; the server still enforces the static required fields and its own closure rules"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_status_needs_expect_plan_rules_unavailable() {
    let r = run_fixture(
        "problem_status_rules500",
        &[
            "problems",
            "status",
            "7",
            "--to",
            "Change Requested",
            "--yes",
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
fn tickets_tasks_list() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &["tickets", "tasks", "list", "INC-125"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 904,
    "title": "Acme cleanup",
    "status": 1,
    "notify_before": 0,
    "custom_fields": {}
  },
  {
    "id": 905,
    "title": "Acme notify",
    "status": 1,
    "notify_before": 0,
    "custom_fields": {}
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_tasks_add_dry_run() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "INC-123",
            "--title",
            "Acme rollback check",
            "--set",
            "Area=Network",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_task_add",
  "method": "POST",
  "path": "/api/_/tickets/123/tasks",
  "payload": {
    "custom_fields": {
      "cf_area": "Network"
    },
    "notify_before": 0,
    "status": 1,
    "title": "Acme rollback check"
  },
  "plan_hash": "f8d4869ee62d0e3cf9e2d4c650f1114fb444fdada35b83abfef9d7c79818c0ae",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_tasks_add_needs_expect_plan() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            "Acme rollback check",
            "--yes",
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
fn tickets_tasks_add_send() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "INC-123",
            "--title",
            "Acme rollback check",
            "--set",
            "Area=Network",
            "--yes",
            "--expect-plan",
            "f8d4869ee62d0e3cf9e2d4c650f1114fb444fdada35b83abfef9d7c79818c0ae",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "f8d4869ee62d0e3cf9e2d4c650f1114fb444fdada35b83abfef9d7c79818c0ae",
  "verified": true,
  "task_id": 902,
  "fields": {
    "custom_fields": "matched",
    "notify_before": "matched",
    "status": "matched",
    "title": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/tasks"]);
}

#[test]
fn tickets_tasks_add_choice_field_dry_run() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            "Acme tiered",
            "--status",
            "In Progress",
            "--set",
            "cf_tier=gold",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_task_add",
  "method": "POST",
  "path": "/api/_/tickets/123/tasks",
  "payload": {
    "custom_fields": {
      "cf_tier": 1
    },
    "notify_before": 0,
    "status": 2,
    "title": "Acme tiered"
  },
  "plan_hash": "efee13b11510dcc68a24fe674c3b58b04b043696ee42841df407289090a7ca59",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_tasks_add_blank_title_refused() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            " ",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"nonempty task title without controls required
"#
    );
}

#[test]
fn tickets_tasks_add_unknown_field_refused() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            "x",
            "--set",
            "Bogus=1",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"custom field must match unique runtime name or label: Bogus
"#
    );
}

#[test]
fn tickets_tasks_add_default_field_refused() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            "x",
            "--set",
            "notify_before=Never",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set accepts only proven custom task fields
"#
    );
}

#[test]
fn tickets_tasks_add_set_without_value_refused() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            "x",
            "--set",
            "Area",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--set requires unique FIELD=VALUE
"#
    );
}

#[test]
fn tickets_tasks_status_dry_run() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "status",
            "124",
            "903",
            "--to",
            "Completed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_task_status",
  "method": "PUT",
  "path": "/api/_/tickets/124/tasks/903",
  "payload": {
    "status": 3
  },
  "plan_hash": "8927b43902641feff3e8b6afbc6df78374ae8468a1a2e67238f5f5f673e77a16",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_tasks_status_send() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "status",
            "124",
            "903",
            "--to",
            "Completed",
            "--yes",
            "--expect-plan",
            "8927b43902641feff3e8b6afbc6df78374ae8468a1a2e67238f5f5f673e77a16",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8927b43902641feff3e8b6afbc6df78374ae8468a1a2e67238f5f5f673e77a16",
  "verified": true,
  "task_id": 903,
  "fields": {
    "status": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/124/tasks/903"]);
}

#[test]
fn tickets_tasks_status_unknown_refused() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "status",
            "124",
            "903",
            "--to",
            "Bogus",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"status must match one active runtime task-status ID or label; 'Bogus' is not one (options: Open, In Progress, Completed)
"#
    );
}

#[test]
fn tickets_tasks_status_missing_task_refused() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "status",
            "124",
            "999",
            "--to",
            "Completed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact task unavailable
"#
    );
}

#[test]
fn tickets_tasks_status_readback_unverified() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "status",
            "126",
            "906",
            "--to",
            "Completed",
            "--yes",
            "--expect-plan",
            "0717688270993dfe88177a7298a63827a6d42a0f8ba7ee1b7076ffdd3011f8a6",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "0717688270993dfe88177a7298a63827a6d42a0f8ba7ee1b7076ffdd3011f8a6",
  "verified": false,
  "task_id": 906,
  "fields": {
    "status": "mismatched"
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/126/tasks/906"]);
}

#[test]
fn tickets_tasks_delete_dry_run() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &["tickets", "tasks", "delete", "125", "904", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_task_delete",
  "method": "PUT",
  "path": "/api/_/tickets/125/tasks/update",
  "payload": {
    "action": "delete",
    "id": "904"
  },
  "plan_hash": "ceebc5ce54abf648834720352e87049f2752458d5f1facfd6a862b9c6e570522",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_tasks_delete_send() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "delete",
            "125",
            "904",
            "--yes",
            "--expect-plan",
            "ceebc5ce54abf648834720352e87049f2752458d5f1facfd6a862b9c6e570522",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ceebc5ce54abf648834720352e87049f2752458d5f1facfd6a862b9c6e570522",
  "verified": true,
  "task_id": 904,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/125/tasks/update"]);
}

#[test]
fn tickets_tasks_add_unoffered_choice_refused() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            "x",
            "--set",
            "cf_tier=platinum",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Tier: 'platinum' is not an active choice; offered: Gold, Silver
"#
    );
}

#[test]
fn tickets_tasks_add_owner_group_dry_run() {
    let r = run_fixture(
        "ticket_tasks_assigned",
        &[
            "tickets",
            "tasks",
            "add",
            "127",
            "--title",
            "Acme handover",
            "--owner",
            "bo@acme.com",
            "--group",
            "DBA",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_task_add",
  "method": "POST",
  "path": "/api/_/tickets/127/tasks",
  "payload": {
    "custom_fields": {},
    "group_id": 5,
    "notify_before": 0,
    "owner_id": 133,
    "status": 1,
    "title": "Acme handover"
  },
  "plan_hash": "9de0f2a7ba5e42bc1bee868c875aa4237e34297bcb55c005e306e47f5f41b268",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_tasks_add_owner_group_send() {
    let r = run_fixture(
        "ticket_tasks_assigned",
        &[
            "tickets",
            "tasks",
            "add",
            "127",
            "--title",
            "Acme handover",
            "--owner",
            "bo@acme.com",
            "--group",
            "DBA",
            "--yes",
            "--expect-plan",
            "9de0f2a7ba5e42bc1bee868c875aa4237e34297bcb55c005e306e47f5f41b268",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "9de0f2a7ba5e42bc1bee868c875aa4237e34297bcb55c005e306e47f5f41b268",
  "verified": true,
  "task_id": 911,
  "fields": {
    "custom_fields": "matched",
    "group_id": "matched",
    "notify_before": "matched",
    "owner_id": "matched",
    "status": "matched",
    "title": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/127/tasks"]);
}

#[test]
fn tickets_tasks_add_unknown_owner() {
    let r = run_fixture(
        "ticket_tasks_assigned",
        &[
            "tickets",
            "tasks",
            "add",
            "127",
            "--title",
            "x",
            "--owner",
            "zed@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"agent must match exactly one active entry with a user ID; 0 exact active candidates
"#
    );
}

#[test]
fn tickets_tasks_add_owner_field_unavailable() {
    let r = run_fixture(
        "ticket_tasks_reviewed",
        &[
            "tickets",
            "tasks",
            "add",
            "123",
            "--title",
            "x",
            "--owner",
            "7",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unique active task field unavailable: owner_id
"#
    );
}

#[test]
fn tickets_tasks_update_dry_run() {
    let r = run_fixture(
        "ticket_tasks_assigned",
        &[
            "tickets",
            "tasks",
            "update",
            "128",
            "912",
            "--title",
            "Acme new",
            "--owner",
            "133",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_task_update",
  "method": "PUT",
  "path": "/api/_/tickets/128/tasks/912",
  "payload": {
    "owner_id": 133,
    "title": "Acme new"
  },
  "plan_hash": "bfbdce41ceff462117f2f641b6c29328a4e7374417b777311a6bbd313c84e6aa",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_tasks_update_send() {
    let r = run_fixture(
        "ticket_tasks_assigned",
        &[
            "tickets",
            "tasks",
            "update",
            "128",
            "912",
            "--title",
            "Acme new",
            "--owner",
            "bo@acme.com",
            "--yes",
            "--expect-plan",
            "bfbdce41ceff462117f2f641b6c29328a4e7374417b777311a6bbd313c84e6aa",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "bfbdce41ceff462117f2f641b6c29328a4e7374417b777311a6bbd313c84e6aa",
  "verified": true,
  "task_id": 912,
  "fields": {
    "owner_id": "matched",
    "title": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/128/tasks/912"]);
}

#[test]
fn tickets_tasks_update_nothing() {
    let r = run_fixture(
        "ticket_tasks_assigned",
        &["tickets", "tasks", "update", "128", "912", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"provide at least one task update field
"#
    );
}

#[test]
fn tickets_tasks_update_unknown_task() {
    let r = run_fixture(
        "ticket_tasks_assigned",
        &[
            "tickets",
            "tasks",
            "update",
            "128",
            "999",
            "--title",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact task unavailable
"#
    );
}

#[test]
fn where_strict_operator_sequence() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "status==Open", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: invalid filter operator sequence; quote a literal value starting with an operator
"#
    );
}

#[test]
fn where_strict_no_field() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "=Open", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: invalid filter '=Open'; expected FIELD followed by = != >= <= > < or ~=
"#
    );
}

#[test]
fn where_strict_empty_value() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "status= ", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: invalid filter; field and value are required (quote literal empty text)
"#
    );
}

#[test]
fn where_strict_unterminated_quote() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "status='Open", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: invalid filter; unterminated quoted value
"#
    );
}

#[test]
fn where_strict_bang_without_equals() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "status!Open", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: invalid filter 'status!Open'; expected FIELD followed by = != >= <= > < or ~=
"#
    );
}

#[test]
fn where_strict_control_character() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "status=Op\ten", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: invalid filter; controls and multiline expressions are not supported
"#
    );
}

#[test]
fn where_strict_quoted_operator_value() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Note='=x'", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no list filter exists for text field 'Note': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword 
search; filterable fields on this tenant: Status, Priority, Requester, Agent, Group, Department, Environment, Count, Due Date
"#
    );
}

#[test]
fn where_strict_period_le_refused() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "created<=today", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: relative date periods support only = or >=; no inverse period wire is proven
"#
    );
}

#[test]
fn where_strict_period_label_le_refused() {
    let r = run_fixture(
        "where",
        &[
            "tickets",
            "ls",
            "--where",
            "Created<=Last 6 months",
            "--debug",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: relative date periods support only = or >=; no inverse period wire is proven
"#
    );
}

#[test]
fn where_contradiction_warning() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "status=Resolved", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 1,
    "display_id": 1,
    "subject": "Item 1",
    "status": 2,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  },
  {
    "id": 2,
    "display_id": 2,
    "subject": "Second [x]",
    "status": 4,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  }
]
"#
    );
    assert_eq!(
        r.err,
        r#"Warning: returned records contradict the requested predicate; do not use this list for writes. IDs: 1
"#
    );
}

#[test]
fn where_unfilterable_subject() {
    let r = run_fixture(
        "where",
        &["tickets", "ls", "--where", "Subject=x", "--debug"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: field not found: Subject; filter by a label shown on the form (Agent, Count, Department, Due Date, Environment, Group, Note, Priority, Requester, Status) or see `fsv tickets fields`
"#
    );
}

#[test]
fn ls_type_name_sends_choice_name() {
    let r = run_fixture(
        "where_type",
        &["tickets", "ls", "--where", "type=Service Request", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 1,
    "display_id": 1,
    "subject": "Item 1",
    "status": 2,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  },
  {
    "id": 2,
    "display_id": 2,
    "subject": "Second [x]",
    "status": 4,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_type_id_sends_choice_name() {
    let r = run_fixture(
        "where_type",
        &["tickets", "ls", "--where", "type=7402", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 1,
    "display_id": 1,
    "subject": "Item 1",
    "status": 2,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  },
  {
    "id": 2,
    "display_id": 2,
    "subject": "Second [x]",
    "status": 4,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_type_not_equal_sends_name() {
    let r = run_fixture(
        "where_type",
        &["tickets", "ls", "--where", "type!=Incident", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 1,
    "display_id": 1,
    "subject": "Item 1",
    "status": 2,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  },
  {
    "id": 2,
    "display_id": 2,
    "subject": "Second [x]",
    "status": 4,
    "priority": 1,
    "group_id": 7,
    "responder_id": 99,
    "requester": {
      "name": "Ann"
    },
    "created_at": "2024-03-01T10:00:00Z",
    "updated_at": "2024-03-02T10:00:00Z",
    "due_by": "2024-03-05T10:00:00Z"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ls_type_unknown_never_lists() {
    let r = run_fixture(
        "where_type",
        &["tickets", "ls", "--where", "type=Case", "--json"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no unique runtime choice 'Case' for Type; this tenant's choices: Incident, Service Request; nothing was sent
"#
    );
}

#[test]
fn ls_type_retired_never_lists() {
    let r = run_fixture(
        "where_type",
        &["tickets", "ls", "--where", "type=Retired", "--json"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: no unique runtime choice 'Retired' for Type; this tenant's choices: Incident, Service Request; nothing was sent
"#
    );
}

#[test]
fn where_tickets_search_error() {
    let r = run_fixture(
        "where_search_error",
        &["tickets", "ls", "--where", "status=Open"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: the tickets list rejected the filter (search_error) and returned unfiltered rows; nothing shown
"#
    );
}

#[test]
fn where_changes_search_error() {
    let r = run_fixture(
        "where_search_error",
        &["changes", "ls", "--where", "status=Open"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: the changes list rejected the filter (search_error) and returned unfiltered rows; nothing shown
"#
    );
}

#[test]
fn where_tickets_raw_search_error() {
    let r = run_fixture(
        "where_search_error",
        &[
            "tickets",
            "ls",
            "--query-hash",
            "[{\"condition\":\"approval_status\",\"operator\":\"is_in\",\"value\":[\"0\"],\"type\":\"default\"}]",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: approval_status is not a tickets list filter (the server ignores it and returns unfiltered rows); read approval_status per record instead (e.g. `get`)
"#
    );
}

#[test]
fn reply_attach_dry_run() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "notes.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_reply_attachments",
  "method": "POST",
  "path": "/api/_/tickets/5/reply",
  "payload": {
    "body": "<div>see attached</div>",
    "cc_emails": [],
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "from_email": "support@acme.com",
    "full_text": "<div>see attached</div>",
    "translate_reply": false
  },
  "plan_hash": "276c18541db1cb54eb3635b12913c336109f7eb8b6a3f496935158d8bd58d725",
  "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.",
  "files": [
    {
      "name": "notes.txt",
      "size": 11,
      "content_type": "text/plain",
      "sha256": "c1505025de226544276e5c0f1cd81e50013a5afd527caa6c3f208d63143f9176"
    }
  ],
  "max_attachment_size_bytes": 1048576,
  "attachment_size_limit": {
    "max_bytes": 1048576,
    "source": "tenant agent page MAX_ATTACHMENT_LIMIT (MiB, total per submit)",
    "published_bytes": 1048576,
    "total_bytes": 11,
    "applies_to": "total of all files in this submit"
  },
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn reply_attach_send() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "notes.txt",
            "--yes",
            "--expect-plan",
            "276c18541db1cb54eb3635b12913c336109f7eb8b6a3f496935158d8bd58d725",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "276c18541db1cb54eb3635b12913c336109f7eb8b6a3f496935158d8bd58d725",
  "uploaded_ids": [
    11
  ],
  "owner_id": 950,
  "verified": false,
  "attachments": [
    11
  ],
  "attachments_verified": true,
  "fields": {
    "body": "matched",
    "cc_emails": "unavailable",
    "from_email": "matched"
  },
  "warning": "Independent owning-record readback not fully verified; do not replay.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/attachments", "POST /api/_/tickets/5/reply"]
    );
}

#[test]
fn reply_attach_needs_sender() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--attach",
            "notes.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attached reply requires explicit configured --sender
"#
    );
}

#[test]
fn reply_attach_over_limit() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "notes.txt",
            "--attach",
            "log.txt",
            "--max-bytes",
            "20",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attachments exceed the 20-byte size limit; no upload attempted
"#
    );
}

#[test]
fn reply_attach_empty_file() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "empty.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attachment is empty: empty.txt
"#
    );
}

#[test]
fn reply_attach_duplicate_file() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "notes.txt",
            "--attach",
            "./notes.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attachments must be unique regular local files
"#
    );
}

#[test]
fn reply_attach_missing_file() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "nope.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attachments must be unique regular local files
"#
    );
}

#[test]
fn reply_attach_unpublished_limit_dry_run() {
    let r = run_files(
        "attachments_unpublished",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "notes.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_reply_attachments",
  "method": "POST",
  "path": "/api/_/tickets/5/reply",
  "payload": {
    "body": "<div>see attached</div>",
    "cc_emails": [],
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "from_email": "support@acme.com",
    "full_text": "<div>see attached</div>",
    "translate_reply": false
  },
  "plan_hash": "5424d23c8851bfced4319326e7f918a1967f44afc1f2ae52a4ce3833e4b3863f",
  "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.",
  "files": [
    {
      "name": "notes.txt",
      "size": 11,
      "content_type": "text/plain",
      "sha256": "c1505025de226544276e5c0f1cd81e50013a5afd527caa6c3f208d63143f9176"
    }
  ],
  "max_attachment_size_bytes": null,
  "attachment_size_limit": {
    "max_bytes": null,
    "source": "not published by the tenant",
    "published_bytes": "not published by the tenant",
    "total_bytes": 11,
    "applies_to": "total of all files in this submit",
    "note": "the tenant publishes no attachment size limit; pass --max-bytes N to set one before sending"
  },
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn reply_attach_unpublished_limit_send_refused() {
    let r = run_files(
        "attachments_unpublished",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "notes.txt",
            "--yes",
            "--expect-plan",
            "5424d23c8851bfced4319326e7f918a1967f44afc1f2ae52a4ce3833e4b3863f",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"the tenant publishes no attachment size limit; review again with --max-bytes N; no upload attempted
"#
    );
}

#[test]
fn reply_attach_max_bytes_override() {
    let r = run_files(
        "attachments_unpublished",
        "attach",
        &[
            "tickets",
            "reply",
            "5",
            "see attached",
            "--sender",
            "support@acme.com",
            "--attach",
            "notes.txt",
            "--max-bytes",
            "100",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_reply_attachments",
  "method": "POST",
  "path": "/api/_/tickets/5/reply",
  "payload": {
    "body": "<div>see attached</div>",
    "cc_emails": [],
    "conversation_from_summary": false,
    "freddy_message_id": null,
    "from_email": "support@acme.com",
    "full_text": "<div>see attached</div>",
    "translate_reply": false
  },
  "plan_hash": "ef1b0ec51836a544cdcd055044a2422e3abd30dae76cd45ccc8059ad96975da9",
  "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.",
  "files": [
    {
      "name": "notes.txt",
      "size": 11,
      "content_type": "text/plain",
      "sha256": "c1505025de226544276e5c0f1cd81e50013a5afd527caa6c3f208d63143f9176"
    }
  ],
  "max_attachment_size_bytes": 100,
  "attachment_size_limit": {
    "max_bytes": 100,
    "source": "--max-bytes override",
    "published_bytes": "not published by the tenant",
    "total_bytes": 11,
    "applies_to": "total of all files in this submit"
  },
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn add_note_attach_dry_run() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "add-note",
            "5",
            "--body",
            "noted",
            "--attach",
            "notes.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_note_attachments",
  "method": "POST",
  "path": "/api/_/tickets/5/notes",
  "payload": {
    "body": "<div>noted</div>",
    "conversation_from_summary": false,
    "notify_emails": [],
    "private": true
  },
  "plan_hash": "80c14a9737d29c834dd0d7304ba1de828f9799568b7d4d43430a591a44cde732",
  "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.",
  "files": [
    {
      "name": "notes.txt",
      "size": 11,
      "content_type": "text/plain",
      "sha256": "c1505025de226544276e5c0f1cd81e50013a5afd527caa6c3f208d63143f9176"
    }
  ],
  "max_attachment_size_bytes": 1048576,
  "attachment_size_limit": {
    "max_bytes": 1048576,
    "source": "tenant agent page MAX_ATTACHMENT_LIMIT (MiB, total per submit)",
    "published_bytes": 1048576,
    "total_bytes": 11,
    "applies_to": "total of all files in this submit"
  },
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn add_note_attach_send() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "add-note",
            "5",
            "--body",
            "noted",
            "--attach",
            "notes.txt",
            "--yes",
            "--expect-plan",
            "80c14a9737d29c834dd0d7304ba1de828f9799568b7d4d43430a591a44cde732",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "80c14a9737d29c834dd0d7304ba1de828f9799568b7d4d43430a591a44cde732",
  "uploaded_ids": [
    11
  ],
  "owner_id": 951,
  "verified": false,
  "attachments": [
    11
  ],
  "attachments_verified": true,
  "fields": {
    "body": "matched",
    "notify_emails": "unavailable",
    "private": "matched"
  },
  "warning": "Independent owning-record readback not fully verified; do not replay.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/attachments", "POST /api/_/tickets/5/notes"]
    );
}

#[test]
fn add_note_attach_set_status_refused() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "tickets",
            "add-note",
            "5",
            "--body",
            "noted",
            "--attach",
            "notes.txt",
            "--set-status",
            "Pending",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"combined attachment/status wire not proven
"#
    );
}

#[test]
fn problem_note_attach_dry_run() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "problems",
            "note",
            "PRB-7",
            "--body",
            "root cause",
            "--attach",
            "notes.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_note_attachments",
  "method": "POST",
  "path": "/api/_/problems/7/notes",
  "payload": {
    "body": "root cause"
  },
  "plan_hash": "7cbf29699f065ea78efc933f2ec8ec74b1dfbf65b69751102a038a8491eedb58",
  "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.",
  "files": [
    {
      "name": "notes.txt",
      "size": 11,
      "content_type": "text/plain",
      "sha256": "c1505025de226544276e5c0f1cd81e50013a5afd527caa6c3f208d63143f9176"
    }
  ],
  "max_attachment_size_bytes": 1048576,
  "attachment_size_limit": {
    "max_bytes": 1048576,
    "source": "tenant agent page MAX_ATTACHMENT_LIMIT (MiB, total per submit)",
    "published_bytes": 1048576,
    "total_bytes": 11,
    "applies_to": "total of all files in this submit"
  },
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_note_attach_send() {
    let r = run_files(
        "attachments",
        "attach",
        &[
            "problems",
            "note",
            "7",
            "--body",
            "root cause",
            "--attach",
            "notes.txt",
            "--yes",
            "--expect-plan",
            "7cbf29699f065ea78efc933f2ec8ec74b1dfbf65b69751102a038a8491eedb58",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7cbf29699f065ea78efc933f2ec8ec74b1dfbf65b69751102a038a8491eedb58",
  "uploaded_ids": [
    11
  ],
  "owner_id": 77,
  "verified": true,
  "attachments": [
    11
  ],
  "attachments_verified": true,
  "fields": {
    "body": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/attachments", "POST /api/_/problems/7/notes"]
    );
}

#[test]
fn attachments_delete_dry_run() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "61",
            "--owner",
            "tickets/5",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "attachment_delete",
  "method": "DELETE",
  "path": "/api/_/attachments/61",
  "payload": null,
  "plan_hash": "682dec3f9be6d9c74700fa23389f19a7e46cd70f8dde4b2568448248a72f4970",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "owner": "tickets/5",
  "child": null,
  "section": null,
  "requires": "--confirm-destructive",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn attachments_delete_send() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "62",
            "--owner",
            "tickets/6",
            "--yes",
            "--expect-plan",
            "1e521910273ace5238031f4973c4da33b0279dce0ea3f5247c8ff5ae437345f2",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attachment deletion is permanent; pass --confirm-destructive with --yes --expect-plan FULL_HASH. Nothing was sent
"#
    );
}

#[test]
fn attachments_delete_send_unverified() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "63",
            "--owner",
            "tickets/8",
            "--yes",
            "--expect-plan",
            "4a5d8b16075dd7506fbf09a73d4bcc9c5c52398adf51abae1874378aa261a8d0",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attachment deletion is permanent; pass --confirm-destructive with --yes --expect-plan FULL_HASH. Nothing was sent
"#
    );
}

#[test]
fn attachments_delete_needs_expect_plan() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "61",
            "--owner",
            "tickets/5",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"review with --dry-run, then --yes --expect-plan FULL_HASH --confirm-destructive
"#
    );
}

#[test]
fn attachments_delete_absent() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "99",
            "--owner",
            "tickets/5",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"attachment not present on exact owner; orphan deletion cannot be verified
"#
    );
}

#[test]
fn attachments_delete_bad_owner() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "61",
            "--owner",
            "assets/5",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"owner must be tickets/ID, problems/ID or changes/ID; choose at most one child or section
"#
    );
}

#[test]
fn attachments_delete_change_child_refused() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "61",
            "--owner",
            "changes/5",
            "--child",
            "3",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"change note attachment deletion not proven
"#
    );
}

#[test]
fn attachments_delete_child_conversation() {
    let r = run_fixture(
        "attachments",
        &[
            "attachments",
            "delete",
            "11",
            "--owner",
            "tickets/5",
            "--child",
            "950",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "attachment_delete",
  "method": "DELETE",
  "path": "/api/_/attachments/11",
  "payload": null,
  "plan_hash": "e8922f7f921b3e31921471d94cdd614b84d68b803700ff0926370e8c2004f611",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "owner": "tickets/5",
  "child": 950,
  "section": null,
  "requires": "--confirm-destructive",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_create_before_create_dry_run() {
    let r = run_files(
        "change_create_attached",
        "attach",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--attach",
            "notes.txt",
            "--attach-flow",
            "before-create",
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
  "action": "change_create_attachments",
  "method": "POST",
  "path": "/api/_/changes",
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltcIm5lZWRzX3Jldmlld185MDAwMDA3NzdcIl0iLCJkaXNhYmxlZCI6IltdIiwiaGlkZGVuIjoiW1wic3RhZ2luZ19zdGFydF85MDAwMDA3NzdcIiwgXCJzdGFnaW5nX2VuZF85MDAwMDA3NzdcIiwgXCJyZXZpZXdlcnNfOTAwMDAwNzc3XCIsIFwicm9sbG91dF90b29sXzkwMDAwMDc3N1wiLCBcImxlZ2FjeV9mbGFnXzkwMDAwMDc3N1wiLCBcImVtZXJnZW5jeV9yZWFzb25fOTAwMDAwNzc3XCIsIFwiY2F0ZWdvcnlcIiwgXCJyZXZpZXdfc2NvcGVfOTAwMDAwNzc3XCJdIn0=",
    "change_type": 1,
    "custom_fields": {},
    "email": "Ann Lee <ann@acme.com>",
    "impact": 1,
    "priority": 1,
    "risk": 1,
    "status": 1,
    "subject": "Hello"
  },
  "plan_hash": "7b16c25d0b95b5bcfe7983a624b50557c7387d9a0a151a44db971558d0a4515c",
  "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.",
  "files": [
    {
      "name": "notes.txt",
      "size": 11,
      "content_type": "text/plain",
      "sha256": "c1505025de226544276e5c0f1cd81e50013a5afd527caa6c3f208d63143f9176"
    }
  ],
  "resolved_requester_id": 51,
  "max_attachment_size_bytes": 1048576,
  "attachment_size_limit": {
    "max_bytes": 1048576,
    "source": "tenant agent page MAX_ATTACHMENT_LIMIT (MiB, total per submit)",
    "published_bytes": 1048576,
    "total_bytes": 11,
    "applies_to": "total of all files in this submit"
  },
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_create_before_create_send() {
    let r = run_files(
        "change_create_attached",
        "attach",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--attach",
            "notes.txt",
            "--attach-flow",
            "before-create",
            "--yes",
            "--expect-plan",
            "7b16c25d0b95b5bcfe7983a624b50557c7387d9a0a151a44db971558d0a4515c",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7b16c25d0b95b5bcfe7983a624b50557c7387d9a0a151a44db971558d0a4515c",
  "uploaded_ids": [
    11
  ],
  "owner_id": 900,
  "verified": false,
  "attachments": [
    11
  ],
  "attachments_verified": true,
  "fields": {
    "change_type": "unavailable",
    "custom_fields": "unavailable",
    "email": "matched",
    "impact": "unavailable",
    "priority": "unavailable",
    "risk": "unavailable",
    "status": "unavailable",
    "subject": "matched"
  },
  "warning": "Independent owning-record readback not fully verified; do not replay.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/attachments", "POST /api/_/changes"]
    );
}

#[test]
fn change_create_before_create_over_limit() {
    let r = run_files(
        "change_create_attached",
        "attach",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--attach",
            "notes.txt",
            "--attach-flow",
            "before-create",
            "--max-bytes",
            "5",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: attachments exceed the 5-byte size limit; no upload attempted
"#
    );
}

#[test]
fn change_create_before_create_with_device_refused() {
    let r = run_files(
        "change_create_attached",
        "attach",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--attach",
            "notes.txt",
            "--attach-flow",
            "before-create",
            "--device",
            "702",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: initial attachments cannot combine with planning, devices or interactive forms
"#
    );
}

#[test]
fn change_create_device_send() {
    let r = run_fixture(
        "change_create_attached",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--device",
            "702",
            "--yes",
            "--expect-plan",
            "8185a9434dbab596cf034b7ebba4376dc554b9fa79e5d07f9c2d186dcc32312f",
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
  "plan_hash": "8185a9434dbab596cf034b7ebba4376dc554b9fa79e5d07f9c2d186dcc32312f",
  "steps": [
    {
      "step": "create",
      "ok": true,
      "id": 900
    }
  ],
  "uploaded": [],
  "device_verification": {
    "verified": true,
    "readback": [
      {
        "item_id": 702,
        "itam_cmdb_request_id": 9002,
        "name": "Acme router",
        "category": "Device"
      }
    ]
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes"]);
}

#[test]
fn change_create_device_send_unverified() {
    let r = run_fixture(
        "change_create_attached",
        &[
            "changes",
            "create",
            "--set",
            "Subject=Hello",
            "--requester",
            "ann@acme.com",
            "--no-validate",
            "--device",
            "703",
            "--yes",
            "--expect-plan",
            "cef5340cfc6accc199ca7f98b5953e20fa8c8c016584ae6a4eede489f7b83b8b",
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
  "plan_hash": "cef5340cfc6accc199ca7f98b5953e20fa8c8c016584ae6a4eede489f7b83b8b",
  "steps": [
    {
      "step": "create",
      "ok": true,
      "id": 900
    }
  ],
  "uploaded": [],
  "device_verification": {
    "verified": false,
    "readback": [
      {
        "item_id": 702,
        "itam_cmdb_request_id": 9002,
        "name": "Acme router",
        "category": "Device"
      }
    ],
    "warning": "Create-time devices not independently verified; do not resend."
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes"]);
}

#[test]
fn change_create_max_bytes_needs_before_create() {
    let r = run_files(
        "change_create_attached",
        "attach",
        &["changes", "create", "--max-bytes", "100", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: --max-bytes requires the reviewed --attach-flow before-create transaction
"#
    );
}

#[test]
fn ticket_create_attach_dry_run() {
    let r = run_files(
        "ticket_create_attached",
        "attach",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
            "--attach",
            "notes.txt",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_create_attachments",
  "method": "POST",
  "path": "/api/_/tickets",
  "payload": {
    "br_validation_excludes": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJvcHRpb25hbCI6IltdIiwiZGlzYWJsZWQiOiJbXSIsImhpZGRlbiI6IltdIn0=",
    "custom_fields": {},
    "email": "Ann Lee <ann@acme.com>",
    "priority": 2,
    "subject": "Hello"
  },
  "plan_hash": "7d4aaf4fe5f1cfb2f4ac4359b625d2415987af685d330e5c5821f821c7f65a1e",
  "warning": "Uploads and owning write are not atomic. A failed/unknown step may leave orphan uploads. Inspect journal and owner; never replay the transaction.",
  "files": [
    {
      "name": "notes.txt",
      "size": 11,
      "content_type": "text/plain",
      "sha256": "c1505025de226544276e5c0f1cd81e50013a5afd527caa6c3f208d63143f9176"
    }
  ],
  "resolved_requester_id": 51,
  "max_attachment_size_bytes": 1048576,
  "attachment_size_limit": {
    "max_bytes": 1048576,
    "source": "tenant agent page MAX_ATTACHMENT_LIMIT (MiB, total per submit)",
    "published_bytes": 1048576,
    "total_bytes": 11,
    "applies_to": "total of all files in this submit"
  },
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_create_attach_send() {
    let r = run_files(
        "ticket_create_attached",
        "attach",
        &[
            "tickets",
            "create",
            "--set",
            "Subject=Hello",
            "--set",
            "Priority=Medium",
            "--requester",
            "ann@acme.com",
            "--attach",
            "notes.txt",
            "--yes",
            "--expect-plan",
            "7d4aaf4fe5f1cfb2f4ac4359b625d2415987af685d330e5c5821f821c7f65a1e",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7d4aaf4fe5f1cfb2f4ac4359b625d2415987af685d330e5c5821f821c7f65a1e",
  "uploaded_ids": [
    11
  ],
  "owner_id": 777,
  "verified": false,
  "attachments": [
    11
  ],
  "attachments_verified": true,
  "fields": {
    "custom_fields": "unavailable",
    "email": "matched",
    "priority": "matched",
    "subject": "matched"
  },
  "warning": "Independent owning-record readback not fully verified; do not replay.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["POST /api/_/attachments", "POST /api/_/tickets"]
    );
}

#[test]
fn assets_search_devices() {
    let r = run_fixture("integrated_assets", &["assets", "search", "acme"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "name": {
        "id": 702,
        "label": "Acme router"
      },
      "type": "Router",
      "device_pk": 702
    }
  ],
  "total_rows": 1,
  "page": 1,
  "kind": "devices",
  "per_page": 50,
  "pages": 1,
  "complete": true
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn assets_search_services_page() {
    let r = run_fixture(
        "integrated_assets",
        &["assets", "search", "--kind", "services", "--page", "2"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "name": {
        "id": 801,
        "label": "Acme payroll"
      },
      "businessservice_pk": 801
    }
  ],
  "total_rows": 4,
  "page": 2,
  "kind": "services",
  "per_page": 50,
  "pages": 1,
  "complete": false
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn assets_search_bad_kind() {
    let r = run_fixture(
        "integrated_assets",
        &["assets", "search", "acme", "--kind", "printers"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"choose devices, assets or services and a positive page
"#
    );
}

#[test]
fn changes_assets_search_devices() {
    let r = run_fixture(
        "integrated_assets",
        &["changes", "assets", "search", "acme"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "name": {
        "id": 702,
        "label": "Acme router"
      },
      "type": "Router",
      "device_pk": 702
    }
  ],
  "total_rows": 1,
  "page": 1,
  "kind": "devices",
  "per_page": 50,
  "pages": 1,
  "complete": true
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assets_list() {
    let r = run_fixture(
        "integrated_assets",
        &["tickets", "assets", "list", "INC-123"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "item_id": 701,
      "itam_cmdb_request_id": 9001,
      "name": "Acme laptop",
      "category": "Device"
    }
  ],
  "complete": true,
  "driver": "integrated"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assets_ls_services() {
    let r = run_fixture(
        "integrated_assets",
        &["tickets", "assets", "ls", "123", "--kind", "services"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "item_id": 801,
      "itam_cmdb_request_id": 9101,
      "name": "Acme payroll",
      "category": "Business service"
    }
  ],
  "complete": true,
  "driver": "integrated"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assets_list_unproven_category() {
    let r = run_fixture(
        "integrated_assets",
        &["tickets", "assets", "list", "125"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unproven association category
"#
    );
}

#[test]
fn sr_assets_list() {
    let r = run_fixture(
        "integrated_assets",
        &["sr", "assets", "list", "SR-124"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "item_id": 704,
      "itam_cmdb_request_id": 9004,
      "name": "Acme printer",
      "category": "Device"
    }
  ],
  "complete": true,
  "driver": "integrated"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_assets_list() {
    let r = run_fixture(
        "integrated_assets",
        &["changes", "assets", "list", "CHN-5"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "item_id": 703,
      "itam_cmdb_request_id": 9003,
      "name": "Acme db",
      "category": "Device"
    }
  ],
  "complete": true,
  "driver": "integrated"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_assets_legacy() {
    let r = run_fixture(
        "integrated_assets",
        &["changes", "assets", "legacy", "5", "--search", "acme"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "name": {
        "id": 702,
        "label": "Acme router"
      },
      "type": "Router",
      "device_pk": 702
    }
  ],
  "total_rows": 1,
  "page": 1,
  "kind": "devices",
  "per_page": 50,
  "pages": 1,
  "complete": true
}
"#
    );
    assert_eq!(
        r.err,
        r#"deprecated: use assets search or assets list
"#
    );
}

#[test]
fn changes_assets_bare_id_legacy_route() {
    let r = run_fixture("integrated_assets", &["changes", "assets", "5"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "item_id": 703,
      "itam_cmdb_request_id": 9003,
      "name": "Acme db",
      "category": "Device"
    }
  ],
  "complete": true,
  "driver": "integrated"
}
"#
    );
    assert_eq!(
        r.err,
        r#"deprecated: use assets search or assets list
"#
    );
}

#[test]
fn tickets_assets_add_dry_run() {
    let r = run_fixture(
        "integrated_assets",
        &["tickets", "assets", "add", "123", "702", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "integrated_asset_add",
  "method": "PUT",
  "path": "/api/_/tickets/123/associate-ci",
  "payload": {
    "device_ids": [
      702
    ]
  },
  "plan_hash": "c2e389d771f659f69e132fd6fdea5ab1bb8a8dfb75b91c137735724728c00630",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assets_add_needs_expect_plan() {
    let r = run_fixture(
        "integrated_assets",
        &["tickets", "assets", "add", "123", "702", "--yes"],
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
fn tickets_assets_add_read_only() {
    let r = run_fixture(
        "integrated_assets",
        &["tickets", "assets", "add", "123", "702", "--read-only"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read-only mode; no write attempted
"#
    );
}

#[test]
fn tickets_assets_add_send() {
    let r = run_fixture(
        "integrated_assets",
        &[
            "tickets",
            "assets",
            "add",
            "123",
            "702",
            "--yes",
            "--expect-plan",
            "c2e389d771f659f69e132fd6fdea5ab1bb8a8dfb75b91c137735724728c00630",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c2e389d771f659f69e132fd6fdea5ab1bb8a8dfb75b91c137735724728c00630",
  "verified": true,
  "readback": [
    {
      "item_id": 701,
      "itam_cmdb_request_id": 9001,
      "name": "Acme laptop",
      "category": "Device"
    },
    {
      "item_id": 702,
      "itam_cmdb_request_id": 9002,
      "name": "Acme router",
      "category": "Device"
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/123/associate-ci"]);
}

#[test]
fn tickets_assets_add_present_refused() {
    let r = run_fixture(
        "integrated_assets",
        &["tickets", "assets", "add", "123", "701", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"requested inventory ID is already associated; omit it
"#
    );
}

#[test]
fn problems_assets_remove_dry_run() {
    let r = run_fixture(
        "integrated_assets",
        &["problems", "assets", "remove", "PRB-7", "701", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "integrated_asset_remove",
  "method": "PUT",
  "path": "/api/_/problems/7/disassociate-ci",
  "payload": {
    "itam_cmdb_request_id": 9001
  },
  "plan_hash": "4231df6ed7a03e8e3f66f1c0c23385e066e0486a5e55c9b20037d8b08b78229d",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_assets_remove_send() {
    let r = run_fixture(
        "integrated_assets",
        &[
            "problems",
            "assets",
            "remove",
            "7",
            "701",
            "--yes",
            "--expect-plan",
            "4231df6ed7a03e8e3f66f1c0c23385e066e0486a5e55c9b20037d8b08b78229d",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "4231df6ed7a03e8e3f66f1c0c23385e066e0486a5e55c9b20037d8b08b78229d",
  "verified": true,
  "readback": [],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/7/disassociate-ci"]);
}

#[test]
fn problems_assets_remove_absent_refused() {
    let r = run_fixture(
        "integrated_assets",
        &["problems", "assets", "remove", "7", "799", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"inventory ID is not associated with this exact record
"#
    );
}

#[test]
fn changes_assets_add_send_unverified() {
    let r = run_fixture(
        "integrated_assets",
        &[
            "changes",
            "assets",
            "add",
            "5",
            "702",
            "--yes",
            "--expect-plan",
            "5a9859554e9f1c304b4412368d54aaaa03eb63da3bb2ab11a3cf9c7fb5cb7b26",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "5a9859554e9f1c304b4412368d54aaaa03eb63da3bb2ab11a3cf9c7fb5cb7b26",
  "verified": false,
  "readback": [
    {
      "item_id": 703,
      "itam_cmdb_request_id": 9003,
      "name": "Acme db",
      "category": "Device"
    }
  ],
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/5/associate-ci"]);
}

#[test]
fn changes_assets_add_wrong_hash() {
    let r = run_fixture(
        "integrated_assets",
        &[
            "changes",
            "assets",
            "add",
            "5",
            "702",
            "--yes",
            "--expect-plan",
            "0000",
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
fn tickets_assets_list_legacy_driver() {
    let r = run_fixture(
        "legacy_assets",
        &["tickets", "assets", "list", "INC-151"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "config_item": {
        "display_id": 3001,
        "name": "Acme laptop 1"
      }
    }
  ],
  "complete": true,
  "driver": "legacy"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assets_list_legacy_services_refused() {
    let r = run_fixture(
        "legacy_assets",
        &["tickets", "assets", "list", "151", "--kind", "services"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"the legacy asset association list has no business services; use the web UI
"#
    );
}

#[test]
fn tickets_assets_list_no_driver() {
    let r = run_fixture("legacy_assets", &["tickets", "assets", "list", "152"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no asset association endpoint found for tickets 152 (associated-cis and the legacy asset list both answered 404); nothing read or sent
"#
    );
}

#[test]
fn changes_assets_list_legacy_driver() {
    let r = run_fixture(
        "legacy_assets",
        &["changes", "assets", "list", "CHN-51"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "config_item": {
        "display_id": 4001,
        "name": "Acme router A"
      }
    },
    {
      "config_item": {
        "display_id": 4002,
        "name": "Acme router B"
      }
    }
  ],
  "complete": true,
  "driver": "legacy"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_assets_list_no_driver() {
    let r = run_fixture("legacy_assets", &["problems", "assets", "list", "61"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no asset association endpoint found for problems 61 (associated-cis and the legacy asset list both answered 404); nothing read or sent
"#
    );
}

#[test]
fn tickets_assets_candidates() {
    let r = run_fixture(
        "legacy_assets",
        &["tickets", "assets", "candidates", "151"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "display_id": 3001,
      "name": "Acme laptop 1"
    },
    {
      "display_id": 3002,
      "name": "Acme laptop 2"
    }
  ],
  "complete": true,
  "driver": "legacy",
  "page": "all",
  "per_page": 30,
  "pages": 1,
  "total_rows": 2
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_assets_candidates() {
    let r = run_fixture(
        "legacy_assets",
        &["changes", "assets", "candidates", "51", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "display_id": 4003,
      "name": "Acme router C"
    }
  ],
  "complete": true,
  "driver": "legacy",
  "page": "all",
  "per_page": 30,
  "pages": 1,
  "total_rows": 1
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_assets_candidates() {
    let r = run_fixture(
        "legacy_assets",
        &["sr", "assets", "candidates", "151"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "data": [
    {
      "display_id": 3001,
      "name": "Acme laptop 1"
    },
    {
      "display_id": 3002,
      "name": "Acme laptop 2"
    }
  ],
  "complete": true,
  "driver": "legacy",
  "page": "all",
  "per_page": 30,
  "pages": 1,
  "total_rows": 2
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_assets_candidates_refused() {
    let r = run_fixture(
        "legacy_assets",
        &["problems", "assets", "candidates", "61"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"HTTP 404 listing device: no route
"#
    );
}

#[test]
fn tickets_assets_add_legacy_dry_run() {
    let r = run_fixture(
        "legacy_assets",
        &["tickets", "assets", "add", "151", "3002", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "legacy_asset_add",
  "method": "PUT",
  "path": "/api/_/tickets/151/associate_assets",
  "payload": {
    "item_ids": [
      3002
    ]
  },
  "plan_hash": "80eb62a873ae3652c72f21bb7c3d8b6aee0e771b8f7a673d7dce983ce22b6f95",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_assets_add_legacy_present_refused() {
    let r = run_fixture(
        "legacy_assets",
        &["tickets", "assets", "add", "151", "3001", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"requested asset is already associated; omit it
"#
    );
}

#[test]
fn tickets_assets_add_legacy_not_offered() {
    let r = run_fixture(
        "legacy_assets",
        &["tickets", "assets", "add", "151", "3999", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"asset display ID is not among the assets the web offers for this record (assets-to-associate); nothing sent
"#
    );
}

#[test]
fn tickets_assets_remove_legacy_refused() {
    let r = run_fixture(
        "legacy_assets",
        &["tickets", "assets", "remove", "151", "3001", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"removing a legacy asset association has no captured wire; use the web UI; nothing sent
"#
    );
}

#[test]
fn sr_approvals_list() {
    let r = run_fixture("sr_approvals", &["sr", "approvals", "list", "SR-130"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "approval_groups": [
    {
      "id": 40,
      "name": "Acme CAB",
      "approval_type": 1,
      "approval_status": 0,
      "approvals": [
        {
          "id": 401,
          "member_id": 31,
          "status": {
            "id": 0,
            "name": "requested"
          },
          "last_reminded_at": null,
          "remark": []
        },
        {
          "id": 402,
          "member_id": 32,
          "status": {
            "id": 0,
            "name": "requested"
          },
          "last_reminded_at": null,
          "remark": []
        },
        {
          "id": 403,
          "member_id": 35,
          "status": {
            "id": 1,
            "name": "approved"
          },
          "last_reminded_at": null,
          "remark": []
        }
      ]
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_approvals_request_dry_run() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "131",
            "--name",
            "Acme release",
            "--message",
            "Please review",
            "--approver",
            "33",
            "--approver",
            "bo@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "sr_approval_request",
  "method": "POST",
  "path": "/api/_/tickets/131/approval-groups",
  "payload": {
    "approval_groups": [
      {
        "approval_type": 1,
        "email_content": "<p>Please review</p>",
        "name": "Acme release",
        "notified_to": [
          33,
          34
        ]
      }
    ]
  },
  "plan_hash": "7a55add406c33962faaf8ca2ac6cdb211f5d2b5b93b6c290cd9bee619db71f02",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_approvals_request_send() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "131",
            "--name",
            "Acme release",
            "--message",
            "Please review",
            "--approver",
            "33",
            "--approver",
            "bo@acme.com",
            "--yes",
            "--expect-plan",
            "7a55add406c33962faaf8ca2ac6cdb211f5d2b5b93b6c290cd9bee619db71f02",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7a55add406c33962faaf8ca2ac6cdb211f5d2b5b93b6c290cd9bee619db71f02",
  "verified": true,
  "readback": [
    {
      "id": 41,
      "name": "Acme release",
      "approval_type": 1,
      "approval_status": 0,
      "approvals": [
        {
          "id": 411,
          "member_id": 33,
          "status": {
            "id": 0,
            "name": "requested"
          },
          "last_reminded_at": null,
          "remark": []
        },
        {
          "id": 412,
          "member_id": 34,
          "status": {
            "id": 0,
            "name": "requested"
          },
          "last_reminded_at": null,
          "remark": []
        }
      ]
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/131/approval-groups"]);
}

#[test]
fn sr_approvals_request_deactivated_refused() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "131",
            "--name",
            "Acme release",
            "--message",
            "Please review",
            "--approver",
            "old@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"approver must resolve to one active requester user ID
"#
    );
}

#[test]
fn sr_approvals_request_bad_rule() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "131",
            "--name",
            "Acme release",
            "--message",
            "Please review",
            "--approver",
            "33",
            "--rule",
            "most",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"nonempty message and captured approval rule required
"#
    );
}

#[test]
fn sr_approvals_request_needs_expect_plan() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "131",
            "--name",
            "Acme release",
            "--message",
            "Please review",
            "--approver",
            "33",
            "--yes",
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
fn sr_approvals_approve_dry_run() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "approve",
            "132",
            "401",
            "--remark",
            "Acme ok",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "sr_approval_approve",
  "method": "PUT",
  "path": "/api/_/tickets/132/approvals/401",
  "payload": {
    "approval_status": 1,
    "remark": "Acme ok"
  },
  "plan_hash": "e7d8f22922a9eb8a5991ab8a948cc301add6b64e2d3fdc6d9aac66f25686e5e8",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_approvals_approve_send() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "approve",
            "132",
            "401",
            "--remark",
            "Acme ok",
            "--yes",
            "--expect-plan",
            "e7d8f22922a9eb8a5991ab8a948cc301add6b64e2d3fdc6d9aac66f25686e5e8",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e7d8f22922a9eb8a5991ab8a948cc301add6b64e2d3fdc6d9aac66f25686e5e8",
  "verified": true,
  "readback": {
    "id": 401,
    "member_id": 31,
    "status": {
      "id": 1,
      "name": "approved"
    },
    "last_reminded_at": null,
    "remark": [
      {
        "status": 1,
        "data": "Acme ok"
      }
    ]
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/132/approvals/401"]);
}

#[test]
fn sr_approvals_approve_not_requested_refused() {
    let r = run_fixture(
        "sr_approvals",
        &["sr", "approvals", "approve", "130", "403", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"target is not a requested approval/group; immutable history cannot be changed
"#
    );
}

#[test]
fn sr_approvals_reject_needs_remark() {
    let r = run_fixture(
        "sr_approvals",
        &["sr", "approvals", "reject", "130", "402", "--dry-run"],
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
fn sr_approvals_cancel_dry_run() {
    let r = run_fixture(
        "sr_approvals",
        &["sr", "approvals", "cancel", "130", "40", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "sr_approval_cancel",
  "method": "PUT",
  "path": "/api/_/tickets/130/approval-groups/40/cancel",
  "payload": null,
  "plan_hash": "a469e0792108f3b84b4593e825cb69a302921c03e4ad1bc56092454996bbaa78",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_approvals_remind_send_unverified() {
    let r = run_fixture(
        "sr_approvals",
        &[
            "sr",
            "approvals",
            "remind",
            "133",
            "402",
            "--yes",
            "--expect-plan",
            "fb8d49b2a73af87dfc2adfca39f73bde984b9cba7774edc8a762ab4be5b31498",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "fb8d49b2a73af87dfc2adfca39f73bde984b9cba7774edc8a762ab4be5b31498",
  "verified": false,
  "readback": {
    "id": 402,
    "member_id": 32,
    "status": {
      "id": 0,
      "name": "requested"
    },
    "last_reminded_at": null,
    "remark": []
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/tickets/133/approvals/402/resend"]
    );
}

#[test]
fn sr_approvals_approve_read_only() {
    let r = run_fixture(
        "sr_approvals",
        &["sr", "approvals", "approve", "130", "401", "--read-only"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read-only mode; no write attempted
"#
    );
}

#[test]
fn sr_approvals_unknown_record() {
    let r = run_fixture(
        "sr_approvals",
        &["sr", "approvals", "approve", "130", "999", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact approval/group ID unavailable
"#
    );
}

#[test]
fn changes_legacy_approval_request_dry_run() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--approver",
            "9",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "legacy_approval_request_changes",
  "method": "POST",
  "path": "/api/_/changes/41/approvals",
  "payload": {
    "approval_type": 1,
    "notified_to": [
      9
    ]
  },
  "plan_hash": "b39135ab5774b043e76af7058fb148cdd29db591f08092600a2d202e09d09ff9",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_legacy_approval_request_send() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--approver",
            "9",
            "--yes",
            "--expect-plan",
            "b39135ab5774b043e76af7058fb148cdd29db591f08092600a2d202e09d09ff9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "b39135ab5774b043e76af7058fb148cdd29db591f08092600a2d202e09d09ff9",
  "verified": true,
  "approval_ids": [
    503
  ],
  "readback": [
    {
      "id": 503,
      "member_id": 9,
      "level_id": 1,
      "status": {
        "id": 0,
        "name": "requested"
      },
      "type": {
        "id": 1,
        "name": "everyone"
      },
      "last_reminded_at": null
    }
  ],
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "legacy_approval_request_changes",
  "method": "POST",
  "path": "/api/_/changes/41/approvals",
  "payload": {
    "approval_type": 1,
    "notified_to": [
      9
    ]
  },
  "plan_hash": "b39135ab5774b043e76af7058fb148cdd29db591f08092600a2d202e09d09ff9",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/changes/41/approvals"]);
}

#[test]
fn changes_legacy_approval_request_no_input() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--approver",
            "9",
            "--no-input",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"{
  "action": "legacy_approval_request_changes",
  "method": "POST",
  "path": "/api/_/changes/41/approvals",
  "payload": {
    "approval_type": 1,
    "notified_to": [
      9
    ]
  },
  "plan_hash": "b39135ab5774b043e76af7058fb148cdd29db591f08092600a2d202e09d09ff9",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
sending requires --yes and --expect-plan in no-input mode
"#
    );
}

#[test]
fn changes_legacy_approval_request_name_refused() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--model",
            "legacy",
            "--name",
            "CAB",
            "--message",
            "hi",
            "--approver",
            "9",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--name/--message do not apply to legacy flat approvals (no group name or message on the wire)
"#
    );
}

#[test]
fn changes_legacy_approval_request_auto_ignores_name() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--name",
            "CAB",
            "--message",
            "hi",
            "--approver",
            "9",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "legacy_approval_request_changes",
  "method": "POST",
  "path": "/api/_/changes/41/approvals",
  "payload": {
    "approval_type": 1,
    "notified_to": [
      9
    ]
  },
  "plan_hash": "b39135ab5774b043e76af7058fb148cdd29db591f08092600a2d202e09d09ff9",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_legacy_approval_request_group_refused() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--approver-group",
            "Ops",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--approver-group has no legacy flat-approval equivalent; use --cab or --approver
"#
    );
}

#[test]
fn changes_legacy_approval_request_bad_rule() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--approver",
            "9",
            "--rule",
            "majority",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"legacy approvals support only the observed rules: everyone, first-responder
"#
    );
}

#[test]
fn changes_approval_request_bad_model() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--approver",
            "9",
            "--model",
            "flat",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--model must be auto, chains or legacy
"#
    );
}

#[test]
fn changes_approval_request_chains_needs_name() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "41",
            "--approver",
            "9",
            "--model",
            "chains",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--name and --message are required on the approval chains model
"#
    );
}

#[test]
fn changes_approval_request_auto_chains_needs_name() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-request",
            "42",
            "--approver",
            "9",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--name and --message are required on the approval chains model
"#
    );
}

#[test]
fn changes_approval_remind_dry_run() {
    let r = run_fixture(
        "legacy_approvals",
        &["changes", "approval-remind", "41", "501", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "legacy_approval_remind_changes",
  "method": "PUT",
  "path": "/api/_/changes/41/approvals/501/resend",
  "payload": null,
  "plan_hash": "d0294fa8acf4d0fbb3cd34020f8031981cd991a67c7d3caef490dc48ba68d3d0",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "approval_id": 501,
  "operation": "remind",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_approval_remind_send() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "changes",
            "approval-remind",
            "41",
            "501",
            "--yes",
            "--expect-plan",
            "d0294fa8acf4d0fbb3cd34020f8031981cd991a67c7d3caef490dc48ba68d3d0",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d0294fa8acf4d0fbb3cd34020f8031981cd991a67c7d3caef490dc48ba68d3d0",
  "verified": true,
  "readback": {
    "id": 501,
    "member_id": 7,
    "level_id": 1,
    "status": {
      "id": 0,
      "name": "requested"
    },
    "type": {
      "id": 1,
      "name": "everyone"
    },
    "last_reminded_at": "2026-11-01T10:00:00Z"
  },
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/changes/41/approvals/501/resend"]
    );
}

#[test]
fn changes_approval_remind_needs_expect_plan() {
    let r = run_fixture(
        "legacy_approvals",
        &["changes", "approval-remind", "41", "501", "--yes"],
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
fn changes_approval_remind_not_requested() {
    let r = run_fixture(
        "legacy_approvals",
        &["changes", "approval-remind", "41", "502", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only a requested approval (raw status 0) can be reminded
"#
    );
}

#[test]
fn changes_approval_remind_chains_refused() {
    let r = run_fixture(
        "legacy_approvals",
        &["changes", "approval-remind", "42", "501", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"approval-remind is not available on the approval-chains model: the only reminder accepted on a chains tenant is the service-request one (`fsv sr approvals remind`, bodyless PUT tickets/<id>/approvals/<approval id>/resend). The same web constant would build changes/<id>/approvals/<approval id>/resend, but no change reminder was ever accepted (the legacy change capture was aborted unsent), so fsv does not guess it; nothing was sent. Use the web UI: change > Approvals > the approval row menu > Send reminder
"#
    );
}

#[test]
fn sr_legacy_approvals_list() {
    let r = run_fixture(
        "legacy_approvals",
        &["sr", "approvals", "list", "SR-141"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"┏━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━┳━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━┓
┃ Level ┃ Approver ┃ Status    ┃ Decided ┃ Type     ┃ Comment ┃
┡━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━╇━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━┩
│ 1     │ 33       │ requested │ -       │ everyone │ -       │
└───────┴──────────┴───────────┴─────────┴──────────┴─────────┘
"#
    );
    assert_eq!(
        r.err,
        r#"this tenant uses legacy flat approvals (approval-chains answered HTTP 400)
"#
    );
}

#[test]
fn sr_legacy_approvals_request_dry_run() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "141",
            "--approver",
            "cy@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "legacy_approval_request_tickets",
  "method": "POST",
  "path": "/api/_/tickets/141/approvals",
  "payload": {
    "approval_type": 1,
    "email_content": "<p>Please approve {{ticket.subject}}</p>",
    "notified_to": [
      34
    ]
  },
  "plan_hash": "bf1e348e7534dedb78308a0748db1f280d67bc6428edcfaf17dcc35f0c36e341",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    34
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_legacy_approvals_request_send() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "141",
            "--approver",
            "cy@acme.com",
            "--yes",
            "--expect-plan",
            "bf1e348e7534dedb78308a0748db1f280d67bc6428edcfaf17dcc35f0c36e341",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "bf1e348e7534dedb78308a0748db1f280d67bc6428edcfaf17dcc35f0c36e341",
  "verified": true,
  "approval_ids": [
    602
  ],
  "readback": [
    {
      "id": 602,
      "member_id": 34,
      "level_id": 1,
      "status": {
        "id": 0,
        "name": "requested"
      },
      "type": {
        "id": 1,
        "name": "everyone"
      },
      "last_reminded_at": null
    }
  ],
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/141/approvals"]);
}

#[test]
fn sr_legacy_approvals_request_name_refused() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "sr",
            "approvals",
            "request",
            "141",
            "--name",
            "x",
            "--approver",
            "34",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--name/--message do not apply to legacy flat approvals; the email is the tenant's approval template
"#
    );
}

#[test]
fn sr_legacy_approvals_remind_dry_run() {
    let r = run_fixture(
        "legacy_approvals",
        &["sr", "approvals", "remind", "141", "601", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "legacy_approval_remind_tickets",
  "method": "PUT",
  "path": "/api/_/tickets/141/approvals/601/resend",
  "payload": null,
  "plan_hash": "0c5a3036e2183813e07e1dbec89cb5499a3ec8be269fe96b0ed3ca84e790d47b",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "approval_id": 601,
  "operation": "remind",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_legacy_approvals_remind_send() {
    let r = run_fixture(
        "legacy_approvals",
        &[
            "sr",
            "approvals",
            "remind",
            "141",
            "601",
            "--yes",
            "--expect-plan",
            "0c5a3036e2183813e07e1dbec89cb5499a3ec8be269fe96b0ed3ca84e790d47b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "0c5a3036e2183813e07e1dbec89cb5499a3ec8be269fe96b0ed3ca84e790d47b",
  "verified": true,
  "readback": {
    "id": 601,
    "member_id": 33,
    "level_id": 1,
    "status": {
      "id": 0,
      "name": "requested"
    },
    "type": {
      "id": 1,
      "name": "everyone"
    },
    "last_reminded_at": "2026-11-02T08:00:00Z"
  },
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/tickets/141/approvals/601/resend"]
    );
}

#[test]
fn sr_legacy_approvals_approve_refused() {
    let r = run_fixture(
        "legacy_approvals",
        &["sr", "approvals", "approve", "141", "601", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"sr approvals approve refused: this tenant uses legacy flat approvals (approval-chains answered HTTP 400); legacy flat approvals: not yet supported
"#
    );
}

#[test]
fn solutions_create_dry_run() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme VPN workaround",
            "--body",
            "Restart the Acme VPN client",
            "--type",
            "workaround",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "solution_create",
  "method": "POST",
  "path": "/api/_/solutions/articles",
  "payload": {
    "article_type": 2,
    "description": "<div>Restart the Acme VPN client</div>",
    "folder_id": 301,
    "source": "problem",
    "status": 1,
    "title": "Acme VPN workaround"
  },
  "plan_hash": "958d1db9fa0a12f731924059fd5d820e94b99084479c859b25c95ee03c8ca700",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn solutions_create_send() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme VPN workaround",
            "--body",
            "Restart the Acme VPN client",
            "--type",
            "workaround",
            "--yes",
            "--expect-plan",
            "958d1db9fa0a12f731924059fd5d820e94b99084479c859b25c95ee03c8ca700",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "958d1db9fa0a12f731924059fd5d820e94b99084479c859b25c95ee03c8ca700",
  "verified": true,
  "article_id": 501,
  "fields": {
    "title": "matched",
    "description": "matched",
    "status": "matched",
    "article_type": "matched",
    "folder_id": "matched"
  },
  "readback": {
    "id": 501,
    "title": "Acme VPN workaround",
    "description": "<div>Restart the Acme VPN client</div>",
    "status": 1,
    "article_type": 2,
    "folder_id": 301
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/solutions/articles"]);
}

#[test]
fn solutions_create_named_folder() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme fix",
            "--body",
            "x",
            "--type",
            "permanent",
            "--category",
            "Acme KB",
            "--folder",
            "Archive",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "solution_create",
  "method": "POST",
  "path": "/api/_/solutions/articles",
  "payload": {
    "article_type": 1,
    "description": "<div>x</div>",
    "folder_id": 302,
    "source": "problem",
    "status": 1,
    "title": "Acme fix"
  },
  "plan_hash": "224027003e694b9503cb11f027c6ff7858ed1a39a73eb9d1432f12e77989d6ca",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn solutions_create_ambiguous_folder() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme fix",
            "--body",
            "x",
            "--type",
            "permanent",
            "--category",
            "202",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"category/folder must resolve uniquely from runtime metadata; specify --category and --folder
"#
    );
}

#[test]
fn solutions_create_bad_type() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme fix",
            "--body",
            "x",
            "--type",
            "faq",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"nonempty title/body and workaround or permanent type required
"#
    );
}

#[test]
fn solutions_create_unknown_category() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme fix",
            "--body",
            "x",
            "--type",
            "permanent",
            "--category",
            "Initech",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"category/folder must resolve uniquely from runtime metadata; specify --category and --folder
"#
    );
}

#[test]
fn solutions_create_read_only() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme fix",
            "--body",
            "x",
            "--type",
            "permanent",
            "--read-only",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read-only mode; no write attempted
"#
    );
}

#[test]
fn solutions_create_needs_expect_plan() {
    let r = run_fixture(
        "solutions",
        &[
            "solutions",
            "create",
            "--title",
            "Acme fix",
            "--body",
            "x",
            "--type",
            "permanent",
            "--yes",
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
fn problems_link_solution_dry_run() {
    let r = run_fixture(
        "solutions",
        &["problems", "link-solution", "PRB-20", "502", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_solution_link",
  "method": "PUT",
  "path": "/api/_/problems/20/solutions/502",
  "payload": {
    "type": "link"
  },
  "plan_hash": "46dbc5d6d131b6d15c13989f547a472b2b9f0623b523263d750f784817474f2f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_link_solution_send() {
    let r = run_fixture(
        "solutions",
        &[
            "problems",
            "link-solution",
            "20",
            "502",
            "--yes",
            "--expect-plan",
            "46dbc5d6d131b6d15c13989f547a472b2b9f0623b523263d750f784817474f2f",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "46dbc5d6d131b6d15c13989f547a472b2b9f0623b523263d750f784817474f2f",
  "verified": true,
  "readback": [
    {
      "id": 502,
      "article_type": 1
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/20/solutions/502"]);
}

#[test]
fn problems_link_solution_faq_refused() {
    let r = run_fixture(
        "solutions",
        &["problems", "link-solution", "20", "503", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only proven workaround/permanent article types can be linked
"#
    );
}

#[test]
fn problems_link_solution_present_refused() {
    let r = run_fixture(
        "solutions",
        &["problems", "link-solution", "21", "504", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"article already linked
"#
    );
}

#[test]
fn problems_unlink_solution_send_unverified() {
    let r = run_fixture(
        "solutions",
        &[
            "problems",
            "unlink-solution",
            "21",
            "504",
            "--yes",
            "--expect-plan",
            "50bc76b262c4175ad4438f2ca4e7a5e7cfe934927764aa868b463d97d65dfb30",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "50bc76b262c4175ad4438f2ca4e7a5e7cfe934927764aa868b463d97d65dfb30",
  "verified": false,
  "readback": [
    {
      "id": 504,
      "article_type": 2
    }
  ],
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/21/solutions/504"]);
}

#[test]
fn problems_unlink_solution_absent_refused() {
    let r = run_fixture(
        "solutions",
        &["problems", "unlink-solution", "20", "502", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"article is not linked to this problem
"#
    );
}

#[test]
fn tickets_bulk_spam_dry_run() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "spam", "INC-141", "140", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_bulk_spam",
  "method": "POST",
  "path": "/api/_/tickets/spam_multiple",
  "payload": {
    "ids": [
      140,
      141
    ]
  },
  "plan_hash": "32a7d9421e0465db821c8c9dda1aead49d6d4235c0d229d8a30c9e0e1235d73d",
  "warning": "Frozen explicit IDs, not server select-all. Partial outcomes are possible; never replay an uncertain bulk action.",
  "ticket_ids": [
    140,
    141
  ],
  "selection": {
    "action": "spam",
    "agent": null,
    "group": null,
    "ids": [
      "INC-141",
      "140"
    ],
    "max_records": null,
    "note_primary": null,
    "note_secondary": null,
    "primary": null,
    "select_all": false,
    "status": null,
    "where": []
  },
  "selected_tickets": [
    {
      "id": 140,
      "subject": "Acme ticket 140"
    },
    {
      "id": 141,
      "subject": "Acme ticket 141"
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 140, 141
  140: Acme ticket 140
  141: Acme ticket 141
"#
    );
}

#[test]
fn tickets_bulk_delete_dry_run() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "delete", "140", "141", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_bulk_delete",
  "method": "DELETE",
  "path": "/api/_/tickets/delete_multiple?ids=140%2C141",
  "payload": null,
  "plan_hash": "d476a1270cf51e68cc8eb4fb457eaafb42ad0101ed99611c7308768050b13985",
  "warning": "Frozen explicit IDs, not server select-all. Partial outcomes are possible; never replay an uncertain bulk action.",
  "ticket_ids": [
    140,
    141
  ],
  "selection": {
    "action": "delete",
    "agent": null,
    "group": null,
    "ids": [
      "140",
      "141"
    ],
    "max_records": null,
    "note_primary": null,
    "note_secondary": null,
    "primary": null,
    "select_all": false,
    "status": null,
    "where": []
  },
  "selected_tickets": [
    {
      "id": 140,
      "subject": "Acme ticket 140"
    },
    {
      "id": 141,
      "subject": "Acme ticket 141"
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 140, 141
  140: Acme ticket 140
  141: Acme ticket 141
"#
    );
}

#[test]
fn tickets_bulk_unspam_send() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "unspam",
            "142",
            "--yes",
            "--expect-plan",
            "0c8e0610cfe6f9cf48ab7a84db781d83da59dd82806c09cb6a69dfde33dfda9f",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 142
  142: Acme ticket 142
bulk selection, metadata or records changed; review again
"#
    );
}

#[test]
fn tickets_bulk_unspam_send_partial() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "unspam",
            "142",
            "143",
            "--yes",
            "--expect-plan",
            "e3ce213f2c2d83ce627ed0779a4840227be4c8cf371f7f6d0ff5423ffdbc3131",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 142, 143
  142: Acme ticket 142
  143: Acme ticket 143
bulk selection, metadata or records changed; review again
"#
    );
}

#[test]
fn tickets_bulk_spam_needs_confirm() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "144",
            "--yes",
            "--expect-plan",
            "c5b520702e529b1b42fae51dd817c29b25fd2c18afd471a738b3e7339461d31b",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 144
  144: Acme ticket 144
spam/delete require --confirm-destructive after inspecting the printed ID list
"#
    );
}

#[test]
fn tickets_bulk_spam_send() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "144",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "c5b520702e529b1b42fae51dd817c29b25fd2c18afd471a738b3e7339461d31b",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 144
  144: Acme ticket 144
bulk selection, metadata or records changed; review again
"#
    );
}

#[test]
fn tickets_bulk_needs_expect_plan() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "spam", "140", "--yes"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 140
  140: Acme ticket 140
review with --dry-run, then --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn tickets_bulk_wrong_hash() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "140",
            "--yes",
            "--expect-plan",
            "0000",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 140
  140: Acme ticket 140
review with --dry-run, then --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn tickets_bulk_no_ids() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "spam", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"select 1 to 100 unique tickets; never silently truncate or chunk
"#
    );
}

#[test]
fn tickets_bulk_duplicate_ids() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "spam", "140", "INC-140", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"select 1 to 100 unique tickets; never silently truncate or chunk
"#
    );
}

#[test]
fn tickets_bulk_where_without_select_all() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filters require --select-all and no explicit IDs
"#
    );
}

#[test]
fn tickets_bulk_select_all_without_where() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "spam", "--select-all", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--select-all requires an explicit filter
"#
    );
}

#[test]
fn tickets_bulk_unsupported_action() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "archive", "140", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unsupported bulk action
"#
    );
}

#[test]
fn tickets_bulk_option_mismatch() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "140",
            "--agent",
            "ann@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"options do not match bulk action
"#
    );
}

#[test]
fn tickets_bulk_status_closure_refused() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "status",
            "140",
            "--status",
            "Closed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"bulk closure needs separate prerequisites; select an ordinary runtime status
"#
    );
}

#[test]
fn tickets_bulk_assign_needs_both() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "assign",
            "140",
            "--agent",
            "ann@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"captured bulk assignment requires both --agent and --group; null/unassignment is unproven
"#
    );
}

#[test]
fn tickets_bulk_merge_primary_outside() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "merge",
            "140",
            "141",
            "--primary",
            "142",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"merge primary must be in the printed selected ID list
"#
    );
}

#[test]
fn tickets_bulk_baseline_flag_missing() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "spam", "145", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"spam/deleted baseline flag unavailable
"#
    );
}

#[test]
fn tickets_bulk_filtered_dry_run() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--select-all",
            "--max-records",
            "5",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_bulk_spam",
  "method": "POST",
  "path": "/api/_/tickets/spam_multiple",
  "payload": {
    "ids": [
      140,
      141
    ]
  },
  "plan_hash": "aaa8f565ede7335dfc7243459f41028f40006762f1b3c3c0f1b05b059b873c4d",
  "warning": "Frozen explicit IDs, not server select-all. Partial outcomes are possible; never replay an uncertain bulk action.",
  "ticket_ids": [
    140,
    141
  ],
  "selection": {
    "action": "spam",
    "agent": null,
    "group": null,
    "ids": [],
    "max_records": 5,
    "note_primary": null,
    "note_secondary": null,
    "primary": null,
    "select_all": true,
    "status": null,
    "where": [
      "status=Open"
    ]
  },
  "selected_tickets": [
    {
      "id": 140,
      "subject": "Acme ticket 140"
    },
    {
      "id": 141,
      "subject": "Acme ticket 141"
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 140, 141
  140: Acme ticket 140
  141: Acme ticket 141
"#
    );
}

#[test]
fn tickets_bulk_filtered_needs_max_records() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--select-all",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filtered writes require --max-records N (1 to 50)
"#
    );
}

#[test]
fn tickets_bulk_filtered_max_records_range() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--select-all",
            "--max-records",
            "51",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filtered writes require --max-records N (1 to 50)
"#
    );
}

#[test]
fn tickets_bulk_filtered_exceeds_max_records() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--select-all",
            "--max-records",
            "1",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filtered selection exceeds --max-records; refusing the whole plan
"#
    );
}

#[test]
fn tickets_bulk_filtered_contradiction() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Pending",
            "--select-all",
            "--max-records",
            "5",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 140 contradicts the predicate; refusing the whole filtered plan
"#
    );
}

#[test]
fn tickets_bulk_filter_with_ids_refused() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "140",
            "--where",
            "status=Open",
            "--select-all",
            "--max-records",
            "5",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filters require --select-all and no explicit IDs
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_acme_status_dry_run() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "status",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--status",
            "Open",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_acme_status_send() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "status",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--status",
            "Open",
            "--yes",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_acme_spam_dry_run() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_acme_spam_send() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_acme_delete_dry_run() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "delete",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_acme_delete_send() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "delete",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_globex_status_dry_run() {
    let r = run_fixture(
        "ticket_bulk_filter_globex",
        &[
            "tickets",
            "bulk",
            "status",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--status",
            "Open",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_globex_status_send() {
    let r = run_fixture(
        "ticket_bulk_filter_globex",
        &[
            "tickets",
            "bulk",
            "status",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--status",
            "Open",
            "--yes",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_globex_spam_dry_run() {
    let r = run_fixture(
        "ticket_bulk_filter_globex",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_globex_spam_send() {
    let r = run_fixture(
        "ticket_bulk_filter_globex",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_globex_delete_dry_run() {
    let r = run_fixture(
        "ticket_bulk_filter_globex",
        &[
            "tickets",
            "bulk",
            "delete",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_subject_filter_globex_delete_send() {
    let r = run_fixture(
        "ticket_bulk_filter_globex",
        &[
            "tickets",
            "bulk",
            "delete",
            "--where",
            "Subject=Acme rollout",
            "--select-all",
            "--max-records",
            "5",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search; filterable fields on this tenant: Status, Type
"#
    );
}

#[test]
fn tickets_bulk_filtered_bound_missing() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--select-all",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filtered writes require --max-records N (1 to 50)
"#
    );
}

#[test]
fn tickets_bulk_filtered_bound_zero() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--select-all",
            "--max-records",
            "0",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filtered writes require --max-records N (1 to 50)
"#
    );
}

#[test]
fn tickets_bulk_filtered_bound_fifty_one() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Open",
            "--select-all",
            "--max-records",
            "51",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"filtered writes require --max-records N (1 to 50)
"#
    );
}

#[test]
fn tickets_bulk_server_ignores_filter_acme() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Closed",
            "--select-all",
            "--max-records",
            "50",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 140 contradicts the predicate; refusing the whole filtered plan
"#
    );
}

#[test]
fn tickets_bulk_server_ignores_filter_globex() {
    let r = run_fixture(
        "ticket_bulk_filter_globex",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "status=Geloest",
            "--select-all",
            "--max-records",
            "50",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 140 contradicts the predicate; refusing the whole filtered plan
"#
    );
}

#[test]
fn tickets_bulk_type_filter_needs_independent_proof() {
    let r = run_fixture(
        "ticket_bulk_filter",
        &[
            "tickets",
            "bulk",
            "spam",
            "--where",
            "type=Service Request",
            "--select-all",
            "--max-records",
            "50",
            "--confirm-destructive",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"predicate field unavailable in exact record: 'ticket_type'
"#
    );
}

#[test]
fn changes_relationships_list_problems() {
    let r = run_fixture(
        "change_relationships",
        &["changes", "relationships", "list", "CHN-50", "problem"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 70,
    "subject": "Acme problem 70"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_relationships_list_projects() {
    let r = run_fixture(
        "change_relationships",
        &["changes", "relationships", "list", "52", "project"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 9,
    "key": "ACME",
    "name": "Acme rollout"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_relationships_list_proxy_refused() {
    let r = run_fixture(
        "change_relationships",
        &["changes", "relationships", "list", "53", "project"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"project proxy did not return an accepted nested status/body
"#
    );
}

#[test]
fn changes_relationships_list_bad_kind() {
    let r = run_fixture(
        "change_relationships",
        &["changes", "relationships", "list", "50", "assets"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unsupported change relationship kind
"#
    );
}

#[test]
fn changes_relationships_add_problem_dry_run() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "50",
            "problem",
            "add",
            "--item",
            "71",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_relationship_problem_add",
  "method": "PUT",
  "path": "/api/_/changes/50/problems/associate",
  "payload": {
    "item_ids": [
      71
    ],
    "module_type": "problem"
  },
  "plan_hash": "b99e3b3978bcabd85ba7572939409e50ff02b17d7da65c35e1eb008040deb2d4",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_relationships_add_problem_send() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "50",
            "problem",
            "add",
            "--item",
            "71",
            "--yes",
            "--expect-plan",
            "b99e3b3978bcabd85ba7572939409e50ff02b17d7da65c35e1eb008040deb2d4",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "b99e3b3978bcabd85ba7572939409e50ff02b17d7da65c35e1eb008040deb2d4",
  "verified": true,
  "item_id": 71,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PUT /api/_/changes/50/problems/associate"]
    );
}

#[test]
fn changes_relationships_add_problem_needs_expect_plan() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "50",
            "problem",
            "add",
            "--item",
            "71",
            "--yes",
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
fn changes_relationships_add_present_refused() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "50",
            "problem",
            "add",
            "--item",
            "70",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"relationship already present or removal target absent
"#
    );
}

#[test]
fn changes_relationships_add_candidate_mismatch() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "50",
            "problem",
            "add",
            "--item",
            "73",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact relationship candidate unavailable
"#
    );
}

#[test]
fn changes_relationships_remove_send_unverified() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "51",
            "problem",
            "remove",
            "--item",
            "72",
            "--yes",
            "--expect-plan",
            "1947a8d5515a8713a91e0f7f03a2d8cc61858702643cb390c50fd46eba05be16",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1947a8d5515a8713a91e0f7f03a2d8cc61858702643cb390c50fd46eba05be16",
  "verified": false,
  "item_id": 72,
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/changes/51/problems/72"]);
}

#[test]
fn changes_relationships_project_options_refused() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "50",
            "problem",
            "add",
            "--item",
            "71",
            "--project-key",
            "ACME",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"project options do not apply to this relationship
"#
    );
}

#[test]
fn changes_relationships_bad_action() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "50",
            "problem",
            "link",
            "--item",
            "71",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"choose a captured relationship kind and add/remove
"#
    );
}

#[test]
fn changes_relationships_project_add_existing_refused() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "52",
            "project",
            "add",
            "--item",
            "ACME",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"relationship already present
"#
    );
}

#[test]
fn changes_relationships_project_add_incomplete() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "52",
            "project",
            "add",
            "--project-json",
            "{\"name\": \"Acme\"}",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"project requires name, description, start_date, end_date, owner_email and project_category text
"#
    );
}

#[test]
fn changes_relationships_project_remove_dry_run() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "52",
            "project",
            "remove",
            "--item",
            "ACME",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_relationship_project_remove",
  "method": "POST",
  "path": "/api/_/freshrelease/settings/unlink_fr_project",
  "payload": {
    "activity_data": {
      "key": "ACME",
      "name": "Acme rollout"
    },
    "project_id": "9",
    "project_key": "ACME",
    "remote_resource_id": "52",
    "remote_resource_type": "Change"
  },
  "plan_hash": "349385c9851d41f850bcd21918ccbca8999a1699f1a274a4108150be30ebc1f5",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_relationships_task_bad_key() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "52",
            "project-task",
            "add",
            "--item",
            "ACME-2",
            "--project-key",
            "acme",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"project-task requires an exact uppercase --project-key
"#
    );
}

#[test]
fn changes_relationships_task_add_dry_run() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "52",
            "project-task",
            "add",
            "--item",
            "ACME-2",
            "--project-key",
            "ACME",
            "--name",
            "Acme step 2",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_relationship_project-task_add",
  "method": "POST",
  "path": "/api/_/freshrelease/settings/create_issue",
  "payload": {
    "activity_data": {
      "key": "ACME-2",
      "name": "Acme step 2"
    },
    "issue": {
      "id": "ACME-2"
    },
    "meta": {
      "subject": "Acme change 52"
    },
    "placeholder": {
      "projectId": "ACME",
      "resourceId": "52",
      "resourceType": "Change"
    },
    "remote_resource_id": "52",
    "remote_resource_type": "Change"
  },
  "plan_hash": "8bbc81b6fb4322ecb5541a398df6e63583fe071ae432d38383dc2bba93d90090",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_relationships_task_remove_dry_run() {
    let r = run_fixture(
        "change_relationships",
        &[
            "changes",
            "relationships",
            "write",
            "52",
            "project-task",
            "remove",
            "--item",
            "ACME-1",
            "--project-key",
            "ACME",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_relationship_project-task_remove",
  "method": "POST",
  "path": "/api/_/freshrelease/settings/unlink_issue",
  "payload": {
    "activity_data": {
      "key": "ACME-1",
      "name": "Acme step 1"
    },
    "issue_id": "ACME-1",
    "placeholder": {
      "projectId": "ACME",
      "resourceId": "52",
      "resourceType": "Change"
    }
  },
  "plan_hash": "a14a0bcb113fb937986e49c64b604155c002acb94e2cc8d40f482ca30e6a443e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_relationships_list_tasks() {
    let r = run_fixture(
        "change_relationships",
        &["changes", "relationships", "list", "52", "project-task"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 91,
    "key": "ACME-1",
    "name": "Acme step 1",
    "project_id": 9
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_tasks_update_dry_run() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "CHN-5",
            "3",
            "--title",
            "Acme patch v2",
            "--owner",
            "bo@acme.com",
            "--group",
            "DBA",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_task_update",
  "method": "PUT",
  "path": "/api/_/changes/5/tasks/3",
  "payload": {
    "group_id": 5,
    "owner_id": 133,
    "title": "Acme patch v2"
  },
  "plan_hash": "838520b9e805f359aadfebcf8cf25e17006fa5a08ea982c8707d169184453003",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_tasks_update_send() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--title",
            "Acme patch v2",
            "--owner",
            "bo@acme.com",
            "--group",
            "DBA",
            "--yes",
            "--expect-plan",
            "838520b9e805f359aadfebcf8cf25e17006fa5a08ea982c8707d169184453003",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "838520b9e805f359aadfebcf8cf25e17006fa5a08ea982c8707d169184453003",
  "verified": true,
  "fields_verified": true,
  "fields": {
    "group_id": "matched",
    "owner_id": "matched",
    "title": "matched"
  },
  "readback": {
    "id": 3,
    "title": "Acme patch v2",
    "status": 1,
    "owner_id": 133,
    "group_id": 5,
    "planned_start_date": "2030-01-01T09:00:00Z",
    "planned_end_date": "2030-01-01T10:00:00Z",
    "planned_effort": "1h",
    "description": "<div>Acme</div>",
    "custom_fields": {}
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/5/tasks/3"]);
}

#[test]
fn changes_tasks_update_needs_expect_plan() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--title",
            "Acme patch v2",
            "--yes",
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
fn changes_tasks_update_read_only() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--title",
            "Acme patch v2",
            "--read-only",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read-only mode; no write attempted
"#
    );
}

#[test]
fn changes_tasks_update_complete_send_unverified() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "7",
            "8",
            "--status",
            "Completed",
            "--yes",
            "--expect-plan",
            "171ef4437f14036963a333ec5edad09061636d46a0c1bf73cb0aa35bcd00c7ab",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "171ef4437f14036963a333ec5edad09061636d46a0c1bf73cb0aa35bcd00c7ab",
  "verified": false,
  "fields_verified": false,
  "fields": {
    "status": "mismatched"
  },
  "readback": {
    "id": 8,
    "title": "Acme close",
    "status": 1,
    "owner_id": 131,
    "group_id": 4,
    "planned_start_date": "2030-01-01T09:00:00Z",
    "planned_end_date": "2030-01-01T10:00:00Z",
    "planned_effort": "1h",
    "description": "<div>Acme</div>",
    "custom_fields": {
      "signoff": "Acme ok"
    }
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/7/tasks/8"]);
}

#[test]
fn changes_tasks_update_reminder_dry_run() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "9",
            "11",
            "--notify-before",
            "15 Minutes",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_task_update",
  "method": "PUT",
  "path": "/api/_/changes/9/tasks/11",
  "payload": {
    "notify_before": 900
  },
  "plan_hash": "a5d3a3e1374140782c59b791dd1db00d1e0757d1e4f9e98fc731a3e48383917d",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_tasks_update_system_dry_run() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "9",
            "11",
            "--system",
            "SAP",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_task_update",
  "method": "PUT",
  "path": "/api/_/changes/9/tasks/11",
  "payload": {
    "custom_fields": {
      "system": 10
    }
  },
  "plan_hash": "0ff01b3f23bfebad4c8379c71860bea11fea3fa46b4b12a0e3866e9b90414073",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_tasks_update_unknown_status() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--status",
            "Bogus",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"status must match one active runtime task-status ID or label; 'Bogus' is not one (options: Open, In Progress, Completed)
"#
    );
}

#[test]
fn changes_tasks_update_unknown_task() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "5",
            "99",
            "--title",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact task unavailable
"#
    );
}

#[test]
fn changes_tasks_update_no_changes() {
    let r = run_fixture(
        "change_task_review",
        &["changes", "tasks-update", "5", "3", "--dry-run"],
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
fn changes_tasks_update_due_date_refused() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--due-date",
            "2030-01-02",
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
fn changes_tasks_update_unknown_owner() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-update",
            "5",
            "3",
            "--owner",
            "zed@acme.com",
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
fn changes_tasks_delete_dry_run() {
    let r = run_fixture(
        "change_task_review",
        &["changes", "tasks-delete", "CHN-6", "5", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_task_delete",
  "method": "PUT",
  "path": "/api/_/changes/6/tasks/update",
  "payload": {
    "action": "delete",
    "id": "5"
  },
  "plan_hash": "247ff75c48b0d6e54254098197d52b519d38016701c98ab0ca09407c9ddfd93e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_tasks_delete_send() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-delete",
            "6",
            "5",
            "--yes",
            "--expect-plan",
            "247ff75c48b0d6e54254098197d52b519d38016701c98ab0ca09407c9ddfd93e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "247ff75c48b0d6e54254098197d52b519d38016701c98ab0ca09407c9ddfd93e",
  "verified": true,
  "task_id": 5,
  "readback": [
    {
      "id": 7,
      "title": "Acme keep",
      "status": 1,
      "owner_id": 131,
      "group_id": 4,
      "planned_start_date": "2030-01-01T09:00:00Z",
      "planned_end_date": "2030-01-01T10:00:00Z",
      "planned_effort": "1h",
      "description": "<div>Acme</div>",
      "custom_fields": {}
    }
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/6/tasks/update"]);
}

#[test]
fn changes_tasks_delete_send_unverified() {
    let r = run_fixture(
        "change_task_review",
        &[
            "changes",
            "tasks-delete",
            "8",
            "6",
            "--yes",
            "--expect-plan",
            "91615545816adce073ab212d40250e099430e4c8ab62dcc5c165966f634bc589",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "91615545816adce073ab212d40250e099430e4c8ab62dcc5c165966f634bc589",
  "verified": false,
  "task_id": 6,
  "readback": [
    {
      "id": 6,
      "title": "Acme stuck",
      "status": 1,
      "owner_id": 131,
      "group_id": 4,
      "planned_start_date": "2030-01-01T09:00:00Z",
      "planned_end_date": "2030-01-01T10:00:00Z",
      "planned_effort": "1h",
      "description": "<div>Acme</div>",
      "custom_fields": {}
    }
  ],
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/8/tasks/update"]);
}

#[test]
fn changes_tasks_delete_needs_expect_plan() {
    let r = run_fixture(
        "change_task_review",
        &["changes", "tasks-delete", "6", "5", "--yes"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: review with --dry-run, then --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn changes_tasks_delete_unknown_task() {
    let r = run_fixture(
        "change_task_review",
        &["changes", "tasks-delete", "6", "99", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: exact task unavailable
"#
    );
}

#[test]
fn changes_delete_dry_run() {
    let r = run_fixture(
        "change_delete",
        &["changes", "delete", "CHN-21", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "change_delete",
  "method": "DELETE",
  "path": "/api/_/changes/21",
  "payload": null,
  "plan_hash": "9cca997e91937d620cc3621387b5e026f0cd50e3282ff2a51fe8f350dd564002",
  "warning": "Destructive soft delete of one change. Restoration is not a captured fsv contract; unknown outcomes must be inspected, never automatically resent.",
  "change_id": 21,
  "subject": "Acme firewall rollout",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_delete_send() {
    let r = run_fixture(
        "change_delete",
        &[
            "changes",
            "delete",
            "21",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "9cca997e91937d620cc3621387b5e026f0cd50e3282ff2a51fe8f350dd564002",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "9cca997e91937d620cc3621387b5e026f0cd50e3282ff2a51fe8f350dd564002",
  "verified": true,
  "change_id": 21,
  "fields": {
    "deleted": "matched"
  },
  "readback_attempts": 1,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/changes/21"]);
}

#[test]
fn changes_delete_send_unverified() {
    let r = run_fixture(
        "change_delete",
        &[
            "changes",
            "delete",
            "22",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "65805d8152bd5d89823a2b18862660a960a0ed816c6b209588e8805fb29e0b1e",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "65805d8152bd5d89823a2b18862660a960a0ed816c6b209588e8805fb29e0b1e",
  "verified": false,
  "change_id": 22,
  "fields": {
    "deleted": "mismatched"
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/changes/22"]);
}

#[test]
fn changes_delete_needs_confirm_destructive() {
    let r = run_fixture(
        "change_delete",
        &[
            "changes",
            "delete",
            "21",
            "--yes",
            "--expect-plan",
            "9cca997e91937d620cc3621387b5e026f0cd50e3282ff2a51fe8f350dd564002",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: change delete requires --confirm-destructive after inspecting the review
"#
    );
}

#[test]
fn changes_delete_needs_expect_plan() {
    let r = run_fixture(
        "change_delete",
        &["changes", "delete", "21", "--confirm-destructive", "--yes"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: review with --dry-run, then --confirm-destructive --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn changes_delete_wrong_hash() {
    let r = run_fixture(
        "change_delete",
        &[
            "changes",
            "delete",
            "21",
            "--confirm-destructive",
            "--yes",
            "--expect-plan",
            "0000",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: review with --dry-run, then --confirm-destructive --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn changes_delete_already_deleted_refused() {
    let r = run_fixture(
        "change_delete",
        &["changes", "delete", "23", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: change must be readable with deleted:false before deletion
"#
    );
}

#[test]
fn changes_delete_missing_refused() {
    let r = run_fixture(
        "change_delete",
        &["changes", "delete", "24", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 404 fetching change: not found
"#
    );
}

#[test]
fn changes_delete_bad_id() {
    let r = run_fixture(
        "change_delete",
        &["changes", "delete", "INC-21", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: prefix INC not valid for changes (expect CHN)
"#
    );
}

#[test]
fn changes_time_list() {
    let r = run_fixture("module_time", &["changes", "time", "list", "CHN-60"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 70,
    "agent_id": 9,
    "billable": true,
    "timer_running": false,
    "time_spent": "01:30:00",
    "note": "Acme work",
    "executed_at": "2030-01-01T09:00:00Z"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_time_list() {
    let r = run_fixture("module_time", &["problems", "time", "list", "PRB-63"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 75,
    "agent_id": 9,
    "billable": true,
    "timer_running": false,
    "time_spent": "01:30:00",
    "note": "Acme work",
    "executed_at": "2030-01-01T09:00:00Z"
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_time_create_dry_run() {
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
fn changes_time_create_send() {
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
            "--yes",
            "--expect-plan",
            "28d4b788bf14df27996bfa30b3d6f25be7d848e5ca9610032205d6e15e2e7ce1",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "28d4b788bf14df27996bfa30b3d6f25be7d848e5ca9610032205d6e15e2e7ce1",
  "verified": false,
  "entry_id": 71,
  "fields": {
    "agent_id": "matched",
    "billable": "mismatched",
    "executed_at": "matched",
    "note": "matched",
    "time_spent": "matched",
    "timer_running": "matched"
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes/60/time_entries"]);
}

#[test]
fn changes_time_create_needs_offset() {
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
            "2030-01-02T09:00:00",
            "--dry-run",
        ],
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
fn changes_time_create_bad_duration() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "60",
            "create",
            "--spent",
            "00:00",
            "--executed-at",
            "2030-01-02T09:00:00+07:00",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"manual time must be a positive HH:MM duration
"#
    );
}

#[test]
fn changes_time_create_missing_spent() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "60",
            "create",
            "--executed-at",
            "2030-01-02T09:00:00+07:00",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"create requires --spent and --executed-at, without an entry ID
"#
    );
}

#[test]
fn changes_time_update_note_dry_run() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "61",
            "update",
            "--entry",
            "72",
            "--note",
            "Acme review",
            "--non-billable",
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
    "billable": false,
    "note": "Acme review",
    "time_spent": "01:30"
  },
  "plan_hash": "5982e812b5c6e9fd1c714ccf9d1c509a107d9a962b1a749650f3f545a704a16c",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn changes_time_update_send_unverified() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "61",
            "update",
            "--entry",
            "72",
            "--note",
            "Acme review",
            "--yes",
            "--expect-plan",
            "f764c494bcd9611ca9ef8ef07160a5f2fb031a5b7c95de10912c32f964bf3974",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "f764c494bcd9611ca9ef8ef07160a5f2fb031a5b7c95de10912c32f964bf3974",
  "verified": false,
  "entry_id": 72,
  "fields": {
    "note": "mismatched",
    "time_spent": "matched"
  },
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/61/time_entries/72"]);
}

#[test]
fn changes_time_update_other_agent_refused() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "61",
            "update",
            "--entry",
            "74",
            "--note",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact self-owned manual entry is unavailable
"#
    );
}

#[test]
fn changes_time_update_nothing() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "61",
            "update",
            "--entry",
            "72",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"update requires --spent or --note
"#
    );
}

#[test]
fn changes_time_delete_send() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "62",
            "delete",
            "--entry",
            "73",
            "--yes",
            "--expect-plan",
            "c10701e31bfe149c363b8c3483208ed5df1a72c88e143e9ead1680e57aa4f692",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c10701e31bfe149c363b8c3483208ed5df1a72c88e143e9ead1680e57aa4f692",
  "verified": true,
  "entry_id": 73,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["DELETE /api/_/changes/62/time_entries/73"]
    );
}

#[test]
fn changes_time_delete_with_note_refused() {
    let r = run_fixture(
        "module_time",
        &[
            "changes",
            "time",
            "write",
            "62",
            "delete",
            "--entry",
            "73",
            "--note",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"delete does not accept time or note changes
"#
    );
}

#[test]
fn changes_time_needs_expect_plan() {
    let r = run_fixture(
        "module_time",
        &[
            "changes", "time", "write", "62", "delete", "--entry", "73", "--yes",
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
fn changes_time_bad_action() {
    let r = run_fixture(
        "module_time",
        &["changes", "time", "write", "60", "edit", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unsupported time-entry action
"#
    );
}

#[test]
fn problems_time_update_refused() {
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
fn problems_time_delete_send() {
    let r = run_fixture(
        "module_time",
        &[
            "problems",
            "time",
            "write",
            "64",
            "delete",
            "--entry",
            "76",
            "--yes",
            "--expect-plan",
            "638dd357aa7ac10c0debcd21d02e6c78e7420efd58a6208b17b55f49a46c4d0a",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "638dd357aa7ac10c0debcd21d02e6c78e7420efd58a6208b17b55f49a46c4d0a",
  "verified": true,
  "entry_id": 76,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["DELETE /api/_/problems/64/time_entries/76"]
    );
}

#[test]
fn problems_task_update_dry_run() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "PRB-80",
            "801",
            "update",
            "--title",
            "Acme fix v2",
            "--owner",
            "bo@acme.com",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_task_update",
  "method": "PUT",
  "path": "/api/_/problems/80/tasks/801",
  "payload": {
    "owner_id": 133,
    "title": "Acme fix v2"
  },
  "plan_hash": "2d83e85d864169c280c02bb8b384d51ebf8ffb22157f5de50a5596f156519092",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_task_update_send() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "80",
            "801",
            "update",
            "--title",
            "Acme fix v2",
            "--owner",
            "bo@acme.com",
            "--yes",
            "--expect-plan",
            "2d83e85d864169c280c02bb8b384d51ebf8ffb22157f5de50a5596f156519092",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2d83e85d864169c280c02bb8b384d51ebf8ffb22157f5de50a5596f156519092",
  "verified": true,
  "fields_verified": true,
  "fields": {
    "owner_id": "matched",
    "title": "matched"
  },
  "readback": {
    "id": 801,
    "title": "Acme fix v2",
    "status": 1,
    "taskable_display_id": 80,
    "taskable_type": "Itil::Problem",
    "owner_id": 133,
    "group_id": 4,
    "notify_before": 0,
    "planned_start_date": "2030-01-01T09:00:00Z",
    "planned_end_date": "2030-01-01T10:00:00Z",
    "planned_effort": "1h",
    "description": "<div>Acme</div>",
    "custom_fields": {}
  },
  "warning": "Task completion may rewrite planned dates and stop OLA timers; inspect the exact readback. Never resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/80/tasks/801"]);
}

#[test]
fn problems_task_update_note_dates_dry_run() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "80",
            "801",
            "update",
            "--note",
            "line 1\nline <2>",
            "--planned-start",
            "2030-01-01T10:00:00+07:00",
            "--planned-end",
            "2030-01-01T12:00:00+07:00",
            "--notify-before",
            "15 Minutes",
            "--group",
            "DBA",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_task_update",
  "method": "PUT",
  "path": "/api/_/problems/80/tasks/801",
  "payload": {
    "description": "<div>line 1<br>line &lt;2&gt;</div>",
    "group_id": 5,
    "notify_before": 900,
    "planned_end_date": "2030-01-01T12:00:00+07:00",
    "planned_start_date": "2030-01-01T10:00:00+07:00"
  },
  "plan_hash": "0b3f55797e8cba824ebd1f55881858269352d9dd558b5eeb2c158629a992cc1f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_task_update_end_before_start() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "80",
            "801",
            "update",
            "--planned-end",
            "2029-12-31T00:00:00Z",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planned end must not precede start
"#
    );
}

#[test]
fn problems_task_update_owner_none_refused() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "80",
            "801",
            "update",
            "--owner",
            "none",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"null owner/group update is not captured
"#
    );
}

#[test]
fn problems_task_update_nothing() {
    let r = run_fixture(
        "problem_tasks",
        &["problems", "task-write", "80", "801", "update", "--dry-run"],
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
fn problems_task_update_unknown_status() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "80",
            "801",
            "update",
            "--status",
            "Bogus",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"status must match one active runtime task-status ID or label; 'Bogus' is not one (options: Open, In Progress, Completed)
"#
    );
}

#[test]
fn problems_task_complete_dry_run() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "81",
            "811",
            "complete",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_task_update",
  "method": "PUT",
  "path": "/api/_/problems/81/tasks/811",
  "payload": {
    "status": 3
  },
  "plan_hash": "a201e7faba1248622c9a3c1e13042ede812edea4ac58d41f0fd7cfc9fa376b54",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_task_complete_send_unverified() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "81",
            "811",
            "complete",
            "--yes",
            "--expect-plan",
            "a201e7faba1248622c9a3c1e13042ede812edea4ac58d41f0fd7cfc9fa376b54",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "a201e7faba1248622c9a3c1e13042ede812edea4ac58d41f0fd7cfc9fa376b54",
  "verified": false,
  "fields_verified": false,
  "fields": {
    "status": "mismatched"
  },
  "readback": {
    "id": 811,
    "title": "Acme fix",
    "status": 1,
    "taskable_display_id": 81,
    "taskable_type": "Itil::Problem",
    "owner_id": 131,
    "group_id": 4,
    "notify_before": 0,
    "planned_start_date": "2030-01-01T09:00:00Z",
    "planned_end_date": "2030-01-01T10:00:00Z",
    "planned_effort": "1h",
    "description": "<div>Acme</div>",
    "custom_fields": {}
  },
  "warning": "Task completion may rewrite planned dates and stop OLA timers; inspect the exact readback. Never resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/81/tasks/811"]);
}

#[test]
fn problems_task_complete_with_fields_refused() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "81",
            "811",
            "complete",
            "--title",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"complete/delete do not accept update fields
"#
    );
}

#[test]
fn problems_task_update_with_completed_status_and_title() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "81",
            "811",
            "update",
            "--status",
            "Completed",
            "--title",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"captured completion is a separate status-only write
"#
    );
}

#[test]
fn problems_task_delete_dry_run() {
    let r = run_fixture(
        "problem_tasks",
        &["problems", "task-write", "82", "821", "delete", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_task_delete",
  "method": "PUT",
  "path": "/api/_/problems/82/tasks/update",
  "payload": {
    "action": "delete",
    "id": "821"
  },
  "plan_hash": "a175f145df003faefed0d77f57fe436dbd5a8525b6a42942063349e9f71eb23a",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_task_delete_send() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "82",
            "821",
            "delete",
            "--yes",
            "--expect-plan",
            "a175f145df003faefed0d77f57fe436dbd5a8525b6a42942063349e9f71eb23a",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "a175f145df003faefed0d77f57fe436dbd5a8525b6a42942063349e9f71eb23a",
  "verified": true,
  "task_id": 821,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/82/tasks/update"]);
}

#[test]
fn problems_task_delete_unknown() {
    let r = run_fixture(
        "problem_tasks",
        &["problems", "task-write", "82", "829", "delete", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact problem task unavailable
"#
    );
}

#[test]
fn problems_task_wrong_parent_refused() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "83",
            "831",
            "update",
            "--title",
            "x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact problem task identity/parent unavailable
"#
    );
}

#[test]
fn problems_task_bad_action() {
    let r = run_fixture(
        "problem_tasks",
        &["problems", "task-write", "80", "801", "close", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"task action must be update, complete or delete
"#
    );
}

#[test]
fn problems_task_needs_expect_plan() {
    let r = run_fixture(
        "problem_tasks",
        &[
            "problems",
            "task-write",
            "80",
            "801",
            "update",
            "--title",
            "x",
            "--yes",
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
fn sr_items_list() {
    let r = run_fixture("sr_items", &["sr", "items", "list", "SR-150"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"[
  {
    "id": 1501,
    "ticket_id": 150,
    "is_parent": true,
    "stage": {
      "id": 1,
      "name": "Stage 1"
    },
    "item": {
      "id": 4400001,
      "display_id": 77,
      "name": "Feature Request"
    },
    "quantity": 1
  },
  {
    "id": 1502,
    "ticket_id": 150,
    "is_parent": false,
    "stage": {
      "id": 1,
      "name": "Stage 1"
    },
    "item": {
      "id": 4400001,
      "display_id": 77,
      "name": "Feature Request"
    },
    "quantity": 1
  }
]
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_items_stage_dry_run() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "151",
            "stage",
            "--item",
            "1511",
            "--stage",
            "2",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "sr_requested_item_stage",
  "method": "PATCH",
  "path": "/api/_/tickets/151/requested_items/1511/update_stage",
  "payload": {
    "stage": 2
  },
  "plan_hash": "c56dd755464974e0b81f0e059cff7dfb22b68ca30ee66c9dc1b89fca60dcd631",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_items_stage_send() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "151",
            "stage",
            "--item",
            "1511",
            "--stage",
            "2",
            "--yes",
            "--expect-plan",
            "c56dd755464974e0b81f0e059cff7dfb22b68ca30ee66c9dc1b89fca60dcd631",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c56dd755464974e0b81f0e059cff7dfb22b68ca30ee66c9dc1b89fca60dcd631",
  "verified": true,
  "item_id": 1511,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["PATCH /api/_/tickets/151/requested_items/1511/update_stage"]
    );
}

#[test]
fn sr_items_stage_other_refused() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "151",
            "stage",
            "--item",
            "1511",
            "--stage",
            "3",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only captured stage 2 (observed Delivered) is supported; no combined answer update
"#
    );
}

#[test]
fn sr_items_remove_dry_run() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "152",
            "remove",
            "--item",
            "1522",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "sr_requested_item_remove",
  "method": "DELETE",
  "path": "/api/_/tickets/152/requested_items/1522",
  "payload": null,
  "plan_hash": "f4485ed7fc331fe201d7871116a03a85fad5a5a87ff32f852de7731ca1abd66f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn sr_items_remove_send_unverified() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "152",
            "remove",
            "--item",
            "1522",
            "--yes",
            "--expect-plan",
            "f4485ed7fc331fe201d7871116a03a85fad5a5a87ff32f852de7731ca1abd66f",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "f4485ed7fc331fe201d7871116a03a85fad5a5a87ff32f852de7731ca1abd66f",
  "verified": false,
  "item_id": 1522,
  "warning": "Readback was not fully verified; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(
        r.write_paths(),
        ["DELETE /api/_/tickets/152/requested_items/1522"]
    );
}

#[test]
fn sr_items_remove_parent_refused() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "150",
            "remove",
            "--item",
            "1501",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"only an explicitly non-parent item may be removed, without answer/stage edits
"#
    );
}

#[test]
fn sr_items_unknown_item() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "150",
            "remove",
            "--item",
            "1599",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact requested-item identity unavailable
"#
    );
}

#[test]
fn sr_items_not_service_request() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "153",
            "stage",
            "--item",
            "1",
            "--stage",
            "2",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact parent requires a proven service request; the record's own fields do not prove it (no type or display id). A tenant with a custom display-id prefix can declare it: fsv config set ids.service_request_prefix PREFIX
"#
    );
}

#[test]
fn sr_items_bad_action() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "150",
            "approve",
            "--item",
            "1502",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unsupported requested-item action
"#
    );
}

#[test]
fn sr_items_add_bad_catalog_id() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "150",
            "add",
            "--catalog-item",
            "x77",
            "--answers-json",
            "{\"title\": \"T\"}",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"add needs an exact positive catalog display ID as text, answers, and no existing item/stage
"#
    );
}

#[test]
fn sr_items_answers_not_object() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr",
            "items",
            "write",
            "150",
            "edit",
            "--item",
            "1502",
            "--answers-json",
            "[1]",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"answers must be a JSON object
"#
    );
}

#[test]
fn sr_items_needs_expect_plan() {
    let r = run_fixture(
        "sr_items",
        &[
            "sr", "items", "write", "151", "stage", "--item", "1511", "--stage", "2", "--yes",
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
