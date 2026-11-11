//! Each helper and completer called directly, against `fsv.completion` on the
//! same inputs (`helpers_expected.json` from
//! `/tmp/fsv_port/completion_helpers_oracle.py`; completer cases reuse
//! `dynamic_expected.json`).

use serde_json::{Value, json};

use super::golden::{FakeRemote, fixture_cache};
use super::*;

const HELPERS: &str = include_str!("../../tests/fixtures/helpers_expected.json");
const DYNAMIC: &str = include_str!("../../tests/fixtures/dynamic_expected.json");

fn helpers() -> Value {
    serde_json::from_str(HELPERS).expect("helpers fixture")
}

fn objs(v: &Value) -> Vec<Obj> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|o| o.as_object().cloned().expect("object"))
        .collect()
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string")
}

fn pairs_json(p: Pairs) -> Value {
    Value::Array(p.into_iter().map(|(a, b)| json!([a, b])).collect())
}

fn res(name: &str) -> &'static Resource {
    resource::lookup(name).expect("resource")
}

#[test]
fn field_accessors_match_python() {
    let h = helpers();
    let fields = objs(&h["fields"]);
    let scope: Vec<&str> = fields.iter().map(field_scope).collect();
    let name: Vec<String> = fields.iter().map(field_name).collect();
    let label: Vec<String> = fields.iter().map(field_label).collect();
    let help: Vec<String> = fields.iter().map(field_help).collect();
    assert_eq!(json!(scope), h["field_scope"]);
    assert_eq!(json!(name), h["field_name"]);
    assert_eq!(json!(label), h["field_label"]);
    assert_eq!(json!(help), h["field_help"]);
}

#[test]
fn choice_text_matches_python() {
    let h = helpers();
    let got: Vec<String> = h["choices"]
        .as_array()
        .expect("choices")
        .iter()
        .map(choice_text)
        .collect();
    assert_eq!(json!(got), h["choice_text"]);
}

#[test]
fn completion_case_matches_python() {
    for row in helpers()["completion_case"].as_array().expect("rows") {
        assert_eq!(completion_case(s(&row[0]), s(&row[1])), s(&row[2]), "{row}");
    }
}

#[test]
fn match_casefolds_prefix_like_python() {
    for row in helpers()["match"].as_array().expect("rows") {
        assert_eq!(matches(s(&row[0]), s(&row[1])), row[2] == true, "{row}");
    }
}

#[test]
fn complete_pairs_matches_python() {
    let h = helpers();
    let table: Vec<(String, String)> = h["pairs"]
        .as_array()
        .expect("pairs")
        .iter()
        .map(|p| (s(&p[0]).to_string(), s(&p[1]).to_string()))
        .collect();
    let table: Vec<(&str, &str)> = table
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    for row in h["complete_pairs"].as_array().expect("rows") {
        assert_eq!(
            pairs_json(complete_pairs(s(&row[0]), &table)),
            row[1],
            "{row}"
        );
    }
}

#[test]
fn cached_documents_match_python() {
    fixture_cache();
    let h = helpers();
    for name in ["changes", "tickets", "problems"] {
        let r = res(name);
        assert_eq!(json!(cached_fields(r)), h["cached_fields"][name], "{name}");
        assert_eq!(
            json!(fields_or_fallback(r)),
            h["fields_or_fallback"][name],
            "{name}"
        );
        let filters: Vec<Value> = cached_filters(r)
            .into_iter()
            .map(|(id, n)| json!({"id": id, "name": n}))
            .collect();
        assert_eq!(json!(filters), h["cached_filters"][name], "{name}");
    }
    assert_eq!(json!(cached_groups()), h["cached_groups"]);
}

#[test]
fn find_field_and_choices_for_match_python() {
    fixture_cache();
    let h = helpers();
    for name in ["changes", "tickets", "problems"] {
        let r = res(name);
        for row in h["find_field"][name].as_array().expect("rows") {
            let got = find_field(r, s(&row[0])).map(|f| field_name(&f));
            assert_eq!(json!(got), row[1], "{name} {row}");
        }
        for row in h["choices_for"][name].as_array().expect("rows") {
            assert_eq!(json!(choices_for(r, s(&row[0]))), row[1], "{name} {row}");
        }
    }
}

#[test]
fn remote_user_values_matches_python() {
    let h = helpers();
    for (mode, network) in [("offline", false), ("network", true)] {
        for row in h["remote_user_values"][mode].as_array().expect("rows") {
            let remote = FakeRemote::new();
            let env = Env {
                network,
                remote: &remote,
            };
            let got = env.remote_user_values(s(&row["args"][0]), s(&row["args"][1]));
            assert_eq!(pairs_json(got), row["out"], "{mode} {row}");
            assert_eq!(
                Value::Array(remote.calls.take()),
                row["calls"],
                "{mode} {row}"
            );
        }
    }
}

/// Every `fsv.completion` completer case, called through the Rust function of
/// the same name rather than the `_complete` dispatcher.
#[test]
fn completer_functions_match_python() {
    fixture_cache();
    let expected: Value = serde_json::from_str(DYNAMIC).expect("dynamic fixture");
    let mut checked = std::collections::BTreeSet::new();
    for (mode, network) in [("offline", false), ("network", true)] {
        for case in expected[mode]["cases"].as_array().expect("cases") {
            let c = &case["case"];
            let (completer, rname, token) = (s(&c[0]), s(&c[1]), s(&c[2]));
            let remote = FakeRemote::new();
            let env = Env {
                network,
                remote: &remote,
            };
            let r = resource::lookup(rname);
            let got = match (completer, r) {
                ("complete_update_agent_id", _) => complete_update_agent_id(&env, token),
                ("complete_update_group_id", _) => complete_update_group_id(token),
                ("complete_group_query", _) => complete_group_query(token),
                ("complete_field_names", Some(r)) => complete_field_names(r, token),
                ("complete_choice_field_names", Some(r)) => complete_choice_field_names(r, token),
                ("complete_planning_field_names", Some(r)) => {
                    complete_planning_field_names(r, token)
                }
                ("complete_lookup_kind", Some(r)) => complete_lookup_kind(r, token),
                ("complete_lookup_query", Some(r)) => {
                    let kind = extract_flag(&strings(&c[3]), &["--kind", "-k"]);
                    complete_lookup_query(&env, r, &kind, token)
                }
                ("complete_filter_name", Some(r)) => complete_filter_name(r, token),
                ("complete_set", Some(r)) => complete_set(&env, r, token),
                ("complete_where", Some(r)) => complete_where(&env, r, token),
                _ => continue,
            };
            checked.insert(completer);
            assert_eq!(pairs_json(got), case["pairs"], "{mode} {c}");
            assert_eq!(
                Value::Array(remote.calls.take()),
                case["calls"],
                "{mode} {c}"
            );
        }
    }
    assert_eq!(checked.len(), 11, "{checked:?}");
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| s(x).to_string())
        .collect()
}

/// The fixed-list completers (`complete_format`, `complete_store`, ...) live
/// in `STATIC_COMPLETERS` plus the `complete_config_key`/`complete_config_value`
/// arms of `static_pairs`/`complete_in`.
#[test]
fn static_completers_match_python() {
    let expected: Value = serde_json::from_str(DYNAMIC).expect("dynamic fixture");
    let names = [
        "complete_format",
        "complete_store",
        "complete_shell",
        "complete_sort_order",
        "complete_config_key",
        "complete_config_value",
        "complete_cache_target",
        "complete_search_sort",
        "complete_duplicate_mode",
    ];
    let mut seen = std::collections::BTreeSet::new();
    for case in expected["offline"]["cases"].as_array().expect("cases") {
        let completer = s(&case["case"][0]);
        if !names.contains(&completer) {
            continue;
        }
        seen.insert(completer);
        let token = s(&case["case"][2]);
        let got = match static_pairs(completer) {
            Some(table) => complete_pairs(token, table),
            None => complete(completer, "-", token, &[], false),
        };
        assert_eq!(pairs_json(got), case["pairs"], "{}", case["case"]);
    }
    assert_eq!(seen.len(), names.len());
}

/// `complete_update_choice` and `complete_help_topic` have no arm in
/// `fsv._complete._completions`, so the thin path prints nothing for them.
#[test]
fn completers_without_thin_dispatch_print_nothing() {
    for completer in ["complete_update_choice", "complete_help_topic"] {
        assert!(complete(completer, "changes", "O", &[], false).is_empty());
        assert!(complete(completer, "-", "a", &[], false).is_empty());
    }
}
