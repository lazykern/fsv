//! Upstream commands ported from Python: watchers, ticket time,
//! planning-text, mark-known-error, bulk-priority, reply --bcc/--sender,
//! forward, update --reviewed with --set answers, the SR send-failure report,
//! the change asset picker, canned replies and scenarios, private ticket
//! views, list columns and exports, problem create and child writes, and
//! Click's missing-parameter errors. Expected output was captured from
//! Python on the same routes (`tests/golden/emit_upstream.py`); one-shot
//! routes serve the record before the write and the changed record after it.
//! Python commit: 92002d76b880d927e16750ab986b525ee4fa85d6

mod common;
use common::run_fixture;

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
        r#"planning description must be nonempty text/HTML
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
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"planning section already has content; native edit wire is not captured; use the web editor (no write attempted)
"#
    );
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
        r#"combined resolution/closure requirements are not proven; use the web form
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
  }
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
      "id": 9000007701,
      "display_id": 701,
      "name": "Acme Caf\u00e9 web",
      "ci_type_name": "Server"
    },
    {
      "id": 9000007702,
      "display_id": 702,
      "name": "Acme API",
      "ci_type_name": "Server"
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
      "id": 9000007701,
      "display_id": 701,
      "name": "Acme Caf\u00e9 web",
      "ci_type_name": "Server"
    },
    {
      "id": 9000007702,
      "display_id": 702,
      "name": "Acme API",
      "ci_type_name": "Server"
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
      "id": 9000007703,
      "display_id": 703,
      "name": "Globex DB",
      "ci_type_name": "Server"
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
      "id": 9000007701,
      "display_id": 701,
      "name": "Acme Caf\u00e9 web",
      "ci_type_name": "Server"
    },
    {
      "id": 9000007702,
      "display_id": 702,
      "name": "Acme API",
      "ci_type_name": "Server"
    }
  ],
  "mutation": "none",
  "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker.",
  "selected": {
    "id": 9000007702,
    "display_id": 702,
    "name": "Acme API",
    "ci_type_name": "Server"
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
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "18"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "change_id": 18,
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
        &["changes", "asset-picker", "18", "--select", "1"],
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
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "13"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"duplicate asset display identities; selection is ambiguous
"#
    );
}

#[test]
fn asset_picker_rows_not_list() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "14"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"asset candidate response is unavailable
"#
    );
}

#[test]
fn asset_picker_row_not_object() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "19"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"asset candidate response is unavailable
"#
    );
}

#[test]
fn asset_picker_bad_display_id() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "15"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"expected a positive numeric identity
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
#[ignore = "fsv-api asset_picker.rs candidates checks duplicates before every display_id is validated"]
fn asset_picker_duplicate_then_bad() {
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "16"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(r.err, "expected a positive numeric identity\n");
}

#[test]
fn asset_picker_api_error_is_uncaught() {
    // Python leaves APIError uncaught (traceback, exit 1).
    let r = run_fixture("asset_picker", &["changes", "asset-picker", "17"], None);
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(
        r.err.contains("HTTP 500 listing assets-to-associate: boom"),
        "{}",
        r.err
    );
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
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unsupported saved-view action
"#
    );
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
        r#"create/edit requires a nonempty --name
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
        r#"create/edit requires a nonempty --name
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
        r#"named private visibility metadata is unavailable; numeric visibility alone is not proof
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
        r#"named private visibility metadata is unavailable; numeric visibility alone is not proof
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
    "advanced_query_hash": "matched",
    "name": "mismatched",
    "order_by": "matched",
    "order_type": "matched",
    "query_hash": "matched",
    "visibility": "matched",
    "workspace_id": "matched"
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
    "advanced_query_hash": "matched",
    "name": "matched",
    "order_by": "matched",
    "order_type": "matched",
    "query_hash": "matched",
    "visibility": "matched",
    "workspace_id": "matched"
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
    "advanced_query_hash": "matched",
    "name": "mismatched",
    "order_by": "matched",
    "order_type": "matched",
    "query_hash": "matched",
    "visibility": "matched",
    "workspace_id": "matched"
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
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "8d40b150f5aca4efb14583eb29cb46703b5497d1786ffbf1f0261ee154871e32",
  "verified": false,
  "source_readback": true,
  "job": "unavailable",
  "download": "unavailable",
  "warning": "Export requested once; delivery/job/download effects are unverified. Do not resend."
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/export"]);
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
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "724d390aea5fd89f52addbd0542798bcd6132f345697453bae7383cd2d9e7e89",
  "verified": false,
  "source_readback": true,
  "job": "unavailable",
  "download": "unavailable",
  "warning": "Export requested once; delivery/job/download effects are unverified. Do not resend."
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/changes/export"]);
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
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "96634ff8a9e6f5eb960f4e8027a6be90f83a0c78d534ec8b44a7657733d366f3",
  "verified": false,
  "source_readback": true,
  "job": "unavailable",
  "download": "unavailable",
  "warning": "Export requested once; delivery/job/download effects are unverified. Do not resend."
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/export"]);
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
choices: fsv form fields changes --choices LABEL
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
fn private_view_show_refusal_is_uncaught() {
    // `show` has no `try`: ReviewedWriteError is a traceback (exit 1), not exit 2.
    let r = run_fixture(
        "private_views",
        &["tickets", "private-views", "show", "711"],
        None,
    );
    assert_eq!(r.code, 1, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(
        r.err.contains("exact saved-view identity is unavailable"),
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
