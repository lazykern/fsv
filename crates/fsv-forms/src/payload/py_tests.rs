//! `fsv.forms.payload` against the Python reference (section `payload` of
//! `testdata/py_golden.json`): change / ticket / service-request bodies over
//! a grid of form states and keyword arguments, compared exactly (values and
//! key order), plus the private helpers they are built from.

use serde_json::{Value as Json, json};

use super::*;
use crate::py_golden::{build_state, check, check_exact, from_v, g, obj, run, strip_tzinfo, v};
use crate::state::Ack;

/// `payload_state(name)` of the generator: set errors are ignored, like
/// `run()` there.
fn payload_state(name: &str) -> FormState {
    let (setup, sets, acks) = state_setup(name);
    let mut st = build_state(&setup).expect("state");
    for s in sets.as_array().unwrap() {
        let _ = st.set(s[0].as_str().unwrap(), from_v(&s[1]));
    }
    if let Some(acks) = acks.as_array().filter(|a| !a.is_empty()) {
        let acks: Vec<Ack> = acks.iter().map(|a| Ack::from(fsv_py::py_str(a))).collect();
        st.acknowledge(&acks).expect("acknowledge");
    }
    st
}

fn state_setup(name: &str) -> (Json, Json, Json) {
    let s = &g(&["payload", "setups"])[name];
    (s[0].clone(), s[1].clone(), s[2].clone())
}

fn body(b: Map<String, Json>) -> Json {
    let custom_keys: Vec<String> = b
        .get("custom_fields")
        .and_then(Json::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    json!({
        "keys": b.keys().cloned().collect::<Vec<_>>(),
        "custom_keys": custom_keys,
        "body": Json::Object(b),
    })
}

fn opt_str(o: &Json, k: &str) -> Option<String> {
    o.get(k).and_then(Json::as_str).map(str::to_string)
}

fn module_opts(o: &Json) -> ModuleOptions {
    ModuleOptions {
        requester_email: opt_str(o, "requester_email"),
        requester_name: opt_str(o, "requester_name"),
        status: o.get("status").map(from_v),
        account_id: o.get("account_id").cloned().unwrap_or(Json::Null),
        allow_incomplete: o
            .get("allow_incomplete")
            .and_then(Json::as_bool)
            .unwrap_or(false),
    }
}

fn sr_opts(o: &Json) -> SrOptions {
    let field_attachments = o
        .get("field_attachments")
        .and_then(Json::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, ids)| (k.clone(), ids.as_array().unwrap().clone()))
                .collect()
        })
        .unwrap_or_default();
    SrOptions {
        field_attachments,
        attachments: o
            .get("attachments")
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default(),
        account_id: o.get("account_id").cloned().unwrap_or(Json::Null),
        allow_incomplete: o
            .get("allow_incomplete")
            .and_then(Json::as_bool)
            .unwrap_or(false),
    }
}

/// `dataclasses.replace(item, **overrides)`.
fn sr_item(st: &FormState, o: &Json) -> Option<ServiceItem> {
    let mut item = st.spec.item.clone()?;
    if let Some(over) = o.get("item").and_then(Json::as_object) {
        for (k, x) in over {
            match k.as_str() {
                "allow_attachments" => item.allow_attachments = x.as_bool().unwrap(),
                "attachment_mandatory" => item.attachment_mandatory = x.as_bool().unwrap(),
                "id" => item.id = x.clone(),
                other => panic!("item override {other}"),
            }
        }
    }
    Some(item)
}

/// Python leaves values it cannot encode untouched, so sending the body
/// fails in `json.dumps`; Rust refuses them here with a `TypeError`.
fn check_encoded(label: &str, got: &Json, want: &Json) {
    if want["ok"].get("unencodable").is_some() {
        assert_eq!(
            got["err"][0], "TypeError",
            "{label}: rust {got}, python {want}"
        );
        return;
    }
    check_exact(label, got, &strip_tzinfo(want));
}

fn states() -> Vec<(&'static String, &'static Json)> {
    obj(g(&["payload", "states"])).iter().collect()
}

#[test]
fn ticket_and_change_payloads_match_python() {
    let mut n = 0;
    for (name, row) in states() {
        let st = payload_state(name);
        for kind in ["ticket", "change"] {
            let Some(cases) = row.get(kind).and_then(Json::as_array) else {
                continue;
            };
            for c in cases {
                let opts = module_opts(&c[0]);
                let built = if kind == "ticket" {
                    build_ticket_payload(&st, &opts)
                } else {
                    build_change_payload(&st, &opts)
                };
                let got = run(built, body);
                check_exact(
                    &format!("{name} build_{kind}_payload {}", c[0]),
                    &got,
                    &c[1],
                );
                n += 1;
            }
        }
    }
    assert!(n > 100, "{n} cases");
}

#[test]
fn sr_payloads_match_python() {
    let mut n = 0;
    for (name, row) in states() {
        let st = payload_state(name);
        for c in row["sr"].as_array().unwrap() {
            let item = sr_item(&st, &c[0]);
            let email = c[1].as_str().unwrap();
            let got = run(
                build_sr_payload(item.as_ref(), &st, email, &sr_opts(&c[0])),
                body,
            );
            check_exact(
                &format!("{name} build_sr_payload {} {email}", c[0]),
                &got,
                &c[2],
            );
            n += 1;
        }
    }
    assert!(n > 60, "{n} cases");
}

#[test]
fn excludes_account_and_problems_match_python() {
    for (name, row) in states() {
        let st = payload_state(name);
        let pv: Vec<Json> = st
            .payload_values()
            .iter()
            .map(|(k, x)| json!([k, v(x)]))
            .collect();
        check(
            &format!("{name} payload_values"),
            &json!(pv),
            &row["payload_values"],
        );
        for c in row["excludes_lists"].as_array().unwrap() {
            let got = run(excludes_lists(&st.evaluated, &c[0]), |m| json!(m));
            check_exact(&format!("{name} excludes_lists {}", c[0]), &got, &c[1]);
        }
        for c in row["account"].as_array().unwrap() {
            let got = run(account(&st.evaluated, &c[0]), |a| json!(a));
            check_exact(&format!("{name} _account {}", c[0]), &got, &c[1]);
        }
        for c in row["problems"].as_array().unwrap() {
            let skip: Vec<String> = serde_json::from_value(c[0].clone()).unwrap();
            let got = problems(&st, &skip).expect("problems");
            check_exact(&format!("{name} _problems {}", c[0]), &json!(got), &c[1]);
        }
    }
}

#[test]
fn module_body_matches_python() {
    for (name, row) in states() {
        let st = payload_state(name);
        for c in row["module_body"].as_array().unwrap() {
            let (keys, order): (&[(&str, &str)], &[&str]) = match c[0].as_str().unwrap() {
                "change" => (&CHANGE_KEYS, &CHANGE_ORDER),
                _ => (&TICKET_KEYS, &TICKET_ORDER),
            };
            let status = (!c[1].is_null()).then(|| from_v(&c[1]));
            let got = run(
                module_body(&st, keys, order, status.as_ref()),
                |(top, custom)| {
                    let tk: Vec<String> = top.keys().cloned().collect();
                    let ck: Vec<String> = custom.keys().cloned().collect();
                    json!([top, custom, tk, ck])
                },
            );
            let want = match c[2].get("ok") {
                Some(ok) => {
                    let tk: Vec<String> = ok[0].as_object().unwrap().keys().cloned().collect();
                    let ck: Vec<String> = ok[1].as_object().unwrap().keys().cloned().collect();
                    json!({"ok": [ok[0], ok[1], tk, ck]})
                }
                None => c[2].clone(),
            };
            check_exact(
                &format!("{name} _module_body {} {}", c[0], c[1]),
                &got,
                &want,
            );
        }
    }
}

#[test]
fn encode_value_matches_python() {
    for (name, row) in states() {
        let st = payload_state(name);
        let values = &st.evaluated.values;
        for c in row["encode_state"].as_array().unwrap() {
            let f = st.spec.get(c[0].as_str().unwrap()).expect("field");
            let value = st
                .payload_values()
                .get(&f.api_name)
                .cloned()
                .expect("value");
            let got = run(encode_value(&st.spec, f, &value, values), |x| x);
            check_encoded(&format!("{name} encode_value {}", c[0]), &got, &c[1]);
        }
    }
    for (name, rows) in obj(g(&["payload", "encode_value"])) {
        let st = payload_state(name);
        let values = &st.evaluated.values;
        for c in rows.as_array().unwrap() {
            let f = st.spec.get(c[0].as_str().unwrap()).expect("field");
            let got = run(encode_value(&st.spec, f, &from_v(&c[1]), values), |x| x);
            check_encoded(
                &format!("{name} encode_value {} {}", c[0], c[1]),
                &got,
                &c[2],
            );
        }
    }
}

#[test]
fn lookup_id_and_name_list_match_python() {
    let f = Field::new("who", "Who", Kind::LookupUser);
    for c in g(&["payload", "lookup_id"]).as_array().unwrap() {
        let got = run(lookup_id(&f, &from_v(&c[0])), |x| x);
        check_exact(&format!("_lookup_id {}", c[0]), &got, &c[1]);
    }
    for c in g(&["payload", "name_list"]).as_array().unwrap() {
        let names: Vec<String> = serde_json::from_value(c[0].clone()).unwrap();
        check_exact(
            &format!("_name_list {}", c[0]),
            &json!(name_list(&names)),
            &c[1],
        );
    }
}

/// `problem_workflows.create_plan`'s calls: `_check(state, False, skip=(requester,))`
/// and `_module_body(state, {}, PROBLEM_ORDER, evaluated status)`, from the
/// public API (`testdata/problem_expected.json`, `gen_problem_body.py`).
#[test]
fn check_and_module_body_as_problem_create_calls_them() {
    const PROBLEM_ORDER: [&str; 5] = ["subject", "description", "status", "priority", "impact"];
    let want: Json = serde_json::from_str(include_str!("../../testdata/problem_expected.json"))
        .expect("problem_expected.json");
    let mut checks = 0;
    for (name, row) in want.as_object().unwrap() {
        let st = payload_state(name);
        for c in row["check"].as_array().unwrap() {
            let skip: Vec<String> = serde_json::from_value(c[1].clone()).unwrap();
            let got = run(super::check(&st, c[0].as_bool().unwrap(), &skip), |()| {
                Json::Null
            });
            let want = if c[2].get("ok").is_some() {
                json!({"ok": null})
            } else {
                c[2].clone()
            };
            check_exact(&format!("{name} _check {} {}", c[0], c[1]), &got, &want);
            checks += 1;
        }
        let status = st.evaluated.values.get("status").cloned();
        let got = run(
            super::module_body(&st, &[], &PROBLEM_ORDER, status.as_ref()),
            |(top, custom)| {
                let tk: Vec<String> = top.keys().cloned().collect();
                let ck: Vec<String> = custom.keys().cloned().collect();
                json!([top, custom, tk, ck])
            },
        );
        let want = &row["module_body"];
        let want = match want.get("ok") {
            Some(ok) => {
                let tk: Vec<String> = ok[0].as_object().unwrap().keys().cloned().collect();
                let ck: Vec<String> = ok[1].as_object().unwrap().keys().cloned().collect();
                json!({"ok": [ok[0], ok[1], tk, ck]})
            }
            None => want.clone(),
        };
        check_exact(&format!("{name} _module_body problem"), &got, &want);
    }
    assert_eq!(checks, 120);
}
