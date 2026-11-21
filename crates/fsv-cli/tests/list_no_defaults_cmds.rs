//! `ls --no-defaults` with no filter lists the module's system All view
//! (`tests/test_list_no_defaults.py`).

mod common;
use common::{Env, fixture};
use serde_json::{Value, json};

fn routes(extra_view: Option<Value>) -> Vec<Value> {
    fixture("ls")
        .into_iter()
        .map(|mut r| {
            if r["path"] == "/api/_/ticket_filters" {
                let views = r["json"]["ticket_filters"].as_array_mut().unwrap();
                views.retain(|v| v["id"] != json!(101));
                if let Some(v) = &extra_view {
                    views.push(v.clone());
                }
            }
            r
        })
        .collect()
}

fn list_filters(r: &common::Run, module: &str) -> Vec<String> {
    r.requests
        .iter()
        .filter(|q| {
            q["method"] == "GET"
                && q["url"]
                    .as_str()
                    .is_some_and(|u| u.contains(&format!("/api/_/{module}?")))
        })
        .map(|q| {
            let url = q["url"].as_str().unwrap();
            url.split(['?', '&'])
                .find_map(|p| p.strip_prefix("filter="))
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

#[test]
fn no_defaults_lists_the_system_all_view() {
    let env = Env::new(routes(Some(
        json!({"id": "all_tickets", "name": "All Tickets", "default": true}),
    )));
    let r = env.run(&["tickets", "ls", "--no-defaults", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(list_filters(&r, "tickets"), ["all_tickets"]);
    assert!(
        r.err.contains("view: All Tickets (all_tickets): --no-defaults lists every record the server's system 'All' view returns, not the default view"),
        "{}",
        r.err
    );
}

#[test]
fn without_all_view_the_default_view_is_named() {
    let env = Env::new(routes(None));
    let r = env.run(&["tickets", "ls", "--no-defaults", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(list_filters(&r, "tickets"), ["new_and_my_open"]);
    assert!(
        r.err
            .contains("Open in my groups (new_and_my_open) still filters this list"),
        "{}",
        r.err
    );
}

#[test]
fn custom_all_named_view_is_not_taken_for_all() {
    let env = Env::new(routes(Some(
        json!({"id": 9001, "name": "All Acme Tickets", "user_id": 99}),
    )));
    let r = env.run(&["tickets", "ls", "--no-defaults", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(list_filters(&r, "tickets"), ["new_and_my_open"]);
}

#[test]
fn default_list_still_uses_the_server_default_view() {
    let env = Env::new(routes(Some(
        json!({"id": "all_tickets", "name": "All Tickets", "default": true}),
    )));
    let r = env.run(&["tickets", "ls", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(list_filters(&r, "tickets"), ["new_and_my_open"]);
    assert!(!r.err.contains("--no-defaults"), "{}", r.err);
}

#[test]
fn explicit_view_wins_over_no_defaults() {
    let env = Env::new(routes(Some(
        json!({"id": "all_tickets", "name": "All Tickets", "default": true}),
    )));
    let r = env.run(&[
        "tickets",
        "ls",
        "--no-defaults",
        "--view",
        "Open in my groups",
        "--json",
    ]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(list_filters(&r, "tickets"), ["new_and_my_open"]);
}
