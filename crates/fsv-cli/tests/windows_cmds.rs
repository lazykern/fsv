//! `fsv templates`, `fsv changes windows`, `tickets create --template` and
//! `tickets add-note --notify-agent` (Python `tests/test_templates_windows.py`,
//! CLI halves). The library behavior (overlap shapes, zones, hashes) is
//! tested in `fsv_api::change_windows`.

mod common;
use common::{Env, fixture, get, route};
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

fn change_schema() -> Value {
    get(
        "change_form_fields",
        json!({"change_fields": [{"name": "change_type", "label": "Change Type", "field_type": "default_change_type",
                                  "default_field": true, "workspace_id": 3,
                                  "choices": [{"id": 1, "value": "Normal"}, {"id": 4, "value": "Emergency"}]}]}),
    )
}

fn agent(time_zone: Option<&str>) -> Value {
    let mut a = json!({"id": 7, "first_name": "Robin", "last_name": "Agent"});
    if let Some(tz) = time_zone {
        a["time_zone"] = json!(tz);
    }
    route("GET", "/api/v2/agents/me", 200, json!({"agent": a}))
}

fn csrf() -> Value {
    json!({"method": "GET", "path": "/a/dashboard",
           "text": "<html><head><meta name=\"csrf-token\" content=\"tok\"></head></html>"})
}

fn window_routes(windows: Value) -> Vec<Value> {
    vec![
        csrf(),
        agent(Some("Eastern Time (US & Canada)")),
        change_schema(),
        get(
            "maintenance_windows",
            json!({"maintenance_windows": windows}),
        ),
    ]
}

fn globex() -> Value {
    json!([{"id": 61, "name": "Globex freeze", "start_time": "2030-11-20T09:00:00Z", "end_time": "2030-11-20T10:00:00Z"}])
}

fn ny_ac() -> Value {
    json!([{"id": 71, "name": "Acme A", "start_time": "2030-11-14T02:00:00-05:00", "end_time": "2030-11-14T04:00:00-05:00"},
           {"id": 72, "name": "Acme C", "start_time": "2030-11-14T04:00:00-05:00", "end_time": "2030-11-14T06:00:00-05:00"}])
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

/// `test_window_delete_and_cli`.
#[test]
fn windows_list_delete_and_create_dry_run() {
    let r = run(
        window_routes(globex()),
        &["changes", "windows", "list", "--json"],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json()[0]["id"], 61);
    let r = run(window_routes(globex()), &["changes", "windows", "list"]);
    assert_eq!(
        r.out,
        "61\tGlobex freeze\t2030-11-20T09:00:00Z -> 2030-11-20T10:00:00Z\n"
    );

    let r = run(
        window_routes(globex()),
        &["changes", "windows", "delete", "61", "--dry-run"],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let h = r.json()["plan_hash"].as_str().unwrap().to_string();
    let mut routes = vec![
        csrf(),
        json!({"method": "GET", "path": "/api/_/maintenance_windows", "once": true,
               "json": {"maintenance_windows": globex()}}),
        json!({"method": "GET", "path": "/api/_/maintenance_windows", "once": true,
               "json": {"maintenance_windows": globex()}}),
        get("maintenance_windows", json!({"maintenance_windows": []})),
        route(
            "DELETE",
            "/api/_/maintenance_windows/61",
            200,
            json!({"success": true, "message": "delete_success"}),
        ),
    ];
    let r = run(
        routes.clone(),
        &[
            "changes",
            "windows",
            "delete",
            "61",
            "--yes",
            "--expect-plan",
            &h,
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json()["fields"], json!({"absent": "matched"}));
    assert_eq!(
        r.writes(),
        [(
            "DELETE".to_string(),
            "/api/_/maintenance_windows/61".to_string(),
            Value::Null
        )]
    );

    routes.truncate(3);
    let r = run(
        routes,
        &[
            "changes",
            "windows",
            "delete",
            "61",
            "--yes",
            "--expect-plan",
            "abc",
        ],
    );
    assert_eq!(r.code, 2);
    assert_eq!(
        r.err,
        "review with --dry-run, then --yes --expect-plan FULL_HASH\n"
    );
    assert!(r.write_paths().is_empty());

    let r = run(
        window_routes(globex()),
        &[
            "changes",
            "windows",
            "create",
            "--name",
            "Acme",
            "--start",
            "2030-11-21 10:00",
            "--end",
            "2030-11-21 11:00",
            "--criteria",
            "change_type=Emergency",
            "--dry-run",
        ],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        r.json()["overlap_check"]["payload"]["criteria"][0]["value"],
        "4"
    );
    assert!(r.write_paths().is_empty());
}

/// `test_window_unknown_tenant_zone_refuses_even_with_fsv_tz` (CLI half).
#[test]
fn windows_create_unknown_tenant_zone_refuses() {
    let mut env = Env::new(vec![
        agent(None),
        change_schema(),
        get("maintenance_windows", json!({"maintenance_windows": []})),
    ]);
    env.vars.push(("FSV_TZ".into(), "Asia/Bangkok".into()));
    let r = env.run(&[
        "changes",
        "windows",
        "create",
        "--name",
        "Acme",
        "--start",
        "2030-11-14 08:00",
        "--end",
        "2030-11-14 09:00",
        "--criteria",
        "change_type=Normal",
        "--dry-run",
    ]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("tenant time zone unknown"), "{}", r.err);
    assert!(r.write_paths().is_empty());
}

fn create_args<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut v = vec![
        "changes",
        "windows",
        "create",
        "--name",
        "Acme",
        "--start",
        "2030-11-14 15:00",
        "--end",
        "2030-11-14 17:00",
        "--criteria",
        "change_type=Normal",
        "--timezone",
        "Asia/Bangkok",
    ];
    v.extend_from_slice(extra);
    v
}

/// `test_window_cli_timezone_option_reviews_both_renderings`,
/// `test_window_dry_run_reports_overlapping_ids` and
/// `test_window_allow_overlap_changes_hash_and_warns` (CLI halves).
#[test]
fn windows_create_dry_run_send_and_allow_overlap() {
    let r = run(window_routes(json!([])), &create_args(&["--dry-run"]));
    assert_eq!(r.code, 0, "{}", r.err);
    let doc = r.json();
    assert_eq!(doc["status"], "dry_run");
    // Plan hashes from the Python CLI dry-run on the same routes.
    assert_eq!(
        doc["plan_hash"],
        "edf7333c8c77daa4b935168a015ec06a60599dec24e7164dd7037ad305313187"
    );
    assert_eq!(
        doc["end"],
        json!({"input": "2030-11-14 17:00 Asia/Bangkok (+07:00)",
                                  "tenant": "2030-11-14 05:00 America/New_York (-05:00)"})
    );
    assert_eq!(doc["payload"]["change_window"]["start_time"], "3:00am");
    assert_eq!(doc["overlapping_window_ids"], json!([]));
    assert!(doc.get("overlap_warning").is_none());
    assert_eq!(
        r.calls()
            .iter()
            .filter(|(_, p)| p == "/api/v2/agents/me")
            .count(),
        1
    );

    let plain = run(window_routes(ny_ac()), &create_args(&["--dry-run"])).json();
    let allowed = run(
        window_routes(ny_ac()),
        &create_args(&["--dry-run", "--allow-overlap"]),
    )
    .json();
    assert_eq!(
        plain["plan_hash"],
        "a44a756d708a1e2b64260099282adce0827ea3acf569642c42409fd60d604d13"
    );
    assert_eq!(
        allowed["plan_hash"],
        "810e45410cc40db30fafc182fddae7615e5512028b19b4adfeeeb9dc503a4a94"
    );
    assert_eq!(
        (&plain["allow_overlap"], &allowed["allow_overlap"]),
        (&json!(false), &json!(true))
    );
    assert_eq!(
        plain["overlap_warning"],
        "overlapping windows exist: 71, 72; sending refuses unless --allow-overlap"
    );
    assert_eq!(
        allowed["overlap_warning"],
        "overlapping windows exist: 71, 72; you chose to create it anyway (--allow-overlap)"
    );
    let ph = plain["plan_hash"].as_str().unwrap();
    let ah = allowed["plan_hash"].as_str().unwrap();

    let r = run(
        window_routes(ny_ac()),
        &create_args(&["--yes", "--expect-plan", ph]),
    );
    assert_eq!(r.code, 2);
    assert!(r.err.contains("already listed (71, 72)"), "{}", r.err);
    assert!(r.write_paths().is_empty());
    for (extra, hash) in [(&["--allow-overlap"][..], ph), (&[][..], ah)] {
        let mut a = extra.to_vec();
        a.extend(["--yes", "--expect-plan", hash]);
        let r = run(window_routes(ny_ac()), &create_args(&a));
        assert_eq!(r.code, 2);
        assert!(r.write_paths().is_empty());
    }

    let stored = json!({"id": 62, "name": "Acme", "start_time": "2030-11-14T03:00:00-05:00",
                        "end_time": "2030-11-14T05:00:00-05:00"});
    let mut after = ny_ac();
    after.as_array_mut().unwrap().push(stored.clone());
    let mut routes = vec![
        csrf(),
        agent(Some("Eastern Time (US & Canada)")),
        change_schema(),
    ];
    for _ in 0..3 {
        routes.push(
            json!({"method": "GET", "path": "/api/_/maintenance_windows", "once": true,
                           "json": {"maintenance_windows": ny_ac()}}),
        );
    }
    routes.push(get(
        "maintenance_windows",
        json!({"maintenance_windows": after}),
    ));
    routes.push(route(
        "POST",
        "/api/_/change_windows/overlapping_windows",
        200,
        json!({"match_found": false}),
    ));
    routes.push(route(
        "POST",
        "/api/_/maintenance_windows",
        201,
        json!({"maintenance_window": stored}),
    ));
    let r = run(
        routes,
        &create_args(&["--allow-overlap", "--yes", "--expect-plan", ah]),
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let out = r.json();
    assert_eq!(out["verified"], json!(true));
    assert_eq!(out["overlapping_window_ids"], json!([71, 72]));
    assert_eq!(
        out["fields"],
        json!({"name": "matched", "start": "matched", "end": "matched"})
    );
    assert_eq!(
        r.write_paths(),
        [
            "POST /api/_/change_windows/overlapping_windows",
            "POST /api/_/maintenance_windows"
        ]
    );
    let overlap_body = &r.writes()[0].2;
    assert_eq!(overlap_body["type"], "blackout_events");
}
