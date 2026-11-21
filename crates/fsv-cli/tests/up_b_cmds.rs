// Python commit: 01a88d41e82448c1585f0e9f68e9e03e9b471834
//! Upstream commands ported by the up-b batch: reviewed problem notes,
//! change note hashes, plain reply, legacy ticket update hash, record-state
//! tokens and replay refusals. Emitted by cmp.py from the Python oracle named
//! above (31a9103 plus the up-b commits).

mod common;
#[allow(unused_imports)]
use common::{Opts, run_files, run_fixture, run_steps, run_with};

#[test]
fn problem_note_reviewed_dry_run() {
    let r = run_fixture(
        "problem_note_reviewed",
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
  "status": "dry_run",
  "dry_run": true,
  "action": "problem_note",
  "method": "POST",
  "path": "/api/_/problems/12/notes",
  "payload": {
    "body": "Acme evidence"
  },
  "plan_hash": "169c2ee6ad36e8ae519027838d276715359685f57b234dd51928113c8d86605f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "id": "PRB-12",
  "body": {
    "body": "Acme evidence"
  },
  "record_state": {
    "sends": 0
  },
  "warnings": [
    "visibility: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problem_note_reviewed_send_json() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme evidence",
            "--yes",
            "--expect-plan",
            "169c2ee6ad36e8ae519027838d276715359685f57b234dd51928113c8d86605f",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "169c2ee6ad36e8ae519027838d276715359685f57b234dd51928113c8d86605f",
  "verified": true,
  "note_id": 9000000077,
  "readback": {
    "id": 9000000077,
    "body": "<div>Acme evidence</div>"
  },
  "identity_verified": true,
  "readback_complete": true,
  "fields": {
    "body": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/12/notes"]);
}

#[test]
fn problem_note_reviewed_send_human() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme evidence",
            "--yes",
            "--expect-plan",
            "169c2ee6ad36e8ae519027838d276715359685f57b234dd51928113c8d86605f",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added note 9000000077 on problem 12 (visibility: tenant default; not controllable on problem notes); verified: true
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/problems/12/notes"]);
}

#[test]
fn problem_note_yes_needs_expect_plan() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-12", "Acme", "--yes"],
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
fn problem_note_wrong_hash() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems",
            "add-note",
            "PRB-12",
            "Acme",
            "--yes",
            "--expect-plan",
            "54a7a3f299d0d36426d174d70d8e7a217a84eeadadacd2e4c66ad7465bb54746",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"note review hash does not match; review the exact body again
"#
    );
}

#[test]
fn problem_note_no_input() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-12", "Acme", "--no-input"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"sending requires --yes and --expect-plan in no-input mode
"#
    );
}

#[test]
fn problem_note_public_refused() {
    let r = run_fixture(
        "problem_note_reviewed",
        &[
            "problems", "add-note", "PRB-12", "Acme", "--public", "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--public: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies
"#
    );
}

#[test]
fn problem_note_empty_body() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-12", "  ", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"note body must be nonempty text without NUL characters
"#
    );
}

#[test]
fn problem_note_bad_id() {
    let r = run_fixture(
        "problem_note_reviewed",
        &["problems", "add-note", "PRB-0", "x", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"problem ID must be positive
"#
    );
}

#[test]
fn problem_note_prompt_declined() {
    let r = run_with(
        "problem_note_reviewed",
        &[],
        &["problems", "add-note", "PRB-12", "Acme"],
        Some("n"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Add this exact reviewed note? [y/N]: Cancelled; no note was sent.
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "problem_note",
  "method": "POST",
  "path": "/api/_/problems/12/notes",
  "payload": {
    "body": "Acme"
  },
  "plan_hash": "ca5dab5f02664118bcf7b7715924796b7164cd6f39077e60ce1d38f885539994",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "id": "PRB-12",
  "body": {
    "body": "Acme"
  },
  "record_state": {
    "sends": 0
  },
  "warnings": [
    "visibility: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
}

#[test]
fn problem_note_prompt_accepted() {
    let r = run_with(
        "problem_note_reviewed",
        &[],
        &["problems", "add-note", "PRB-12", "Acme evidence"],
        Some("y"),
        Opts {
            tty: true,
            env: &[],
        },
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"Add this exact reviewed note? [y/N]: added note 9000000077 on problem 12 (visibility: tenant default; not controllable on problem notes); verified: true
"#
    );
    assert_eq!(
        r.err,
        r#"{
  "action": "problem_note",
  "method": "POST",
  "path": "/api/_/problems/12/notes",
  "payload": {
    "body": "Acme evidence"
  },
  "plan_hash": "169c2ee6ad36e8ae519027838d276715359685f57b234dd51928113c8d86605f",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "id": "PRB-12",
  "body": {
    "body": "Acme evidence"
  },
  "record_state": {
    "sends": 0
  },
  "warnings": [
    "visibility: problem notes accept only a body (captured `{body}`; the tenant rejects `private` with HTTP 400): visibility is not controllable and the tenant default applies"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/problems/12/notes"]);
}
#[test]
fn change_note_dry_run_hash() {
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
  },
  "plan_hash": "5fc475abf941a95ac415a091d2d5492d3f9de14aaa5b2d7a8a937215972bd850",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_public_note_dry_run_hash() {
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
  "plan_hash": "3fdd859d0dc1c8801bd04bb99b015b08ee8d0c76128764956f9e98a3d8150f97",
  "requires_confirmation": [
    "public note: visible to the requester and may send notifications"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_note_expect_plan_sends() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--expect-plan",
            "5fc475abf941a95ac415a091d2d5492d3f9de14aaa5b2d7a8a937215972bd850",
        ],
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
fn change_note_expect_plan_uppercase_ok() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--expect-plan",
            "5FC475ABF941A95AC415A091D2D5492D3F9DE14AAA5B2D7A8A937215972BD850",
        ],
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
fn change_note_expect_plan_edited_body() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note (edited)",
            "--expect-plan",
            "5fc475abf941a95ac415a091d2d5492d3f9de14aaa5b2d7a8a937215972bd850",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3 does not match the reviewed plan (body, visibility or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn change_note_expect_plan_visibility() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--public",
            "--yes",
            "--expect-plan",
            "5fc475abf941a95ac415a091d2d5492d3f9de14aaa5b2d7a8a937215972bd850",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3 does not match the reviewed plan (body, visibility or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn change_note_expect_plan_prefix() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "Acme change note",
            "--expect-plan",
            "5fc475abf941a95a",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3 does not match the reviewed plan (body, visibility or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn change_note_no_expect_plan_private() {
    let r = run_fixture(
        "problem_add_note",
        &["changes", "add-note", "CHN-3", "private words"],
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
fn change_note_public_needs_yes() {
    let r = run_fixture(
        "problem_add_note",
        &[
            "changes",
            "add-note",
            "CHN-3",
            "public words",
            "--public",
            "--no-input",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: note on CHN-3: public note: visible to the requester and may send notifications. Pass --yes to confirm.
"#
    );
}

#[test]
fn change_note_help_documents_expect_plan() {
    let r = run_fixture("problem_add_note", &["changes", "add-note", "-h"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("--expect-plan <TEXT>"));
    assert!(!r.out.contains("unhashed"));
}

#[test]
fn problem_note_help_lists_the_reviewed_flags() {
    let r = run_fixture("problem_add_note", &["problems", "add-note", "-h"], None);
    assert_eq!(r.code, 0, "{}", r.err);
    for flag in [
        "--expect-plan",
        "--json",
        "--dry-run",
        "--yes",
        "--no-input",
        "--public",
    ] {
        assert!(r.out.contains(flag), "{flag}: {}", r.out);
    }
}
#[test]
fn ticket_update_legacy_dry_run() {
    let r = run_fixture(
        "ticket_update_legacy",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "SR-11",
  "body": {
    "priority": 2
  },
  "plan_hash": "31c7920f7d7df04b3d0a6632ca199f0e9ead5ab901ee4a76be0c789fa0de81cf",
  "record_state": {
    "sends": 0,
    "server_version": "2030-01-01T00:00:00Z"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        "warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default\n"
    );
}

#[test]
fn ticket_update_legacy_send_hash() {
    let r = run_fixture(
        "ticket_update_legacy",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--yes",
            "--expect-plan",
            "31c7920f7d7df04b3d0a6632ca199f0e9ead5ab901ee4a76be0c789fa0de81cf",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "id": 11,
  "human_display_id": "SR-11",
  "subject": "Acme laptop",
  "priority": 2,
  "updated_at": "2030-01-01T00:00:01Z",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        "warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default\n"
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/11"]);
}

#[test]
fn ticket_update_legacy_plain_send() {
    let r = run_fixture(
        "ticket_update_legacy",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"updated SR-11
"#
    );
    assert_eq!(
        r.err,
        "warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default\n"
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/11"]);
}

#[test]
fn ticket_update_legacy_hash_body() {
    let r = run_fixture(
        "ticket_update_legacy",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Low",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "SR-11",
  "body": {
    "priority": 1
  },
  "plan_hash": "3d4d818c507470e99db81b1b6f6618da8097ad948343b0d30cb92098d101fe14",
  "record_state": {
    "sends": 0,
    "server_version": "2030-01-01T00:00:00Z"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        "warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default\n"
    );
}

#[test]
fn ticket_update_legacy_hash_set() {
    let r = run_fixture(
        "ticket_update_legacy",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--set",
            "subject=Acme desk",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "SR-11",
  "body": {
    "priority": 2,
    "subject": "Acme desk"
  },
  "plan_hash": "f6722341837c5534346a6ad58733bee7c42e46fef6d32ddf86dacdb4b0891639",
  "record_state": {
    "sends": 0,
    "server_version": "2030-01-01T00:00:00Z"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        "warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default\n"
    );
}

#[test]
fn ticket_update_legacy_body_changed() {
    let r = run_fixture(
        "ticket_update_legacy",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Low",
            "--yes",
            "--expect-plan",
            "31c7920f7d7df04b3d0a6632ca199f0e9ead5ab901ee4a76be0c789fa0de81cf",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default
error: INC-11 or the update changed since review (or the record could not be read); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn ticket_update_legacy_prefix() {
    let r = run_fixture(
        "ticket_update_legacy",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--yes",
            "--expect-plan",
            "31c7920f7d7df04b",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default
error: INC-11 or the update changed since review (or the record could not be read); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn ticket_update_legacy_record_changed() {
    let r = run_fixture(
        "ticket_update_legacy_edited",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--yes",
            "--expect-plan",
            "31c7920f7d7df04b3d0a6632ca199f0e9ead5ab901ee4a76be0c789fa0de81cf",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default
error: INC-11 or the update changed since review (or the record could not be read); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn ticket_update_legacy_unreadable_dry_run() {
    let r = run_fixture(
        "ticket_update_legacy_unreadable",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "SR-11",
  "body": {
    "priority": 2
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        "warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default\n"
    );
}

#[test]
fn ticket_update_legacy_unreadable_expect() {
    let r = run_fixture(
        "ticket_update_legacy_unreadable",
        &[
            "tickets",
            "update",
            "--no-review",
            "SR-11",
            "--priority",
            "Medium",
            "--yes",
            "--expect-plan",
            "31c7920f7d7df04b3d0a6632ca199f0e9ead5ab901ee4a76be0c789fa0de81cf",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"warning: tickets update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default
error: INC-11 or the update changed since review (or the record could not be read); review again with --dry-run. Nothing was sent
"#
    );
}
#[test]
fn problem_update_dry_run_state() {
    let r = run_fixture(
        "problem_update",
        &[
            "problems",
            "update",
            "--no-review",
            "PRB-71",
            "--set",
            "subject=Acme renamed",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "update",
  "id": "PRB-71",
  "body": {
    "subject": "Acme renamed"
  },
  "plan_hash": "e231ec06d4208df09eb490f59aa3779171cdcafde33c581cb38cd42dd3c338e3",
  "record_state": {
    "sends": 0,
    "server_version": null
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        "warning: problems update --no-review is deprecated and will be removed in the next release: it sends a body-only update with no journal or readback; drop --no-review for the reviewed default\n"
    );
}
#[test]
fn ticket_reply_plain_dry_run() {
    let r = run_fixture(
        "ticket_reply_plain",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "dry_run": true,
  "action": "reply",
  "id": "INC-123",
  "body": {
    "body": "Acme is working on it"
  },
  "plan_hash": "302a69ff8f50073170673b78115d4b7d649094c4061082d539fd43185a2c444a",
  "record_state": {
    "sends": 0
  },
  "requires_confirmation": [
    "ticket reply: sent to the requester by email"
  ],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn ticket_reply_plain_send_json() {
    let r = run_fixture(
        "ticket_reply_plain",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
            "--expect-plan",
            "302a69ff8f50073170673b78115d4b7d649094c4061082d539fd43185a2c444a",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 123,
  "conversation_id": 501,
  "plan_hash": "302a69ff8f50073170673b78115d4b7d649094c4061082d539fd43185a2c444a",
  "verified": true,
  "readback": {
    "id": 501,
    "body": "<div>Acme is working on it</div>",
    "private": false
  },
  "fields": {
    "body": "matched"
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/reply"]);
}

#[test]
fn ticket_reply_plain_send_human() {
    let r = run_fixture(
        "ticket_reply_plain",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added reply 501; verified: true
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/reply"]);
}

#[test]
fn ticket_reply_plain_wrong_hash() {
    let r = run_fixture(
        "ticket_reply_plain",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
            "--expect-plan",
            "54a7a3f299d0d36426d174d70d8e7a217a84eeadadacd2e4c66ad7465bb54746",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: reply on INC-123 does not match the reviewed plan (body or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn ticket_reply_plain_other_body() {
    let r = run_fixture(
        "ticket_reply_plain",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Globex text",
            "--yes",
            "--expect-plan",
            "302a69ff8f50073170673b78115d4b7d649094c4061082d539fd43185a2c444a",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: reply on INC-123 does not match the reviewed plan (body or tenant changed, or a partial hash); review again with --dry-run. Nothing was sent
"#
    );
}

#[test]
fn ticket_reply_plain_empty() {
    let r = run_fixture(
        "ticket_reply_plain",
        &["tickets", "reply", "INC-123", "  ", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: reply body must be nonempty text without NUL characters
"#
    );
}

#[test]
fn ticket_reply_plain_unverified() {
    let r = run_fixture(
        "ticket_reply_plain_mismatch",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "ticket_id": 123,
  "conversation_id": 501,
  "plan_hash": "302a69ff8f50073170673b78115d4b7d649094c4061082d539fd43185a2c444a",
  "verified": false,
  "readback": {
    "id": 501,
    "body": "<div>Globex</div>"
  },
  "fields": {
    "body": "mismatched"
  },
  "warning": "Reply sent but conversation read-back did not verify it; do not resend.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/reply"]);
}

#[test]
fn ticket_reply_plain_unverified_human() {
    let r = run_fixture(
        "ticket_reply_plain_mismatch",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"added reply 501; verified: false
"#
    );
    assert_eq!(
        r.err,
        r#"Reply sent but conversation read-back did not verify it; do not resend.
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/reply"]);
}

#[test]
fn ticket_reply_plain_rejected() {
    let r = run_fixture(
        "ticket_reply_plain_rejected",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: reply rejected (HTTP 400): Validation failed; no retry
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/reply"]);
}

#[test]
fn ticket_reply_plain_rejected_json() {
    let r = run_fixture(
        "ticket_reply_plain_rejected",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
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
  "plan_hash": "302a69ff8f50073170673b78115d4b7d649094c4061082d539fd43185a2c444a",
  "verified": false,
  "error": "reply rejected (HTTP 400): Validation failed; no retry",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"error: reply rejected (HTTP 400): Validation failed; no retry
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/reply"]);
}

#[test]
fn ticket_reply_plain_unknown() {
    let r = run_fixture(
        "ticket_reply_plain_500",
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: reply outcome unknown; inspect the ticket conversations and journal before any retry; do not resend (HTTP 500)
"#
    );
    assert_eq!(r.write_paths(), ["POST /api/_/tickets/123/reply"]);
}

#[test]
fn ticket_reply_plain_unknown_not_resent() {
    let r = run_steps(
        "ticket_reply_plain_500",
        &[&[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
        ]],
        &[
            "tickets",
            "reply",
            "INC-123",
            "Acme is working on it",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: reply outcome unknown; inspect the ticket conversations and journal before any retry; do not resend
"#
    );
}

const REPLY_HASH: &str = "302a69ff8f50073170673b78115d4b7d649094c4061082d539fd43185a2c444a";
const REPLY_BODY: &str = "Acme is working on it";

#[test]
fn ticket_reply_plain_replay_names_the_earlier_send() {
    let send = [
        "tickets",
        "reply",
        "INC-123",
        REPLY_BODY,
        "--yes",
        "--expect-plan",
        REPLY_HASH,
    ];
    let r = run_steps("ticket_reply_plain", &[&send[..]], &send, None);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert!(
        r.err
            .contains(&format!("plan {REPLY_HASH} was already sent at "))
            && r.err.contains("nothing was sent now"),
        "{}",
        r.err
    );
    assert!(r.write_paths().is_empty());
}

#[test]
fn ticket_reply_plain_repeat_plans_a_fresh_hash() {
    let send = ["tickets", "reply", "INC-123", REPLY_BODY, "--yes"];
    let review = ["tickets", "reply", "INC-123", REPLY_BODY, "--dry-run"];
    let r = run_steps("ticket_reply_plain", &[&send[..]], &review, None);
    assert_eq!(r.code, 0, "{}", r.err);
    let doc = r.json();
    assert_eq!(doc["record_state"], serde_json::json!({"sends": 1}));
    assert_ne!(doc["plan_hash"], REPLY_HASH);
}
#[test]
fn planning_text_clear_empty_is_nothing_to_change() {
    let r = run_fixture(
        "planning_clear_empty",
        &[
            "changes",
            "planning-text",
            "126",
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
  "status": "nothing_to_change",
  "message": "planning section is already empty; nothing to change, no write attempted",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn planning_text_clear_empty_yes_is_nothing_to_change() {
    let r = run_fixture(
        "planning_clear_empty",
        &[
            "changes",
            "planning-text",
            "126",
            "--section",
            "Rollout Plan",
            "--clear",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "nothing_to_change",
  "message": "planning section is already empty; nothing to change, no write attempted",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn planning_text_clear_empty_with_expect_plan_exits_2() {
    let r = run_fixture(
        "planning_clear_empty",
        &[
            "changes",
            "planning-text",
            "126",
            "--section",
            "Rollout Plan",
            "--clear",
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
        r#"nothing to change; the reviewed plan does not exist. Nothing was sent
"#
    );
}

#[test]
fn problem_planning_text_clear_empty_is_nothing_to_change() {
    let r = run_fixture(
        "planning_clear_empty",
        &[
            "problems",
            "planning-text",
            "126",
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
  "status": "nothing_to_change",
  "message": "planning section is already empty; nothing to change, no write attempted",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}
#[test]
fn change_update_noop_dry_run_json() {
    let r = run_fixture(
        "change_update_clean",
        &[
            "changes",
            "update",
            "12",
            "--priority",
            "Low",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "action": "update",
  "target": 12,
  "changed": false,
  "message": "nothing to change: CHN-12 already has these values",
  "warnings": [],
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_update_noop_dry_run_text() {
    let r = run_fixture(
        "change_update_clean",
        &["changes", "update", "12", "--priority", "Low", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"nothing to change CHN-12: it already has these values
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_update_noop_dry_run_expect_plan() {
    let r = run_fixture(
        "change_update_clean",
        &[
            "changes",
            "update",
            "12",
            "--priority",
            "Low",
            "--dry-run",
            "--expect-plan",
            "54a7a3f299d0d36426d174d70d8e7a217a84eeadadacd2e4c66ad7465bb54746",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.out,
        r#"nothing to change CHN-12: it already has these values
"#
    );
    assert_eq!(
        r.err,
        r#"error: nothing to change; the reviewed plan does not exist
"#
    );
}

#[test]
fn change_update_noop_yes_expect_plan() {
    let r = run_fixture(
        "change_update_clean",
        &[
            "changes",
            "update",
            "12",
            "--priority",
            "Low",
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
        r#"error: nothing to change; the reviewed plan does not exist
"#
    );
}

#[test]
fn change_update_noop_yes_plain() {
    let r = run_fixture(
        "change_update_clean",
        &["changes", "update", "12", "--priority", "Low", "--yes"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"no changes CHN-12
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn change_update_send_hash() {
    let r = run_fixture(
        "change_update_clean",
        &[
            "changes",
            "update",
            "12",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "1186920fe65b25acbbe934c2c94ed1aab4bf3aec99ecdf1724e012f50c6a1a5c",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: Change update readback unverified; do not resend
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/changes/12"]);
}

#[test]
fn change_update_stale_hash() {
    let r = run_fixture(
        "change_update_clean",
        &[
            "changes",
            "update",
            "12",
            "--priority",
            "High",
            "--yes",
            "--expect-plan",
            "54a7a3f299d0d36426d174d70d8e7a217a84eeadadacd2e4c66ad7465bb54746",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"error: plan changed: expected 54a7a3f299d0d36426d174d70d8e7a217a84eeadadacd2e4c66ad7465bb54746, computed 1186920fe65b25acbbe934c2c94ed1aab4bf3aec99ecdf1724e012f50c6a1a5c. Review again with --dry-run.
"#
    );
}

#[test]
fn change_update_dry_run_binds_the_record_state_token() {
    let r = run_fixture(
        "change_update_clean",
        &[
            "changes",
            "update",
            "12",
            "--priority",
            "High",
            "--dry-run",
            "--json",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let doc = r.json();
    assert_eq!(
        doc["plan_hash"],
        "1186920fe65b25acbbe934c2c94ed1aab4bf3aec99ecdf1724e012f50c6a1a5c"
    );
    assert_eq!(doc["payload"], serde_json::json!({"priority": 3}));
    assert_eq!(
        doc["record_state"],
        serde_json::json!({"sends": 0, "server_version": "2030-01-01T00:00:00Z"})
    );
}
