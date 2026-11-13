//! problems status Closed reviews show the closure preflight up front (Python 84813b6).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: 84813b6241f861a026c0f38a520ed5ff953a401c

mod common;
use common::{run_fixture, run_steps};

#[test]
fn closed_dry_run_shows_missing_solution_up_front() {
    let r = run_fixture(
        "problem_status_closure",
        &["problems", "status", "12", "--to", "Closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_status",
  "method": "PUT",
  "path": "/api/_/problems/12",
  "payload": {
    "status": 3
  },
  "plan_hash": "357407033c3cc309cd384902b09fcf69c059e86016f5090140564aa181ac4035",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "to": "Closed",
  "before": {
    "custom_fields": {
      "cf_owner_note": "Acme owner"
    },
    "human_display_id": "PRB-12",
    "id": 12,
    "planned_end_date": null,
    "planned_start_date": null,
    "status": 2,
    "subject": "Acme problem 12"
  },
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values.",
  "closure_check": {
    "analysis": "Root Cause, Impact and Symptoms are filled",
    "linked_solution": "missing: this tenant may require a linked workaround / solution for closure (not proven here; the server decides and fsv remembers a rejection for later dry-runs)"
  },
  "warnings": [
    "no linked workaround / solution: some tenants reject closure without one (check_workaround_or_solution_added); not blocked because this tenant has not proven that rule (observed on one tenant: only 2 of 62 closed problems had a linked workaround / solution, so this closure rule is tenant configuration, not a platform rule); the server decides"
  ]
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn closed_dry_run_shows_linked_solution_up_front() {
    let r = run_fixture(
        "problem_status_closure",
        &["problems", "status", "13", "--to", "Closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "problem_status",
  "method": "PUT",
  "path": "/api/_/problems/13",
  "payload": {
    "status": 3
  },
  "plan_hash": "d27eb5790b6979966b63a9a00798de05ae4918ee221d5bd4f61f221cac1c12bb",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "to": "Closed",
  "before": {
    "custom_fields": {
      "cf_owner_note": "Acme owner"
    },
    "human_display_id": "PRB-13",
    "id": 13,
    "planned_end_date": null,
    "planned_start_date": null,
    "status": 2,
    "subject": "Acme problem 13"
  },
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values.",
  "closure_check": {
    "analysis": "Root Cause, Impact and Symptoms are filled",
    "linked_solution": "linked (1)"
  }
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn non_closure_dry_run_has_no_closure_check() {
    let r = run_fixture(
        "problem_status_closure",
        &[
            "problems",
            "status",
            "12",
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
  "path": "/api/_/problems/12",
  "payload": {
    "status": 2
  },
  "plan_hash": "359584202f3ae3f9ccd2bd8e005f7db7f4e48432dc25f5770a617dbcda94b68c",
  "warning": "Single-attempt write. Unknown outcomes must be inspected, never automatically resent.",
  "to": "Change Requested",
  "before": {
    "custom_fields": {
      "cf_owner_note": "Acme owner"
    },
    "human_display_id": "PRB-12",
    "id": 12,
    "planned_end_date": null,
    "planned_start_date": null,
    "status": 2,
    "subject": "Acme problem 12"
  },
  "side_effects": "Server may auto-fill empty planned_start_date and planned_end_date on status transitions; readback reports these values."
}
"#
    );
    assert_eq!(r.err, r#""#);
}

#[test]
fn closed_rejection_remembers_solution_rule() {
    let r = run_fixture(
        "problem_status_closure",
        &[
            "problems",
            "status",
            "12",
            "--to",
            "Closed",
            "--yes",
            "--expect-plan",
            "357407033c3cc309cd384902b09fcf69c059e86016f5090140564aa181ac4035",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"violated_closure_rules: a workaround / solution must be linked
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/problems/12"]);
}

#[test]
fn remembered_solution_rule_blocks_the_dry_run_up_front() {
    let r = run_steps(
        "problem_status_closure",
        &[&[
            "problems",
            "status",
            "12",
            "--to",
            "Closed",
            "--yes",
            "--expect-plan",
            "357407033c3cc309cd384902b09fcf69c059e86016f5090140564aa181ac4035",
        ]],
        &["problems", "status", "12", "--to", "Closed", "--dry-run"],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"violated_closure_rules: missing linked workaround / solution (this tenant rejected a closure without a linked workaround / solution earlier; fsv remembers that per tenant, so the dry-run blocks instead of letting the server refuse)
"#
    );
}
