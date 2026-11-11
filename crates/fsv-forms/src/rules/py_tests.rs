//! `fsv.forms.rules` against the Python reference (section `rules` of
//! `testdata/py_golden.json`). Private engine steps are called directly
//! with the same inputs the generator gave their Python counterparts.

use std::collections::HashSet;

use serde_json::{Map, Value as Json, json};

use super::*;
use crate::py_golden::{
    self as pg, check, conflict, decision, engine, evaluated, exclude, explanation, from_v, g,
    message, obj, opt_decision, restriction, run, spec_of, v, values_from_v,
};
use crate::value::{Value, Values};

fn bare_engine(opts: EngineOptions) -> RulesEngine {
    RulesEngine::new(pg::change_spec(), Some(&json!([])), opts).expect("engine")
}

fn clear_warnings(e: &RulesEngine) {
    e.warnings.lock().expect("warnings").clear();
}

// ---------------------------------------------------------------------------
// projections

fn group_json(g: &ConditionGroup) -> Json {
    json!({
        "mode": g.mode,
        "items": g.items.iter().map(|i| match i {
            ConditionItem::Group(g) => group_json(g),
            ConditionItem::Condition(c) => json!({
                "name": c.name, "field": c.field, "operator": c.operator,
                "values": c.values, "source": c.source,
            }),
        }).collect::<Vec<_>>(),
    })
}

fn actions_json(acts: &[Action]) -> Json {
    Json::Array(
        acts.iter()
            .map(|a| {
                json!({
                    "property": a.property, "targets": a.targets, "values": a.values,
                    "all_targets": a.all_targets.iter().map(|t| json!([t.api, t.raw, t.base, t.custom])).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn rule_json(r: &Rule) -> Json {
    json!({
        "id": r.id, "name": r.name, "condition": group_json(&r.condition),
        "actions": actions_json(&r.actions), "reversible": r.reversible,
        "form_type": r.form_type, "user_type": r.user_type, "is_validation": r.is_validation,
        "index": r.index, "fields": r.fields, "unconditional": r.unconditional,
    })
}

fn rules_json(rs: &[Rule]) -> Json {
    Json::Array(rs.iter().map(rule_json).collect())
}

fn excludes_json(x: &Excludes) -> Json {
    Json::Object(
        x.items()
            .iter()
            .map(|(k, xs)| (k.to_string(), Json::Array(xs.iter().map(exclude).collect())))
            .collect(),
    )
}

fn cr_opt(r: Option<&ChoiceRestriction>) -> Json {
    r.map(restriction).unwrap_or(Json::Null)
}

fn state_json(st: &State) -> Json {
    let reg_names = ["visible", "required", "disabled"];
    json!({
        "vis": st.vis.iter().map(|(k, b)| (k.clone(), json!(b))).collect::<Map<_, _>>(),
        "req": st.req.iter().map(|(k, b)| (k.clone(), json!(b))).collect::<Map<_, _>>(),
        "values": st.values.iter().map(|(k, x)| json!([k, v(x)])).collect::<Vec<_>>(),
        "dis": st.dis,
        "populated": st.populated.iter().map(|(k, id)| json!([k, id])).collect::<Vec<_>>(),
        "messages": st.messages.iter().map(|(i, ms)| json!([i, ms.iter().map(message).collect::<Vec<_>>()])).collect::<Vec<_>>(),
        "last_match": st.last_match.iter().map(|(i, id)| json!([i, id])).collect::<Vec<_>>(),
        "why": st.why.iter().map(|((n, k), d)| json!([n, k, decision(d)])).collect::<Vec<_>>(),
        "matched_decisions": st.matched_decisions.iter().map(|((n, k), ds)| json!([n, k, ds.iter().map(decision).collect::<Vec<_>>()])).collect::<Vec<_>>(),
        "reg": reg_names.iter().zip(st.reg.iter()).map(|(p, m)| (p.to_string(), Json::Array(m.iter().map(|(k, x)| json!([k, exclude(x)])).collect()))).collect::<Map<_, _>>(),
        "pseudo_req": st.pseudo_req,
        "restricted": st.restricted.iter().map(|(k, r)| (k.clone(), restriction(r))).collect::<Map<_, _>>(),
        "restricted_by": st.restricted_by.iter().map(|(k, d)| (k.clone(), decision(d))).collect::<Map<_, _>>(),
        "refused": st.refused.iter().map(|(n, d)| json!([n, opt_decision(d.as_ref())])).collect::<Vec<_>>(),
        "overridden": st.overridden.iter().map(|(k, (a, id))| json!([k, v(a), id])).collect::<Vec<_>>(),
    })
}

fn names_of(e: &RulesEngine) -> Vec<String> {
    e.spec
        .ordered()
        .iter()
        .map(|f| f.api_name.clone())
        .collect()
}

fn strs(j: &Json) -> Vec<String> {
    j.as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_str().expect("str").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn events_from(j: &Json) -> Vec<(String, Value)> {
    j.as_array()
        .expect("events")
        .iter()
        .map(|e| (e[0].as_str().expect("name").to_string(), from_v(&e[1])))
        .collect()
}

// ---------------------------------------------------------------------------
// engine construction, loading and parsing

#[test]
fn engines_load_like_python() {
    for (name, want) in obj(g(&["rules", "engines"])) {
        let e = engine(name);
        let got = json!({
            "rules": rules_json(&e.rules), "skipped": e.skipped, "duplicates": e.duplicates,
            "unsupported": e.unsupported, "warnings": e.warnings(),
        });
        for k in ["rules", "skipped", "duplicates", "unsupported", "warnings"] {
            check(&format!("{name}.{k}"), &got[k], &want[k]);
        }
        for (mode, w) in obj(&want["rules_for"]) {
            let got: Vec<Json> = e
                .rules_for(mode)
                .iter()
                .map(|r| json!([r.id, r.index]))
                .collect();
            check(&format!("{name}.rules_for({mode})"), &json!(got), w);
        }
        for (mode, w) in obj(&want["unavailable_dependencies"]) {
            check(
                &format!("{name}.unavailable_dependencies({mode})"),
                &json!(e.unavailable_dependencies(mode)),
                w,
            );
        }
        let mut names: Vec<String> = e
            .spec
            .fields()
            .iter()
            .rev()
            .map(|f| f.api_name.clone())
            .collect();
        names.push("zzz_unknown".into());
        names.push("aaa_unknown".into());
        check(
            &format!("{name}.position_order"),
            &json!(e.position_order(names.iter())),
            &want["position_order"],
        );
        let cf: Vec<Vec<String>> = e
            .rules
            .iter()
            .map(|r| {
                let mut out = BTreeSet::new();
                condition_fields(&r.condition, &mut out);
                out.into_iter().collect()
            })
            .collect();
        check(
            &format!("{name}.condition_fields"),
            &json!(cf),
            &want["condition_fields"],
        );
    }
}

#[test]
fn filter_matches_python() {
    let case = g(&["rules", "filter"]);
    let raws: Vec<&Map<String, Json>> = case["raws"]
        .as_array()
        .expect("raws")
        .iter()
        .map(|r| r.as_object().expect("rule"))
        .collect();
    for (ename, want) in obj(&case["out"]) {
        let e = engine(ename);
        let got: Vec<Option<String>> = raws.iter().map(|r| e.filter(r)).collect();
        check(&format!("filter {ename}"), &json!(got), want);
    }
}

#[test]
fn dedupe_matches_python() {
    let case = g(&["rules", "dedupe"]);
    let raws: Vec<&Map<String, Json>> = case["raws"]
        .as_array()
        .expect("raws")
        .iter()
        .map(|r| r.as_object().expect("rule"))
        .collect();
    for (policy, want) in obj(&case["out"]) {
        let mut e = bare_engine(EngineOptions {
            dedupe: Dedupe::parse(policy).expect("policy"),
            ..EngineOptions::default().module("change")
        });
        let kept: Vec<Json> = e
            .dedupe_rules(raws.clone())
            .iter()
            .map(|r| r.get("name").cloned().unwrap_or(Json::Null))
            .collect();
        check(
            &format!("dedupe {policy} kept"),
            &json!(kept),
            &want["kept"],
        );
        check(
            &format!("dedupe {policy} duplicates"),
            &json!(e.duplicates),
            &want["duplicates"],
        );
    }
}

#[test]
fn load_matches_python() {
    let case = g(&["rules", "load"]);
    for (name, want) in obj(&case["out"]) {
        let mut e = engine("ops");
        e.skipped.clear();
        e.duplicates.clear();
        let data = &case["cases"][name];
        let got = run(e.load(Some(data).filter(|d| !d.is_null())), |rs| {
            rules_json(&rs)
        });
        check(&format!("load {name}"), &got, &want["out"]);
        check(
            &format!("load {name} skipped"),
            &json!(e.skipped),
            &want["skipped"],
        );
        check(
            &format!("load {name} duplicates"),
            &json!(e.duplicates),
            &want["duplicates"],
        );
        check(
            &format!("load {name} warnings"),
            &json!(e.warnings()),
            &want["warnings"],
        );
    }
}

#[test]
fn parse_rule_matches_python() {
    let case = g(&["rules", "parse_rule"]);
    let raws = case["cases"].as_array().expect("cases");
    for (strict, rows) in obj(&case["out"]) {
        for (i, (raw, want)) in raws.iter().zip(rows.as_array().expect("rows")).enumerate() {
            let mut e = bare_engine(EngineOptions {
                strict: strict == "true",
                ..EngineOptions::default().module("change")
            });
            let got = run(e.parse_rule(raw.as_object().expect("rule"), i), |r| {
                r.as_ref().map(rule_json).unwrap_or(Json::Null)
            });
            let label = format!("parse_rule strict={strict} #{i}");
            check(&label, &got, &want["out"]);
            check(&label, &json!(e.unsupported), &want["unsupported"]);
            check(&label, &json!(e.warnings()), &want["warnings"]);
        }
    }
}

#[test]
fn parse_group_matches_python() {
    let case = g(&["rules", "parse_group"]);
    let datas = case["cases"].as_array().expect("cases");
    for (i, (data, want)) in datas
        .iter()
        .zip(case["out"].as_array().expect("out"))
        .enumerate()
    {
        let e = bare_engine(EngineOptions::default().module("change"));
        let got = match e.parse_group(data, "R") {
            Ok(g) => json!({ "ok": group_json(&g) }),
            Err(Skip(msg)) => json!({ "skip": msg }),
        };
        check(&format!("parse_group #{i} {data}"), &got, &want["out"]);
        check(
            &format!("parse_group #{i} warnings"),
            &json!(e.warnings()),
            &want["warnings"],
        );
    }
}

#[test]
fn parse_actions_matches_python() {
    let case = g(&["rules", "parse_actions"]);
    let datas = case["cases"].as_array().expect("cases");
    for (i, (data, want)) in datas
        .iter()
        .zip(case["out"].as_array().expect("out"))
        .enumerate()
    {
        let e = bare_engine(EngineOptions::default().module("change"));
        let got = match e.parse_actions(data.as_array().expect("list"), "R") {
            Ok(acts) => json!({ "ok": actions_json(&acts) }),
            Err(Skip(msg)) => json!({ "skip": msg }),
        };
        check(&format!("parse_actions #{i}"), &got, &want["out"]);
        check(
            &format!("parse_actions #{i} warnings"),
            &json!(e.warnings()),
            &want["warnings"],
        );
    }
}

// ---------------------------------------------------------------------------
// small value types

#[test]
fn rule_context_workspaces_matches_python() {
    for row in g(&["rules", "workspaces"]).as_array().expect("rows") {
        let ctx = RuleContext {
            workspace_id: Some(row[0].clone()).filter(|w| !w.is_null()),
            ..RuleContext::default()
        };
        let got = ctx.workspaces().map(|s| {
            let mut v: Vec<String> = s.into_iter().collect();
            v.sort();
            v
        });
        check(&format!("workspaces({})", row[0]), &json!(got), &row[1]);
    }
}

#[test]
fn rule_applies_to_matches_python() {
    for row in g(&["rules", "applies_to"]).as_array().expect("rows") {
        let r = Rule {
            id: json!(1),
            name: "r".into(),
            condition: ConditionGroup {
                mode: "all".into(),
                items: vec![],
            },
            actions: vec![],
            reversible: false,
            form_type: row[0].as_str().unwrap().into(),
            user_type: row[1].as_str().unwrap().into(),
            is_validation: false,
            index: 0,
            fields: BTreeSet::new(),
            unconditional: true,
        };
        let got = r.applies_to(row[2].as_str().unwrap(), row[3].as_str().unwrap());
        check(&row.to_string(), &json!(got), &row[4]);
    }
}

#[test]
fn message_decision_exclude_name_match_python() {
    for row in g(&["rules", "message_key"]).as_array().expect("rows") {
        let m = &row[0];
        let msg = Message {
            text: m[0].as_str().unwrap().into(),
            rule_id: m[1].clone(),
            rule_name: m[2].as_str().unwrap().into(),
            blocking: m[3].as_bool().unwrap(),
        };
        let (a, b) = msg.key();
        check(&format!("key {m}"), &json!([a, b]), &row[1]);
    }
    for row in g(&["rules", "decision_describe"]).as_array().expect("rows") {
        let d = &row[0];
        let dec = Decision {
            rule_id: d[0].clone(),
            rule_name: d[1].as_str().unwrap().into(),
            action: d[2].as_str().unwrap().into(),
            matched: d[3].as_bool().unwrap(),
            event: d[4].as_str().map(str::to_string),
            rule_index: d[5].as_i64().unwrap(),
        };
        check(&format!("describe {d}"), &json!(dec.describe()), &row[1]);
    }
    for row in g(&["rules", "exclude_name"]).as_array().expect("rows") {
        let x = ExcludeName {
            base: row[0][0].as_str().unwrap().into(),
            custom: row[0][1].as_bool().unwrap(),
        };
        check(
            &format!("name {row}"),
            &run(x.name(row[1].as_str()), |s| json!(s)),
            &row[2],
        );
    }
}

#[test]
fn choice_restriction_offers_below_match_python() {
    let mut inner = IndexMap::new();
    inner.insert("vpn".to_string(), None);
    let mut allowed = IndexMap::new();
    allowed.insert("live".to_string(), None);
    allowed.insert(
        "cannot log in".to_string(),
        Some(ChoiceRestriction {
            allowed: inner,
            rule_id: json!(1),
            rule_name: "x".into(),
        }),
    );
    let cr = ChoiceRestriction {
        allowed,
        rule_id: json!(1),
        rule_name: "x".into(),
    };
    let want = g(&["rules", "choice_restriction"]);
    for row in want["offers"].as_array().unwrap() {
        check(
            &format!("offers {}", row[0]),
            &json!(cr.offers(&row[0])),
            &row[1],
        );
    }
    for row in want["below"].as_array().unwrap() {
        check(
            &format!("below {}", row[0]),
            &cr_opt(cr.below(&row[0])),
            &row[1],
        );
    }
}

#[test]
fn is_empty_and_same_match_python() {
    for row in g(&["rules", "is_empty"]).as_array().unwrap() {
        let val = from_v(&row[0]);
        let got = is_empty(Some(&val).filter(|v| !v.is_null()));
        check(&format!("is_empty {}", row[0]), &json!(got), &row[1]);
        check(
            &format!("is_empty {}", row[0]),
            &json!(is_empty(Some(&val))),
            &row[1],
        );
    }
    for row in g(&["rules", "same"]).as_array().unwrap() {
        let a = from_v(&row[0]);
        let b = from_v(&row[1]);
        check(
            &format!("same {} {}", row[0], row[1]),
            &json!(same(Some(&a), Some(&b))),
            &row[2],
        );
    }
}

// ---------------------------------------------------------------------------
// evaluation

#[test]
fn evaluate_matches_python() {
    for (i, row) in g(&["rules", "evaluate"])
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let ename = row["engine"].as_str().unwrap();
        let e = engine(ename);
        let values = values_from_v(&row["values"]);
        let mode = row["mode"].as_str().unwrap();
        let kw = &row["kw"];
        let base = kw.get("base").map(values_from_v);
        let order = kw.get("order").map(strs);
        let events = kw.get("events").map(events_from);
        let model = kw
            .get("model")
            .and_then(|m| m.as_str())
            .map(|m| Model::parse(m).unwrap());
        let label = format!("evaluate #{i} {ename} {}", row["values"]);
        if kw.as_object().is_none_or(|o| o.is_empty())
            && let Ok(ev) = e.evaluate(&values, mode)
        {
            check(
                &format!("{label} defaults"),
                &evaluated(&ev),
                &row["out"]["ok"],
            );
        }
        let res = e.evaluate_with(
            &values,
            mode,
            EvalOptions {
                base: base.as_ref(),
                order: order.as_deref(),
                model,
                events: events.as_deref(),
            },
        );
        let ev = match res {
            Ok(ev) => ev,
            Err(err) => {
                check(&label, &run::<()>(Err(err), |_| Json::Null), &row["out"]);
                continue;
            }
        };
        check(&label, &evaluated(&ev), &row["out"]["ok"]);
        let expl: Vec<Json> = ev.explain(None).unwrap().iter().map(explanation).collect();
        check(&format!("{label} explain"), &json!(expl), &row["explain"]);
        check(
            &format!("{label} explain_some"),
            &run(ev.explain(Some(&["Stage", "nope"])), |xs| {
                Json::Array(xs.iter().map(explanation).collect())
            }),
            &row["explain_some"],
        );
        let names = names_of(&e);
        for n in &names {
            let f = e.spec.get(n).unwrap();
            if f.kind.is_choice() {
                let got: Vec<String> = ev
                    .choices(n)
                    .unwrap()
                    .iter()
                    .map(|c| c.value.clone())
                    .collect();
                check(
                    &format!("{label} choices({n})"),
                    &json!(got),
                    &row["choices"][n],
                );
                let pool = offered_choices(&e.spec, f, &ev.values, &ev.restricted).unwrap();
                let got: Vec<&str> = pool.iter().map(|c| c.value.as_str()).collect();
                check(
                    &format!("{label} offered_choices({n})"),
                    &json!(got),
                    &row["offered_choices"][n],
                );
            }
            let (r, d) = ev.restriction_for(n).unwrap();
            check(
                &format!("{label} restriction_for({n})"),
                &json!([cr_opt(r), opt_decision(d)]),
                &row["restriction_for"][n],
            );
            for p in ["visible", "required", "disabled"] {
                check(
                    &format!("{label} decision({n}, {p})"),
                    &opt_decision(ev.decision(n, p)),
                    &row["decision"][n][p],
                );
            }
        }
        for n in names.iter().map(String::as_str).chain(["nope"]) {
            check(
                &format!("{label} is_visible({n})"),
                &json!(ev.is_visible(n)),
                &row["is_visible"][n],
            );
            check(
                &format!("{label} is_required({n})"),
                &json!(ev.is_required(n)),
                &row["is_required"][n],
            );
        }
        let blocking: Vec<Json> = ev.blocking_messages().into_iter().map(message).collect();
        check(
            &format!("{label} blocking_messages"),
            &json!(blocking),
            &row["blocking_messages"],
        );
        for (c, w) in ev
            .conflicts
            .iter()
            .zip(row["out"]["ok"]["conflicts"].as_array().unwrap())
        {
            check(
                &format!("{label} conflict.describe"),
                &json!(c.describe()),
                &w["describe"],
            );
            check(&format!("{label} conflict"), &conflict(c), w);
        }
        check(
            &format!("{label} warnings"),
            &json!(e.warnings()),
            &row["warnings"],
        );
    }
}

#[test]
fn event_order_matches_python() {
    let e = engine("change_legacy");
    for row in g(&["rules", "event_order"]).as_array().unwrap() {
        let values = values_from_v(&row[0]);
        let order = (!row[1].is_null()).then(|| strs(&row[1]));
        check(
            &format!("event_order {row}"),
            &json!(e.event_order(&values, order.as_deref())),
            &row[2],
        );
    }
}

#[test]
fn evaluate_replay_excludes_finish_match_python() {
    for row in g(&["rules", "evaluate_replay"]).as_array().unwrap() {
        let ename = row["engine"].as_str().unwrap();
        let e = engine(ename);
        let user = values_from_v(&row["user"]);
        let only = strs(&row["user_only"]);
        let only_refs: Vec<&String> = only.iter().collect();
        let st = e
            .evaluate_replay(&e.rules_for("new"), &user, &only_refs)
            .expect("replay");
        let label = format!("evaluate_replay {ename} {}", row["user"]);
        check(&label, &state_json(&st), &row["state"]);
        check(
            &format!("{label} excludes"),
            &excludes_json(&e.excludes(&st)),
            &row["excludes"],
        );
        let ev = e.finish(st, Model::Replay).expect("finish");
        check(&format!("{label} finish"), &evaluated(&ev), &row["finish"]);
        check(
            &format!("{label} warnings"),
            &json!(e.warnings()),
            &row["warnings"],
        );
    }
}

#[test]
fn evaluate_events_excludes_finish_match_python() {
    for row in g(&["rules", "evaluate_events"]).as_array().unwrap() {
        let ename = row["engine"].as_str().unwrap();
        let e = engine(ename);
        let base = values_from_v(&row["base"]);
        let events = events_from(&row["events"]);
        let st = e
            .evaluate_events(&e.rules_for("new"), base, &events)
            .expect("events");
        let label = format!("evaluate_events {ename} {}", row["events"]);
        check(&label, &state_json(&st), &row["state"]);
        check(
            &format!("{label} excludes"),
            &excludes_json(&e.excludes(&st)),
            &row["excludes"],
        );
        let ev = e.finish(st, Model::Event).expect("finish");
        check(&format!("{label} finish"), &evaluated(&ev), &row["finish"]);
        check(
            &format!("{label} warnings"),
            &json!(e.warnings()),
            &row["warnings"],
        );
    }
}

#[test]
fn clear_lower_levels_matches_python() {
    for row in g(&["rules", "clear_lower_levels"]).as_array().unwrap() {
        let e = engine(row["engine"].as_str().unwrap());
        let mut st = e.fresh(values_from_v(&row["values"]));
        let keys: Vec<String> = st.values.keys().cloned().collect();
        for k in keys {
            st.populated.insert(k, json!(1));
        }
        let cleared = e.clear_lower_levels(row["name"].as_str().unwrap(), &mut st);
        let mut cleared: Vec<String> = cleared.into_iter().collect();
        cleared.sort();
        let label = format!("clear_lower_levels {row}");
        check(&label, &json!(cleared), &row["cleared"]);
        let after: Vec<Json> = st.values.iter().map(|(k, x)| json!([k, v(x)])).collect();
        check(&label, &json!(after), &row["values_after"]);
        let pop: Vec<&String> = st.populated.keys().collect();
        check(&label, &json!(pop), &row["populated"]);
    }
}

#[test]
fn apply_matches_python() {
    for row in g(&["rules", "apply"]).as_array().unwrap() {
        let ename = row["engine"].as_str().unwrap();
        let e = engine(ename);
        let mut st = e.fresh(values_from_v(&row["values"]));
        for (step, want) in row["steps"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["states"].as_array().unwrap())
        {
            let rule = &e.rules[step[0].as_u64().unwrap() as usize];
            let ok = step[1].as_bool().unwrap();
            let event = step[2].as_str();
            let opts = step.get(3).cloned().unwrap_or(json!({}));
            let fixed: Option<Values> = opts.get("fixed").map(|names| {
                strs(names)
                    .into_iter()
                    .map(|k| {
                        let val = st.values.get(&k).cloned().unwrap_or(Value::Null);
                        (k, val)
                    })
                    .collect()
            });
            let user_set: Option<HashSet<String>> =
                opts.get("user_set").map(|n| strs(n).into_iter().collect());
            e.apply(rule, ok, &mut st, event, fixed.as_ref(), user_set.as_ref())
                .expect("apply");
            check(&format!("apply {ename} {step}"), &state_json(&st), want);
        }
        check(
            &format!("apply {ename} warnings"),
            &json!(e.warnings()),
            &row["warnings"],
        );
    }
}

#[test]
fn restriction_and_restrict_match_python() {
    let e = engine("restrict");
    let rule = |id: i64| e.rules.iter().find(|r| r.id == json!(id)).expect("rule");
    for row in g(&["rules", "restriction"]).as_array().unwrap() {
        clear_warnings(&e);
        let f = e.spec.get(row["field"].as_str().unwrap()).unwrap();
        let vals = row["values"].as_array().unwrap();
        let got = e.restriction(f, vals, rule(row["rule"].as_i64().unwrap()));
        let label = format!("restriction {row}");
        check(&label, &cr_opt(got.as_ref()), &row["out"]);
        check(&label, &json!(e.warnings()), &row["warnings"]);
    }
    let e = engine("restrict");
    let mut st = e.fresh(Values::new());
    for row in g(&["rules", "restrict"]).as_array().unwrap() {
        let r = e.rules.iter().find(|r| r.id == row["rule"]).expect("rule");
        e.restrict(
            r,
            &r.actions[0],
            row["ok"].as_bool().unwrap(),
            &mut st,
            row["event"].as_str(),
        );
        check(&format!("restrict {row}"), &state_json(&st), &row["state"]);
    }
}

// ---------------------------------------------------------------------------
// matching

#[test]
fn match_group_and_match_condition_match_python() {
    let case = g(&["rules", "match"]);
    let sets: Vec<Values> = case["value_sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(values_from_v)
        .collect();
    for (ename, want) in obj(&case["out"]) {
        let e = engine(ename);
        for (r, wrow) in e.rules.iter().zip(want["rows"].as_array().unwrap()) {
            for (vals, w) in sets.iter().zip(wrow.as_array().unwrap()) {
                let label = format!("match {ename} rule {} {vals:?}", r.id);
                check(
                    &label,
                    &json!(e.match_group(&r.condition, vals).unwrap()),
                    &w[0],
                );
                let conds: Vec<bool> = r
                    .condition
                    .items
                    .iter()
                    .filter_map(|c| match c {
                        ConditionItem::Condition(c) => Some(e.match_condition(c, vals).unwrap()),
                        ConditionItem::Group(_) => None,
                    })
                    .collect();
                check(&label, &json!(conds), &w[1]);
            }
        }
        check(
            &format!("match {ename} warnings"),
            &json!(e.warnings()),
            &want["warnings"],
        );
    }
}

#[test]
fn value_keys_matches_python() {
    for row in g(&["rules", "value_keys"]).as_array().unwrap() {
        let e = engine(row[0].as_str().unwrap());
        let f = row[1].as_str().map(|n| e.spec.get(n).expect("field"));
        let val = from_v(&row[2]);
        let eff = values_from_v(&row[3]);
        let got = run(e.value_keys(f, Some(&val), &eff), |keys| {
            json!(
                keys.into_iter()
                    .map(|k| {
                        let mut k: Vec<String> = k.into_iter().collect();
                        k.sort();
                        k
                    })
                    .collect::<Vec<_>>()
            )
        });
        check(&format!("value_keys {row}"), &got, &row[4]);
    }
}

#[test]
fn populate_value_matches_python() {
    for row in g(&["rules", "populate_value"]).as_array().unwrap() {
        let e = engine(row[0].as_str().unwrap());
        clear_warnings(&e);
        let f = e.spec.get(row[1].as_str().unwrap()).unwrap();
        let got = e
            .populate_value(f, row[2].as_array().unwrap(), &e.rules[0])
            .expect("populate");
        let label = format!("populate_value {row}");
        check(&label, &got.as_ref().map(v).unwrap_or(Json::Null), &row[3]);
        check(&label, &json!(e.warnings()), &row[4]);
    }
}

#[test]
fn person_matches_python() {
    for row in g(&["rules", "person"]).as_array().unwrap() {
        let ename = row[0].as_str().unwrap();
        let e = engine(ename);
        clear_warnings(&e);
        let mut got = Map::new();
        for src in ["user", "requester", "requested_for", "system", "user"] {
            got.insert(
                src.into(),
                e.person(src)
                    .map(|m| Json::Object(m.clone()))
                    .unwrap_or(Json::Null),
            );
        }
        check(&format!("person {ename}"), &Json::Object(got), &row[1]);
        check(
            &format!("person {ename} warnings"),
            &json!(e.warnings()),
            &row[2],
        );
    }
}

// ---------------------------------------------------------------------------
// free helpers

#[test]
fn nested_chain_matches_python() {
    for (sname, want) in obj(g(&["rules", "nested_chain"])) {
        let s = spec_of(sname);
        for f in s.fields() {
            let got: Vec<&str> = nested_chain(&s, f)
                .iter()
                .map(|x| x.api_name.as_str())
                .collect();
            check(
                &format!("nested_chain {sname}.{}", f.api_name),
                &json!(got),
                &want[&f.api_name],
            );
        }
    }
}

#[test]
fn choice_in_matches_python() {
    for row in g(&["rules", "choice_in"]).as_array().unwrap() {
        let s = spec_of(row[0].as_str().unwrap());
        let f = s.get(row[1].as_str().unwrap()).unwrap();
        for w in row[2].as_array().unwrap() {
            let got = choice_in_pool(&f.choices, &w[0])
                .map(|c| json!(c.value))
                .unwrap_or(Json::Null);
            check(&format!("_choice_in {} {}", row[1], w[0]), &got, &w[1]);
        }
        for w in row[3].as_array().unwrap() {
            let got = run(f.choice_in(&w[0], &f.choices), |c| {
                c.map(|c| json!(c.value)).unwrap_or(Json::Null)
            });
            check(
                &format!("Field.choice(pool) {} {}", row[1], w[0]),
                &got,
                &w[1],
            );
        }
    }
}

#[test]
fn choice_by_any_matches_python() {
    for row in g(&["rules", "choice_by_any"]).as_array().unwrap() {
        let s = spec_of(row[0].as_str().unwrap());
        let f = s.get(row[1].as_str().unwrap()).unwrap();
        for w in row[2].as_array().unwrap() {
            let got = run(choice_by_any(f, &w[0]), |c| {
                c.map(|c| json!(c.value)).unwrap_or(Json::Null)
            });
            check(&format!("_choice_by_any {} {}", row[1], w[0]), &got, &w[1]);
        }
    }
}

#[test]
fn state_touch_matches_python() {
    let mut st = State::default();
    for row in g(&["rules", "touch"]).as_array().unwrap() {
        let t = &row[1];
        let target = Target {
            api: t[0].as_str().map(str::to_string),
            raw: t[1].as_str().unwrap().into(),
            base: t[2].as_str().unwrap().into(),
            custom: t[3].as_bool().unwrap(),
        };
        st.touch(row[0].as_str().unwrap(), &target);
        check(&format!("touch {row}"), &state_json(&st)["reg"], &row[2]);
    }
}

#[test]
fn target_matches_python() {
    let e = bare_engine(EngineOptions::default());
    let t = |api: Option<&str>, raw: &str, base: &str, custom: bool| Target {
        api: api.map(str::to_string),
        raw: raw.to_string(),
        base: base.to_string(),
        custom,
    };
    for want in [
        t(Some("priority"), "priority", "priority", false),
        t(Some("priority"), "Priority", "Priority", false),
        t(Some("priority"), "custom_fields.priority", "priority", false),
        t(Some("stage"), "stage", "stage", true),
        t(Some("stage"), "Stage", "stage", true),
        t(Some("stage"), "custom_fields.stage", "stage", true),
        t(Some("stage"), "customFields.stage", "stage", true),
        t(None, "cf_risk_notes", "cf_risk_notes", false),
        t(None, "custom_fields.cf_risk_notes", "cf_risk_notes", true),
        t(None, "nope", "nope", false),
        t(None, "custom_fields.nope", "nope", true),
    ] {
        assert_eq!(e.target(&want.raw), want);
    }
}
