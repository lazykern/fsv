//! `tests/test_ticket_properties_http.py`: reviewed property writes over a
//! real `fsv_core::Client` and a mock transport, on two tenant schemas.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Map, Value, json};

use fsv_core::client::mock::MockTransport;
use fsv_core::client::{Request, Response, TransportError};
use fsv_core::config::Paths;
use fsv_core::{Client, Ctx};

use super::*;
use crate::formmeta::MemStore;
use crate::service::testing::MemJournal;

const CSRF_PAGE: &str =
    r#"<html><head><meta name="csrf-token" content="synthetic-token"></head></html>"#;

/// `(domain, cid, opened, pending, fid)` of the two Python fixture tenants.
pub(crate) const TENANTS: [(&str, i64, i64, i64, i64); 2] = [
    ("acme.example.com", 123, 2, 3, 9000008101),
    ("globex.example.com", 742, 2, 3, 9000008201),
];

#[derive(Clone, Copy, Debug)]
pub(crate) enum Failure {
    Status(u16),
    Disconnect,
}

pub(crate) const FAILURES: [Failure; 4] = [
    Failure::Status(403),
    Failure::Status(429),
    Failure::Status(500),
    Failure::Disconnect,
];

/// The mutable server side of the Python `wire` fixture.
pub(crate) struct State {
    pub cid: i64,
    pub failure: Option<Failure>,
    pub readback: Option<&'static str>,
    pub row: Map<String, Value>,
    pub fields: Vec<Value>,
    pub conv_id: i64,
    pub conversation: Option<Map<String, Value>>,
    pub conv_readback: Option<&'static str>,
    pub lookup_name: String,
    pub requests: Vec<Request>,
}

pub(crate) struct Wire {
    pub client: Client,
    pub cid: i64,
    pub pending: i64,
    pub domain: &'static str,
    pub state: Arc<Mutex<State>>,
    pub journal: MemJournal,
    pub store: MemStore,
    _home: tempfile::TempDir,
}

fn path_of(url: &str) -> String {
    let rest = url.splitn(4, '/').nth(3).unwrap_or("");
    format!("/{}", rest.split('?').next().unwrap_or(""))
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = fsv_core::pystr::urlsplit(url).ok()?.query;
    fsv_core::pystr::parse_qs_first(&query)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn handle(w: &mut State, req: &Request) -> std::result::Result<Response, TransportError> {
    w.requests.push(req.clone());
    let cid = w.cid;
    let full = path_of(&req.url);
    if req.method == "GET" && full.starts_with("/a/") {
        return Ok(Response::html(200, CSRF_PAGE));
    }
    let path = full.strip_prefix("/api/_/").unwrap_or(&full).to_string();
    if req.method == "PUT" || req.method == "POST" {
        match w.failure {
            Some(Failure::Disconnect) => {
                return Err(TransportError::new("ReadError", "synthetic disconnect"));
            }
            Some(Failure::Status(s)) => {
                return Ok(Response::json(s, &json!({"error": "synthetic denial"})));
            }
            None => {}
        }
        let payload: Value = serde_json::from_slice(req.body.as_deref().unwrap()).unwrap();
        let payload = payload.as_object().unwrap();
        if req.method == "PUT" {
            w.row.extend(payload.clone());
            return Ok(Response::json(200, &json!({"ticket": {"id": cid}})));
        }
        let mut conversation = obj(json!({"id": w.conv_id, "body": payload["body"]}));
        for key in ["cc_emails", "bcc_emails", "private", "from_email"] {
            if let Some(v) = payload.get(key) {
                conversation.insert(key.into(), v.clone());
            }
        }
        if let Some(v) = payload.get("notify_emails") {
            conversation.insert("to_emails".into(), v.clone());
        }
        if let Some(h) = payload.get("helpdesk_ticket") {
            w.row.insert("status".into(), h["status"].clone());
        }
        w.conversation = Some(conversation);
        return Ok(Response::json(
            200,
            &json!({"conversation": {"id": w.conv_id}}),
        ));
    }
    if path == "ticket_form_fields" {
        return Ok(Response::json(200, &json!({"ticket_fields": w.fields})));
    }
    if path.ends_with("business_rules") {
        return Ok(Response::json(200, &json!({"business_rules": []})));
    }
    if full.starts_with("/search/autocomplete/") {
        return Ok(Response::json(
            200,
            &json!({"results": [{"id": cid + 8000, "value": w.lookup_name, "email": query_param(&req.url, "q")}]}),
        ));
    }
    if path == format!("tickets/{cid}/conversations") {
        let mut conversation = w.conversation.clone();
        if let Some(c) = conversation.as_mut() {
            match w.conv_readback {
                Some("wrong-id") => c["id"] = json!(w.conv_id + 1),
                Some("contradict") => c["body"] = json!("different"),
                Some(key) => {
                    c.shift_remove(key);
                }
                None => {}
            }
        }
        let rows: Vec<Value> = conversation.into_iter().map(Value::Object).collect();
        return Ok(Response::json(
            200,
            &json!({"conversations": rows, "meta": {"count": 1}}),
        ));
    }
    if path == format!("tickets/{cid}") {
        let mut row = w.row.clone();
        let mutated = w
            .requests
            .iter()
            .any(|r| r.method == "PUT" || r.method == "POST");
        match w.readback {
            Some("wrong-id") if mutated => row["id"] = json!(cid + 1),
            Some("missing") if mutated => {
                row.shift_remove("tags");
            }
            Some("contradict") if mutated => row["tags"] = json!(["other"]),
            _ => {}
        }
        return Ok(Response::json(200, &json!({"ticket": row})));
    }
    panic!("unexpected request {} {}", req.method, req.url);
}

fn client_for(
    domain: &str,
    home: &std::path::Path,
    handler: impl Fn(&Request) -> std::result::Result<Response, TransportError> + Send + Sync + 'static,
) -> Client {
    let ctx = Arc::new(Ctx::new(Paths::under(home)).with_tenant(domain));
    Client::builder(ctx)
        .transport(MockTransport::new(handler))
        .cookies(Default::default())
        .sleeper(|_| {})
        .journal(|_, _, _, _| {})
        .build()
        .unwrap()
}

impl Wire {
    pub fn new((domain, cid, opened, pending, fid): (&'static str, i64, i64, i64, i64)) -> Wire {
        let home = tempfile::tempdir().unwrap();
        let label = domain.split('.').next().unwrap();
        let state = Arc::new(Mutex::new(State {
            cid,
            failure: None,
            readback: None,
            row: obj(
                json!({"id": cid, "subject": "Acme review", "status": opened, "tags": ["before"],
                            "planned_start_date": null, "planned_end_date": null}),
            ),
            fields: vec![
                json!({"id": fid, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1}),
                json!({"id": fid + 1, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
                       "choices": [{"id": opened, "value": "Open"}, {"id": pending, "value": "Pending"},
                                   {"id": 4, "value": "Resolved"}, {"id": 5, "value": "Closed"}]}),
            ],
            conv_id: cid + 9000000000,
            conversation: None,
            conv_readback: None,
            lookup_name: format!("{}{} Operator", label[..1].to_uppercase(), &label[1..]),
            requests: Vec::new(),
        }));
        let server = state.clone();
        let client = client_for(domain, home.path(), move |req| {
            handle(&mut server.lock().unwrap(), req)
        });
        Wire {
            client,
            cid,
            pending,
            domain,
            state,
            journal: MemJournal::default(),
            store: MemStore {
                workspace: json!(1),
                ..MemStore::default()
            },
            _home: home,
        }
    }

    pub fn server(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    pub fn writes(&self, method: &str) -> Vec<Request> {
        self.server()
            .requests
            .iter()
            .filter(|r| r.method == method)
            .cloned()
            .collect()
    }

    /// A second client on the same tenant whose transport fails the test on
    /// any request (`mock_client(..., lambda r: pytest.fail(...))`).
    pub fn replacement(&self) -> Client {
        client_for(self.domain, self._home.path(), |r| {
            panic!("restart may not resend: {} {}", r.method, r.url)
        })
    }
}

fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
    r.unwrap_err().to_string()
}

fn after() -> Vec<String> {
    vec!["after".into()]
}

/// Python tests/test_ticket_properties_http.py::test_exact_property_http_headers_dates_token_and_metadata
#[test]
fn exact_property_http_headers_dates_token_and_metadata() {
    for tenant in TENANTS {
        let w = Wire::new(tenant);
        let tags = after();
        let answers = obj(json!({"Status": "Pending"}));
        let args = PropertyArgs {
            tags: Some(&tags),
            planned_start: Some("2030-01-02T10:00:00+07:00"),
            planned_end: Some("2030-01-02T12:00:00+07:00"),
            answers: Some(&answers),
        };
        let p = build_plan_with(&w.client, &w.store, &w.cid.to_string(), &args, None).unwrap();
        assert!(w.writes("PUT").is_empty());
        assert_eq!(p.payload()["status"], json!(w.pending));
        let result = send_with(&p, &w.client, &w.journal, &w.store, Some(&p.hash()), None).unwrap();
        assert_eq!(result["verified"], json!(true), "{result}");
        let writes = w.writes("PUT");
        assert_eq!(writes.len(), 1);
        assert_eq!(
            writes[0].url,
            format!("https://{}/api/_/tickets/{}", w.domain, w.cid)
        );
        assert_eq!(writes[0].header("content-type"), Some("application/json"));
        assert_eq!(writes[0].header("x-csrf-token"), Some("synthetic-token"));
        assert_eq!(
            serde_json::from_slice::<Value>(writes[0].body.as_deref().unwrap()).unwrap(),
            p.payload()
        );
        let payload = p.payload();
        assert!(
            payload["planned_start_date"]
                .as_str()
                .unwrap()
                .contains("+07:00")
                && fsv_py::truthy(&payload["br_validation_excludes"])
        );
    }
}

/// Python tests/test_ticket_properties_http.py::test_http_ambiguous_failure_never_replayed_even_with_new_client
#[test]
fn http_ambiguous_failure_never_replayed_even_with_new_client() {
    for tenant in TENANTS {
        for failure in FAILURES {
            let w = Wire::new(tenant);
            let tags = after();
            let p = build_plan(
                &w.client,
                &w.store,
                &w.cid.to_string(),
                Some(&tags),
                None,
                None,
            )
            .unwrap();
            w.server().failure = Some(failure);
            let first = err(send(&p, &w.client, &w.journal, &w.store, Some(&p.hash())));
            assert!(first.contains("unknown"), "{failure:?}: {first}");
            let replacement = w.replacement();
            let again = err(send(
                &p,
                &replacement,
                &w.journal,
                &w.store,
                Some(&p.hash()),
            ));
            assert!(again.contains("unknown"), "{failure:?}: {again}");
            assert_eq!(w.writes("PUT").len(), 1, "{failure:?}");
            let events: Vec<Value> = w
                .journal
                .read()
                .into_iter()
                .filter(|r| r["kind"] == "plan:ticket_properties")
                .collect();
            assert_eq!(events.last().unwrap()["event"], "started", "{failure:?}");
        }
    }
}

/// Python tests/test_ticket_properties_http.py::test_bad_readback_stays_unverified_without_resend
#[test]
fn bad_readback_stays_unverified_without_resend() {
    for tenant in TENANTS {
        for readback in ["wrong-id", "missing", "contradict"] {
            let w = Wire::new(tenant);
            w.server().readback = Some(readback);
            let tags = after();
            let p = build_plan(
                &w.client,
                &w.store,
                &w.cid.to_string(),
                Some(&tags),
                None,
                None,
            )
            .unwrap();
            let result = send(&p, &w.client, &w.journal, &w.store, Some(&p.hash())).unwrap();
            assert_eq!(result["verified"], json!(false), "{readback}");
            assert!(
                result["warning"]
                    .as_str()
                    .unwrap()
                    .contains("do not resend"),
                "{readback}"
            );
            assert_eq!(w.writes("PUT").len(), 1, "{readback}");
        }
    }
}

/// Python tests/test_ticket_properties_http.py::test_changed_state_or_metadata_blocks_write
#[test]
fn changed_state_or_metadata_blocks_write() {
    for tenant in TENANTS {
        let w = Wire::new(tenant);
        let answers = obj(json!({"Status": "Pending"}));
        let args = PropertyArgs {
            answers: Some(&answers),
            ..PropertyArgs::default()
        };
        let p = build_plan_with(&w.client, &w.store, &w.cid.to_string(), &args, None).unwrap();
        {
            w.server().fields[1]["choices"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": 9, "value": "Assigned"}));
        }
        let msg = err(send_with(
            &p,
            &w.client,
            &w.journal,
            &w.store,
            Some(&p.hash()),
            None,
        ));
        assert!(msg.contains("changed"), "{msg}");
        assert!(w.writes("PUT").is_empty());
    }
}
