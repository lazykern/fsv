//! `collect_pages`, `load_views` and `record_for_form` against `fsv.views`
//! on scripted responses (`tests/fixtures/views_expected.json`, from
//! `port/golden/oracles/fsv_port/views_oracle.py`).

use std::sync::Mutex;

use serde_json::{Map, Value, json};

use fsv_core::resource;

use super::fake::{raised, read_only_api};
use super::*;

const EXPECTED: &str = include_str!("../../tests/fixtures/views_expected.json");

fn expected() -> Value {
    serde_json::from_str(EXPECTED).expect("views fixture")
}

/// Answers each call with the next scripted response (`{"ok": data}`,
/// `{"api": status}`, `{"exc": true}` or `{"session": msg}`); once they run
/// out, an empty `key` page.
struct Scripted {
    responses: Mutex<Vec<Value>>,
    key: String,
    calls: Mutex<Vec<Value>>,
}

impl Scripted {
    fn new(responses: &Value, key: &str) -> Self {
        let mut responses = responses.as_array().cloned().unwrap_or_default();
        responses.reverse();
        Scripted {
            responses: Mutex::new(responses),
            key: key.into(),
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Value {
        Value::Array(self.calls.lock().unwrap().clone())
    }
}

impl Api for Scripted {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> fsv_core::Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push(json!([path, params.map_or(Value::Null, Value::Object)]));
        let Some(r) = self.responses.lock().unwrap().pop() else {
            return Ok(json!({ self.key.clone(): [] }));
        };
        raised(if let Some(status) = r.get("api") {
            Err(FetchError::Status(status.as_i64()))
        } else if let Some(msg) = r.get("session") {
            Err(FetchError::Session(msg.as_str().unwrap_or_default().into()))
        } else if r.get("exc").is_some() {
            Err(FetchError::Status(None))
        } else {
            Ok(r["ok"].clone())
        })
    }

    fn v2_get(&self, path: &str, _: Option<Map<String, Value>>) -> fsv_core::Result<Value> {
        panic!("unexpected v2 call {path}")
    }

    read_only_api!();
}

fn miss_json(m: Option<&Missing>) -> Value {
    m.map_or(Value::Null, |m| json!({"what": m.what, "status": m.status}))
}

#[test]
fn collect_pages_matches_python() {
    for case in expected()["collect_pages"].as_array().expect("cases") {
        let key = case["key"].as_str().expect("key");
        let src = Scripted::new(&case["responses"], key);
        let params = case["params"].as_object().cloned().unwrap_or_default();
        let max_pages = case
            .get("max_pages")
            .and_then(Value::as_i64)
            .unwrap_or(MAX_DETAIL_PAGES);
        let what = case.get("what").and_then(Value::as_str).unwrap_or("");
        let got = match collect_pages(
            &src,
            "tickets/5/conversations",
            key,
            &params,
            what,
            max_pages,
        ) {
            Ok(c) => {
                json!({"rows": c.rows, "meta": c.meta, "failure": miss_json(c.failure.as_ref())})
            }
            Err(e) => json!({"session": e.0}),
        };
        assert_eq!(got, case["result"], "{}", case["name"]);
        assert_eq!(src.calls(), case["calls"], "{} calls", case["name"]);
    }
}

#[test]
fn load_views_matches_python() {
    for case in expected()["load_views"].as_array().expect("cases") {
        let res =
            resource::lookup(case["resource"].as_str().expect("resource")).expect("known resource");
        let src = Scripted::new(&json!([case["response"]]), "x");
        let got = match load_views(&src, res) {
            Ok((views, miss)) => {
                let views: Vec<Value> = views
                    .iter()
                    .map(|v| {
                        let mut j = v.to_json();
                        j["raw"] = v.raw.clone();
                        j
                    })
                    .collect();
                json!({"views": views, "missing": miss_json(miss.as_ref())})
            }
            Err(e) => json!({"session": e.0}),
        };
        assert_eq!(got, case["result"], "{case}");
        assert_eq!(src.calls(), case["calls"], "{case}");
    }
}

#[test]
fn record_for_form_matches_python() {
    for case in expected()["record_for_form"].as_array().expect("cases") {
        let res =
            resource::lookup(case["resource"].as_str().expect("resource")).expect("known resource");
        let record = case["record"].as_object().expect("record");
        assert_eq!(
            Value::Object(record_for_form(res, record)),
            case["result"],
            "{case}"
        );
    }
}
