//! Query payload matrix (Python `92d76ff`: `tests/test_query_matrix.py`).
//! Expected values come from `port/golden/gen_fsv_api_query_flow_matrix.py`.

use std::cell::RefCell;
use std::sync::LazyLock;

use serde_json::{Map, Value, json};

use super::*;

static ORACLE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("testdata/query_flow_oracle.json")).unwrap()
});

fn q() -> &'static Value {
    &ORACLE["query"]
}

fn outcome<T: Into<Value>>(r: Result<T, QueryError>) -> Value {
    match r {
        Ok(v) => json!({"ok": v.into()}),
        Err(e) => json!({"err": e.0}),
    }
}

/// The oracle case minus its inputs: `{"ok": …}` or `{"err": …}`.
fn want(case: &Value) -> Value {
    match case.get("ok") {
        Some(ok) => json!({"ok": ok}),
        None => json!({"err": case["err"]}),
    }
}

fn from_schema(exprs: &[String]) -> Value {
    outcome(build_query_hash_from_schema(None, "changes", &q()["schema"], exprs).map(|h| json!(h)))
}

type Built = (Option<String>, Vec<Value>, Vec<Explain>);

fn built(r: Result<Built, QueryError>) -> Value {
    outcome(r.map(|(hash, items, explain)| {
        let explain: Vec<Value> = explain
            .iter()
            .map(|e| json!({"field": e.field, "condition": e.condition, "operator": e.operator, "type": e.type_name, "value": e.value}))
            .collect();
        json!([hash, items, explain])
    }))
}

#[derive(Default)]
struct Dir {
    rows: Vec<Value>,
    groups: Value,
    calls: RefCell<Vec<Value>>,
}

impl Directory for Dir {
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        extra: &[(&str, &str)],
    ) -> Result<Vec<Value>, QueryError> {
        let params = (!extra.is_empty()).then(|| {
            extra
                .iter()
                .map(|(k, v)| ((*k).to_string(), json!(v)))
                .collect::<Map<String, Value>>()
        });
        self.calls.borrow_mut().push(json!([kind, query, params]));
        Ok(self.rows.clone())
    }
    fn agent_groups(&self) -> Result<Value, QueryError> {
        self.calls
            .borrow_mut()
            .push(json!("bootstrap/agents_groups"));
        Ok(self.groups.clone())
    }
}

#[test]
fn schema_filters_produce_exact_wire_items() {
    for (expr, case) in q()["wire"].as_object().unwrap() {
        assert_eq!(from_schema(std::slice::from_ref(expr)), *case, "{expr}");
    }
}

#[test]
fn invalid_filter_fails_locally_with_actionable_message() {
    for (expr, case) in q()["invalid"].as_object().unwrap() {
        assert_eq!(from_schema(std::slice::from_ref(expr)), *case, "{expr}");
    }
}

#[test]
fn date_bounds_are_inclusive_merged_and_do_not_erase_other_fields() {
    for case in q()["date_bounds"].as_array().unwrap() {
        let (f, lo, hi) = (
            case["field"].as_str().unwrap(),
            case["lower"].as_str().unwrap(),
            case["upper"].as_str().unwrap(),
        );
        let exprs = [
            format!("{f}{lo}2026-01-01"),
            "status=47".into(),
            format!("{f}{hi}2026-12-31"),
        ];
        assert_eq!(from_schema(&exprs), want(case), "{f} {lo} {hi}");
    }
}

#[test]
fn exact_date_and_last_repeated_bound_are_preserved() {
    let exprs = ["due_by=2026-01-01".into(), "due_by>=2026-02-01".into()];
    assert_eq!(from_schema(&exprs), q()["exact_date"]);
}

#[test]
fn field_name_precedence_scope_and_ambiguity() {
    for case in q()["find_field"].as_array().unwrap() {
        let schema = json!({"fields": case["fields"]});
        let text = case["text"].as_str().unwrap();
        let got = outcome(
            find_schema_field(&schema, text).map(|f| f.map_or(Value::Null, |f| f["name"].clone())),
        );
        assert_eq!(got, want(case), "{text}");
    }
}

#[test]
fn named_people_resolve_server_ids_not_display_names() {
    for (res, case) in q()["people"].as_object().unwrap() {
        let dir = Dir {
            rows: vec![
                json!({"id": 71, "user_id": 171, "value": "Alex Acme", "email": "alex@example.com"}),
            ],
            ..Default::default()
        };
        let filters = [
            "requester=alex@example.com".into(),
            "agent=Alex Acme".into(),
        ];
        let got = built(build_query_hash(
            Some(&dir),
            res,
            Some(&q()["schema"]),
            &filters,
            None,
            true,
        ));
        assert_eq!(got, want(case), "{res}");
        assert_eq!(Value::Array(dir.calls.take()), case["calls"], "{res}");
    }
}

#[test]
fn agent_lookup_errors_never_fall_back_to_wrong_id() {
    for case in q()["agent_errors"].as_array().unwrap() {
        let dir = Dir {
            rows: case["rows"].as_array().unwrap().clone(),
            ..Default::default()
        };
        assert_eq!(
            outcome(resolve_agent(Some(&dir), "Alex")),
            want(case),
            "{}",
            case["rows"]
        );
    }
}

#[test]
fn group_lookup_exact_then_unique_and_missing_id() {
    let g = &q()["groups"];
    let dir = Dir {
        groups: json!({"groups": [{"id": 91, "name": "Acme"}, {"id": 92, "name": "Acme Ops"}]}),
        ..Default::default()
    };
    for (value, case) in g["found"].as_object().unwrap() {
        assert_eq!(outcome(resolve_group(&dir, value)), *case, "{value}");
    }
    assert_eq!(Value::Array(dir.calls.take()), g["calls"]);
    let nameless = Dir {
        groups: json!({"groups": [{"name": "Acme"}]}),
        ..Default::default()
    };
    assert_eq!(outcome(resolve_group(&nameless, "Acme")), g["missing_id"]);
}

#[test]
fn raw_query_hash_decodes_only_once_and_combines_with_schema() {
    for case in q()["raw"].as_array().unwrap() {
        let raw = case["raw"].as_str().unwrap();
        let got = built(build_query_hash(
            None,
            "changes",
            Some(&q()["schema"]),
            &["status=47".into()],
            Some(raw),
            false,
        ));
        assert_eq!(got, want(case), "{raw}");
    }
}

/// Decode errors keep Python's prefix; the rest is serde_json's wording
/// (PARITY.md, `--query-hash` with bad JSON).
#[test]
fn malformed_raw_query_hash_is_rejected() {
    for (raw, case) in q()["malformed_raw"].as_object().unwrap() {
        let got = built(build_query_hash(
            None,
            "changes",
            None,
            &[],
            Some(raw),
            false,
        ));
        let want = case["err"].as_str().unwrap();
        let got = got["err"]
            .as_str()
            .unwrap_or_else(|| panic!("{raw}: {got}"));
        match want.split_once("JSON: ") {
            Some((prefix, _)) if want.starts_with("invalid query-hash JSON") => {
                assert!(got.starts_with(&format!("{prefix}JSON: ")), "{raw}: {got}");
            }
            _ => assert_eq!(got, want, "{raw}"),
        }
    }
}

#[test]
fn empty_filters_do_not_load_schema() {
    let e = &q()["empty"];
    assert_eq!(
        built(build_query_hash(None, "changes", None, &[], None, false)),
        json!({"ok": e["build"]})
    );
    assert_eq!(
        json!(build_query_hash_from_schema(None, "changes", &q()["schema"], &[]).unwrap()),
        e["from_schema"]
    );
}
