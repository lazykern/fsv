//! Port of tests/test_legacy_write_guards.py: caller-level legacy transport
//! checks, not durable-plan/readback certification.
//!
//! These wrappers predate reviewed plans. The tests pin no automatic replay
//! and CSRF preparation at each caller over a real `fsv_core::Client`;
//! transport audit rows alone are not restart guards.
#![allow(clippy::result_large_err)]

use std::sync::{Arc, Mutex};

use fsv_api::{create, service};
use fsv_core::client::mock::MockTransport;
use fsv_core::client::{Request, Response, TransportError};
use fsv_core::config::Paths;
use fsv_core::resource::CHANGES;
use fsv_core::{Client, Ctx, Result};
use serde_json::{Map, Value, json};

const CASES: [(&str, &str, &str, Option<&str>); 7] = [
    (
        "note",
        "POST",
        "changes/123/notes",
        Some(r#"{"body": "Acme evidence", "private": true}"#),
    ),
    ("status", "PUT", "changes/123", Some(r#"{"status": 47}"#)),
    (
        "asset-add",
        "PUT",
        "changes/123/assets/associate",
        Some(r#"{"item_ids": [701]}"#),
    ),
    (
        "asset-remove",
        "PUT",
        "changes/123/assets/detach",
        Some(r#"{"cmdb_request_id": 801}"#),
    ),
    (
        "task-update",
        "PUT",
        "changes/123/tasks/701",
        Some(r#"{"title": "Acme revised"}"#),
    ),
    (
        "task-delete",
        "PUT",
        "changes/123/tasks/update",
        Some(r#"{"id": "701", "action": "delete"}"#),
    ),
    ("clone-task", "POST", "changes/456/tasks", None),
];

const CSRF_PAGE: &str =
    r#"<html><head><meta name="csrf-token" content="synthetic-token"></head></html>"#;

#[derive(Clone, Copy, Debug)]
enum Failure {
    None,
    Status(u16),
    Disconnect,
}

struct Wire {
    client: Client,
    mutations: Arc<Mutex<Vec<Request>>>,
    audit: Arc<Mutex<Vec<(String, bool)>>>,
    _home: tempfile::TempDir,
}

fn path_of(url: &str) -> String {
    let path = url.splitn(4, '/').nth(3).unwrap_or("");
    format!("/{}", path.split('?').next().unwrap_or(""))
}

/// `csrf_ok = false` makes every app page fail, so the CSRF token can't be prepared.
fn wire(failure: Failure, csrf_ok: bool) -> Wire {
    let home = tempfile::tempdir().unwrap();
    let ctx = Arc::new(Ctx::new(Paths::under(home.path())).with_tenant("acme.example.com"));
    let mutations: Arc<Mutex<Vec<Request>>> = Arc::default();
    let audit: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let seen = mutations.clone();
    let mock = MockTransport::new(move |req| {
        let path = path_of(&req.url);
        if req.method == "GET" {
            return Ok(match path.as_str() {
                "/a/changes" | "/a/tickets" | "/a/dashboard" if csrf_ok => {
                    Response::html(200, CSRF_PAGE)
                }
                "/a/changes" | "/a/tickets" | "/a/dashboard" => Response::html(500, "down"),
                "/api/_/changes/123/assets" => Response::json(
                    200,
                    &json!({"associated_assets": [{"id": 801, "config_item": {"display_id": 701}}]}),
                ),
                "/api/_/changes/123/tasks" => Response::json(
                    200,
                    &json!({"tasks": [{"id": 701, "title": "Acme task"}, {"id": 702, "title": "Globex task"}]}),
                ),
                other => panic!("unexpected read {other}"),
            });
        }
        seen.lock().unwrap().push(req.clone());
        match failure {
            Failure::Disconnect => Err(TransportError::new("ReadError", "synthetic disconnect")),
            Failure::Status(s) => Ok(Response::json(s, &json!({"error": "synthetic denial"}))),
            Failure::None => Ok(Response::json(
                200,
                &json!({"note": {"id": 901}, "task": {"id": 902}, "change": {"id": 123}}),
            )),
        }
    });
    let rows = audit.clone();
    let client = Client::builder(ctx)
        .transport(mock)
        .cookies(Default::default())
        .sleeper(|_| {})
        .journal(move |method, _url, _body, outcome| {
            rows.lock()
                .unwrap()
                .push((method.to_uppercase(), outcome.ok));
        })
        .build()
        .unwrap();
    Wire {
        client,
        mutations,
        audit,
        _home: home,
    }
}

fn invoke(kind: &str, c: &Client) -> Result<()> {
    match kind {
        "note" => service::add_note(c, &CHANGES, 123, "Acme evidence", false).map(drop),
        "status" => service::update_item(c, &CHANGES, 123, &json!({"status": 47})).map(drop),
        "asset-add" => create::associate_assets(c, 123, &[701]),
        "asset-remove" => create::dissociate_assets(c, 123, &[701]),
        "task-update" => {
            let mut body = Map::new();
            body.insert("title".into(), json!("Acme revised"));
            create::update_task(c, 123, 701, &body).map(drop)
        }
        "task-delete" => create::delete_task(c, 123, 701),
        _ => create::clone_tasks(c, 123, 456, false, None, None).map(drop),
    }
}

fn body(req: &Request) -> Value {
    serde_json::from_slice(req.body.as_deref().unwrap()).unwrap()
}

fn assert_target(req: &Request, method: &str, path: &str, ctx: &str) {
    assert_eq!(req.method, method, "{ctx}");
    assert!(
        path_of(&req.url).ends_with(&format!("/{path}")),
        "{ctx}: {}",
        req.url
    );
}

#[test]
fn each_legacy_caller_never_automatically_replays() {
    for (kind, method, path, payload) in CASES {
        for failure in [
            Failure::Status(403),
            Failure::Status(429),
            Failure::Status(500),
            Failure::Disconnect,
        ] {
            let ctx = format!("{kind} {failure:?}");
            let w = wire(failure, true);
            let err = invoke(kind, &w.client).expect_err(&ctx);
            match failure {
                Failure::Status(s) => assert_eq!(err.status(), Some(s), "{ctx}: {err}"),
                _ => assert!(matches!(err, fsv_core::Error::Http { .. }), "{ctx}: {err}"),
            }
            let mutations = w.mutations.lock().unwrap();
            assert_eq!(mutations.len(), 1, "{ctx}");
            assert_target(&mutations[0], method, path, &ctx);
            assert_eq!(
                mutations[0].header("x-csrf-token"),
                Some("synthetic-token"),
                "{ctx}"
            );
            if let Some(p) = payload {
                assert_eq!(
                    body(&mutations[0]),
                    serde_json::from_str::<Value>(p).unwrap(),
                    "{ctx}"
                );
            }
            let audit = w.audit.lock().unwrap();
            let rows: Vec<_> = audit.iter().filter(|(m, _)| m == method).collect();
            assert!(rows.last().is_some_and(|(_, ok)| !ok), "{ctx}: {audit:?}");
        }
    }
}

#[test]
fn csrf_preparation_failure_prevents_each_legacy_mutation() {
    for (kind, ..) in CASES {
        let w = wire(Failure::None, false);
        let err = invoke(kind, &w.client).expect_err(kind);
        assert!(err.to_string().contains("CSRF token"), "{kind}: {err}");
        assert!(w.mutations.lock().unwrap().is_empty(), "{kind}");
    }
}

#[test]
fn legacy_success_contracts_are_not_confused_with_independent_readback() {
    for (kind, method, path, payload) in CASES {
        let w = wire(Failure::None, true);
        invoke(kind, &w.client).expect(kind);
        let mutations = w.mutations.lock().unwrap();
        let expected = if kind == "clone-task" { 2 } else { 1 };
        assert_eq!(mutations.len(), expected, "{kind}");
        for req in mutations.iter() {
            assert_target(req, method, path, kind);
        }
        if let Some(p) = payload {
            assert_eq!(
                body(&mutations[0]),
                serde_json::from_str::<Value>(p).unwrap(),
                "{kind}"
            );
        }
        if kind == "clone-task" {
            assert_eq!(body(&mutations[0])["change_task"]["title"], "Acme task");
        }
        assert!(w.audit.lock().unwrap().iter().all(|(_, ok)| *ok), "{kind}");
    }
}
