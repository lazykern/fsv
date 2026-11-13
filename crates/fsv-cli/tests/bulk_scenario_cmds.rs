//! `tickets bulk scenario` against canned routes (Python 2ffd5f0,
//! `tests/golden/cmp.py --emit`).

mod common;
use common::run_fixture;

#[test]
fn bulk_scenario_dry_run() {
    let r = run_fixture(
        "bulk_scenario",
        &[
            "tickets",
            "bulk",
            "scenario",
            "30",
            "31",
            "--scenario",
            "Assign to Acme Team",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "dry_run",
  "action": "ticket_bulk_scenario",
  "method": "PUT",
  "path": "/api/_/tickets/execute_scenario",
  "payload": {
    "ids": [
      30,
      31
    ],
    "scenario_id": 502
  },
  "plan_hash": "544abc8b9f96db8f0dd08186aeeb2956002ee596eb1bd3aff3b786ec1e5f35ac",
  "warning": "Runs scenario 502 ('Assign to Acme Team') on every listed ticket. Its actions (ticket type changes, assignments, emails, notes) run on the server asynchronously (HTTP 202) and cannot be proven by fsv; check the scenario name above. Never replay an uncertain run.",
  "ticket_ids": [
    30,
    31
  ],
  "selection": {
    "action": "scenario",
    "agent": null,
    "group": null,
    "ids": [
      "30",
      "31"
    ],
    "max_records": null,
    "note_primary": null,
    "note_secondary": null,
    "primary": null,
    "scenario": "Assign to Acme Team",
    "select_all": false,
    "status": null,
    "where": []
  },
  "selected_tickets": [
    {
      "id": 30,
      "subject": "Acme 30"
    },
    {
      "id": 31,
      "subject": "Acme 31"
    }
  ],
  "scenario": {
    "id": 502,
    "name": "Assign to Acme Team"
  }
}
"#
    );
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 30, 31
  30: Acme 30
  31: Acme 31
Scenario to run: 502 (Assign to Acme Team); its effects cannot be verified
"#
    );
}

#[test]
fn bulk_scenario_yes_without_plan() {
    let r = run_fixture(
        "bulk_scenario",
        &[
            "tickets",
            "bulk",
            "scenario",
            "30",
            "31",
            "--scenario",
            "Assign to Acme Team",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 30, 31
  30: Acme 30
  31: Acme 31
Scenario to run: 502 (Assign to Acme Team); its effects cannot be verified
review with --dry-run, then --yes --expect-plan FULL_HASH
"#
    );
}

#[test]
fn bulk_scenario_send_unverified() {
    let r = run_fixture(
        "bulk_scenario",
        &[
            "tickets",
            "bulk",
            "scenario",
            "30",
            "31",
            "--scenario",
            "Assign to Acme Team",
            "--yes",
            "--expect-plan",
            "544abc8b9f96db8f0dd08186aeeb2956002ee596eb1bd3aff3b786ec1e5f35ac",
        ],
        None,
    );
    assert_eq!(r.code, 4, "{}", r.err);
    assert_eq!(
        r.out,
        r#"{
  "status": "submitted",
  "plan_hash": "544abc8b9f96db8f0dd08186aeeb2956002ee596eb1bd3aff3b786ec1e5f35ac",
  "verified": false,
  "ticket_ids": [
    30,
    31
  ],
  "identity_verified": {
    "30": true,
    "31": true
  },
  "scenario": {
    "scenario_id": 502,
    "scenario_name": "Assign to Acme Team"
  },
  "warning": "Accepted for asynchronous execution; exact tickets were read back, but the scenario's effects cannot be proven. Inspect the tickets; do not resend.",
  "readback_attempts": 4,
  "delegated_plan_hash": "bc24841791a0e7e46690a67fb4ad3e83b08d3ec1631224e2071254660506b5c4",
  "selected_tickets": [
    {
      "id": 30,
      "subject": "Acme 30"
    },
    {
      "id": 31,
      "subject": "Acme 31"
    }
  ]
}
"#
    );
    assert_eq!(
        r.err,
        r#"Selected ticket IDs: 30, 31
  30: Acme 30
  31: Acme 31
Scenario to run: 502 (Assign to Acme Team); its effects cannot be verified
"#
    );
    assert_eq!(r.write_paths(), ["PUT /api/_/tickets/execute_scenario"]);
}

#[test]
fn bulk_scenario_ambiguous() {
    let r = run_fixture(
        "bulk_scenario",
        &[
            "tickets",
            "bulk",
            "scenario",
            "30",
            "--scenario",
            "ASSIGN TO ACME TEAM",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"scenario 'ASSIGN TO ACME TEAM' must match exactly one scenario by ID or exact name (matches: 502: 'Assign to Acme Team', 503: 'assign to acme team'); nothing was sent
"#
    );
}

#[test]
fn bulk_scenario_option_mismatch() {
    let r = run_fixture(
        "bulk_scenario",
        &[
            "tickets",
            "bulk",
            "spam",
            "30",
            "--scenario",
            "501",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"--scenario NAME_OR_ID is required by, and only valid for, bulk scenario
"#
    );
}
