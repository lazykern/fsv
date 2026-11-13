//! Ticket trash list, restore and delete-forever (`ticket_trash_cli`).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: 8c4ee36734d8dcb8b6b75e85376add578c2771a5

mod common;
use common::run_fixture;

#[test]
fn tickets_trash_list() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "trash", "list", "--json"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "tickets": [
    {
      "id": 31,
      "human_display_id": "INC-31",
      "subject": "Acme 31",
      "deleted": true,
      "updated_at": "2026-11-01T10:00:00Z"
    },
    {
      "id": 32,
      "human_display_id": "INC-32",
      "subject": "Acme caf\u00e9 32",
      "deleted": true,
      "updated_at": "2026-11-01T11:00:00Z"
    }
  ],
  "page": 1,
  "has_next": false,
  "count": 2,
  "note": "the trash holds every agent's deleted tickets; act on explicit IDs only"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_trash_list_page() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "trash", "list", "--page", "2", "--per-page", "5"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "tickets": [
    {
      "id": 31,
      "human_display_id": "INC-31",
      "subject": "Acme 31",
      "deleted": true,
      "updated_at": "2026-11-01T10:00:00Z"
    },
    {
      "id": 32,
      "human_display_id": "INC-32",
      "subject": "Acme caf\u00e9 32",
      "deleted": true,
      "updated_at": "2026-11-01T11:00:00Z"
    }
  ],
  "page": 2,
  "has_next": false,
  "count": 2,
  "note": "the trash holds every agent's deleted tickets; act on explicit IDs only"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_trash_list_read_failed() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "trash", "list", "--page", "9"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read failed (HTTP 500); nothing was sent
"#
    );
}

#[test]
fn tickets_trash_list_unreadable() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "trash", "list", "--page", "8"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"trash list is unreadable
"#
    );
}

#[test]
fn tickets_restore_dry_run() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "restore", "INC-34", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_restore",
  "method": "PUT",
  "path": "/api/_/tickets/34/restore",
  "payload": null,
  "plan_hash": "337dbce8936ce3d667663f690adeb1788cadcfd8c7ed3dcb4da15f8b6c37a881",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn tickets_restore_not_in_trash() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "restore", "33", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 33 is not in the trash; nothing was sent
"#
    );
}

#[test]
fn tickets_restore_not_found() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "restore", "99", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 99 not found (HTTP 404); nothing was sent
"#
    );
}

#[test]
fn tickets_restore_read_failed() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "restore", "38", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"read failed (HTTP 500); nothing was sent
"#
    );
}

#[test]
fn tickets_restore_flag_unavailable() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "restore", "39", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"ticket 39: deleted flag unavailable; nothing was sent
"#
    );
}

#[test]
fn tickets_restore_needs_expect_plan() {
    let r = run_fixture("ticket_trash", &["tickets", "restore", "34", "--yes"], None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"review with --dry-run, then --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn tickets_restore_send() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "restore",
            "34",
            "--yes",
            "--expect-plan",
            "337dbce8936ce3d667663f690adeb1788cadcfd8c7ed3dcb4da15f8b6c37a881",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "337dbce8936ce3d667663f690adeb1788cadcfd8c7ed3dcb4da15f8b6c37a881",
  "verified": true,
  "ticket_id": 34,
  "deleted": false
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/34/restore"]);
}

#[test]
fn tickets_restore_send_unverified() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "restore",
            "35",
            "--yes",
            "--expect-plan",
            "e2e9f9fe96e0959b48b2b8cda70a4d4c0511f89b563d31cc6a38e1435ee16dfc",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "e2e9f9fe96e0959b48b2b8cda70a4d4c0511f89b563d31cc6a38e1435ee16dfc",
  "verified": false,
  "ticket_id": 35,
  "deleted": true,
  "warning": "Readback was not fully verified; do not resend."
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/35/restore"]);
}

#[test]
fn tickets_delete_forever_needs_max_records() {
    let r = run_fixture(
        "ticket_trash",
        &["tickets", "delete-forever", "31", "32", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"delete-forever requires --max-records N (1 to 50)
"#
    );
}

#[test]
fn tickets_delete_forever_more_than_max() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "31",
            "32",
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
        r#"more IDs than --max-records; refusing the whole plan
"#
    );
}

#[test]
fn tickets_delete_forever_max_records_range() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "31",
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
        r#"delete-forever requires --max-records N (1 to 50)
"#
    );
}

#[test]
fn tickets_delete_forever_distinct() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "31",
            "INC-31",
            "--max-records",
            "2",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"give one or more distinct explicit ticket IDs
"#
    );
}

#[test]
fn tickets_delete_forever_not_in_trash() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "31",
            "33",
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
        r#"ticket 33 is not in the trash; delete it first; nothing was sent
"#
    );
}

#[test]
fn tickets_delete_forever_dry_run() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "31",
            "INC-32",
            "--max-records",
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
  "action": "ticket_delete_forever",
  "method": "DELETE",
  "requests": [
    "/api/_/tickets/delete_forever?ids=31",
    "/api/_/tickets/delete_forever?ids=32"
  ],
  "ticket_ids": [
    31,
    32
  ],
  "selected_tickets": [
    {
      "id": 31,
      "subject": "Acme 31"
    },
    {
      "id": 32,
      "subject": "Acme 32"
    }
  ],
  "plan_hash": "a9874b072223c505bbcb73505e97260404b392627cc3cccea4e8b8fdde2a95e7",
  "warning": "PERMANENT: the tickets cannot be restored. One request per ticket, in order; stops at the first failed or unverified ticket. Never replay an uncertain outcome."
}
"#
    );
    assert_eq!(
        r.err,
        r#"Tickets to delete PERMANENTLY: 31, 32
  31: Acme 31
  32: Acme 32
"#
    );
}

#[test]
fn tickets_delete_forever_needs_confirm() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "31",
            "INC-32",
            "--max-records",
            "2",
            "--yes",
            "--expect-plan",
            "a9874b072223c505bbcb73505e97260404b392627cc3cccea4e8b8fdde2a95e7",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Tickets to delete PERMANENTLY: 31, 32
  31: Acme 31
  32: Acme 32
delete-forever requires --confirm-destructive after inspecting the printed ID list
"#
    );
}

#[test]
fn tickets_delete_forever_send() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "31",
            "INC-32",
            "--max-records",
            "2",
            "--yes",
            "--expect-plan",
            "a9874b072223c505bbcb73505e97260404b392627cc3cccea4e8b8fdde2a95e7",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "verified": true,
  "plan_hash": "a9874b072223c505bbcb73505e97260404b392627cc3cccea4e8b8fdde2a95e7",
  "ticket_ids": [
    31,
    32
  ],
  "tickets": {
    "31": {
      "verified": true
    },
    "32": {
      "verified": true
    }
  }
}
"#
    );
    assert_eq!(
        r.err,
        r#"Tickets to delete PERMANENTLY: 31, 32
  31: Acme 31
  32: Acme 32
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "DELETE /api/_/tickets/delete_forever",
            "DELETE /api/_/tickets/delete_forever"
        ]
    );
}

#[test]
fn tickets_delete_forever_stops_at_failure() {
    let r = run_fixture(
        "ticket_trash",
        &[
            "tickets",
            "delete-forever",
            "36",
            "37",
            "--max-records",
            "2",
            "--yes",
            "--expect-plan",
            "102950c1f91289607384f7587ddf4f91453e46f51e394a2010c99983322273af",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "failed",
  "verified": false,
  "plan_hash": "102950c1f91289607384f7587ddf4f91453e46f51e394a2010c99983322273af",
  "ticket_ids": [
    36,
    37
  ],
  "tickets": {
    "36": {
      "verified": false,
      "error": "write outcome unknown; inspect target and journal before retry"
    }
  },
  "stopped_at": 36,
  "not_attempted": [
    37
  ],
  "warning": "Stopped at the first failed or unverified ticket; inspect it and the journal; never replay."
}
"#
    );
    assert_eq!(
        r.err,
        r#"Tickets to delete PERMANENTLY: 36, 37
  36: Acme 36
  37: Acme 37
"#
    );
    assert_eq!(r.write_paths(), ["DELETE /api/_/tickets/delete_forever"]);
}
