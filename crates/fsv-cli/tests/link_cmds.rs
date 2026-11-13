//! Change relationship read refusals (Python 618593b), ticket kind proof
//! from the record's own fields (e12c5ea) and ticket-side problem, change
//! and child-ticket associations (7ea60aa). Routes are synthetic; unmatched
//! requests answer 404.

mod common;
use common::{Env, get};
use serde_json::{Value, json};

fn change_routes() -> Vec<Value> {
    vec![
        get(
            "changes/123",
            json!({"change": {"id": 123, "subject": "Acme change"}}),
        ),
        get(
            "changes/123/releases",
            json!({"releases": [], "meta": {"has_next": false}}),
        ),
        get(
            "changes/123/problems",
            json!({"problems": [], "meta": {"has_next": false}}),
        ),
        get(
            "changes/123/tickets",
            json!({"tickets": [], "meta": {"has_next": false}}),
        ),
        get("releases/71", json!({"release": {"id": 71}})),
    ]
}

/// Python tests/test_change_relationships.py::test_missing_ids_are_one_line_refusals
#[test]
fn change_relationship_missing_ids_are_one_line_refusals() {
    for (kind, missing, named) in [
        (
            "release",
            "/api/_/releases/62",
            "release 62 not found (HTTP 404); nothing was sent",
        ),
        (
            "problem",
            "/api/_/problems/62",
            "problem 62 not found (HTTP 404); nothing was sent",
        ),
        (
            "caused-incident",
            "/api/_/tickets/62",
            "incident 62 not found (HTTP 404); nothing was sent",
        ),
        (
            "release",
            "/api/_/changes/123",
            "change 123 not found (HTTP 404); nothing was sent",
        ),
        (
            "problem",
            "/api/_/changes/123/problems",
            "mutation inventory unavailable",
        ),
    ] {
        let routes: Vec<Value> = change_routes()
            .into_iter()
            .filter(|r| r["path"] != missing)
            .collect();
        let r = Env::new(routes).run(&[
            "changes",
            "relationships",
            "write",
            "CHN-123",
            kind,
            "add",
            "--item",
            "62",
            "--dry-run",
            "--json",
        ]);
        assert_eq!(r.code, 2, "{kind} {missing}: {}", r.err);
        assert!(r.err.contains(named), "{kind} {missing}: {}", r.err);
        assert!(!r.err.contains("Traceback"));
        assert_eq!(r.err.trim().lines().count(), 1, "{}", r.err);
        assert_eq!(r.out, "");
        assert!(r.writes().is_empty());
    }
}

#[test]
fn change_relationship_list_api_failure_is_one_line() {
    let r =
        Env::new(change_routes()).run(&["changes", "relationships", "list", "CHN-123", "project"]);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(r.out, "");
    assert_eq!(
        r.err,
        "the project relationships of CHN-123 not found (HTTP 404)\n"
    );
}

/// e12c5ea: a renamed type with a custom display-id prefix proves nothing
/// until the tenant declares the prefix in the config.
#[test]
fn caused_incident_proof_uses_configured_prefix() {
    let mut routes = change_routes();
    routes.push(get(
        "tickets/62",
        json!({"ticket": {"id": 62, "display_id": 62, "type": "Fault", "human_display_id": "IT-62"}}),
    ));
    let env = Env::new(routes);
    let args = [
        "changes",
        "relationships",
        "write",
        "CHN-123",
        "caused-incident",
        "add",
        "--item",
        "62",
        "--dry-run",
    ];
    let r = env.run(&args);
    assert_eq!(r.code, 2, "{}", r.err);
    assert_eq!(
        r.err,
        "caused relationship requires a proven incident; the record's own fields do not prove it \
         (type='Fault', human_display_id='IT-62'). A tenant with a custom display-id prefix can \
         declare it: fsv config set ids.incident_prefix PREFIX\n"
    );
    let set = env.run(&["config", "set", "ids.incident_prefix", "IT"]);
    assert_eq!(set.code, 0, "{}", set.err);
    let r = env.run(&args);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json()["path"], "/api/_/changes/123/tickets/associate");
    assert!(r.writes().is_empty());
}

/// Python output of the same commands on the `tenant` fake of
/// tests/test_ticket_associations.py (domain acme.freshservice.com).
fn golden(name: &str) -> Value {
    let all: Value = serde_json::from_str(include_str!("golden/ticket_links_cli.json")).unwrap();
    all[name].clone()
}

fn assert_golden(r: &common::Run, name: &str) {
    let want = golden(name);
    assert_eq!(
        r.code,
        want["code"].as_i64().unwrap() as i32,
        "{name}: {}",
        r.err
    );
    assert_eq!(r.out, want["stdout"].as_str().unwrap(), "{name}");
    assert_eq!(r.err, want["stderr"].as_str().unwrap(), "{name}");
}

fn ticket(t: i64, kind: &str, prefix: &str) -> Value {
    get(
        &format!("tickets/{t}"),
        json!({"ticket": {"id": t, "type": kind, "subject": format!("Acme {t}"),
                          "human_display_id": format!("{prefix}-{t}")}}),
    )
}

fn changes_list(ids: &[i64], once: bool) -> Value {
    let rows: Vec<Value> = ids.iter().map(|i| json!({"id": i})).collect();
    let mut r = get(
        "tickets/41/changes",
        json!({"changes": rows, "meta": {"has_next": false}}),
    );
    r["query"] = json!({"change_type": "change"});
    r["once"] = json!(once);
    r
}

/// The `tenant` routes before any write.
fn ticket_routes() -> Vec<Value> {
    let mut picker = get(
        "tickets/41/changes",
        json!({"changes": [{"id": 9, "subject": "Globex patch"}], "meta": {"has_next": false}}),
    );
    picker["query"] = json!({"filter": "list"});
    let mut dashboard = common::route("GET", "/a/dashboard", 200, Value::Null);
    dashboard["text"] =
        json!(r#"<html><head><meta name="csrf-token" content="synthetic-token"></head></html>"#);
    vec![
        dashboard,
        ticket(41, "Incident", "INC"),
        ticket(43, "Service Request", "SR"),
        get(
            "problems/7",
            json!({"problem": {"id": 7, "subject": "Acme outage"}}),
        ),
        get(
            "changes/9",
            json!({"change": {"id": 9, "subject": "Globex patch"}}),
        ),
        picker,
        get(
            "tickets/41/suggest_child_tickets",
            json!({"tickets": [{"id": 42, "subject": "Acme printer"},
                               {"id": 44, "subject": "Globex laptop"}]}),
        ),
        get(
            "problems/search",
            json!({"problems": [{"id": 7, "subject": "Acme outage", "status": 1}]}),
        ),
    ]
}

/// tests/test_ticket_associations.py::test_cli_candidates_review_gate_and_kind_required
#[test]
fn ticket_links_cli_candidates_review_gate_and_kind_required() {
    let mut routes = ticket_routes();
    routes.push(changes_list(&[], false));
    let env = Env::new(routes);
    let r = env.run(&[
        "tickets",
        "children",
        "candidates",
        "INC-41",
        "--search",
        "printer",
    ]);
    assert_golden(&r, "cand_child");
    let ids: Vec<Value> = r.json()["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].clone())
        .collect();
    assert_eq!(ids, [json!(42)]);
    let r = env.run(&[
        "tickets",
        "problems",
        "candidates",
        "INC-41",
        "--search",
        "acme",
    ]);
    assert_eq!(r.json()["candidates"][0]["id"], 7);
    let r = env.run(&[
        "tickets",
        "changes",
        "candidates",
        "INC-41",
        "--kind",
        "initiating",
    ]);
    assert_eq!(r.json()["candidates"][0]["id"], 9);
    let r = env.run(&["tickets", "changes", "candidates", "INC-41"]);
    assert_golden(&r, "cand_no_kind");
    assert_golden(
        &env.run(&["tickets", "changes", "link", "INC-41", "CHN-9", "--dry-run"]),
        "no_kind",
    );
    assert_golden(
        &env.run(&["tickets", "changes", "list", "INC-41"]),
        "list_no_kind",
    );
    assert_golden(
        &env.run(&[
            "tickets",
            "problems",
            "link",
            "INC-41",
            "PRB-7",
            "--kind",
            "initiating",
        ]),
        "kind_on_problem",
    );
    let args = [
        "tickets",
        "changes",
        "link",
        "INC-41",
        "CHN-9",
        "--kind",
        "initiated-by",
    ];
    let review = env.run(&[&args[..], &["--dry-run"]].concat());
    assert_golden(&review, "dry");
    let digest = review.json()["plan_hash"].as_str().unwrap().to_string();
    let gate = env.run(&[&args[..], &["--yes"]].concat());
    assert_golden(&gate, "gate");
    assert!(gate.writes().is_empty());
    let missing = env.run(&[
        "tickets",
        "problems",
        "link",
        "INC-41",
        "PRB-77",
        "--dry-run",
    ]);
    assert_golden(&missing, "missing");
    assert_eq!(missing.err.trim().lines().count(), 1);

    let mut routes = ticket_routes();
    routes.extend([
        changes_list(&[], true),
        changes_list(&[], true),
        changes_list(&[9], false),
    ]);
    routes.push(common::route(
        "PUT",
        "/api/_/tickets/41/changes/associate",
        200,
        json!({}),
    ));
    let done = Env::new(routes).run(&[&args[..], &["--yes", "--expect-plan", &digest]].concat());
    assert_golden(&done, "send");
    assert_eq!(
        done.writes(),
        [(
            "PUT".to_string(),
            "/api/_/tickets/41/changes/associate".to_string(),
            json!({"change_type": "change", "item_ids": [9], "type": "link"}),
        )]
    );

    let mut routes = ticket_routes();
    routes.push(changes_list(&[9], false));
    let listed = Env::new(routes).run(&[
        "tickets",
        "changes",
        "list",
        "INC-41",
        "--kind",
        "initiated-by",
    ]);
    assert_golden(&listed, "list");
}

#[test]
fn ticket_links_cli_problem_link_refuses_service_request_and_read_failures() {
    let mut routes = ticket_routes();
    routes.push(common::route(
        "GET",
        "/api/_/tickets/41/problems",
        403,
        json!({}),
    ));
    let env = Env::new(routes);
    let r = env.run(&["tickets", "problems", "link", "SR-43", "PRB-7", "--dry-run"]);
    assert_eq!(r.code, 2);
    assert_eq!(
        r.err,
        "problems associate with incidents only; this ticket is not a verified incident\n"
    );
    let r = env.run(&["tickets", "problems", "list", "INC-41"]);
    assert_eq!(r.code, 2);
    assert_eq!(
        r.err,
        "complete associated problems list unavailable; nothing was sent\n"
    );
    let r = env.run(&["tickets", "problems", "candidates", "INC-41"]);
    assert_eq!(r.code, 2);
    assert_eq!(r.err, "read failed (HTTP 403); nothing was sent\n");
    let r = env.run(&[
        "tickets",
        "children",
        "link",
        "INC-41",
        "INC-41",
        "--dry-run",
    ]);
    assert_eq!(r.code, 2);
    assert_eq!(r.err, "a ticket cannot be its own child\n");
    assert!(r.writes().is_empty());
}

fn tabs(t: i64, level: i64, times: usize) -> Vec<Value> {
    let mut r = get(
        &format!("tickets/{t}/tabs"),
        json!([{"name": "related_tickets", "count": 0, "hierarchy": level},
               {"name": "associations", "count": 0}]),
    );
    if times == 0 {
        return vec![r];
    }
    r["once"] = json!(true);
    vec![r; times]
}

fn child_routes(levels: &[(i64, i64, usize)], children: &[i64]) -> Vec<Value> {
    let mut routes = ticket_routes();
    routes.push(ticket(42, "Incident", "INC"));
    for (t, level, times) in levels {
        routes.extend(tabs(*t, *level, *times));
    }
    let rows: Vec<Value> = children
        .iter()
        .map(|c| json!({"id": c, "deleted": false, "spam": false}))
        .collect();
    let mut list = get(
        "tickets/41/child-tickets",
        json!({"child_tickets": rows, "meta": {"has_next": false}}),
    );
    list["query"] = json!({"type": "active"});
    routes.push(list);
    routes
}

/// tests/test_ticket_associations.py::test_child_link_unlink_hierarchy_and_404_free_readback
/// through the real client: POST link, then DELETE with a query string, no
/// body and a 204 answer.
#[test]
fn ticket_links_cli_child_link_unlink_wires() {
    let args = ["tickets", "children", "link", "INC-41", "INC-42"];
    let mut routes = child_routes(&[(41, 0, 4), (41, 2, 0), (42, 0, 2), (42, 1, 0)], &[42]);
    routes.push(common::route(
        "POST",
        "/api/_/tickets/41/associate-child-tickets",
        200,
        json!({"success": true}),
    ));
    let env = Env::new(routes);
    let dry = env.run(&[&args[..], &["--dry-run"]].concat());
    assert_golden(&dry, "child_link_dry");
    let digest = dry.json()["plan_hash"].as_str().unwrap().to_string();
    let done = env.run(&[&args[..], &["--yes", "--expect-plan", &digest]].concat());
    assert_golden(&done, "child_link_send");
    assert_eq!(
        done.writes(),
        [(
            "POST".to_string(),
            "/api/_/tickets/41/associate-child-tickets".to_string(),
            json!({"ticket_ids": [42]}),
        )]
    );

    let args = ["tickets", "children", "unlink", "INC-41", "INC-42"];
    let mut routes = child_routes(&[(41, 2, 4), (41, 0, 0), (42, 1, 2), (42, 0, 0)], &[42]);
    let mut delete = common::route("DELETE", "/api/_/tickets/41/remove_child", 204, Value::Null);
    delete["query"] = json!({"ticket_id": "42"});
    routes.push(delete);
    let env = Env::new(routes);
    let dry = env.run(&[&args[..], &["--dry-run"]].concat());
    assert_golden(&dry, "child_unlink_dry");
    let digest = dry.json()["plan_hash"].as_str().unwrap().to_string();
    let done = env.run(&[&args[..], &["--yes", "--expect-plan", &digest]].concat());
    assert_golden(&done, "child_unlink_send");
    let writes = done.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        (writes[0].0.as_str(), writes[0].1.as_str()),
        ("DELETE", "/api/_/tickets/41/remove_child?ticket_id=42")
    );
    assert!(writes[0].2.is_null(), "{:?}", writes[0].2);
}
