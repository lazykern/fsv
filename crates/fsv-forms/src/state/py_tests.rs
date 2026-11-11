//! `fsv.forms.state` against the Python reference (section `state` of
//! `testdata/py_golden.json`): scripted sessions (set / unset /
//! acknowledge, with every query after each step), direct parser calls and
//! the free helpers.

use serde_json::{Map, Value as Json, json};

use super::*;
use crate::py_golden::{
    self as pg, build_state, check, conflict, evaluated, explanation, from_v, g, message, obj,
    opt_decision, run, strip_tzinfo, v, values_from_v,
};
use crate::rules::Message;
use crate::tz::zone;

fn pairs(values: &Values) -> Json {
    Json::Array(values.iter().map(|(k, x)| json!([k, v(x)])).collect())
}

fn diff_json(d: &Diff, spec: &FormSpec) -> Json {
    json!({
        "field": d.field, "appeared": d.appeared, "disappeared": d.disappeared,
        "became_required": d.became_required, "no_longer_required": d.no_longer_required,
        "messages_added": d.messages_added.iter().map(message).collect::<Vec<_>>(),
        "messages_removed": d.messages_removed.iter().map(message).collect::<Vec<_>>(),
        "populated": d.populated.iter().map(|(k, x)| (k.clone(), v(x))).collect::<Map<_, _>>(),
        "cleared": d.cleared, "restricted": d.restricted, "unrestricted": d.unrestricted,
        "describe": d.describe(spec),
    })
}

/// Compare the snapshot keys the generator recorded.
fn check_snap(label: &str, st: &FormState, want: &Json) {
    let names: Vec<String> = st
        .spec
        .ordered()
        .iter()
        .map(|f| f.api_name.clone())
        .collect();
    let mut acked: Vec<Vec<String>> = st
        .acknowledged
        .iter()
        .map(|(a, b)| vec![a.clone(), b.clone()])
        .collect();
    acked.sort();
    let names_of =
        |fs: Vec<&Field>| json!(fs.iter().map(|f| f.api_name.clone()).collect::<Vec<_>>());
    let mut got = json!({
        "values": pairs(&st.values),
        "sources": st.sources.iter().map(|(k, s)| json!([k, s])).collect::<Vec<_>>(),
        "base": pairs(&st.base),
        "tz": [crate::tz::tz_name(&st.tz), st.tz_source],
        "evaluated": evaluated(&st.evaluated),
        "acknowledged": acked,
        "unacknowledged": st.unacknowledged().iter().map(message).collect::<Vec<_>>(),
        "visible_fields": names_of(st.visible_fields()),
        "messages": st.messages().iter().map(message).collect::<Vec<_>>(),
        "missing_required": names_of(st.missing_required()),
        "unsupported_errors": st.unsupported_errors(),
        "unavailable_errors": run(st.unavailable_errors(), |e| json!(e))["ok"],
        "errors": run(st.errors(), |e| json!(e))["ok"],
        "payload_values": pairs(&st.payload_values()),
        "to_review": run(st.to_review(), |rows| json!(rows.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>()))["ok"],
        "check_refused": run(st.check_refused(), |_| Json::Null),
    });
    if want.get("explain").is_some() {
        let m = got.as_object_mut().unwrap();
        m.insert(
            "is_populated".into(),
            Json::Object(
                names
                    .iter()
                    .map(|n| (n.clone(), json!(st.is_populated(n).unwrap())))
                    .collect(),
            ),
        );
        m.insert(
            "dropped".into(),
            Json::Array(
                st.dropped()
                    .iter()
                    .map(|(f, x, d)| json!([f.api_name, v(x), opt_decision(d.as_ref())]))
                    .collect(),
            ),
        );
        m.insert(
            "explain".into(),
            Json::Array(st.explain(None).unwrap().iter().map(explanation).collect()),
        );
        m.insert(
            "conflicts".into(),
            Json::Array(st.conflicts().iter().map(conflict).collect()),
        );
        m.insert(
            "choices".into(),
            Json::Object(
                names
                    .iter()
                    .filter(|n| st.spec.get(n).unwrap().kind.is_choice())
                    .map(|n| {
                        let cs: Vec<String> = st
                            .choices(n)
                            .unwrap()
                            .iter()
                            .map(|c| c.value.clone())
                            .collect();
                        (n.clone(), json!(cs))
                    })
                    .collect(),
            ),
        );
    }
    for (k, w) in obj(want) {
        check(&format!("{label} {k}"), &got[k], w);
    }
    let again = st.evaluate().expect("evaluate");
    check(
        &format!("{label} evaluate()"),
        &evaluated(&again),
        &want["evaluated"],
    );
}

fn run_step(st: &mut FormState, s: &Json) -> Json {
    let name = s.get("name").and_then(Json::as_str).unwrap_or("");
    match s["op"].as_str().unwrap() {
        "set" => {
            let r = st.set(name, from_v(&s["raw"]));
            let spec = st.spec.clone();
            run(r, |d| diff_json(&d, &spec))
        }
        "unset" => {
            let r = st.unset(name);
            let spec = st.spec.clone();
            run(r, |d| diff_json(&d, &spec))
        }
        "ack" => {
            let acks: Vec<Ack> = s["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| Ack::from(a.as_str().unwrap()))
                .collect();
            run(st.acknowledge(&acks), |_| Json::Null)
        }
        "ack_msg" => {
            let acks: Vec<Ack> = s["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| {
                    Ack::Message(Message {
                        text: m[0].as_str().unwrap().into(),
                        rule_id: m[1].clone(),
                        rule_name: m[2].as_str().unwrap().into(),
                        blocking: m[3].as_bool().unwrap(),
                    })
                })
                .collect();
            run(st.acknowledge(&acks), |_| Json::Null)
        }
        other => panic!("op {other}"),
    }
}

#[test]
fn scripted_sessions_match_python() {
    for (name, script) in obj(g(&["state", "scripts"])) {
        let built = build_state(&script["setup"]);
        let mut st = match built {
            Ok(st) => st,
            Err(e) => {
                check(
                    &format!("{name} ctor"),
                    &run::<()>(Err(e), |_| Json::Null),
                    &script["ctor"],
                );
                continue;
            }
        };
        check(
            &format!("{name} ctor"),
            &json!({"ok": null}),
            &script["ctor"],
        );
        check_snap(&format!("{name} initial"), &st, &script["initial"]);
        for (i, s) in script["steps"].as_array().unwrap().iter().enumerate() {
            let label = format!("{name} step {i} {}", s["step"]);
            let got = run_step(&mut st, &s["step"]);
            check(&label, &got, &s["out"]);
            check_snap(&label, &st, &s["snap"]);
        }
    }
}

#[test]
fn parse_matches_python() {
    let case = g(&["state", "parse"]);
    for (sname, rows) in obj(&case["out"]) {
        let setup = if sname == "change" {
            json!({"engine": "change_legacy"})
        } else {
            json!({ "spec": sname })
        };
        let st = build_state(&setup).expect("state");
        for row in rows.as_array().unwrap() {
            let fname = row[0].as_str().unwrap();
            let got = run(st.parse(fname, &from_v(&row[1])), |x| v(&x));
            check(
                &format!("parse {sname}.{fname} {}", row[1]),
                &got,
                &strip_tzinfo(&row[2]),
            );
        }
    }
}

#[test]
fn parse_choice_multi_lookup_match_python() {
    let st = build_state(&json!({"engine": "change_legacy"})).expect("state");
    let spec = st.spec.clone();
    let direct = g(&["state", "direct"]);
    for row in direct["parse_choice"].as_array().unwrap() {
        let f = spec.get(row[0].as_str().unwrap()).unwrap();
        let got = run(st.parse_choice(f, &from_v(&row[1])), |c| json!(c.value));
        check(&format!("_parse_choice {row}"), &got, &row[2]);
    }
    for row in direct["parse_multi"].as_array().unwrap() {
        let f = spec.get(row[0].as_str().unwrap()).unwrap();
        let got = run(st.parse_multi(f, &from_v(&row[1])), |cs| {
            json!(cs.iter().map(|c| c.value.clone()).collect::<Vec<_>>())
        });
        check(&format!("_parse_multi {row}"), &got, &row[2]);
    }
    for row in direct["parse_lookup"].as_array().unwrap() {
        let f = spec.get(row[0].as_str().unwrap()).unwrap();
        let raw = from_v(&row[1]);
        let got = run(st.parse_lookup(f, &raw), |x| v(&x));
        check(&format!("_parse_lookup {row}"), &got, &row[2]);
        let got = run(st.lookup_one(f, &raw), |l| pg::lookup(&l));
        check(&format!("_lookup_one {row}"), &got, &row[3]);
    }
}

#[test]
fn clear_lower_levels_matches_python() {
    for row in g(&["state", "direct", "clear_lower_levels"])
        .as_array()
        .unwrap()
    {
        let mut st = build_state(&json!({"engine": "ops"})).expect("state");
        let spec = st.spec.clone();
        for (k, x) in values_from_v(&row[0]) {
            st.store(spec.get(&k).unwrap(), x, "user");
        }
        let parent = if row[0].get("tier").is_some() {
            "tier"
        } else {
            "area"
        };
        let old = from_v(&row[1]);
        let cleared = st.clear_lower_levels(
            spec.get(parent).unwrap(),
            Some(&old).filter(|o| !o.is_null()),
        );
        let label = format!("_clear_lower_levels {row}");
        check(&label, &json!(cleared), &row[2]);
        check(&label, &pairs(&st.values), &row[3]);
        check(&label, &json!(st.seq), &row[4]);
    }
}

fn field(api: &str, label: &str, kind: Kind) -> Field {
    Field::new(api, label, kind)
}

#[test]
fn free_parsers_match_python() {
    let free = g(&["state", "free"]);
    let num = field("n", "Count", Kind::Number);
    for row in free["parse_number"].as_array().unwrap() {
        let got = run(parse_number(&num, &from_v(&row[0])), |x| v(&x));
        check(&format!("_parse_number {}", row[0]), &got, &row[1]);
    }
    let chk = field("c", "Flag", Kind::Checkbox);
    for row in free["parse_bool"].as_array().unwrap() {
        let got = run(parse_bool(&chk, &from_v(&row[0])), |b| json!(b));
        check(&format!("_parse_bool {}", row[0]), &got, &row[1]);
    }
    let day = field("d", "Day", Kind::Date);
    for row in free["parse_date"].as_array().unwrap() {
        let got = run(parse_date(&day, &from_v(&row[0])), |d| v(&Value::Date(d)));
        check(&format!("_parse_date {}", row[0]), &got, &row[1]);
    }
    for row in free["same_value"].as_array().unwrap() {
        let a = from_v(&row[0]);
        let b = from_v(&row[1]);
        let got = same_value(
            Some(&a).filter(|x| !x.is_null()),
            Some(&b).filter(|x| !x.is_null()),
        );
        check(&format!("_same_value {row}"), &json!(got), &row[2]);
    }
    let when_f = field("w", "When", Kind::Datetime);
    for row in free["parse_datetime"].as_array().unwrap() {
        let z = zone(row[0].as_str().unwrap()).unwrap();
        let got = run(parse_datetime(&when_f, &from_v(&row[1]), &z), |x| v(&x));
        check(&format!("_parse_datetime {row}"), &got, &row[2]);
    }
    let plain = field("a", "Files", Kind::Attachment);
    let mut lim = field("b", "Deck", Kind::Attachment);
    lim.max_files = Some(2);
    lim.allowed_file_types = vec![".pptx".into(), "PDF".into()];
    for row in free["parse_attachments"].as_array().unwrap() {
        let f = if row[0] == "a" { &plain } else { &lim };
        let got = run(parse_attachments(f, &from_v(&row[1])), |x| v(&x));
        check(&format!("_parse_attachments {row}"), &got, &row[2]);
    }
}
