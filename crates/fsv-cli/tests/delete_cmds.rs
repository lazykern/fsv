//! Reviewed deletes ported from Python: problems delete, restore and
//! delete-forever, solutions delete and delete-forever (`record_delete_cli`).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: 4ad278dcc74c643fec6bbea89c8b4a46f1cd36ee

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
  "plan_hash": "821dd90b4cf6784bbbe2b80658b682c5a23f6fc276280495a97abcebccfb3ebf",
  "warning": "Single attempt; an unknown outcome must be inspected, never automatically resent.",
  "problem_id": 12,
  "title": "Acme outage",
  "deleted": false,
  "effect": "Moves the problem to the trash; `fsv problems restore` brings it back.",
  "requires": null
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
            "821dd90b4cf6784bbbe2b80658b682c5a23f6fc276280495a97abcebccfb3ebf",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "821dd90b4cf6784bbbe2b80658b682c5a23f6fc276280495a97abcebccfb3ebf",
  "verified": true,
  "problem_id": 12,
  "fields": {
    "deleted": "matched"
  },
  "readback_attempts": 1
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
            "44c96bfcef6ef86f6670d7d91101a528ae03778343a3fcaa04450a223f288446",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "44c96bfcef6ef86f6670d7d91101a528ae03778343a3fcaa04450a223f288446",
  "verified": false,
  "problem_id": 13,
  "fields": {
    "deleted": "mismatched"
  },
  "readback_attempts": 4,
  "warning": "Readback was not fully verified; do not resend."
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
            "11b79ec1be36ec4f4578539d34a5ed38ce9f1738bdb32676b9a334a2bb17c5f6",
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
  "plan_hash": "1537ce883730be63be6d87f6d8473559236687cdcee085619f8a76e47ad970b9",
  "warning": "Single attempt; an unknown outcome must be inspected, never automatically resent.",
  "problem_id": 21,
  "title": "Acme trashed",
  "deleted": true,
  "effect": "Restores the problem from the trash.",
  "requires": null
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
            "1537ce883730be63be6d87f6d8473559236687cdcee085619f8a76e47ad970b9",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "1537ce883730be63be6d87f6d8473559236687cdcee085619f8a76e47ad970b9",
  "verified": true,
  "problem_id": 21,
  "fields": {
    "deleted": "matched"
  },
  "readback_attempts": 1
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
  "plan_hash": "b26f1ecfbfeff4f0aa100b5db561f3b46fe10a1776fb20ebe161e56633a0c1d7",
  "warning": "One request per problem, in order; stops at the first failed or unverified one. Never replay an uncertain outcome."
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
            "b26f1ecfbfeff4f0aa100b5db561f3b46fe10a1776fb20ebe161e56633a0c1d7",
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
            "b26f1ecfbfeff4f0aa100b5db561f3b46fe10a1776fb20ebe161e56633a0c1d7",
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
  "plan_hash": "b26f1ecfbfeff4f0aa100b5db561f3b46fe10a1776fb20ebe161e56633a0c1d7",
  "problem_ids": [
    22
  ],
  "results": {
    "22": {
      "verified": true
    }
  }
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
  "plan_hash": "a34904ffd5f518780ed8b9d6f05fb8b7857069182519d359ac948643afffb6ca",
  "warning": "Single attempt; an unknown outcome must be inspected, never automatically resent.",
  "article_id": 31,
  "title": "Globex workaround",
  "deleted": false,
  "effect": "Moves the article to the solutions trash; fsv has no captured article restore.",
  "requires": "--confirm-destructive"
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
            "a34904ffd5f518780ed8b9d6f05fb8b7857069182519d359ac948643afffb6ca",
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
            "a34904ffd5f518780ed8b9d6f05fb8b7857069182519d359ac948643afffb6ca",
            "--confirm-destructive",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "a34904ffd5f518780ed8b9d6f05fb8b7857069182519d359ac948643afffb6ca",
  "verified": true,
  "article_id": 31,
  "fields": {
    "deleted": "matched"
  },
  "readback_attempts": 1
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
  "plan_hash": "3dd892b5b7094e9d9caff1fb55b0cd9b865c7e7a54bfbf52f2142f1157a148d8",
  "warning": "One request per article, in order; stops at the first failed or unverified one. Never replay an uncertain outcome."
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
            "db3a4c5cd66ce72654ebe980e001f614919e66e1af495865810ea0631e7b86ff",
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
            "db3a4c5cd66ce72654ebe980e001f614919e66e1af495865810ea0631e7b86ff",
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
  "plan_hash": "db3a4c5cd66ce72654ebe980e001f614919e66e1af495865810ea0631e7b86ff",
  "article_ids": [
    33
  ],
  "results": {
    "33": {
      "verified": true
    }
  }
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
  "plan_hash": "4aa2d696811a7dc31878b28e70da549b17d9984b996aafe604d9e27e16b8d0d3",
  "warning": "One request per problem, in order; stops at the first failed or unverified one. Never replay an uncertain outcome."
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
            "4aa2d696811a7dc31878b28e70da549b17d9984b996aafe604d9e27e16b8d0d3",
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
            "4aa2d696811a7dc31878b28e70da549b17d9984b996aafe604d9e27e16b8d0d3",
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
  "plan_hash": "4aa2d696811a7dc31878b28e70da549b17d9984b996aafe604d9e27e16b8d0d3",
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
  }
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
            "8a733a45e79b5519d774f303ffbc4a1e9eeda40bca5dc5991df062d5aefa0519",
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
  "plan_hash": "8a733a45e79b5519d774f303ffbc4a1e9eeda40bca5dc5991df062d5aefa0519",
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
      "error": "write outcome unknown; inspect target and journal before retry"
    }
  },
  "stopped_at": 45,
  "not_attempted": [
    46
  ],
  "warning": "Stopped at the first failed or unverified problem; inspect it and the journal; never replay."
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
            "ad94fba1abd7cd746a3d099a52e8bb42d74959a04715d6aa4e470736b79dd695",
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
  "plan_hash": "ad94fba1abd7cd746a3d099a52e8bb42d74959a04715d6aa4e470736b79dd695",
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
  }
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
