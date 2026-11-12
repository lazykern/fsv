//! Every golden fixture this crate loads records the Python commit it was
//! generated from (`port/golden/provenance.py`).

use serde_json::Value;

fn assert_stamped(name: &str, text: &str) {
    let doc: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let rev = &doc["_python"];
    let commit = rev["commit"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: no _python.commit"));
    assert!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "{name}: {commit}"
    );
    assert!(rev["dirty"].is_boolean(), "{name}: _python.dirty");
}

#[test]
fn fixtures_record_their_python_commit() {
    for (name, text) in [
        (
            "catalog_oracle",
            include_str!("testdata/catalog_oracle.json"),
        ),
        (
            "coverage_oracle",
            include_str!("testdata/coverage_oracle.json"),
        ),
        ("create_oracle", include_str!("testdata/create_oracle.json")),
        (
            "failure_matrix_oracle",
            include_str!("testdata/failure_matrix_oracle.json"),
        ),
        (
            "query_flow_oracle",
            include_str!("testdata/query_flow_oracle.json"),
        ),
        (
            "parseaddr_oracle",
            include_str!("testdata/parseaddr_oracle.json"),
        ),
        (
            "reviewed_oracle",
            include_str!("testdata/reviewed_oracle.json"),
        ),
        ("views_oracle", include_str!("testdata/views_oracle.json")),
        (
            "closure_conversation_oracle",
            include_str!("testdata/closure_conversation_oracle.json"),
        ),
        (
            "note_readback_oracle",
            include_str!("testdata/note_readback_oracle.json"),
        ),
        (
            "sr_attachments_oracle",
            include_str!("testdata/sr_attachments_oracle.json"),
        ),
        (
            "ticket_assignment_oracle",
            include_str!("testdata/ticket_assignment_oracle.json"),
        ),
        (
            "ticket_lifecycle_oracle",
            include_str!("testdata/ticket_lifecycle_oracle.json"),
        ),
        (
            "ticket_merge_oracle",
            include_str!("testdata/ticket_merge_oracle.json"),
        ),
        (
            "workflows_oracle",
            include_str!("testdata/workflows_oracle.json"),
        ),
        (
            "attachments_oracle",
            include_str!("testdata/attachments_oracle.json"),
        ),
        (
            "problem_incidents_oracle",
            include_str!("testdata/problem_incidents_oracle.json"),
        ),
        (
            "problem_status_oracle",
            include_str!("testdata/problem_status_oracle.json"),
        ),
        (
            "ticket_note_mutation_oracle",
            include_str!("testdata/ticket_note_mutation_oracle.json"),
        ),
        (
            "ticket_tasks_oracle",
            include_str!("testdata/ticket_tasks_oracle.json"),
        ),
        (
            "integrated_assets_oracle",
            include_str!("testdata/integrated_assets_oracle.json"),
        ),
        (
            "sr_approvals_oracle",
            include_str!("testdata/sr_approvals_oracle.json"),
        ),
        (
            "change_task_update_oracle",
            include_str!("testdata/change_task_update_oracle.json"),
        ),
        (
            "solutions_oracle",
            include_str!("testdata/solutions_oracle.json"),
        ),
        (
            "approval_recipients_oracle",
            include_str!("testdata/approval_recipients_oracle.json"),
        ),
        (
            "change_tasks_required_oracle",
            include_str!("testdata/change_tasks_required_oracle.json"),
        ),
    ] {
        assert_stamped(name, text);
    }
}
