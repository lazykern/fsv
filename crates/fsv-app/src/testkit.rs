//! An offline tenant: a real [`Client`] over [`MockTransport`] that answers
//! the CSRF page itself and routes every API call to a test closure.

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

use fsv_core::client::mock::MockTransport;
use fsv_core::client::{Request, Response};
use fsv_core::{Client, Ctx, Paths};

pub const HOST: &str = "acme.freshservice.com";
const CSRF_PAGE: &str = r#"<html><head><meta name="csrf-token" content="tok"></head></html>"#;

/// One request as a route sees it.
pub struct Call {
    pub method: String,
    /// Path without the host and without the query (`/api/_/changes/7`).
    pub path: String,
    /// Query pairs in wire order.
    pub query: Vec<(String, String)>,
    /// Raw body text.
    pub body: Option<String>,
}

impl Call {
    fn of(r: &Request) -> Call {
        let u = url::Url::parse(&r.url).expect("url");
        Call {
            method: r.method.clone(),
            path: u.path().to_string(),
            query: u
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
            body: r
                .body
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned()),
        }
    }

    pub fn json(&self) -> Option<Value> {
        self.body
            .as_deref()
            .and_then(|b| serde_json::from_str(b).ok())
    }

    /// `METHOD path?query` (query pairs joined with `&`, undecoded order).
    pub fn line(&self) -> String {
        if self.query.is_empty() {
            format!("{} {}", self.method, self.path)
        } else {
            let q: Vec<String> = self.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{} {}?{}", self.method, self.path, q.join("&"))
        }
    }
}

pub struct Tenant {
    pub client: Arc<Client>,
    pub mock: Arc<MockTransport>,
    pub dir: tempfile::TempDir,
}

impl Tenant {
    /// API calls (app pages such as the CSRF fetch left out), in order.
    pub fn calls(&self) -> Vec<Call> {
        self.mock
            .requests()
            .iter()
            .map(Call::of)
            .filter(|c| c.path.starts_with("/api/") || c.path.starts_with("/search/"))
            .collect()
    }

    pub fn lines(&self) -> Vec<String> {
        self.calls().iter().map(Call::line).collect()
    }
}

/// A tenant whose API answers come from `route`; an unrouted call is a 404.
pub fn tenant(route: impl Fn(&Call) -> Option<Response> + Send + Sync + 'static) -> Tenant {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = Arc::new(Ctx::new(Paths::under(dir.path())).with_tenant(HOST));
    let mock = MockTransport::new(move |r| {
        let call = Call::of(r);
        if call.path.starts_with("/a/") {
            return Ok(Response::html(200, CSRF_PAGE));
        }
        Ok(route(&call)
            .unwrap_or_else(|| Response::json(404, &json!({"description": "not found"}))))
    });
    let cookies = [("_x_w", "w"), ("fw-session-id", "fw1")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let client = Client::builder(ctx)
        .transport(mock.clone())
        .cookies(cookies)
        .sleeper(|_| {})
        .build()
        .expect("client");
    Tenant {
        client: Arc::new(client),
        mock,
        dir,
    }
}

pub fn ok(v: Value) -> Option<Response> {
    Some(Response::json(200, &v))
}

pub fn status(code: u16, v: Value) -> Option<Response> {
    Some(Response::json(code, &v))
}

pub fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// The write journal in memory (both journal traits; rows as
/// `journal.record_plan` writes them).
#[derive(Default)]
pub struct MemJournal {
    pub rows: Mutex<Vec<Value>>,
}

impl MemJournal {
    fn push(
        &self,
        event: &str,
        plan: &str,
        action: &str,
        target: Option<&Value>,
        created_id: Option<&Value>,
    ) {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        self.rows.lock().unwrap().push(json!({
            "ts": "2026-11-12T00:00:00+00:00", "epoch": epoch, "kind": format!("plan:{action}"), "event": event,
            "plan": plan, "target": target.cloned(), "created_id": created_id.cloned(),
        }));
    }

    /// `(kind, event, plan, created_id)` per row.
    pub fn events(&self) -> Vec<Value> {
        self.rows.lock().unwrap().iter().map(|r| json!({"kind": r["kind"], "event": r["event"], "plan": r["plan"], "created_id": r["created_id"]})).collect()
    }
}

impl fsv_flows::change::PlanJournal for MemJournal {
    fn read(&self) -> Vec<Value> {
        self.rows.lock().unwrap().clone()
    }
    fn record_plan(
        &self,
        event: &str,
        plan: &str,
        action: &str,
        extra: fsv_flows::change::PlanRecord<'_>,
    ) {
        self.push(event, plan, action, extra.target, extra.created_id);
    }
    fn recent_plan(&self, plan: &str, action: &str, _hours: f64) -> Option<Value> {
        fsv_flows::change::recent_plan_rows(&self.rows.lock().unwrap(), plan, action, 0.0)
    }
}

impl fsv_api::service::Journal for MemJournal {
    fn read(&self) -> Vec<Value> {
        self.rows.lock().unwrap().clone()
    }
    fn record_plan(
        &self,
        event: &str,
        plan: &str,
        action: &str,
        target: Option<&Value>,
        created_id: Option<&Value>,
    ) {
        self.push(event, plan, action, target, created_id);
    }
    fn recent_plan(&self, plan: &str, action: &str, _hours: f64) -> Option<Value> {
        fsv_flows::change::recent_plan_rows(&self.rows.lock().unwrap(), plan, action, 0.0)
    }
}

/// Query pairs from `[("k", "v"), ...]`.
pub fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
