//! `fsv templates`, `fsv changes windows`, `tickets create --template` and
//! `tickets add-note --notify-agent` (Python `tests/test_templates_windows.py`,
//! CLI halves). The library behavior (overlap shapes, zones, hashes) is
//! tested in `fsv_api::change_windows`.

mod common;
use common::{Env, fixture, get};
use serde_json::{Value, json};

fn templates() -> Vec<Value> {
    vec![
        get(
            "ticket-templates",
            json!({"ticket_templates": [{"id": 501, "name": "Acme outage", "type": "shared", "workspace_id": 3},
                                        {"id": 502, "name": "Globex access", "type": "shared", "workspace_id": 3}],
                   "meta": {"max_template_count": 2}}),
        ),
        get(
            "ticket-templates/501",
            json!({"ticket_template": {"id": 501, "name": "Acme outage", "template_data": {
                "subject": "Acme outage", "description": "<p>Please help</p>", "status": null, "tags": ["x"]}}}),
        ),
    ]
}

fn run(routes: Vec<Value>, args: &[&str]) -> common::Run {
    Env::new(routes).run(args)
}

#[test]
fn templates_list_and_show() {
    let r = run(templates(), &["templates", "list"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.out,
        "501\tAcme outage\tshared\n502\tGlobex access\tshared\n"
    );
    let r = run(templates(), &["templates", "list", "--json"]);
    assert_eq!(r.json()[1]["name"], "Globex access");
    let r = run(templates(), &["templates", "show", "acme OUTAGE"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.json(),
        json!({"id": 501, "name": "Acme outage", "prefills": ["subject", "description"], "not_applied": ["tags"],
               "template_data": {"subject": "Acme outage", "description": "<p>Please help</p>", "status": null,
                                 "tags": ["x"]}})
    );
    let r = run(templates(), &["templates", "show", "Initech"]);
    assert_eq!(r.code, 2);
    assert_eq!(
        r.err,
        "no unique ticket template 'Initech'; templates: Acme outage, Globex access\n"
    );
    let r = run(
        vec![get("ticket-templates", json!({"ticket_templates": []}))],
        &["templates", "list"],
    );
    assert_eq!(r.out, "No ticket templates.\n");
}

/// `test_create_with_template_goes_through_the_form_review` and
/// `test_template_create_reports_server_changed_prefill`.
#[test]
fn ticket_create_with_template() {
    let mut routes = templates();
    routes.extend(fixture("ticket_create"));
    let args = [
        "tickets",
        "create",
        "--requester",
        "ann@acme.com",
        "--template",
        "Acme outage",
        "--set",
        "Priority=Medium",
        "--json",
    ];
    let review = run(routes.clone(), &[&args[..], &["--dry-run"]].concat());
    assert_eq!(review.code, 0, "{}", review.err);
    assert!(
        review
            .err
            .contains("template 'Acme outage' prefills: subject, description; not applied: tags"),
        "{}",
        review.err
    );
    let doc = review.json();
    assert_eq!(doc["payload"]["subject"], "Acme outage");
    assert!(
        doc["payload"]["description"]
            .as_str()
            .unwrap()
            .contains("Please help")
    );
    assert!(review.write_paths().is_empty());
    let hash = doc["plan_hash"].as_str().unwrap().to_string();

    let r = run(
        routes.clone(),
        &[
            "tickets",
            "create",
            "--requester",
            "ann@acme.com",
            "--template",
            "Initech",
            "--dry-run",
        ],
    );
    assert_eq!(r.code, 2);
    assert!(
        r.err.contains("no unique ticket template 'Initech'"),
        "{}",
        r.err
    );
    assert!(r.write_paths().is_empty());

    for (stored, expect) in [
        (
            Some("Acme outage (P2)"),
            Some("server changed subject from Acme outage to Acme outage (P2)"),
        ),
        (Some("Acme outage"), None),
        (None, Some("could not read the ticket back")),
    ] {
        let mut rs = routes.clone();
        if let Some(subject) = stored {
            rs.insert(
                0,
                get(
                    "tickets/777",
                    json!({"ticket": {"id": 777, "subject": subject, "custom_fields": {}}}),
                ),
            );
        }
        let r = run(
            rs,
            &[&args[..], &["--yes", "--expect-plan", &hash]].concat(),
        );
        assert_eq!(r.code, 0, "{}", r.err);
        let check = &r.json()["template_readback"];
        match expect {
            None => {
                assert_eq!(check["checked"], json!(true));
                assert_eq!(check["changes"], json!([]));
                assert!(!r.err.contains("server changed"));
            }
            Some(e) => assert!(r.err.contains(e), "{}", r.err),
        }
        if stored == Some("Acme outage (P2)") {
            assert_eq!(
                check["changes"],
                json!([{"field": "subject", "sent": "Acme outage", "server": "Acme outage (P2)"}])
            );
        }
        assert_eq!(r.write_paths(), ["POST /api/_/tickets"]);
    }
}

#[test]
fn add_note_notify_agent_resolves_name_and_email() {
    let rows = json!([{"id": "robin@example.com", "value": "Robin Agent", "user_id": 77},
                      {"id": 8, "value": "Robin Agentson", "email": "ra@example.com"}]);
    let routes = vec![
        json!({"method": "GET", "path": "/search/autocomplete/agents",
                             "query": {"qf": "name"}, "json": {"results": rows}}),
    ];
    let r = run(
        routes.clone(),
        &[
            "tickets",
            "add-note",
            "5",
            "--body",
            "hi",
            "--notify-agent",
            "Robin Agent",
            "--notify-email",
            "ops@example.com",
            "--dry-run",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.json()["payload"]["notify_emails"],
        json!(["ops@example.com", "Robin Agent <robin@example.com>"])
    );
    let r = run(
        routes,
        &[
            "tickets",
            "add-note",
            "5",
            "--body",
            "hi",
            "--notify-agent",
            "Robin",
            "--dry-run",
        ],
    );
    assert_eq!(r.code, 2);
    assert_eq!(
        r.err,
        "no unique active agent 'Robin' for --notify-agent; use the exact name or email\n"
    );
}
