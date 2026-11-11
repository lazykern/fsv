//! `collect_pages`, `load_views` and `record_for_form` against `fsv.views`
//! on scripted responses (`tests/fixtures/views_expected.json`, from
//! `/tmp/fsv_port/views_oracle.py`).

use serde_json::{Map, Value, json};

use fsv_core::resource;

use super::*;

const EXPECTED: &str = include_str!("../../tests/fixtures/views_expected.json");

fn expected() -> Value {
    serde_json::from_str(EXPECTED).expect("views fixture")
}

/// Answers each call with the next scripted response (`{"ok": data}`,
/// `{"api": status}`, `{"exc": true}` or `{"session": msg}`); once they run
/// out, an empty `key` page.
struct Scripted {
    responses: Vec<Value>,
    key: String,
    calls: Vec<Value>,
}

impl Scripted {
    fn new(responses: &Value, key: &str) -> Self {
        let mut responses = responses.as_array().cloned().unwrap_or_default();
        responses.reverse();
        Scripted {
            responses,
            key: key.into(),
            calls: Vec::new(),
        }
    }
}

impl Source for Scripted {
    fn int_get(
        &mut self,
        path: &str,
        params: Option<&Map<String, Value>>,
    ) -> Result<Value, FetchError> {
        self.calls.push(json!([
            path,
            params.cloned().map_or(Value::Null, Value::Object)
        ]));
        let Some(r) = self.responses.pop() else {
            return Ok(json!({ self.key.clone(): [] }));
        };
        if let Some(status) = r.get("api") {
            return Err(FetchError::Status(status.as_i64()));
        }
        if let Some(msg) = r.get("session") {
            return Err(FetchError::Session(msg.as_str().unwrap_or_default().into()));
        }
        if r.get("exc").is_some() {
            return Err(FetchError::Status(None));
        }
        Ok(r["ok"].clone())
    }

    fn v2_get(&mut self, path: &str) -> Result<Value, FetchError> {
        panic!("unexpected v2 call {path}")
    }
}

fn miss_json(m: Option<&Missing>) -> Value {
    m.map_or(Value::Null, |m| json!({"what": m.what, "status": m.status}))
}

#[test]
fn collect_pages_matches_python() {
    for case in expected()["collect_pages"].as_array().expect("cases") {
        let key = case["key"].as_str().expect("key");
        let mut src = Scripted::new(&case["responses"], key);
        let params = case["params"].as_object().cloned().unwrap_or_default();
        let max_pages = case
            .get("max_pages")
            .and_then(Value::as_i64)
            .unwrap_or(MAX_DETAIL_PAGES);
        let what = case.get("what").and_then(Value::as_str).unwrap_or("");
        let got = match collect_pages(
            &mut src,
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
        assert_eq!(
            Value::Array(src.calls),
            case["calls"],
            "{} calls",
            case["name"]
        );
    }
}

#[test]
fn load_views_matches_python() {
    for case in expected()["load_views"].as_array().expect("cases") {
        let res =
            resource::lookup(case["resource"].as_str().expect("resource")).expect("known resource");
        let mut src = Scripted::new(&json!([case["response"]]), "x");
        let got = match load_views(&mut src, res) {
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
        assert_eq!(Value::Array(src.calls), case["calls"], "{case}");
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
