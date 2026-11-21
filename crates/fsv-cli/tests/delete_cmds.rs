//! Reviewed deletes ported from Python: problems delete, restore and
//! delete-forever, solutions delete and delete-forever (`record_delete_cli`).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: 041d86589726f01be5bc5ef8d00e21ee6132c660

mod common;
use common::run_fixture;

#[test]
fn problems_delete_dry_run() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete", "PRB-12", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_delete",
  "method": "DELETE",
  "path": "/api/_/problems/12",
  "payload": null,
  "plan_hash": "76bcdb1ab4a60e93b3fe861af9ffb1b96c095f05c25cc269646d091ab122b23e",
  "warning": "Single attempt; an unknown outcome must be inspected, never automatically resent.",
  "problem_id": 12,
  "title": "Acme outage",
  "deleted": false,
  "effect": "Moves the problem to the trash; `fsv problems restore` brings it back.",
  "requires": null,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_delete_wrong_hash() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "delete",
            "PRB-12",
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
        r#"review with --dry-run, then --yes --expect-plan FULL_HASH (plus --confirm-destructive where the review requires it)
"#
    );
}

#[test]
fn problems_delete_no_yes() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "delete",
            "12",
            "--expect-plan",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"review with --dry-run, then --yes --expect-plan FULL_HASH (plus --confirm-destructive where the review requires it)
"#
    );
}

#[test]
fn problems_delete_send() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "delete",
            "PRB-12",
            "--yes",
            "--expect-plan",
            "76bcdb1ab4a60e93b3fe861af9ffb1b96c095f05c25cc269646d091ab122b23e",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "76bcdb1ab4a60e93b3fe861af9ffb1b96c095f05c25cc269646d091ab122b23e",
  "verified": true,
  "problem_id": 12,
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
    assert_eq!(r.write_paths(), ["DELETE /api/_/problems/12"]);
}

#[test]
fn problems_delete_send_unverified() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "delete",
            "13",
            "--yes",
            "--expect-plan",
            "535724d489851cd839422d4dc6f2a00893084e50464aa3598025a9d3dd8d6b0e",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "535724d489851cd839422d4dc6f2a00893084e50464aa3598025a9d3dd8d6b0e",
  "verified": false,
  "problem_id": 13,
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
    assert_eq!(r.write_paths(), ["DELETE /api/_/problems/13"]);
}

#[test]
fn problems_delete_rejected() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "delete",
            "14",
            "--yes",
            "--expect-plan",
            "5963dd3f2d8fec2793cf865fa4f07929e3533ff288e6761bfeae8fe1c14a174b",
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
    assert_eq!(r.write_paths(), ["DELETE /api/_/problems/14"]);
}

#[test]
fn problems_delete_already_trashed() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete", "21", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"problem is already in the trash; use restore or delete-forever
"#
    );
}

#[test]
fn problems_delete_not_found() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete", "99", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"problem 99 not found; nothing to delete
"#
    );
}

#[test]
fn problems_delete_flag_unavailable() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete", "15", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact problem deleted flag unavailable
"#
    );
}

#[test]
fn problems_delete_other_identity() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete", "16", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact problem unavailable
"#
    );
}

#[test]
fn problems_delete_read_500() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete", "17", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"exact problem unavailable (HTTP 500)
"#
    );
}

#[test]
fn problems_delete_bad_id() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete", "CHN-12", "--dry-run"],
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
fn problems_restore_dry_run() {
    let r = run_fixture(
        "record_delete",
        &["problems", "restore", "21", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_restore",
  "method": "PUT",
  "path": "/api/_/problems/21/restore",
  "payload": null,
  "plan_hash": "c6093524a0c6eab64071eca46c91a00dfcc7d00a0b068e8f68497cc5c12f206c",
  "warning": "Single attempt; an unknown outcome must be inspected, never automatically resent.",
  "problem_id": 21,
  "title": "Acme trashed",
  "deleted": true,
  "effect": "Restores the problem from the trash.",
  "requires": null,
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn problems_restore_not_trashed() {
    let r = run_fixture(
        "record_delete",
        &["problems", "restore", "12", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"restore applies only to a problem in the trash (delete it first)
"#
    );
}

#[test]
fn problems_restore_send() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "restore",
            "21",
            "--yes",
            "--expect-plan",
            "c6093524a0c6eab64071eca46c91a00dfcc7d00a0b068e8f68497cc5c12f206c",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "c6093524a0c6eab64071eca46c91a00dfcc7d00a0b068e8f68497cc5c12f206c",
  "verified": true,
  "problem_id": 21,
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
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/21/restore"]);
}

#[test]
fn problems_delete_forever_dry_run() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete-forever", "22", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_delete_forever",
  "method": "DELETE",
  "requests": [
    "/api/_/problems/delete-forever?ids=22"
  ],
  "problem_ids": [
    22
  ],
  "selected": [
    {
      "id": 22,
      "title": "Acme trashed twice"
    }
  ],
  "max_records": null,
  "effect": "PERMANENT: removes the listed problem(s); they cannot be restored.",
  "requires": "--confirm-destructive",
  "plan_hash": "d8dc14b71b6ba88028d0400d256c10d8db401856753dc191091940020f3194ba",
  "warning": "One request per problem, in order; stops at the first failed or unverified one. Never replay an uncertain outcome.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Problems to delete PERMANENTLY: 22
  22: Acme trashed twice
"#
    );
}

#[test]
fn problems_delete_forever_not_trashed() {
    let r = run_fixture(
        "record_delete",
        &["problems", "delete-forever", "12", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"delete-forever applies only to a problem in the trash (delete it first)
"#
    );
}

#[test]
fn problems_delete_forever_needs_confirm() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "delete-forever",
            "22",
            "--yes",
            "--expect-plan",
            "d8dc14b71b6ba88028d0400d256c10d8db401856753dc191091940020f3194ba",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Problems to delete PERMANENTLY: 22
  22: Acme trashed twice
problem delete-forever requires --confirm-destructive after inspecting the printed ID list
"#
    );
}

#[test]
fn problems_delete_forever_send() {
    let r = run_fixture(
        "record_delete",
        &[
            "problems",
            "delete-forever",
            "22",
            "--yes",
            "--expect-plan",
            "d8dc14b71b6ba88028d0400d256c10d8db401856753dc191091940020f3194ba",
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
  "plan_hash": "d8dc14b71b6ba88028d0400d256c10d8db401856753dc191091940020f3194ba",
  "problem_ids": [
    22
  ],
  "results": {
    "22": {
      "verified": true
    }
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Problems to delete PERMANENTLY: 22
  22: Acme trashed twice
"#
    );
    assert_eq!(r.write_paths(), ["DELETE /api/_/problems/delete-forever"]);
}

#[test]
fn solutions_delete_dry_run() {
    let r = run_fixture(
        "record_delete",
        &["solutions", "delete", "31", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "solution_delete",
  "method": "DELETE",
  "path": "/api/_/solutions/articles/31",
  "payload": null,
  "plan_hash": "2db6ffffa9bd5576f9a0a1baffcc24a0c7bc549fe6776f33b45185338d128050",
  "warning": "Single attempt; an unknown outcome must be inspected, never automatically resent.",
  "article_id": 31,
  "title": "Globex workaround",
  "deleted": false,
  "effect": "Moves the article to the solutions trash; fsv has no captured article restore.",
  "requires": "--confirm-destructive",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn solutions_delete_needs_confirm() {
    let r = run_fixture(
        "record_delete",
        &[
            "solutions",
            "delete",
            "31",
            "--yes",
            "--expect-plan",
            "2db6ffffa9bd5576f9a0a1baffcc24a0c7bc549fe6776f33b45185338d128050",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"article delete requires --confirm-destructive after inspecting the review
"#
    );
}

#[test]
fn solutions_delete_send() {
    let r = run_fixture(
        "record_delete",
        &[
            "solutions",
            "delete",
            "31",
            "--yes",
            "--expect-plan",
            "2db6ffffa9bd5576f9a0a1baffcc24a0c7bc549fe6776f33b45185338d128050",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "2db6ffffa9bd5576f9a0a1baffcc24a0c7bc549fe6776f33b45185338d128050",
  "verified": true,
  "article_id": 31,
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
    assert_eq!(r.write_paths(), ["DELETE /api/_/solutions/articles/31"]);
}

#[test]
fn solutions_delete_already_trashed() {
    let r = run_fixture(
        "record_delete",
        &["solutions", "delete", "32", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"article is already in the trash; use delete-forever
"#
    );
}

#[test]
fn solutions_delete_not_found() {
    let r = run_fixture(
        "record_delete",
        &["solutions", "delete", "99", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"article 99 not found; nothing to delete
"#
    );
}

#[test]
fn solutions_delete_bad_id() {
    let r = run_fixture(
        "record_delete",
        &["solutions", "delete", "KB-3", "--dry-run"],
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
fn solutions_delete_forever_dry_run() {
    let r = run_fixture(
        "record_delete",
        &["solutions", "delete-forever", "32", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "solution_delete_forever",
  "method": "DELETE",
  "requests": [
    "/api/_/solutions/articles/32/delete_forever"
  ],
  "article_ids": [
    32
  ],
  "selected": [
    {
      "id": 32,
      "title": "Globex trashed été"
    }
  ],
  "max_records": null,
  "effect": "PERMANENT: removes the listed article(s); they cannot be restored.",
  "requires": "--confirm-destructive",
  "plan_hash": "fe2885f9283123c821477b2f64a1ed1fa0d783f316a2569f23bfb2f99e6bdc58",
  "warning": "One request per article, in order; stops at the first failed or unverified one. Never replay an uncertain outcome.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Articles to delete PERMANENTLY: 32
  32: Globex trashed été
"#
    );
}

#[test]
fn solutions_delete_forever_needs_confirm() {
    let r = run_fixture(
        "record_delete",
        &[
            "solutions",
            "delete-forever",
            "33",
            "--yes",
            "--expect-plan",
            "66a2ac182f9b26f859fd943e45740a68a39349d1fee98a6833be7a7fc0c58eb5",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Articles to delete PERMANENTLY: 33
  33: Globex permanent
article delete-forever requires --confirm-destructive after inspecting the printed ID list
"#
    );
}

#[test]
fn solutions_delete_forever_send() {
    let r = run_fixture(
        "record_delete",
        &[
            "solutions",
            "delete-forever",
            "33",
            "--yes",
            "--expect-plan",
            "66a2ac182f9b26f859fd943e45740a68a39349d1fee98a6833be7a7fc0c58eb5",
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
  "plan_hash": "66a2ac182f9b26f859fd943e45740a68a39349d1fee98a6833be7a7fc0c58eb5",
  "article_ids": [
    33
  ],
  "results": {
    "33": {
      "verified": true
    }
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Articles to delete PERMANENTLY: 33
  33: Globex permanent
"#
    );
    assert_eq!(
        r.write_paths(),
        ["DELETE /api/_/solutions/articles/33/delete_forever"]
    );
}

#[test]
fn problems_forever_needs_max_records() {
    let r = run_fixture(
        "record_delete_forever",
        &["problems", "delete-forever", "41", "42", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"deleting more than one problem permanently requires --max-records N (1 to 50)
"#
    );
}

#[test]
fn problems_forever_more_than_max() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "problems",
            "delete-forever",
            "41",
            "42",
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
        r#"more IDs than --max-records; refusing the whole plan; nothing was sent
"#
    );
}

#[test]
fn problems_forever_max_records_range() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "problems",
            "delete-forever",
            "41",
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
        r#"--max-records must be 1 to 50
"#
    );
}

#[test]
fn problems_forever_distinct() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "problems",
            "delete-forever",
            "41",
            "41",
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
        r#"give distinct problem IDs
"#
    );
}

#[test]
fn solutions_forever_needs_max_records() {
    let r = run_fixture(
        "record_delete_forever",
        &["solutions", "delete-forever", "51", "52", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"deleting more than one article permanently requires --max-records N (1 to 50)
"#
    );
}

#[test]
fn problems_forever_many_dry_run() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "problems",
            "delete-forever",
            "41",
            "42",
            "43",
            "--max-records",
            "3",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_delete_forever",
  "method": "DELETE",
  "requests": [
    "/api/_/problems/delete-forever?ids=41",
    "/api/_/problems/delete-forever?ids=42",
    "/api/_/problems/delete-forever?ids=43"
  ],
  "problem_ids": [
    41,
    42,
    43
  ],
  "selected": [
    {
      "id": 41,
      "title": "Acme trashed 41"
    },
    {
      "id": 42,
      "title": "Acme trashed 42"
    },
    {
      "id": 43,
      "title": "Acme trashed 43"
    }
  ],
  "max_records": 3,
  "effect": "PERMANENT: removes the listed problem(s); they cannot be restored.",
  "requires": "--confirm-destructive",
  "plan_hash": "4dc9247d990882d1f5bb30f19bef138aa881adf590a87267e695ac04c9704d26",
  "warning": "One request per problem, in order; stops at the first failed or unverified one. Never replay an uncertain outcome.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Problems to delete PERMANENTLY: 41, 42, 43
  41: Acme trashed 41
  42: Acme trashed 42
  43: Acme trashed 43
"#
    );
}

#[test]
fn problems_forever_many_needs_confirm() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "problems",
            "delete-forever",
            "41",
            "42",
            "43",
            "--max-records",
            "3",
            "--yes",
            "--expect-plan",
            "4dc9247d990882d1f5bb30f19bef138aa881adf590a87267e695ac04c9704d26",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Problems to delete PERMANENTLY: 41, 42, 43
  41: Acme trashed 41
  42: Acme trashed 42
  43: Acme trashed 43
problem delete-forever requires --confirm-destructive after inspecting the printed ID list
"#
    );
}

#[test]
fn problems_forever_many_send() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "problems",
            "delete-forever",
            "41",
            "42",
            "43",
            "--max-records",
            "3",
            "--yes",
            "--expect-plan",
            "4dc9247d990882d1f5bb30f19bef138aa881adf590a87267e695ac04c9704d26",
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
  "plan_hash": "4dc9247d990882d1f5bb30f19bef138aa881adf590a87267e695ac04c9704d26",
  "problem_ids": [
    41,
    42,
    43
  ],
  "results": {
    "41": {
      "verified": true
    },
    "42": {
      "verified": true
    },
    "43": {
      "verified": true
    }
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Problems to delete PERMANENTLY: 41, 42, 43
  41: Acme trashed 41
  42: Acme trashed 42
  43: Acme trashed 43
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "DELETE /api/_/problems/delete-forever",
            "DELETE /api/_/problems/delete-forever",
            "DELETE /api/_/problems/delete-forever"
        ]
    );
}

#[test]
fn problems_forever_stops_at_first_failure() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "problems",
            "delete-forever",
            "44",
            "45",
            "46",
            "--max-records",
            "3",
            "--yes",
            "--expect-plan",
            "8290b90fa056ba62088c2506866a73c1daa62369b3377f51ca7b45d72b45c349",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "partial",
  "verified": false,
  "plan_hash": "8290b90fa056ba62088c2506866a73c1daa62369b3377f51ca7b45d72b45c349",
  "problem_ids": [
    44,
    45,
    46
  ],
  "results": {
    "44": {
      "verified": true
    },
    "45": {
      "verified": false,
      "error": "write outcome unknown (HTTP 500: no); inspect target and journal before retry"
    }
  },
  "stopped_at": 45,
  "not_attempted": [
    46
  ],
  "warning": "Stopped at the first failed or unverified problem; inspect it and the journal; never replay.",
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Problems to delete PERMANENTLY: 44, 45, 46
  44: Acme trashed 44
  45: Acme trashed 45
  46: Acme trashed 46
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "DELETE /api/_/problems/delete-forever",
            "DELETE /api/_/problems/delete-forever"
        ]
    );
}

#[test]
fn solutions_forever_many_send() {
    let r = run_fixture(
        "record_delete_forever",
        &[
            "solutions",
            "delete-forever",
            "51",
            "52",
            "--max-records",
            "2",
            "--yes",
            "--expect-plan",
            "ba3f512bfd6d533c219e1757d3009f3a4546cc5b43738849568ddb6d2c3c01c8",
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
  "plan_hash": "ba3f512bfd6d533c219e1757d3009f3a4546cc5b43738849568ddb6d2c3c01c8",
  "article_ids": [
    51,
    52
  ],
  "results": {
    "51": {
      "verified": true
    },
    "52": {
      "verified": true
    }
  },
  "tenant": "acme.freshservice.com",
  "config_dir": "<CONFIG_DIR>"
}
"#
    );
    assert_eq!(
        r.err,
        r#"Articles to delete PERMANENTLY: 51, 52
  51: Globex article 51
  52: Globex article 52
"#
    );
    assert_eq!(
        r.write_paths(),
        [
            "DELETE /api/_/solutions/articles/51/delete_forever",
            "DELETE /api/_/solutions/articles/52/delete_forever"
        ]
    );
}
