//! Deletes and bulk parity regressions; expected values from Python fsv
//! at 31a9103.

mod common;
use common::Env;
use serde_json::{Value, json};

#[test]
fn deletes_without_session_exit_3_before_any_check() {
    for args in [
        &["tickets", "trash", "list"][..],
        &["tickets", "restore", "5"],
        &["tickets", "spam", "5"],
        &["tickets", "unspam", "5"],
        &["tickets", "delete-forever", "5"],
        &["problems", "delete", "5"],
        &["problems", "restore", "5"],
        &["problems", "delete-forever", "5"],
        &["solutions", "delete", "5"],
        &["solutions", "delete-forever", "5"],
    ] {
        let mut env = Env::new(vec![]);
        env.session = false;
        let r = env.run(args);
        assert_eq!(r.code, 3, "{args:?}: {}", r.err);
        assert_eq!(r.out, "", "{args:?}");
        assert_eq!(r.err, "error: no session; run `fsv auth login`\n", "{args:?}");
        assert!(r.requests.is_empty(), "{args:?}");
    }
}

#[test]
fn bulk_set_is_parsed_before_the_session() {
    let mut env = Env::new(vec![]);
    env.session = false;
    let r = env.run(&["tickets", "bulk", "status", "5", "--set", "bad", "--status", "Open"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.err, "--set requires unique FIELD=VALUE answers\n");
}

fn bulk_where_routes() -> Vec<Value> {
    let ticket = |i: i64| {
        json!({"ticket": {"id": i, "human_display_id": format!("INC-{i}"),
            "subject": format!("Acme ticket {i}"), "status": 2, "responder_id": 77,
            "spam": false, "deleted": false}})
    };
    vec![
        json!({"method": "GET", "path": "/api/_/ticket_form_fields", "json": {"ticket_fields": [
            {"name": "status", "label": "Status", "field_type": "default_status", "default_field": true,
             "choices": [{"id": 2, "value": "Open"}, {"id": 5, "value": "Closed"}]},
            {"name": "agent", "label": "Agent", "field_type": "default_agent", "default_field": true}]}}),
        json!({"method": "GET", "path": "/api/v2/ticket_form_fields", "json": {}}),
        json!({"method": "GET", "path": "/search/autocomplete/agents",
               "json": {"results": [{"id": 9, "user_id": 77, "value": "Jane"}]}}),
        json!({"method": "GET", "path": "/api/_/tickets", "json": {"tickets": [
            {"id": 140, "subject": "Acme ticket 140", "status": 2},
            {"id": 141, "subject": "Acme ticket 141", "status": 2}], "meta": {"has_next": false}}}),
        json!({"method": "GET", "path": "/api/_/tickets/140", "json": ticket(140)}),
        json!({"method": "GET", "path": "/api/_/tickets/141", "json": ticket(141)}),
    ]
}

const BULK_WHERE_OUT: &str = r#"{
  "status": "dry_run",
  "action": "ticket_bulk_spam",
  "method": "POST",
  "path": "/api/_/tickets/spam_multiple",
  "payload": {
    "ids": [
      140,
      141
    ]
  },
  "plan_hash": "0ec3781638748104f9971f8dabc82497cb29014c6a030a2cf4e4dc4e104c5907",
  "warning": "Frozen explicit IDs, not server select-all. Partial outcomes are possible; never replay an uncertain bulk action.",
  "ticket_ids": [
    140,
    141
  ],
  "selection": {
    "action": "spam",
    "agent": null,
    "group": null,
    "ids": [],
    "max_records": 5,
    "note_primary": null,
    "note_secondary": null,
    "primary": null,
    "select_all": true,
    "status": null,
    "where": [
      "agent=Jane"
    ]
  },
  "selected_tickets": [
    {
      "id": 140,
      "subject": "Acme ticket 140"
    },
    {
      "id": 141,
      "subject": "Acme ticket 141"
    }
  ]
}
"#;

#[test]
fn bulk_where_resolves_agent_names_and_caches_the_schema() {
    let env = Env::new(bulk_where_routes());
    let args = [
        "tickets", "bulk", "spam", "--where", "agent=Jane", "--select-all", "--max-records", "5",
        "--dry-run",
    ];
    let query = "/api/_/tickets?query_hash=%5B%7B%22value%22%3A%5B%2277%22%5D%2C%22condition%22%3A%22responder_id%22%2C%22operator%22%3A%22is_in%22%2C%22type%22%3A%22default%22%7D%5D&page=1&per_page=100";
    let tail = [
        "/search/autocomplete/agents?q=Jane&qf=value",
        query,
        "/api/_/tickets/140",
        "/api/_/tickets/141",
        "/api/_/tickets/140",
        "/api/_/tickets/141",
        "/api/_/tickets/140",
        "/api/_/tickets/141",
    ];
    for (round, head) in [
        &["/api/_/ticket_form_fields", "/api/v2/ticket_form_fields"][..],
        &[],
    ]
    .into_iter()
    .enumerate()
    {
        let r = env.run(&args);
        assert_eq!(r.code, 0, "round {round}: {}", r.err);
        assert_eq!(r.out, BULK_WHERE_OUT);
        assert_eq!(
            r.err,
            "Selected ticket IDs: 140, 141\n  140: Acme ticket 140\n  141: Acme ticket 141\n"
        );
        let paths: Vec<String> = r.calls().into_iter().map(|(_, p)| p).collect();
        let want: Vec<&str> = head.iter().chain(tail.iter()).copied().collect();
        assert_eq!(paths, want, "round {round}");
    }
}
