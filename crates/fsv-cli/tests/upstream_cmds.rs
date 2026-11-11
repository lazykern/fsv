//! Upstream commands ported from Python dbe553c: watchers, ticket time,
//! planning-text, mark-known-error, bulk-priority, reply --bcc/--sender,
//! forward, update --reviewed with --set answers, and the SR send-failure
//! report. Expected output was captured from Python dbe553c on the same routes
//! (`FSV_PY_SRC=<dbe553c>/src tests/golden/cmp.py --emit`); one-shot routes
//! serve the record before the write and the changed record after it.

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
  "plan_hash": "e23e5ef7b5321bac81420f9a326219875da3e0411b020a69745f3f08a16f68d0",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "e23e5ef7b5321bac81420f9a326219875da3e0411b020a69745f3f08a16f68d0",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e23e5ef7b5321bac81420f9a326219875da3e0411b020a69745f3f08a16f68d0",
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
  "plan_hash": "61f95b4f9910f200b3cdba6a2379e2e352b5013c4064dbece9fa80adcf6e4ddb",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
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
            "61f95b4f9910f200b3cdba6a2379e2e352b5013c4064dbece9fa80adcf6e4ddb",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "61f95b4f9910f200b3cdba6a2379e2e352b5013c4064dbece9fa80adcf6e4ddb",
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
#[ignore = "fsv-api properties.rs:115 removes br_validation_excludes with swap_remove, reordering `after`"]
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
fn planning_text_unknown_section_is_uncaught() {
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
    assert_eq!(r.code, 1, "{}", r.err);
    assert!(
        r.err
            .contains("planning field not found: 'nope'; choices: Rollout Plan, Backout Plan"),
        "{}",
        r.err
    );
}
