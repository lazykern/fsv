//! Debug builds only: `FSV_TEST_ROUTES=<file.json>` answers HTTP from canned
//! routes so integration tests can drive the binary offline.
//!
//! The file is a JSON list of `{"method", "path", "query"?, "status"?,
//! "json"? | "text"?, "headers"?}`. A request matches the first route with
//! the same method and URL path whose `query` pairs all appear in the URL.
//! Unmatched requests get a 404. With `FSV_TEST_LOG=<file>` every request is
//! appended there as one JSON line `{"method", "url", "body"}`.

use std::io::Write;
use std::sync::Arc;

use serde_json::{Value, json};

use fsv_core::client::mock::MockTransport;
use fsv_core::client::{Request, Response};

fn matches(route: &Value, req: &Request) -> bool {
    let Ok(url) = url::Url::parse(&req.url) else {
        return false;
    };
    if route.get("method").and_then(Value::as_str).unwrap_or("GET") != req.method {
        return false;
    }
    if route.get("path").and_then(Value::as_str) != Some(url.path()) {
        return false;
    }
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    route
        .get("query")
        .and_then(Value::as_object)
        .is_none_or(|q| {
            q.iter().all(|(k, v)| {
                let want = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                pairs.iter().any(|(pk, pv)| pk == k && *pv == want)
            })
        })
}

fn respond(route: &Value) -> Response {
    let status = route.get("status").and_then(Value::as_u64).unwrap_or(200) as u16;
    let mut resp = if let Some(text) = route.get("text").and_then(Value::as_str) {
        Response::html(status, text)
    } else {
        Response::json(status, route.get("json").unwrap_or(&json!({})))
    };
    if let Some(h) = route.get("headers").and_then(Value::as_object) {
        for (k, v) in h {
            resp = resp.with_header(k, v.as_str().unwrap_or_default());
        }
    }
    resp
}

/// The routed transport when `FSV_TEST_ROUTES` is set.
pub fn transport() -> Option<Arc<MockTransport>> {
    let path = std::env::var_os("FSV_TEST_ROUTES").filter(|v| !v.is_empty())?;
    let text = std::fs::read_to_string(&path).ok()?;
    let routes: Vec<Value> = serde_json::from_str(&text).ok()?;
    let log = std::env::var_os("FSV_TEST_LOG").filter(|v| !v.is_empty());
    Some(MockTransport::new(move |req| {
        if let Some(log) = &log
            && let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
        {
            let body = req.json().unwrap_or_else(|| {
                req.body
                    .as_deref()
                    .filter(|b| !b.is_empty())
                    .map_or(Value::Null, |b| json!(String::from_utf8_lossy(b)))
            });
            let line = json!({"method": req.method, "url": req.url, "body": body});
            let _ = writeln!(f, "{line}");
        }
        let resp = routes.iter().find(|r| matches(r, req)).map_or_else(
            || Response::json(404, &json!({"errors": "no route"})),
            respond,
        );
        Ok(Response {
            url: req.url.clone(),
            ..resp
        })
    }))
}
