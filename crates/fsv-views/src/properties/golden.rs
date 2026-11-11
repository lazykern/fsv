//! `load_properties` / `load_edit_rules` against `fsv.views` on the Acme
//! forms (`tests/fixtures/properties_expected.json`, from
//! `/tmp/fsv_port/properties_oracle.py`): same props, note, requests and
//! cache writes.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use jiff::tz::{self, TimeZone};
use serde_json::{Map, Value, json};

use fsv_core::client::{Body, Response};
use fsv_core::error::{ApiError, Error, Result};
use fsv_core::resource;

use super::*;

const EXPECTED: &str = include_str!("../../tests/fixtures/properties_expected.json");

/// Answers `int_get` from `routes` (`{"ok": data}`, `{"api": status}` or
/// `{"session": msg}`); any other path is a 404.
struct FakeApi {
    routes: Map<String, Value>,
    calls: Mutex<Vec<Value>>,
}

impl FakeApi {
    fn log(&self, call: Value) {
        self.calls.lock().unwrap().push(call);
    }
}

fn not_found() -> Error {
    Error::Api(ApiError::new(404, json!({})))
}

impl Api for FakeApi {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        let params = params
            .filter(|p| !p.is_empty())
            .map_or(Value::Null, Value::Object);
        self.log(json!(["int_get", path, params]));
        let Some(r) = self.routes.get(path) else {
            return Err(not_found());
        };
        if let Some(status) = r.get("api").and_then(Value::as_u64) {
            return Err(Error::Api(ApiError::new(status as u16, json!({}))));
        }
        if let Some(msg) = r.get("session").and_then(Value::as_str) {
            return Err(Error::session(msg));
        }
        Ok(r["ok"].clone())
    }
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Option<Map<String, Value>>,
    ) -> Result<Vec<Value>> {
        self.log(json!([
            "autocomplete",
            kind,
            query,
            params.map_or(Value::Null, Value::Object)
        ]));
        Err(not_found())
    }
    fn lookup_choices(&self, link: &str, query: &str) -> Result<Vec<Value>> {
        self.log(json!(["lookup_choices", link, query, null]));
        Err(not_found())
    }
    fn int_post(&self, path: &str, _body: &Value) -> Result<Value> {
        panic!("unexpected POST {path}")
    }
    fn int_put(&self, path: &str, _body: &Value) -> Result<Value> {
        panic!("unexpected PUT {path}")
    }
    fn int_patch(&self, path: &str, _body: &Value) -> Result<Value> {
        panic!("unexpected PATCH {path}")
    }
    fn int_delete(
        &self,
        path: &str,
        _body: Option<&Value>,
        _params: Option<Map<String, Value>>,
    ) -> Result<Value> {
        panic!("unexpected DELETE {path}")
    }
    fn fulltext_search(
        &self,
        entity: &str,
        _term: &str,
        _page: i64,
        _sort: Option<&str>,
    ) -> Result<Value> {
        panic!("unexpected search {entity}")
    }
    fn get_csrf(&self) -> Result<String> {
        panic!("unexpected csrf")
    }
    fn send_once(
        &self,
        method: &str,
        path: &str,
        _body: Body,
        _params: Option<Map<String, Value>>,
        _csrf: &str,
    ) -> Result<Value> {
        panic!("unexpected {method} {path}")
    }
    fn fetch(&self, url: &str) -> Result<Response> {
        panic!("unexpected fetch {url}")
    }
    fn domain(&self) -> String {
        "acme.freshservice.com".into()
    }
}

/// In-memory `formmeta._cached` / `_save` / `configured_workspace`.
struct Store {
    data: RefCell<Map<String, Value>>,
    configured: Value,
    saves: RefCell<Vec<Value>>,
}

impl MetaStore for Store {
    fn cached(&self, name: &str) -> Option<Value> {
        self.data.borrow().get(name).cloned()
    }
    fn save(&self, name: &str, kind: &str, data: &Value) {
        self.saves.borrow_mut().push(json!([name, kind]));
        self.data.borrow_mut().insert(name.into(), data.clone());
    }
    fn configured_workspace(&self) -> Value {
        self.configured.clone()
    }
}

fn cases() -> Vec<Value> {
    let cases: Value = serde_json::from_str(EXPECTED).expect("properties fixture");
    cases.as_array().cloned().expect("cases")
}

fn case(name: &str) -> Value {
    cases()
        .into_iter()
        .find(|c| c["name"] == name)
        .expect("case")
}

fn store(configured: Value) -> Store {
    Store {
        data: RefCell::new(Map::new()),
        configured,
        saves: RefCell::new(Vec::new()),
    }
}

/// `_RecordRules` from `tests/test_qa_regressions.py`: one rule named after
/// the `module_id` the request carried.
struct RecordRules {
    rules: Value,
    calls: Mutex<Vec<(String, Value)>>,
}

impl Api for RecordRules {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        let params = params.unwrap_or_default();
        self.calls
            .lock()
            .unwrap()
            .push((path.into(), Value::Object(params.clone())));
        if path != "ws/2/business_rules" {
            return Ok(json!({}));
        }
        let mut r = self.rules.clone();
        let first = r["business_rules"][0].clone();
        r["business_rules"] = json!([first]);
        r["business_rules"][0]["name"] =
            json!(format!("rule for {}", fsv_py::py_str(&params["module_id"])));
        Ok(r)
    }
    fn autocomplete(
        &self,
        _kind: &str,
        _query: &str,
        _params: Option<Map<String, Value>>,
    ) -> Result<Vec<Value>> {
        Err(not_found())
    }
    fn lookup_choices(&self, _link: &str, _query: &str) -> Result<Vec<Value>> {
        Err(not_found())
    }
    fn int_post(&self, path: &str, _body: &Value) -> Result<Value> {
        panic!("unexpected POST {path}")
    }
    fn int_put(&self, path: &str, _body: &Value) -> Result<Value> {
        panic!("unexpected PUT {path}")
    }
    fn int_patch(&self, path: &str, _body: &Value) -> Result<Value> {
        panic!("unexpected PATCH {path}")
    }
    fn int_delete(
        &self,
        path: &str,
        _body: Option<&Value>,
        _params: Option<Map<String, Value>>,
    ) -> Result<Value> {
        panic!("unexpected DELETE {path}")
    }
    fn fulltext_search(
        &self,
        entity: &str,
        _term: &str,
        _page: i64,
        _sort: Option<&str>,
    ) -> Result<Value> {
        panic!("unexpected search {entity}")
    }
    fn get_csrf(&self) -> Result<String> {
        panic!("unexpected csrf")
    }
    fn send_once(
        &self,
        method: &str,
        path: &str,
        _body: Body,
        _params: Option<Map<String, Value>>,
        _csrf: &str,
    ) -> Result<Value> {
        panic!("unexpected {method} {path}")
    }
    fn fetch(&self, url: &str) -> Result<Response> {
        panic!("unexpected fetch {url}")
    }
    fn domain(&self) -> String {
        "acme.freshservice.com".into()
    }
}

/// `test_detail_rules_are_cached_per_record`.
#[test]
fn load_edit_rules_caches_per_record() {
    let c = case("change_ws2");
    let api = RecordRules {
        rules: c["routes"]["ws/2/business_rules"]["ok"].clone(),
        calls: Mutex::new(Vec::new()),
    };
    let st = store(Value::Null);
    let fields = c["fields"].as_array().cloned().unwrap_or_default();
    let rule_name = |r: Option<Value>| r.expect("rules")["business_rules"][0]["name"].clone();
    let (r111, n) = load_edit_rules(
        &api,
        &st,
        "change",
        &json!({"display_id": 111, "workspace_id": 2}),
        &fields,
    )
    .unwrap();
    assert_eq!((rule_name(r111), n), (json!("rule for 111"), None));
    let (r222, _) = load_edit_rules(
        &api,
        &st,
        "change",
        &json!({"display_id": 222, "workspace_id": 2}),
        &fields,
    )
    .unwrap();
    assert_eq!(rule_name(r222), json!("rule for 222"));
    let module_ids = |api: &RecordRules| -> Vec<Value> {
        api.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.ends_with("business_rules"))
            .map(|(_, p)| p["module_id"].clone())
            .collect()
    };
    assert_eq!(module_ids(&api), [json!("111"), json!("222")]);
    let (again, _) = load_edit_rules(
        &api,
        &st,
        "change",
        &json!({"display_id": 111, "workspace_id": 2}),
        &fields,
    )
    .unwrap();
    assert_eq!(rule_name(again), json!("rule for 111"));
    assert_eq!(module_ids(&api).len(), 2);
    assert_eq!(
        st.saves.take(),
        [
            json!(["change-rules-edit_form-ws2-111", "form_rules"]),
            json!(["change-rules-edit_form-ws2-222", "form_rules"])
        ]
    );
}

/// `_prop_value` per visible field equals the value Python put in each prop.
#[test]
fn prop_value_matches_python() {
    let tz7 = TimeZone::fixed(tz::offset(7));
    for name in ["change_ws2", "ticket"] {
        let c = case(name);
        let res = resource::lookup(c["resource"].as_str().unwrap()).unwrap();
        let module = module_of(res.name);
        let spec = Arc::new(
            FormSpec::from_form_fields(&json!({ res.form_fields_key: c["fields"] }), &module)
                .unwrap(),
        );
        let rules = c["routes"].as_object().unwrap().values().next().unwrap()["ok"].clone();
        let engine = RulesEngine::new(
            spec.clone(),
            Some(&rules),
            EngineOptions {
                module: Some(module.clone()),
                workspace_id: c["record"]
                    .get("workspace_id")
                    .filter(|v| !v.is_null())
                    .cloned(),
                ..Default::default()
            },
        )
        .unwrap();
        let rec = record_for_form(res, c["record"].as_object().unwrap());
        let existing = formmeta::existing_from_record(&spec, &rec);
        let opts = StateOptions {
            tz: Some("Asia/Bangkok".into()),
            existing: Some(existing),
            ..Default::default()
        };
        let state = FormState::new(spec.clone(), Some(engine), "edit", None, opts).unwrap();
        for p in c["result"]["props"].as_array().unwrap() {
            let f = spec.field(p["name"].as_str().unwrap()).unwrap();
            let want = p["value"].as_str().unwrap();
            // Without a resolver, ids stay ids, as they did in Python (lookups answered 404).
            assert_eq!(
                prop_value(&state, f, Some(&tz7)),
                want,
                "{name} {}",
                f.api_name
            );
        }
    }
}

#[test]
fn module_of_matches_python_map() {
    assert_eq!(
        ["changes", "tickets", "problems", "assets", "ss"].map(module_of),
        ["change", "ticket", "problem", "asset", ""]
    );
}

#[test]
fn load_properties_matches_python() {
    let cases: Value = serde_json::from_str(EXPECTED).expect("properties fixture");
    let tz7 = TimeZone::fixed(tz::offset(7));
    let mut failures = Vec::new();
    for case in cases.as_array().expect("cases") {
        let name = case["name"].as_str().expect("name");
        let api = Arc::new(FakeApi {
            routes: case["routes"].as_object().cloned().unwrap_or_default(),
            calls: Mutex::new(Vec::new()),
        });
        let store = Store {
            data: RefCell::new(
                case.get("seed")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            ),
            configured: case.get("configured").cloned().unwrap_or(Value::Null),
            saves: RefCell::new(Vec::new()),
        };
        let res =
            resource::lookup(case["resource"].as_str().expect("resource")).expect("known resource");
        let schema = json!({"fields": case["fields"]});
        let user = case.get("user").filter(|u| !u.is_null());
        let got = match load_properties(
            api.clone(),
            &store,
            res,
            &case["record"],
            Some(&schema),
            Some(&tz7),
            user,
        ) {
            Ok((props, note)) => {
                let props: Vec<Value> = props
                    .iter()
                    .map(|p| json!({"name": p.name, "label": p.label, "value": p.value, "required": p.required}))
                    .collect();
                json!({"props": props, "note": note})
            }
            Err(e) => json!({"session": e.0}),
        };
        if got != case["result"] {
            failures.push(format!(
                "{name}\n  python: {}\n  rust:   {got}",
                case["result"]
            ));
        }
        let calls = Value::Array(api.calls.lock().unwrap().clone());
        if calls != case["calls"] {
            failures.push(format!(
                "{name} calls\n  python: {}\n  rust:   {calls}",
                case["calls"]
            ));
        }
        let saves = Value::Array(store.saves.take());
        if saves != case["saves"] {
            failures.push(format!(
                "{name} saves\n  python: {}\n  rust:   {saves}",
                case["saves"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
