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
//! Python commit: 7fcdea40050b962d13c70c38328c6d5ea4455ae1

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
  "plan_hash": "3550252fbb8b8eff91ad10123e3af0291e8972d4a3dbb5b734daa9cce18c3a6e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "3550252fbb8b8eff91ad10123e3af0291e8972d4a3dbb5b734daa9cce18c3a6e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "3550252fbb8b8eff91ad10123e3af0291e8972d4a3dbb5b734daa9cce18c3a6e",
  "verified": true,
  "watcher_ids": [
    3,
    9
  ]
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
  "plan_hash": "be559b20226d34837e0affd47ed3b1529cfc13a4f09f29983895d421818bbeb2",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "be559b20226d34837e0affd47ed3b1529cfc13a4f09f29983895d421818bbeb2",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "be559b20226d34837e0affd47ed3b1529cfc13a4f09f29983895d421818bbeb2",
  "verified": true,
  "watcher_ids": [
    4
  ]
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
            "3f67af5e89c7796fcf83001ea60895cb8adb8c42f4ecb184d1cef67fb337cbc3",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "already_in_requested_state",
  "verified": true,
  "plan_hash": "3f67af5e89c7796fcf83001ea60895cb8adb8c42f4ecb184d1cef67fb337cbc3"
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
  "plan_hash": "d3ca3bff6a8b0b8e48f8529ad51750fb66e6959b216ee9667cd994c3f1440a52",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "d3ca3bff6a8b0b8e48f8529ad51750fb66e6959b216ee9667cd994c3f1440a52",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d3ca3bff6a8b0b8e48f8529ad51750fb66e6959b216ee9667cd994c3f1440a52",
  "verified": true,
  "entry_id": 31,
  "fields": {
    "agent_id": "matched",
    "billable": "matched",
    "executed_at": "matched",
    "note": "matched",
    "time_spent": "matched",
    "timer_running": "matched"
  }
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
  "plan_hash": "dd042d69b0381cfce9e840d2757039ef73203ba1cbcd367a49fc391fd74210f6",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "dd042d69b0381cfce9e840d2757039ef73203ba1cbcd367a49fc391fd74210f6",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "dd042d69b0381cfce9e840d2757039ef73203ba1cbcd367a49fc391fd74210f6",
  "verified": true,
  "entry_id": 41,
  "fields": {
    "note": "matched",
    "time_spent": "matched"
  }
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
            "0d3139b2f9da73285914f8a1556e7f3ba92d618cbd4863393950fdbebc951146",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "0d3139b2f9da73285914f8a1556e7f3ba92d618cbd4863393950fdbebc951146",
  "verified": true,
  "entry_id": 51
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
  "plan_hash": "c24b210f056a6adac79499f156f6426d9cfd567548dd5d76f467cb5ddeca1ee0",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "create",
  "before": "",
  "after": "<p>Globex rollout</p>"
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
            "c24b210f056a6adac79499f156f6426d9cfd567548dd5d76f467cb5ddeca1ee0",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c24b210f056a6adac79499f156f6426d9cfd567548dd5d76f467cb5ddeca1ee0",
  "verified": true,
  "section": "acme_plan",
  "description": "matched"
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
  "plan_hash": "a2da8700f9883b0573a9c87ab44e3e1d6c5c5404e88c3725c43e02dd7ecb22d7",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "<p>old</p>",
  "after": "x"
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
  "plan_hash": "9b08b333071d53755058fcef4d3167fbbc93eea205000470bf7785782c5f2173",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "create",
  "before": "",
  "after": "Acme DNS outage"
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
            "9b08b333071d53755058fcef4d3167fbbc93eea205000470bf7785782c5f2173",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "9b08b333071d53755058fcef4d3167fbbc93eea205000470bf7785782c5f2173",
  "verified": true,
  "section": "problem_cause",
  "description": "matched"
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
  "plan_hash": "ac447771e70b1c15ab26e19add62ef1d7dbecbc53f3c944987d1b8fcf591d9b1",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "<div>Acme original</div>",
  "after": "Acme new"
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
            "ac447771e70b1c15ab26e19add62ef1d7dbecbc53f3c944987d1b8fcf591d9b1",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ac447771e70b1c15ab26e19add62ef1d7dbecbc53f3c944987d1b8fcf591d9b1",
  "verified": true,
  "section": "acme_plan",
  "description": "matched"
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
  "plan_hash": "2e39db850f451625f78daa2f832f1223d71169be821b9cb4990067beb8aaeb7b",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "clear",
  "before": "<div>Acme original</div>",
  "after": ""
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
            "2e39db850f451625f78daa2f832f1223d71169be821b9cb4990067beb8aaeb7b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2e39db850f451625f78daa2f832f1223d71169be821b9cb4990067beb8aaeb7b",
  "verified": true,
  "section": "acme_plan",
  "description": "matched"
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
  "plan_hash": "f15c3d2339b690f4f445b2a51795ebdccc3361bb48fe79f68f81475389f40daa",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "Acme original",
  "after": "Acme"
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
  "plan_hash": "e33c72165368d8c13662c77458450eab0339c86bdb977db0841862240d9cc96f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "<div>Acme original</div>",
  "after": "Acme new"
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
            "e33c72165368d8c13662c77458450eab0339c86bdb977db0841862240d9cc96f",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e33c72165368d8c13662c77458450eab0339c86bdb977db0841862240d9cc96f",
  "verified": true,
  "section": "problem_cause",
  "description": "matched"
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
  "plan_hash": "d505aac8cc150eb5568d39aae2f18cd888b7f6f5bd2d4e4667f11aff17b6bf0b",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "clear",
  "before": "<div>Acme original</div>",
  "after": ""
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
            "d505aac8cc150eb5568d39aae2f18cd888b7f6f5bd2d4e4667f11aff17b6bf0b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d505aac8cc150eb5568d39aae2f18cd888b7f6f5bd2d4e4667f11aff17b6bf0b",
  "verified": true,
  "section": "problem_cause",
  "description": "matched"
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
  "plan_hash": "5f90b0f07360dfe22c64244bfe70d616c4926518e3f1103f979990f71eaa17f4",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "operation": "edit",
  "before": "Acme original",
  "after": "Acme"
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
  "plan_hash": "ac51138eacf22a51e00652581c6c4c83cb6f9235611456543550a85c75bd3102",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "ac51138eacf22a51e00652581c6c4c83cb6f9235611456543550a85c75bd3102",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ac51138eacf22a51e00652581c6c4c83cb6f9235611456543550a85c75bd3102",
  "verified": true,
  "problem_id": 7
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
            "76ed54825bfff7204593f9c86373ade92bcc68ee91eae8126df18bc898df7b12",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "already_in_requested_state",
  "verified": true
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
  "plan_hash": "ade2cf41762d10e5a608995dbf6486185908ca50a877f3fd543f29e0c6687e80",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."
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
            "ade2cf41762d10e5a608995dbf6486185908ca50a877f3fd543f29e0c6687e80",
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
  "plan_hash": "ade2cf41762d10e5a608995dbf6486185908ca50a877f3fd543f29e0c6687e80",
  "verified": true,
  "fields": {
    "body": "matched",
    "bcc_emails": "matched",
    "from_email": "matched"
  }
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
  "plan_hash": "ade2cf41762d10e5a608995dbf6486185908ca50a877f3fd543f29e0c6687e80",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."
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
  "plan_hash": "5c24fced7a7b4c77e468569a8fa7cbaa16dbda8cb34d184b3fe5daa0188a8263",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."
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
            "5c24fced7a7b4c77e468569a8fa7cbaa16dbda8cb34d184b3fe5daa0188a8263",
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
  "plan_hash": "5c24fced7a7b4c77e468569a8fa7cbaa16dbda8cb34d184b3fe5daa0188a8263",
  "verified": true,
  "fields": {
    "body": "matched",
    "private": "matched",
    "notify_emails": "matched",
    "from_email": "matched"
  }
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
  "plan_hash": "ea141a6dc6a011baf0ef52b8cdd432752efab0829be7418249e20d3303194278",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."
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
            "ea141a6dc6a011baf0ef52b8cdd432752efab0829be7418249e20d3303194278",
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
  "plan_hash": "ea141a6dc6a011baf0ef52b8cdd432752efab0829be7418249e20d3303194278",
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
  "verification_scope": "delivery fields; source link is not exposed by independent readback"
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
  "plan_hash": "3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."
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
  "plan_hash": "ee8614676d5f0a6d50b10744d54aa188a2f1eda2dcdcd142084de187b903c63d",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."
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
            "3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 5,
  "plan_hash": "3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7",
  "verified": true,
  "fields": {
    "priority": "matched"
  }
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
  "plan_hash": "3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."
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
  "plan_hash": "3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."
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
  "plan_hash": "3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows."
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
        r#"could not evaluate ticket edit form and rules; no update attempted
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
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--tag and planned dates are supported only for tickets
"#
    );
}

#[test]
fn update_ack_requires_reviewed() {
    let r = run_fixture(
        "ticket_reviewed",
        &["tickets", "update", "5", "--priority", "High", "--ack"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(r.err, "error: --ack requires tickets --reviewed\n");
}

#[test]
fn problems_update_ack_requires_reviewed() {
    let r = run_fixture(
        "ticket_reviewed",
        &["problems", "update", "5", "--ack"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(r.err, "error: --ack requires tickets --reviewed\n");
}

#[test]
fn update_reviewed_ack_without_blocking_keeps_hash() {
    let r = run_fixture(
        "ticket_reviewed",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--ack",
            "--priority",
            "High",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains(
            "\"plan_hash\": \"3212f8ceb7a5590061f639e5731caeaa0955a7617781938856beb69db8b6e8e7\""
        ),
        "{}",
        r.out
    );
    assert!(!r.out.contains("acknowledged_rule_messages"), "{}", r.out);
}

#[test]
fn update_reviewed_blocking_needs_ack() {
    let r = run_fixture(
        "ticket_reviewed_ack",
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
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(
        r.err,
        "ticket form validation failed: blocked by rule (acknowledge to continue): High priority needs the Acme lead; pass --ack to acknowledge the blocking rule message(s) after reading them\n"
    );
}

#[test]
fn update_reviewed_ack_dry_run() {
    let r = run_fixture(
        "ticket_reviewed_ack",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--ack",
            "--priority",
            "High",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.err, "");
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
  "plan_hash": "ef717dbaced486c6b863cfff6004a4384ac28c021f09ebaec43df1b22827dcb5",
  "warning": "Tags replace the full existing list. Updates may trigger tenant workflows.",
  "acknowledged_rule_messages": [
    "High priority needs the Acme lead"
  ]
}
"#
    );
}

#[test]
fn update_reviewed_ack_send() {
    let r = run_fixture(
        "ticket_reviewed_ack",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--ack",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "ef717dbaced486c6b863cfff6004a4384ac28c021f09ebaec43df1b22827dcb5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 5,
  "plan_hash": "ef717dbaced486c6b863cfff6004a4384ac28c021f09ebaec43df1b22827dcb5",
  "verified": true,
  "fields": {
    "priority": "matched"
  }
}
"#
    );
    assert!(
        r.err.contains("\"acknowledged_rule_messages\""),
        "{}",
        r.err
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/5"]);
}

#[test]
fn update_reviewed_acked_hash_without_ack() {
    let r = run_fixture(
        "ticket_reviewed_ack",
        &[
            "tickets",
            "update",
            "5",
            "--reviewed",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "ef717dbaced486c6b863cfff6004a4384ac28c021f09ebaec43df1b22827dcb5",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(
        r.err,
        "ticket form validation failed: blocked by rule (acknowledge to continue): High priority needs the Acme lead; pass --ack to acknowledge the blocking rule message(s) after reading them\n"
    );
    assert!(r.write_paths().is_empty(), "{:?}", r.write_paths());
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
        r#"error: --expect-plan requires --reviewed, --tag, --planned-start or --planned-end
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
  "plan_hash": "30c3701174e08ef1909b742d678003f03bdc9f025653d2e3410f8f0fe79d98c6",
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
  "priority_label": "High"
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
            "30c3701174e08ef1909b742d678003f03bdc9f025653d2e3410f8f0fe79d98c6",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "30c3701174e08ef1909b742d678003f03bdc9f025653d2e3410f8f0fe79d98c6",
  "verified": true,
  "tickets": {
    "5": "matched",
    "6": "matched"
  },
  "readback_attempts": 1
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
            "2d1b32c2ff8fff0541e21780bcefc8817ff4150c6a4a7b498d524f09215514ff",
        ],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2d1b32c2ff8fff0541e21780bcefc8817ff4150c6a4a7b498d524f09215514ff",
  "verified": false,
  "tickets": {
    "5": "matched",
    "7": "mismatched"
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend."
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
        r#"could not evaluate ticket edit form and rules; no update attempted
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
            "5e0581d3ec26d93df27b1323a4704c3d88408b3ffe759d5cbad55d7b2332cda2",
        ],
        None,
    );
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"  uploading deck.pptx... ok (4 B, server name "x" differs)
  uploading notes.txt... ok (5 B, server name "x" differs)
uploaded attachment ids (not attached to any request): 901 (deck.pptx), 901 (notes.txt)
error: place-request failed: HTTP 500; the outcome is unknown: check `fsv journal tail` and the requester's requests before retrying
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
  "plan_hash": "0b42cf4872bd8fa5b3592c1d34220179affeeffeadac93979894532555bda78b",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration.",
  "source": {
    "id": 701,
    "hash": "f136a71f4682d335d1cff7e3510f7e17a3a3c0be74155646755b18ebedfe2573"
  },
  "insertion": "Literal selected HTML only; no attachments, signatures or placeholder expansion."
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
            "0b42cf4872bd8fa5b3592c1d34220179affeeffeadac93979894532555bda78b",
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
  "plan_hash": "0b42cf4872bd8fa5b3592c1d34220179affeeffeadac93979894532555bda78b",
  "verified": true,
  "fields": {
    "body": "matched",
    "cc_emails": "matched",
    "from_email": "matched"
  }
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
  "plan_hash": "4726ade70372c6d6530c4707815a9c37b17b26a6a42868313d3d31715d35d81b",
  "warning": "Native automation may send mail or change records. No complete side-effect verification contract is captured.",
  "definition": {
    "id": 703,
    "name": "Globex"
  }
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
            "4726ade70372c6d6530c4707815a9c37b17b26a6a42868313d3d31715d35d81b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "4726ade70372c6d6530c4707815a9c37b17b26a6a42868313d3d31715d35d81b",
  "verified": false,
  "ticket_id": 5,
  "identity_verified": true,
  "warning": "Exact ticket was read back; scenario side effects (including mail/workflows) are unverified. Do not resend."
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
  "plan_hash": "f2279611eb0365ec3f3c99e4061b2013038ad7c5c7825901ba7c92aac04cb010",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "af7255ec8f4d3148c71b7c83bde4811a8c75d6afda269d5347645bbe18795817",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "8d0d36217062692720ca0a02bad6de70f3885c0177d88a76ee2e36bb33404f3b",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "e5413d712d79190ed9f407b9e68c6905df579832cab501cbb78a89b4bf14055e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "5c16cc288e892dbbd92783a6a92049f6802032d818b64c9991d1dcec0823ad70",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "f2279611eb0365ec3f3c99e4061b2013038ad7c5c7825901ba7c92aac04cb010",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "f2279611eb0365ec3f3c99e4061b2013038ad7c5c7825901ba7c92aac04cb010",
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
  "warning": "Readback was not fully verified; do not resend."
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
            "af7255ec8f4d3148c71b7c83bde4811a8c75d6afda269d5347645bbe18795817",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "af7255ec8f4d3148c71b7c83bde4811a8c75d6afda269d5347645bbe18795817",
  "verified": true,
  "view_id": 703,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "matched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  }
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
            "2c8884374517a2291e001c5ca542943f754217ea5e94e2f9f26d7d5235b661b5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2c8884374517a2291e001c5ca542943f754217ea5e94e2f9f26d7d5235b661b5",
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
  "warning": "Readback was not fully verified; do not resend."
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
            "8d0d36217062692720ca0a02bad6de70f3885c0177d88a76ee2e36bb33404f3b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8d0d36217062692720ca0a02bad6de70f3885c0177d88a76ee2e36bb33404f3b",
  "verified": true,
  "view_id": 704
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/ticket_filters/704"]);
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
  "plan_hash": "af7255ec8f4d3148c71b7c83bde4811a8c75d6afda269d5347645bbe18795817",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "3a8921a0810f9cfc5bd13e64bdcc76b847ef84a45751482cfff300b43524baf7",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "3a8921a0810f9cfc5bd13e64bdcc76b847ef84a45751482cfff300b43524baf7",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "3a8921a0810f9cfc5bd13e64bdcc76b847ef84a45751482cfff300b43524baf7",
  "verified": true,
  "view_id": 901,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "matched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  }
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
  "plan_hash": "76226cc21cb86df81dfb7a89b536bab25dc80a8071d7a8a8ccdaedb4a3389592",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "76226cc21cb86df81dfb7a89b536bab25dc80a8071d7a8a8ccdaedb4a3389592",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "76226cc21cb86df81dfb7a89b536bab25dc80a8071d7a8a8ccdaedb4a3389592",
  "verified": true,
  "view_id": 900,
  "fields": {
    "visibility": "matched",
    "workspace_id": "matched",
    "name": "matched",
    "order_by": "matched",
    "order_type": "matched",
    "conditions": "matched"
  }
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
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"changes private-view delete is not captured (PUT/DELETE /api/_/change-filters/<id> has no accepted wire); nothing was sent; use the web UI
"#
    );
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
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"changes private-view rename is not captured (PUT/DELETE /api/_/change-filters/<id> has no accepted wire); nothing was sent; use the web UI
"#
    );
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
  "plan_hash": "dfa7e59748090f63ce098e3d242a154f26cd10c352c4c8fcf07bbd5834a2fb54",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "dfa7e59748090f63ce098e3d242a154f26cd10c352c4c8fcf07bbd5834a2fb54",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "dfa7e59748090f63ce098e3d242a154f26cd10c352c4c8fcf07bbd5834a2fb54",
  "verified": true,
  "fields": {
    "display_id": "matched",
    "status": "matched",
    "ticket_subject": "matched"
  },
  "unverified_omissions": []
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
  "plan_hash": "8d40b150f5aca4efb14583eb29cb46703b5497d1786ffbf1f0261ee154871e32",
  "warning": "Review only: fsv never sends an export. The captured request carries the keys above; the server's job, file and delivery (presumably mail to the recipient below) are unknown, so a send could not be verified. Run the export from the web UI if you need it.",
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
  "review_only": true,
  "delivery": "unknown"
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
            "8d40b150f5aca4efb14583eb29cb46703b5497d1786ffbf1f0261ee154871e32",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"export is review-only in fsv (delivery cannot be verified); nothing was sent; use --dry-run to review, and the web UI to export
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
  "plan_hash": "a8b00903cbf7a14e987baff6b8042698e2f72f429b331bd756f382d38856c2fe",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "a8b00903cbf7a14e987baff6b8042698e2f72f429b331bd756f382d38856c2fe",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "a8b00903cbf7a14e987baff6b8042698e2f72f429b331bd756f382d38856c2fe",
  "verified": true,
  "fields": {
    "display_id": "matched",
    "requester": "matched",
    "status": "matched",
    "ticket_subject": "matched"
  },
  "unverified_omissions": []
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
  "plan_hash": "724d390aea5fd89f52addbd0542798bcd6132f345697453bae7383cd2d9e7e89",
  "warning": "Review only: fsv never sends an export. The captured request carries the keys above; the server's job, file and delivery (presumably mail to the recipient below) are unknown, so a send could not be verified. Run the export from the web UI if you need it.",
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
  "review_only": true,
  "delivery": "unknown"
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
            "724d390aea5fd89f52addbd0542798bcd6132f345697453bae7383cd2d9e7e89",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"export is review-only in fsv (delivery cannot be verified); nothing was sent; use --dry-run to review, and the web UI to export
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
  "plan_hash": "87152724fa035261a560ce41df1cfaf9add1265591b4269d584ed600af1742fc",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "87152724fa035261a560ce41df1cfaf9add1265591b4269d584ed600af1742fc",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "87152724fa035261a560ce41df1cfaf9add1265591b4269d584ed600af1742fc",
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
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "96634ff8a9e6f5eb960f4e8027a6be90f83a0c78d534ec8b44a7657733d366f3",
  "warning": "Review only: fsv never sends an export. The captured request carries the keys above; the server's job, file and delivery (presumably mail to the recipient below) are unknown, so a send could not be verified. Run the export from the web UI if you need it.",
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
  "review_only": true,
  "delivery": "unknown"
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
            "96634ff8a9e6f5eb960f4e8027a6be90f83a0c78d534ec8b44a7657733d366f3",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"export is review-only in fsv (delivery cannot be verified); nothing was sent; use --dry-run to review, and the web UI to export
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
  "plan_hash": "8d40b150f5aca4efb14583eb29cb46703b5497d1786ffbf1f0261ee154871e32",
  "warning": "Review only: fsv never sends an export. The captured request carries the keys above; the server's job, file and delivery (presumably mail to the recipient below) are unknown, so a send could not be verified. Run the export from the web UI if you need it.",
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
  "review_only": true,
  "delivery": "unknown"
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
        r#"export is review-only in fsv (delivery cannot be verified); nothing was sent; use --dry-run to review, and the web UI to export
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
        r#"export is review-only in fsv (delivery cannot be verified); nothing was sent; use --dry-run to review, and the web UI to export
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
  "plan_hash": "5baf2437f0de8ce64f064102eaab12d22a3510bd78c6ac6fe588bb0f761ca236",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "5baf2437f0de8ce64f064102eaab12d22a3510bd78c6ac6fe588bb0f761ca236",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "5baf2437f0de8ce64f064102eaab12d22a3510bd78c6ac6fe588bb0f761ca236",
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
  }
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
  "plan_hash": "d4e8552a8fd24b5316f03803e693a6e5f171ddef5218e39ba9e6eed406a6d7b6",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "c5c3e5fa8ec3fe6ac947470dd990b8d31ebaba64595dea542f2b7b5ffcd3f7fe",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "d4e8552a8fd24b5316f03803e693a6e5f171ddef5218e39ba9e6eed406a6d7b6",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d4e8552a8fd24b5316f03803e693a6e5f171ddef5218e39ba9e6eed406a6d7b6",
  "verified": true,
  "note_id": 456,
  "fields": {
    "body": "matched"
  }
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
            "c5c3e5fa8ec3fe6ac947470dd990b8d31ebaba64595dea542f2b7b5ffcd3f7fe",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c5c3e5fa8ec3fe6ac947470dd990b8d31ebaba64595dea542f2b7b5ffcd3f7fe",
  "verified": true,
  "task_id": 456,
  "fields": {
    "notify_before": "matched",
    "owner_id": "matched",
    "planned_effort": "matched",
    "status": "matched",
    "title": "matched"
  }
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
            "ed23c5dca4c8b43cc57f520bb06b700326f8fcc1d156d90d4b80cb4482f635c9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ed23c5dca4c8b43cc57f520bb06b700326f8fcc1d156d90d4b80cb4482f635c9",
  "verified": false,
  "note_id": 456,
  "fields": {
    "body": "mismatched"
  },
  "warning": "Readback was not fully verified; do not resend."
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
  }
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
  }
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
  }
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
  }
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
  "status": 1
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
        r#"{}
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
        "status": 62
      }
    }
  ],
  "plan_hash": "7c49d4b0c04c0bbe0619b9be73e451751847577b668d8d0991e55dd2921e674d",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."
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
        "status": 62
      }
    }
  ],
  "plan_hash": "ecff7776669056fd3454d7f2a862ff235a364cfed26f0552c0092f06ee1a224d",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."
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
            "7c49d4b0c04c0bbe0619b9be73e451751847577b668d8d0991e55dd2921e674d",
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
  "plan_hash": "7c49d4b0c04c0bbe0619b9be73e451751847577b668d8d0991e55dd2921e674d",
  "verified": true
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
        "status": 89
      }
    }
  ],
  "plan_hash": "809b34350b79011c765c2372918154b8d0eae87d6c0a084dc5c74b5f5029d920",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."
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
            "809b34350b79011c765c2372918154b8d0eae87d6c0a084dc5c74b5f5029d920",
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
  "plan_hash": "809b34350b79011c765c2372918154b8d0eae87d6c0a084dc5c74b5f5029d920",
  "verified": true
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/124"]);
}

/// `close`/`resolve`/`reopen`/`status` share `lifecycle`, which prints the
/// send object. A workflow status rewrite adds `verified_with_note`.
#[test]
fn tickets_close_verified_with_note() {
    let r = run_fixture(
        "ticket_lifecycle_rewrite",
        &[
            "tickets",
            "close",
            "124",
            "--yes",
            "--expect-plan",
            "809b34350b79011c765c2372918154b8d0eae87d6c0a084dc5c74b5f5029d920",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.err, "");
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 124,
  "status_label": "Closed",
  "plan_hash": "809b34350b79011c765c2372918154b8d0eae87d6c0a084dc5c74b5f5029d920",
  "verified": true,
  "verified_with_note": [
    "status rewritten server-side to 5 after the write (workflow)"
  ]
}
"#
    );
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
        "status": 89
      }
    }
  ],
  "plan_hash": "0086ebfa6080639b7fff523a8a8d8f7397978ec643ac0c8441cc6526ab555466",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."
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
        "status": 31
      }
    }
  ],
  "plan_hash": "f060bab230b722d25b590adcaa8bcb7184b6dd39464f15ef6e72497136192564",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."
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
            "f060bab230b722d25b590adcaa8bcb7184b6dd39464f15ef6e72497136192564",
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
  "plan_hash": "f060bab230b722d25b590adcaa8bcb7184b6dd39464f15ef6e72497136192564",
  "verified": true
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
        "status": 47
      }
    }
  ],
  "plan_hash": "2ce0f9888d30450783860d343bc6a93d1a1bfe22e8efc1e64a646d1528898509",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people."
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
        r#"status must identify one visible runtime status label
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
            "7c2da2bb62f2da4b14c11139ec37af3b5b224fd2168787ab821ee3fc35eed3c1",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"transition readback mismatch (status is still 62); outcome unknown; do not resend
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
  "plan_hash": "8a9efa8c3d1d67c739cdbdd1ae3a377d90f1325a1da54a9d6e9d3c976560b98e",
  "warning": "Irreversible: secondary tickets are closed and their conversations move to the surviving primary. Tenant workflows may notify people. Unknown outcomes must not be resent.",
  "surviving_ticket": 123,
  "merged_tickets": [
    124,
    125
  ]
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
  "plan_hash": "53d303e9a5422166146cee2214b8f71280a04b5fa3fd7c664f786ee332835449",
  "warning": "Irreversible: secondary tickets are closed and their conversations move to the surviving primary. Tenant workflows may notify people. Unknown outcomes must not be resent.",
  "surviving_ticket": 123,
  "merged_tickets": [
    124
  ]
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
            "8a9efa8c3d1d67c739cdbdd1ae3a377d90f1325a1da54a9d6e9d3c976560b98e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8a9efa8c3d1d67c739cdbdd1ae3a377d90f1325a1da54a9d6e9d3c976560b98e",
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
  "readback_attempts": 1
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
            "8a9efa8c3d1d67c739cdbdd1ae3a377d90f1325a1da54a9d6e9d3c976560b98e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8a9efa8c3d1d67c739cdbdd1ae3a377d90f1325a1da54a9d6e9d3c976560b98e",
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
  "warning": "Readback was not fully verified; do not resend."
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
            "8a9efa8c3d1d67c739cdbdd1ae3a377d90f1325a1da54a9d6e9d3c976560b98e",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"write outcome unknown; inspect target and journal before retry
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
  "plan_hash": "d067767325ce33d846f2738d5792b5a43755127f74df9e2dd6c3caec9484dc4d",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "before": [
    {
      "group_id": 81,
      "responder_id": 71,
      "ticket_id": 123
    }
  ]
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
            "d067767325ce33d846f2738d5792b5a43755127f74df9e2dd6c3caec9484dc4d",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d067767325ce33d846f2738d5792b5a43755127f74df9e2dd6c3caec9484dc4d",
  "verified": true,
  "tickets": {
    "123": {
      "responder_id": "matched"
    }
  },
  "readback_attempts": 1
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
  "plan_hash": "1594a3873b1e7eec0c528867478413fc483cff9c1924e7f03fa6abca6874da36",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "before": [
    {
      "group_id": 81,
      "responder_id": null,
      "ticket_id": 124
    }
  ]
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
            "1594a3873b1e7eec0c528867478413fc483cff9c1924e7f03fa6abca6874da36",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1594a3873b1e7eec0c528867478413fc483cff9c1924e7f03fa6abca6874da36",
  "verified": true,
  "tickets": {
    "124": {
      "responder_id": "matched"
    }
  },
  "readback_attempts": 1
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
  "plan_hash": "71a03c4d518b01765c2b0bd514c68a98847059282ac9e77c191d01b0796f2472",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "before": [
    {
      "group_id": 81,
      "responder_id": 71,
      "ticket_id": 123
    }
  ]
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
  "plan_hash": "97c00714dc0acfb737760f7eb5720c989b3f8562a68c1fbb3a326a7cacf1a69f",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
  "before": [
    {
      "group_id": 81,
      "responder_id": 71,
      "ticket_id": 123
    }
  ]
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
            "a056715e758b8fb9ffbe5d466dc32534c48973bc9c76efcbdcee2a256bd05007",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "a056715e758b8fb9ffbe5d466dc32534c48973bc9c76efcbdcee2a256bd05007",
  "verified": false,
  "tickets": {
    "126": {
      "responder_id": "mismatched"
    }
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "59754f3bdb37289d5ca03cc2b2be757004067bc185091e0362f631d8c2805b79",
  "warning": "Assignment can notify people. Inspect each ticket after an unknown or partial outcome; never resend automatically.",
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
  ]
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
            "59754f3bdb37289d5ca03cc2b2be757004067bc185091e0362f631d8c2805b79",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "59754f3bdb37289d5ca03cc2b2be757004067bc185091e0362f631d8c2805b79",
  "verified": true,
  "tickets": {
    "124": {
      "responder_id": "matched"
    },
    "125": {
      "responder_id": "matched"
    }
  },
  "readback_attempts": 1
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
      "status": 89
    },
    "translate_reply": false
  },
  "status_label": "Closed",
  "plan_hash": "be11dde146d1a83e4e7d87c7d7ac698f52db9fbc48cf08fd06d3c91f8b13d69d",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."
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
        r#"only the Pending status transition was captured; use the web form for other statuses
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
      "status": 89
    },
    "notify_emails": [],
    "private": true
  },
  "status_label": "Closed",
  "plan_hash": "ab4d38dd09e8d0835893191df8d134811e8b22643d72da2af02984255b463b7b",
  "warning": "Replies and forwards send mail; notes may trigger tenant workflows. Explicit sender addresses are user-asserted, not verified mailbox configuration."
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
        r#"only the Pending status transition was captured; use the web form for other statuses
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
        r#"only the Pending status transition was captured; use the web form for other statuses
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
            "e323d17f803276921d22db41c0f3b45bf7accd93a8bacf88b30f0c8f927f80f6",
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
  "plan_hash": "e323d17f803276921d22db41c0f3b45bf7accd93a8bacf88b30f0c8f927f80f6",
  "risks": [],
  "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications."
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
            "cb78404c474288d741d0285b54c6e2425be0feb24cc0c206737f533d5f6f376d",
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
  "plan_hash": "cb78404c474288d741d0285b54c6e2425be0feb24cc0c206737f533d5f6f376d",
  "risks": [],
  "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications."
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
            "e323d17f803276921d22db41c0f3b45bf7accd93a8bacf88b30f0c8f927f80f6",
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
  "plan_hash": "e323d17f803276921d22db41c0f3b45bf7accd93a8bacf88b30f0c8f927f80f6",
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
  }
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
  "plan_hash": "e323d17f803276921d22db41c0f3b45bf7accd93a8bacf88b30f0c8f927f80f6",
  "risks": [],
  "warning": "Not a reply. Empty notify_emails does not disable tenant workflows or guarantee no notifications."
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
  "plan_hash": "707f83252578e90aa650ea3b33aa094594b8d770801a9df450f5f7fd56ac1342",
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
  }
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
            "707f83252578e90aa650ea3b33aa094594b8d770801a9df450f5f7fd56ac1342",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "707f83252578e90aa650ea3b33aa094594b8d770801a9df450f5f7fd56ac1342",
  "verified": true,
  "conversation_id": 501,
  "history_complete": true,
  "body": "matched"
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
            "948873906d5e0dc9eaa2d561ddd56bbd8411e491e182f287765479f2420282e8",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "948873906d5e0dc9eaa2d561ddd56bbd8411e491e182f287765479f2420282e8",
  "verified": false,
  "conversation_id": 503,
  "history_complete": true,
  "body": "mismatched",
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "816dcf228d810ace5208834b67d161bbb03529168d3056a4556bc5541497cfdc",
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
  }
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
            "816dcf228d810ace5208834b67d161bbb03529168d3056a4556bc5541497cfdc",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "816dcf228d810ace5208834b67d161bbb03529168d3056a4556bc5541497cfdc",
  "verified": true,
  "conversation_id": 502,
  "history_complete": true
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
  "plan_hash": "1313ae1a94c98a148989cf0be31123d248a904eb1d92b8d60c2a47eb34543ea9",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "problem_id": 7,
  "incident_id": "INC-123"
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
            "1313ae1a94c98a148989cf0be31123d248a904eb1d92b8d60c2a47eb34543ea9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1313ae1a94c98a148989cf0be31123d248a904eb1d92b8d60c2a47eb34543ea9",
  "verified": true,
  "problem_id": 7,
  "incident_id": 123,
  "linked": true
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
            "47f71f08a282d72b8d62300d8d4b802d5ef4eab942e4e873419ae6b5bef9a7e5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "47f71f08a282d72b8d62300d8d4b802d5ef4eab942e4e873419ae6b5bef9a7e5",
  "verified": false,
  "problem_id": 9,
  "incident_id": 123,
  "linked": false,
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "63220f128cd7d0c5dc67c3a9ccb42a8c8e2f22fc40530dc7d2c0911996ed4c5d",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "problem_id": 8,
  "incident_id": "INC-124"
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
            "63220f128cd7d0c5dc67c3a9ccb42a8c8e2f22fc40530dc7d2c0911996ed4c5d",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "63220f128cd7d0c5dc67c3a9ccb42a8c8e2f22fc40530dc7d2c0911996ed4c5d",
  "verified": true,
  "problem_id": 8,
  "incident_id": 124,
  "linked": false
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
  "plan_hash": "28ccd8db0856f97d127a6d82e4fec3fa7e9bb028bbb57edd6edffc670af5b729",
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
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values."
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
            "28ccd8db0856f97d127a6d82e4fec3fa7e9bb028bbb57edd6edffc670af5b729",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "28ccd8db0856f97d127a6d82e4fec3fa7e9bb028bbb57edd6edffc670af5b729",
  "verified": true,
  "problem_id": 7,
  "readback": {
    "status": 2,
    "planned_start_date": "2030-01-02T00:00:00Z",
    "planned_end_date": "2030-01-03T00:00:00Z"
  },
  "unavailable": []
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/7"]);
}

#[test]
fn problems_status_verified_with_note() {
    let r = run_fixture(
        "problem_status_rewrite",
        &[
            "problems",
            "status",
            "PRB-7",
            "--to",
            "Change Requested",
            "--yes",
            "--expect-plan",
            "28ccd8db0856f97d127a6d82e4fec3fa7e9bb028bbb57edd6edffc670af5b729",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.err, "");
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "28ccd8db0856f97d127a6d82e4fec3fa7e9bb028bbb57edd6edffc670af5b729",
  "verified": true,
  "problem_id": 7,
  "readback": {
    "status": 9,
    "planned_start_date": "2030-01-02T00:00:00Z",
    "planned_end_date": "2030-01-03T00:00:00Z"
  },
  "unavailable": [],
  "verified_with_note": "status rewritten server-side to 9 after the write (workflow)"
}
"#
    );
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
  "plan_hash": "2e3389e436dc56373dfab815125e119b35c9da3c85bcda2c57aa49d0e62d79b9",
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
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values."
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
            "2e3389e436dc56373dfab815125e119b35c9da3c85bcda2c57aa49d0e62d79b9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2e3389e436dc56373dfab815125e119b35c9da3c85bcda2c57aa49d0e62d79b9",
  "verified": true,
  "problem_id": 8,
  "readback": {
    "status": 3,
    "planned_start_date": null,
    "planned_end_date": null
  },
  "unavailable": []
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
            "2e3389e436dc56373dfab815125e119b35c9da3c85bcda2c57aa49d0e62d79b9",
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
        r#"status must match one active runtime task-status ID or label
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
            "47d0d3046b2d2965292733d0b0bc8e030b2dc88af977c7144d202b6297c5e587",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "47d0d3046b2d2965292733d0b0bc8e030b2dc88af977c7144d202b6297c5e587",
  "verified": false,
  "problem_id": 11,
  "readback": {
    "status": 1,
    "planned_start_date": null,
    "planned_end_date": null
  },
  "unavailable": [],
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "7f055221c7526683c96032d787164d89d2ec00e4a299c296258066f9470e7bd4",
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
  "side_effects": "Tenant workflows may react to planning changes."
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
            "7f055221c7526683c96032d787164d89d2ec00e4a299c296258066f9470e7bd4",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7f055221c7526683c96032d787164d89d2ec00e4a299c296258066f9470e7bd4",
  "verified": true,
  "problem_id": 71,
  "fields": {
    "planned_effort": "matched",
    "planned_end_date": "matched",
    "planned_start_date": "matched"
  }
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
            "b0017b921cd5cae4f4c4a882d384729fa20177b302c1529e9aadc4b00134626f",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "b0017b921cd5cae4f4c4a882d384729fa20177b302c1529e9aadc4b00134626f",
  "verified": false,
  "problem_id": 76,
  "fields": {
    "planned_effort": "mismatched"
  },
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "850aebf59aca79bb9cdf94424c45f6c431c86e4b720dc2edecca4b6295c7b292",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "before": {
    "planned_effort": null
  },
  "after": {
    "planned_effort": "2h 0m"
  },
  "side_effects": "Tenant workflows may react to planning changes.",
  "warnings": [
    "rules unavailable (the workspace-less path answered HTTP 500 (server error)): conditional mandates, field locks, choice limits and validation messages of the web form were not checked; the server still enforces the static required fields and its own closure rules"
  ]
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
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "PRB-74",
  "body": {
    "status": 2,
    "custom_fields": {
      "cf_owner_note": "Acme new owner"
    }
  },
  "requires_confirmation": [
    "problem status -> Change Requested (status semantics and workflows require confirmation)"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
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
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 74,
  "human_display_id": "PRB-74",
  "subject": "Acme problem 74",
  "status": 2,
  "deleted": false,
  "planned_start_date": null,
  "planned_end_date": null,
  "planned_effort": null,
  "custom_fields": {
    "cf_owner_note": "Acme new owner"
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/74"]);
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
        r#"error: PRB-72 is deleted (in trash); restore it before updating
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
    assert_eq!(r.code, 5, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: HTTP 405 updating problem: PRB-73 is not editable (deleted or in trash?); nothing was changed
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/73"]);
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
  "plan_hash": "adc6cd21127a0c145263da19ed8a9908920811d35ee02a0714456b9aedcc280b",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "warnings": [
    "rules unavailable (the workspace-less path answered HTTP 500 (server error)): conditional mandates, field locks, choice limits and validation messages of the web form were not checked; the server still enforces the static required fields and its own closure rules"
  ]
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
            "adc6cd21127a0c145263da19ed8a9908920811d35ee02a0714456b9aedcc280b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "adc6cd21127a0c145263da19ed8a9908920811d35ee02a0714456b9aedcc280b",
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
  }
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
  "plan_hash": "70a9d47607069930b1f910c99fd2f99bc4dd047f09fc26615c60431456d44e60",
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
  "warnings": [
    "rules unavailable (the workspace-less path answered HTTP 500 (server error)): conditional mandates, field locks, choice limits and validation messages of the web form were not checked; the server still enforces the static required fields and its own closure rules"
  ]
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
  "plan_hash": "1dd665c08f4fe0f2e426028685bf2ff7708eb86c5556004d5beeab83057496c8",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "1dd665c08f4fe0f2e426028685bf2ff7708eb86c5556004d5beeab83057496c8",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1dd665c08f4fe0f2e426028685bf2ff7708eb86c5556004d5beeab83057496c8",
  "verified": true,
  "task_id": 902,
  "fields": {
    "custom_fields": "matched",
    "notify_before": "matched",
    "status": "matched",
    "title": "matched"
  }
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
  "plan_hash": "5a9cf60ef1a6fe9d64467d3dcfd67c93d8a1f5623cb601c076f22312b76ca493",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "8b7801ca1b5d8dc5c32a09ab5649e049e109b6cabf820b278ffba5022b211053",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "8b7801ca1b5d8dc5c32a09ab5649e049e109b6cabf820b278ffba5022b211053",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8b7801ca1b5d8dc5c32a09ab5649e049e109b6cabf820b278ffba5022b211053",
  "verified": true,
  "task_id": 903,
  "fields": {
    "status": "matched"
  }
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
        r#"status must match one active runtime task-status ID or label
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
            "71f4b2b9bb701125b126eca4ffe16589fe109e2837872f4ff9e5b8fa90691520",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "71f4b2b9bb701125b126eca4ffe16589fe109e2837872f4ff9e5b8fa90691520",
  "verified": false,
  "task_id": 906,
  "fields": {
    "status": "mismatched"
  },
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "26affb7e686e7643871d6b0f487e23cf224133be2dad8e2c87981d630983117e",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "26affb7e686e7643871d6b0f487e23cf224133be2dad8e2c87981d630983117e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "26affb7e686e7643871d6b0f487e23cf224133be2dad8e2c87981d630983117e",
  "verified": true,
  "task_id": 904
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
  "plan_hash": "cb1435d6e3ed29a07ad18ae5a6ce0bdf3d89acf17a07636002a5dd41b9025f09",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "cb1435d6e3ed29a07ad18ae5a6ce0bdf3d89acf17a07636002a5dd41b9025f09",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "cb1435d6e3ed29a07ad18ae5a6ce0bdf3d89acf17a07636002a5dd41b9025f09",
  "verified": true,
  "task_id": 911,
  "fields": {
    "custom_fields": "matched",
    "group_id": "matched",
    "notify_before": "matched",
    "owner_id": "matched",
    "status": "matched",
    "title": "matched"
  }
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
  "plan_hash": "8073b6861a37ce2d25a683279e2cc0854b610d66b31929d4b67ea103b96d131d",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "8073b6861a37ce2d25a683279e2cc0854b610d66b31929d4b67ea103b96d131d",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8073b6861a37ce2d25a683279e2cc0854b610d66b31929d4b67ea103b96d131d",
  "verified": true,
  "task_id": 912,
  "fields": {
    "owner_id": "matched",
    "title": "matched"
  }
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
search
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
        r#"error: no unique runtime choice 'Case' for Type
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
        r#"error: no unique runtime choice 'Retired' for Type
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
        r#"error: the tickets list rejected the filter (search_error) and returned unfiltered rows; nothing shown
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
  "plan_hash": "34994e2af399fea67e7d2834cd8d3385ab3e6bc3bf277ce91d069910e64b8454",
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
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments"
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
            "34994e2af399fea67e7d2834cd8d3385ab3e6bc3bf277ce91d069910e64b8454",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "34994e2af399fea67e7d2834cd8d3385ab3e6bc3bf277ce91d069910e64b8454",
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
  "warning": "Independent owning-record readback not fully verified; do not replay."
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
  "plan_hash": "ff74547927b7cb83ed7bb20d450c70b1b7e2844487f2dce138b78930f11f8999",
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
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments"
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
            "ff74547927b7cb83ed7bb20d450c70b1b7e2844487f2dce138b78930f11f8999",
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
  "plan_hash": "d0d4b376bacaacd0588a4f3a664d621f4edfecde32c03d9877242869dcb27070",
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
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments"
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
  "plan_hash": "50d2e899e389f69d2c04fd7ad1ccac15d9f1ff4d019653980985f27a31ec6802",
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
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments"
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
            "50d2e899e389f69d2c04fd7ad1ccac15d9f1ff4d019653980985f27a31ec6802",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "50d2e899e389f69d2c04fd7ad1ccac15d9f1ff4d019653980985f27a31ec6802",
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
  "warning": "Independent owning-record readback not fully verified; do not replay."
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
  "plan_hash": "1d9f7771d1f923614728661b050bd5f73adadc40360182c42880543fa7a0d3f1",
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
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments"
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
            "1d9f7771d1f923614728661b050bd5f73adadc40360182c42880543fa7a0d3f1",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1d9f7771d1f923614728661b050bd5f73adadc40360182c42880543fa7a0d3f1",
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
  }
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
  "plan_hash": "b94526fc24eee0a097e1471298a40ae8e457df4ce54d8ede1a242a1fad906440",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "owner": "tickets/5",
  "child": null,
  "section": null
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
            "ec9e611d62683d7924817e440437e392cc74f4e9e0d11525910e6c3555ae0f3a",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ec9e611d62683d7924817e440437e392cc74f4e9e0d11525910e6c3555ae0f3a",
  "verified": true,
  "attachment_id": 62
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/attachments/62"]);
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
            "b543497e0ec2e43bcdcf810686166714f25a5accfd7f8ee13a4badbdeaa831d4",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "b543497e0ec2e43bcdcf810686166714f25a5accfd7f8ee13a4badbdeaa831d4",
  "verified": false,
  "attachment_id": 63,
  "warning": "Readback was not fully verified; do not resend."
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["DELETE /api/_/attachments/63"]);
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
        r#"review with --dry-run, then --yes --expect-plan FULL_HASH
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
  "plan_hash": "37ddd3cb976012a5d355d002d938adf14814b46021e43fad797d8e0afe624847",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "owner": "tickets/5",
  "child": 950,
  "section": null
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
  "plan_hash": "3b2900d5c89c79c1e08028d062d7e6d0c7fcb8258a6ec0030c8e6652431d2d97",
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
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments"
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
            "3b2900d5c89c79c1e08028d062d7e6d0c7fcb8258a6ec0030c8e6652431d2d97",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "3b2900d5c89c79c1e08028d062d7e6d0c7fcb8258a6ec0030c8e6652431d2d97",
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
  "warning": "Independent owning-record readback not fully verified; do not replay."
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
            "74211ef8d2058797710625fd916d8b3bcccd351a39bed21e6877b0caadaab170",
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
  "plan_hash": "74211ef8d2058797710625fd916d8b3bcccd351a39bed21e6877b0caadaab170",
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
  }
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
            "448038f3a4c50b28a4639d947ddb390840d7057d1232ff1b9db4c569dbef1983",
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
  "plan_hash": "448038f3a4c50b28a4639d947ddb390840d7057d1232ff1b9db4c569dbef1983",
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
  }
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
  "plan_hash": "af56502cf00bd624eafd8c819fcae7a448a378b50149ed7f1515f271dfe7e9f2",
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
  "upload": "POST /api/_/attachments, JSON base64 data URI, once per file; returned IDs become owning payload attachments"
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
            "af56502cf00bd624eafd8c819fcae7a448a378b50149ed7f1515f271dfe7e9f2",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "af56502cf00bd624eafd8c819fcae7a448a378b50149ed7f1515f271dfe7e9f2",
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
  "warning": "Independent owning-record readback not fully verified; do not replay."
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
  "plan_hash": "d19e0c8c13b1527ecd0172384defc5bfdd140554dfef9af321df2e5bcd9e3dd7",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "0618c43448bada19366f3f2275b1f0cf575c2c265292786c8ea1da18e2380b43",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
}
"#
    );
    assert_eq!(r.err, r#""#);
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
            "6430186e53f16ea8b10248de884c4e2c0b4f057c79ff6f443e7dc34fa9d82ff9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "6430186e53f16ea8b10248de884c4e2c0b4f057c79ff6f443e7dc34fa9d82ff9",
  "verified": false,
  "readback": [
    {
      "item_id": 703,
      "itam_cmdb_request_id": 9003,
      "name": "Acme db",
      "category": "Device"
    }
  ],
  "warning": "Readback was not fully verified; do not resend."
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
  "complete": true
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
  "complete": true
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
  "complete": true
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
        r#"no legacy asset association for this module
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
  "plan_hash": "bc387ef9c51c23eed5bba351a9b635e3287bcc4137ae6d5a2a0be223b1fdc5a8",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "d19e0c8c13b1527ecd0172384defc5bfdd140554dfef9af321df2e5bcd9e3dd7",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d19e0c8c13b1527ecd0172384defc5bfdd140554dfef9af321df2e5bcd9e3dd7",
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
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/123/associate-ci"]);
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
            "0618c43448bada19366f3f2275b1f0cf575c2c265292786c8ea1da18e2380b43",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "0618c43448bada19366f3f2275b1f0cf575c2c265292786c8ea1da18e2380b43",
  "verified": true,
  "readback": []
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/7/disassociate-ci"]);
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
  "plan_hash": "75b567e5683325df96708d71a0e88cdb2cded3002d885521564e994ebd9e7a5f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
}
"#
    );
    assert_eq!(r.err, r#""#);
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
  "plan_hash": "8b92b7dca6c615da81c833754e4b2e3645c3f5a0a7b8c66d26b0ec786d15d39a",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
}
"#
    );
    assert_eq!(r.err, r#""#);
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
  "plan_hash": "bb388f5ae1f18bd48fc73381749dd847d8068182379de2f7c9ea3031338d7025",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "3d6e267ff46e2d5703a73fe6710982d3357f58f0e05388b53963f25163fb61fc",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "3d6e267ff46e2d5703a73fe6710982d3357f58f0e05388b53963f25163fb61fc",
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
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "916110e317f45641553de1847f1942866f64325835be99d9593013f01e3d77dd",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted."
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
            "916110e317f45641553de1847f1942866f64325835be99d9593013f01e3d77dd",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "916110e317f45641553de1847f1942866f64325835be99d9593013f01e3d77dd",
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
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant"
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
  "plan_hash": "916110e317f45641553de1847f1942866f64325835be99d9593013f01e3d77dd",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted."
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
  "plan_hash": "916110e317f45641553de1847f1942866f64325835be99d9593013f01e3d77dd",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted."
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
  "plan_hash": "916110e317f45641553de1847f1942866f64325835be99d9593013f01e3d77dd",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    9
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted."
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
  "plan_hash": "7b3ef6dac6f92c414b23ee7774b47d87f96a7393aa5fd2c6c0f977c836641ea5",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "approval_id": 501,
  "operation": "remind",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted."
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
            "7b3ef6dac6f92c414b23ee7774b47d87f96a7393aa5fd2c6c0f977c836641ea5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7b3ef6dac6f92c414b23ee7774b47d87f96a7393aa5fd2c6c0f977c836641ea5",
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
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant"
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
        r#"{
  "model": "legacy",
  "note": "this tenant uses legacy flat approvals (approval-chains answered HTTP 400)",
  "approvals": [
    {
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
      "last_reminded_at": null
    }
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
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
  "plan_hash": "7c7e0d7275ea20e44a54f90a013ac20c47559bd1e6c3fc45f6c69a5aa8d02f68",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "operation": "request",
  "recipient_sources": [],
  "recipients": [
    34
  ],
  "rule": "everyone",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted."
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
            "7c7e0d7275ea20e44a54f90a013ac20c47559bd1e6c3fc45f6c69a5aa8d02f68",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7c7e0d7275ea20e44a54f90a013ac20c47559bd1e6c3fc45f6c69a5aa8d02f68",
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
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant"
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
  "plan_hash": "bdb9974790129a90841ad2654ac206ba7877ee5e134bbdd9c19dcd919e17d763",
  "warning": "Approval emails are irreversible and cannot be recalled (no legacy cancel). Single-attempt write; an unknown outcome must be inspected, never resent.",
  "driver": "legacy",
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant",
  "approval_id": 601,
  "operation": "remind",
  "verification": "Independent read of the flat approvals list only; the write response is not trusted."
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
            "bdb9974790129a90841ad2654ac206ba7877ee5e134bbdd9c19dcd919e17d763",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "bdb9974790129a90841ad2654ac206ba7877ee5e134bbdd9c19dcd919e17d763",
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
  "provenance": "legacy flat approvals driver: capture-based, not yet live-proven on any tenant"
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
            "75b567e5683325df96708d71a0e88cdb2cded3002d885521564e994ebd9e7a5f",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "75b567e5683325df96708d71a0e88cdb2cded3002d885521564e994ebd9e7a5f",
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
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/131/approval-groups"]);
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
            "8b92b7dca6c615da81c833754e4b2e3645c3f5a0a7b8c66d26b0ec786d15d39a",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8b92b7dca6c615da81c833754e4b2e3645c3f5a0a7b8c66d26b0ec786d15d39a",
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
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/132/approvals/401"]);
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
  "plan_hash": "ce6bc022fd3be3532758ca1bf35d746d3bb409c40c25237e8f9c876551a85e85",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "ce6bc022fd3be3532758ca1bf35d746d3bb409c40c25237e8f9c876551a85e85",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ce6bc022fd3be3532758ca1bf35d746d3bb409c40c25237e8f9c876551a85e85",
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
  }
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
  "plan_hash": "54ed51c5346fe2c31325f3d59f52dbcff45f44a4fe76d81d3b82bf01b95c13d6",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "d9f646dfe17d0790df3f29e696ba7f9d57dc46cdb12c52d654bf2de1382e7dc0",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "d9f646dfe17d0790df3f29e696ba7f9d57dc46cdb12c52d654bf2de1382e7dc0",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "d9f646dfe17d0790df3f29e696ba7f9d57dc46cdb12c52d654bf2de1382e7dc0",
  "verified": true,
  "readback": [
    {
      "id": 502,
      "article_type": 1
    }
  ]
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
            "57eadafdf38f689f9a03c578b6654af28558a515992969206a66d2959f85c2cf",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "57eadafdf38f689f9a03c578b6654af28558a515992969206a66d2959f85c2cf",
  "verified": false,
  "readback": [
    {
      "id": 504,
      "article_type": 2
    }
  ],
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "fcf5f251d39674b59bd9ad93c1b637730611df92eb13a4e2c1f0bdf8b16f9b4c",
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
  ]
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
  "plan_hash": "722d0b5d90e5c1645a9571562baa4fb99ce4e1e76beaec8ef27179118bdccd4d",
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
  ]
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
            "957cac4c29003866bda027ba605d9d42b6773d8f8888bf4d7c9e9d440b0b62c4",
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
fn tickets_bulk_unspam_stable_dry_run() {
    let r = run_fixture(
        "ticket_bulk",
        &["tickets", "bulk", "unspam", "146", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}\n{}", r.err, r.out);
    assert_eq!(r.err, "Selected ticket IDs: 146\n  146: Acme ticket 146\n");
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_bulk_unspam",
  "method": "POST",
  "path": "/api/_/tickets/unspam_multiple",
  "payload": {
    "ids": [
      146
    ]
  },
  "plan_hash": "29bef3a1d82e0c047b37a8fcdc180fa4f129bf14fb694b130ea4917d8e9c7d33",
  "warning": "Frozen explicit IDs, not server select-all. Partial outcomes are possible; never replay an uncertain bulk action.",
  "ticket_ids": [
    146
  ],
  "selection": {
    "action": "unspam",
    "agent": null,
    "group": null,
    "ids": [
      "146"
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
      "id": 146,
      "subject": "Acme ticket 146"
    }
  ]
}
"#
    );
}

#[test]
fn tickets_bulk_unspam_send_reports_selection() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "unspam",
            "146",
            "--yes",
            "--expect-plan",
            "29bef3a1d82e0c047b37a8fcdc180fa4f129bf14fb694b130ea4917d8e9c7d33",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}\n{}", r.err, r.out);
    assert_eq!(r.err, "Selected ticket IDs: 146\n  146: Acme ticket 146\n");
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "29bef3a1d82e0c047b37a8fcdc180fa4f129bf14fb694b130ea4917d8e9c7d33",
  "verified": true,
  "ticket_ids": [
    146
  ],
  "tickets": {
    "146": {
      "spam": "matched"
    }
  },
  "readback_attempts": 1,
  "delegated_plan_hash": "4d96c858add3673ca018005931b31266669e47f01bc735ab76cbb0ae0b2b8522",
  "selected_tickets": [
    {
      "id": 146,
      "subject": "Acme ticket 146"
    }
  ]
}
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
            "824c00188985d037ce4270dadb70f456dc9ed504d6d4abfc455926278a16dd98",
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
            "0e7b2d024f187c3d8ba57133c5fa23c1597334177cfd98e7db6ef09cce23b6c4",
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
            "0e7b2d024f187c3d8ba57133c5fa23c1597334177cfd98e7db6ef09cce23b6c4",
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
  "plan_hash": "b1d3d957a7b264afc37f0b7cd54248a58bb5124add62efa98bb8de8fa1e112a3",
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
  ]
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
        r#"no list filter exists for text field 'Subject': the web's filter panels and builders offer no subject/description/text condition, so nothing was sent; use `fsv tickets search TEXT` for a keyword search
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
  "plan_hash": "9017bd98ac0c051db0977786c203f5a9b82e517982c9e78693f4107a8e295a58",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "9017bd98ac0c051db0977786c203f5a9b82e517982c9e78693f4107a8e295a58",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "9017bd98ac0c051db0977786c203f5a9b82e517982c9e78693f4107a8e295a58",
  "verified": true,
  "item_id": 71
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
            "e62edf6bdea38f1a0194c2bfd2bf2f359f226f5c82bc7e27a6365933784b642b",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e62edf6bdea38f1a0194c2bfd2bf2f359f226f5c82bc7e27a6365933784b642b",
  "verified": false,
  "item_id": 72,
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "1a2c8f45fff87e611aa4c22a364497342736335252c1a452adfba6143c3043fe",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "b8fadba0aff12fd2769fbe4cf49890000ef850096b9598dd9a67b4ceb9aa2167",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "fbb245f6355b51da76f47aad1324b88f91b2d4886b1a85fef85d8f2d2840f730",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "672af9e5da98ac2788eae3a8849fe2aed9562537842f4af6c43895787aae8688",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "672af9e5da98ac2788eae3a8849fe2aed9562537842f4af6c43895787aae8688",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "672af9e5da98ac2788eae3a8849fe2aed9562537842f4af6c43895787aae8688",
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
  }
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
            "c8fec8833e1055af4ca3d6c6df1da389d65f636693f5d6fc4af396beb5526adf",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c8fec8833e1055af4ca3d6c6df1da389d65f636693f5d6fc4af396beb5526adf",
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
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "2dfb5e2a95f62c9ee3f974d3f5098489105f6e068a9e1622d3d193651789f43a",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
  "plan_hash": "6b17d664cd2c7c1a1e01f6625ee4e9d78e8519c57df945940e17d2572a7337b9",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
        r#"status must match one active runtime task-status ID or label
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
  "plan_hash": "f67db594bc5d443c91f2e42203a723659a171395d40fd7f85f8e757c7db3993b",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "f67db594bc5d443c91f2e42203a723659a171395d40fd7f85f8e757c7db3993b",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "f67db594bc5d443c91f2e42203a723659a171395d40fd7f85f8e757c7db3993b",
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
  ]
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
            "f0f13f08efd1e1e28dfb44b32fa12ee312a58525e2c9e5933e784e69adb1d452",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "f0f13f08efd1e1e28dfb44b32fa12ee312a58525e2c9e5933e784e69adb1d452",
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
  "warning": "Readback was not fully verified; do not resend."
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
  "plan_hash": "dcf6800b8cc153e9260da4b02d90fc82c65378bef6a0c8a3ec43c6703291967a",
  "warning": "Destructive soft delete of one change. Restoration is not a captured fsv contract; unknown outcomes must be inspected, never automatically resent.",
  "change_id": 21,
  "subject": "Acme firewall rollout"
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
            "dcf6800b8cc153e9260da4b02d90fc82c65378bef6a0c8a3ec43c6703291967a",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "dcf6800b8cc153e9260da4b02d90fc82c65378bef6a0c8a3ec43c6703291967a",
  "verified": true,
  "change_id": 21,
  "fields": {
    "deleted": "matched"
  },
  "readback_attempts": 1
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
            "3d40a78de7476dc411b9d65d744699f80f355c5ecb20427826c2b901c897bf01",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "3d40a78de7476dc411b9d65d744699f80f355c5ecb20427826c2b901c897bf01",
  "verified": false,
  "change_id": 22,
  "fields": {
    "deleted": "mismatched"
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend."
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
            "dcf6800b8cc153e9260da4b02d90fc82c65378bef6a0c8a3ec43c6703291967a",
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
    "billable": true,
    "executed_at": "2030-01-02T09:00:00+07:00",
    "note": "Acme deploy",
    "time_spent": "00:45",
    "timer_running": false
  },
  "plan_hash": "e81b967f00f9ee82d7e5c8ec7f63a629737464d126513a44fafeefefe7c60752",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "e81b967f00f9ee82d7e5c8ec7f63a629737464d126513a44fafeefefe7c60752",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e81b967f00f9ee82d7e5c8ec7f63a629737464d126513a44fafeefefe7c60752",
  "verified": true,
  "entry_id": 71,
  "fields": {
    "agent_id": "matched",
    "billable": "matched",
    "executed_at": "matched",
    "note": "matched",
    "time_spent": "matched",
    "timer_running": "matched"
  }
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
  "plan_hash": "894075e6f67c2b0de0062c4d8bf4ea750306f53b819a8313d6fcaf675cadc31d",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "ab6b6ee293e9c28a0421a030f3fa4a4bf6f05313e85e702816d02b89dd2f301e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "ab6b6ee293e9c28a0421a030f3fa4a4bf6f05313e85e702816d02b89dd2f301e",
  "verified": false,
  "entry_id": 72,
  "fields": {
    "note": "mismatched",
    "time_spent": "matched"
  },
  "warning": "Readback was not fully verified; do not resend."
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
            "7c56146c576256639fef80e94f1999e9d566ec6d4cea4c6442acc3d09a8be45d",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "7c56146c576256639fef80e94f1999e9d566ec6d4cea4c6442acc3d09a8be45d",
  "verified": true,
  "entry_id": 73
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
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"problem time update has no captured accepted wire; only create/delete
"#
    );
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
            "aca7a27a7ae3e4641728942418f981b0f15927bf50b8b91fbc58fbe0376ee0b1",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "aca7a27a7ae3e4641728942418f981b0f15927bf50b8b91fbc58fbe0376ee0b1",
  "verified": true,
  "entry_id": 76
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
  "plan_hash": "90ee4c7cfaec0e4f0dd87c6dd130580dbbb528f7d093234908ba8bf3f10042e5",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible."
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
            "90ee4c7cfaec0e4f0dd87c6dd130580dbbb528f7d093234908ba8bf3f10042e5",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "90ee4c7cfaec0e4f0dd87c6dd130580dbbb528f7d093234908ba8bf3f10042e5",
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
  "warning": "Task completion may rewrite planned dates and stop OLA timers; inspect the exact readback. Never resend."
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
  "plan_hash": "1b47a5e4ca3448a12fe48d1131200ba6c11749bbd7877a89895bfad5ab2ce397",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible."
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
        r#"status must match one active runtime task-status ID or label
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
  "plan_hash": "67b7a68ee8919303da35a9e07eb2a45989bd0f27788167d0406e8e238bcb0f04",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible."
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
            "67b7a68ee8919303da35a9e07eb2a45989bd0f27788167d0406e8e238bcb0f04",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "67b7a68ee8919303da35a9e07eb2a45989bd0f27788167d0406e8e238bcb0f04",
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
  "warning": "Task completion may rewrite planned dates and stop OLA timers; inspect the exact readback. Never resend."
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
  "plan_hash": "a79cff879815c25fbaa17dd9ad5e80a4aa8a64d108c85a7bdebbc694eaa5fdf9",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "side_effects": "Completion may rewrite planned start/end and stop OLA timers; delete is irreversible."
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
            "a79cff879815c25fbaa17dd9ad5e80a4aa8a64d108c85a7bdebbc694eaa5fdf9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "a79cff879815c25fbaa17dd9ad5e80a4aa8a64d108c85a7bdebbc694eaa5fdf9",
  "verified": true,
  "task_id": 821
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
  "plan_hash": "473b858d16c295019e6a2cdb05c33c6a8244744acff57c0a7285e6033be21248",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "473b858d16c295019e6a2cdb05c33c6a8244744acff57c0a7285e6033be21248",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "473b858d16c295019e6a2cdb05c33c6a8244744acff57c0a7285e6033be21248",
  "verified": true,
  "item_id": 1511
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
  "plan_hash": "cd6e3b885a95d84ee6425a5b9aabec6a80f4627141e6a9cc2bc7646d3d3c14a9",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "cd6e3b885a95d84ee6425a5b9aabec6a80f4627141e6a9cc2bc7646d3d3c14a9",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "cd6e3b885a95d84ee6425a5b9aabec6a80f4627141e6a9cc2bc7646d3d3c14a9",
  "verified": false,
  "item_id": 1522,
  "warning": "Readback was not fully verified; do not resend."
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

#[test]
fn tickets_bulk_set_not_status() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "spam",
            "140",
            "--set",
            "note=x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(r.err, "--set is supported by bulk status only\n",);
}

#[test]
fn tickets_bulk_set_missing_eq() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "status",
            "140",
            "--status",
            "Open",
            "--set",
            "note",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(r.err, "--set requires unique FIELD=VALUE answers\n",);
}

#[test]
fn tickets_bulk_set_empty_key() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "status",
            "140",
            "--status",
            "Open",
            "--set",
            "=x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(r.err, "--set requires unique FIELD=VALUE answers\n",);
}

#[test]
fn tickets_bulk_set_duplicate() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "status",
            "140",
            "--status",
            "Open",
            "--set",
            "note=a",
            "--set",
            "note=b",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(r.err, "--set requires unique FIELD=VALUE answers\n",);
}

#[test]
fn tickets_bulk_set_status_key() {
    let r = run_fixture(
        "ticket_bulk",
        &[
            "tickets",
            "bulk",
            "status",
            "140",
            "--status",
            "Open",
            "--set",
            "status=Closed",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        "use --status for the status; --set is for the fields its rules require\n",
    );
}

#[test]
fn tickets_close_blocking_needs_ack() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &["tickets", "close", "124", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        "ticket form validation failed: blocked by rule (acknowledge to continue): Only the requester closes Acme tickets; pass --ack to acknowledge the blocking rule message(s) after reading them\n",
    );
}

#[test]
fn tickets_close_ack_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &["tickets", "close", "124", "--ack", "--dry-run"],
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
        "status": 89
      }
    }
  ],
  "plan_hash": "c68ec0e68a0d55eced743aae14b0721b3c91099269ce2308fb7152f714569c1b",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people. Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them, and the server or a workflow may still reject or revert the transition.",
  "acknowledged_rule_messages": [
    "Only the requester closes Acme tickets"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_status_blocking_needs_ack() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &["tickets", "status", "124", "Closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        "ticket form validation failed: blocked by rule (acknowledge to continue): Only the requester closes Acme tickets; pass --ack to acknowledge the blocking rule message(s) after reading them\n",
    );
}

#[test]
fn tickets_status_ack_dry_run() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &["tickets", "status", "124", "Closed", "--ack", "--dry-run"],
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
        "status": 89
      }
    }
  ],
  "plan_hash": "c68ec0e68a0d55eced743aae14b0721b3c91099269ce2308fb7152f714569c1b",
  "warning": "Resolution save and transition are separate writes, not atomic. Unknown or partial outcomes must not be resent. Tenant workflows may notify people. Acknowledged blocking rule messages are tenant policy: the web form blocks submit on them, and the server or a workflow may still reject or revert the transition.",
  "acknowledged_rule_messages": [
    "Only the requester closes Acme tickets"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_close_ack_send() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "close",
            "124",
            "--ack",
            "--yes",
            "--expect-plan",
            "c68ec0e68a0d55eced743aae14b0721b3c91099269ce2308fb7152f714569c1b",
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
  "plan_hash": "c68ec0e68a0d55eced743aae14b0721b3c91099269ce2308fb7152f714569c1b",
  "verified": true
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/124"]);
}

#[test]
fn tickets_close_acked_hash_without_ack() {
    let r = run_fixture(
        "ticket_lifecycle_ack",
        &[
            "tickets",
            "close",
            "124",
            "--yes",
            "--expect-plan",
            "c68ec0e68a0d55eced743aae14b0721b3c91099269ce2308fb7152f714569c1b",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        "ticket form validation failed: blocked by rule (acknowledge to continue): Only the requester closes Acme tickets; pass --ack to acknowledge the blocking rule message(s) after reading them\n",
    );
}
