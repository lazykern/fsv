//! tickets resolve/close/status: invalid --set choices/labels are one-line refusals (Python 15bcf8e).
//! Emitted by cmp.py from Python; the header line names its commit.

// Python commit: 15bcf8eb2f2e41d8a9f051374b57671dcd93f48e

mod common;
use common::run_fixture;

#[test]
fn resolve_set_invalid_choice() {
    let r = run_fixture(
        "ticket_lifecycle_dropdown",
        &[
            "tickets",
            "resolve",
            "128",
            "--note",
            "Acme fix",
            "--set",
            "Incident Kind=Incident",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Incident Kind: 'Incident' is not a valid choice; choose one of: Service, Event; nothing was sent
"#
    );
}

#[test]
fn resolve_set_unknown_label() {
    let r = run_fixture(
        "ticket_lifecycle_dropdown",
        &[
            "tickets",
            "resolve",
            "128",
            "--note",
            "Acme fix",
            "--set",
            "No Such Field=x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unknown field: 'No Such Field'; nothing was sent
"#
    );
}

#[test]
fn close_set_invalid_choice() {
    let r = run_fixture(
        "ticket_lifecycle_dropdown",
        &[
            "tickets",
            "close",
            "128",
            "--set",
            "Incident Kind=Incident",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Incident Kind: 'Incident' is not a valid choice; choose one of: Service, Event; nothing was sent
"#
    );
}

#[test]
fn close_set_unknown_label() {
    let r = run_fixture(
        "ticket_lifecycle_dropdown",
        &[
            "tickets",
            "close",
            "128",
            "--set",
            "No Such Field=x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unknown field: 'No Such Field'; nothing was sent
"#
    );
}

#[test]
fn status_closed_set_invalid_choice() {
    let r = run_fixture(
        "ticket_lifecycle_dropdown",
        &[
            "tickets",
            "status",
            "128",
            "Closed",
            "--set",
            "Incident Kind=Incident",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Incident Kind: 'Incident' is not a valid choice; choose one of: Service, Event; nothing was sent
"#
    );
}

#[test]
fn status_pending_set_invalid_choice() {
    let r = run_fixture(
        "ticket_lifecycle_dropdown",
        &[
            "tickets",
            "status",
            "128",
            "Pending",
            "--set",
            "Incident Kind=Incident",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"Incident Kind: 'Incident' is not a valid choice; choose one of: Service, Event; nothing was sent
"#
    );
}

#[test]
fn status_pending_set_unknown_label() {
    let r = run_fixture(
        "ticket_lifecycle_dropdown",
        &[
            "tickets",
            "status",
            "128",
            "Pending",
            "--set",
            "No Such Field=x",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, r#""#);
    assert_eq!(
        r.err,
        r#"unknown field: 'No Such Field'; nothing was sent
"#
    );
}
