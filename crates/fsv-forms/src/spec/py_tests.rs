//! `fsv.forms.spec` against the Python reference (cases and expected values
//! in `testdata/py_golden.json`, section `spec`).

use std::sync::Arc;

use serde_json::{Value as Json, json};

use super::*;
use crate::py_golden::{
    self as pg, check, choices, field, g, obj, run, sr_spec, ticket_internal_spec, ticket_spec,
};
use crate::value::Values;

fn specs() -> Vec<(&'static str, Arc<FormSpec>)> {
    vec![
        ("change", pg::change_spec()),
        ("ticket", ticket_spec()),
        ("ticket_internal", ticket_internal_spec()),
        ("ops", pg::ops_spec()),
        ("sr", sr_spec()),
    ]
}

fn fields_json(fs: &[Field]) -> Json {
    Json::Array(fs.iter().map(field).collect())
}

#[test]
fn string_predicates_match_python() {
    for (t, want) in obj(g(&["spec", "is_content_type"])) {
        check(t, &json!(is_content_type(t)), want);
    }
    for (t, want) in obj(g(&["spec", "is_planning_type"])) {
        check(t, &json!(is_planning_type(t)), want);
    }
    for (t, want) in obj(g(&["spec", "camel_to_snake"])) {
        check(t, &json!(camel_to_snake(t)), want);
    }
    for (t, want) in obj(g(&["spec", "plain_text"])) {
        check(t, &json!(plain_text(t)), want);
    }
}

#[test]
fn parse_choices_matches_python() {
    let case = g(&["spec", "parse_choices"]);
    check(
        "raw",
        &choices(&parse_choices(Some(&case["raw"]))),
        &case["out"],
    );
    check("none", &choices(&parse_choices(None)), &case["none"]);
}

#[test]
fn split_and_attach_choice_help_match_python() {
    let case = g(&["spec", "split_choice_help"]);
    let base = parse_choices(Some(&json!([
        {"value": "Very High"}, {"value": "High"}, {"value": "Low", "description": "kept"},
        {"value": "Medium"}, {"value": ""}
    ])));
    check("base", &choices(&base), &case["base"]);
    for (name, c) in obj(&case["cases"]) {
        let ht = c["helptext"].as_str().expect("helptext");
        let (chs, rest) = split_choice_help(&base, ht);
        check(name, &choices(&chs), &c["choices"]);
        check(name, &json!(rest), &c["rest"]);
        check(
            name,
            &choices(&attach_choice_descriptions(&base, ht)),
            &c["attach"],
        );
    }
}

#[test]
fn choice_keys_match_python() {
    let cs = parse_choices(Some(&json!([
        {"id": 1, "display_id": 11, "value": "Low", "label": "Low  Prio"},
        {"value": "Only"}, {"id": "X", "value": "X"}
    ])));
    for c in &cs {
        let mut keys: Vec<String> = c.keys().into_iter().collect();
        keys.sort();
        check(
            &c.value,
            &json!(keys),
            &g(&["spec", "choice_keys"])[&c.value],
        );
    }
}

#[test]
fn content_of_and_opt_true_match_python() {
    for (name, c) in obj(g(&["spec", "content_of"])) {
        check(name, &json!(content_of(&c["in"])), &c["out"]);
    }
    for row in g(&["spec", "opt_true"]).as_array().expect("rows") {
        let key = row[1].as_str().expect("key");
        check(&row.to_string(), &json!(opt_true(&row[0], key)), &row[2]);
    }
}

#[test]
fn unwrap_fields_matches_python() {
    for (name, c) in obj(g(&["spec", "unwrap_fields"])) {
        check(
            name,
            &run(unwrap_fields(&c["in"]), |fs| json!(fs)),
            &c["out"],
        );
    }
}

#[test]
fn account_ids_match_python() {
    for (name, c) in obj(g(&["spec", "account_from_form_fields"])) {
        let raw = c["in"].as_array().expect("list");
        check(name, &json!(account_from_form_fields(raw)), &c["out"]);
    }
    for (name, c) in obj(g(&["spec", "account_from_item"])) {
        check(name, &json!(account_from_item(&c["in"])), &c["out"]);
    }
}

#[test]
fn fields_from_form_field_matches_python() {
    for (name, c) in obj(g(&["spec", "fields_from_form_field"])) {
        let mut w = Vec::new();
        let got = run(fields_from_form_field(&c["in"], &mut w), |fs| {
            fields_json(&fs)
        });
        check(name, &got, &c["out"]);
        check(name, &json!(w), &c["warnings"]);
    }
}

#[test]
fn add_requested_for_matches_python() {
    let requester = || {
        let mut f = Field::new("requester", "Requester", Kind::LookupUser);
        f.position = Some(2.0);
        f.field_type = "default_requester".into();
        f
    };
    let cases: Vec<(&str, Vec<Field>)> = vec![
        ("adds", vec![requester()]),
        ("no_pos", {
            let mut f = requester();
            f.position = None;
            f.visible_base = false;
            vec![f]
        }),
        ("present", {
            let mut r = requester();
            r.label = "R".into();
            let mut f = Field::new("requested_for", "For", Kind::LookupUser);
            f.position = Some(3.0);
            vec![r, f]
        }),
        ("no_requester", {
            let mut f = Field::new("subject", "Subject", Kind::Text);
            f.position = Some(1.0);
            vec![f]
        }),
    ];
    for (name, mut fields) in cases {
        add_requested_for(&mut fields);
        check(
            name,
            &fields_json(&fields),
            &g(&["spec", "add_requested_for"])[name],
        );
    }
}

#[test]
fn field_from_sr_matches_python() {
    for (name, c) in obj(g(&["spec", "field_from_sr"])) {
        let mut w = Vec::new();
        let got = run(
            field_from_sr(&c["in"], Some("parent_x"), &mut w),
            |(f, secs)| {
                json!({
                    "field": field(&f),
                    "sections": secs.iter().map(|s| json!({"fields": s.fields, "choices": s.choices})).collect::<Vec<_>>(),
                })
            },
        );
        check(name, &got, &c["out"]);
        check(name, &json!(w), &c["warnings"]);
    }
}

#[test]
fn fixture_specs_match_python() {
    for (name, s) in specs() {
        check(name, &pg::spec(&s), &g(&["spec", "specs"])[name]);
        let ordered: Vec<&str> = s.ordered().iter().map(|f| f.api_name.as_str()).collect();
        check(name, &json!(ordered), &g(&["spec", "ordered"])[name]);
        let inputs: Vec<&str> = s.inputs().iter().map(|f| f.api_name.as_str()).collect();
        check(name, &json!(inputs), &g(&["spec", "inputs"])[name]);
        for f in s.fields() {
            let got = s
                .section_parent(&f.api_name)
                .map(|(p, chs)| json!([p.api_name, chs]))
                .unwrap_or(Json::Null);
            check(
                &format!("{name}.section_parent({})", f.api_name),
                &got,
                &g(&["spec", "section_parent"])[name][&f.api_name],
            );
        }
    }
}

#[test]
fn find_resolve_name_and_field_match_python() {
    for (name, s) in specs() {
        for (q, want) in obj(&g(&["spec", "find"])[name]) {
            let got = s.find(q).map(|f| json!(f.api_name)).unwrap_or(Json::Null);
            check(&format!("{name}.find({q:?})"), &got, want);
        }
        for (q, want) in obj(&g(&["spec", "resolve_name"])[name]) {
            check(
                &format!("{name}.resolve_name({q:?})"),
                &json!(s.resolve_name(q)),
                want,
            );
        }
        for (q, want) in obj(&g(&["spec", "field"])[name]) {
            let got = run(s.field(q), |f| json!(f.api_name));
            check(&format!("{name}.field({q:?})"), &got, want);
        }
    }
}

#[test]
fn choices_for_matches_python() {
    let s = pg::change_spec();
    for (key, want) in obj(g(&["spec", "choices_for"])) {
        let raw: Json = serde_json::from_str(key).expect("values");
        let values: Values = crate::value::values_from_json(raw.as_object().expect("obj"));
        for (fname, w) in obj(want) {
            let f = s.get(fname).expect("field");
            let got: Vec<&str> = s
                .choices_for(f, &values)
                .expect("choices")
                .iter()
                .map(|c| c.value.as_str())
                .collect();
            check(&format!("{key} {fname}"), &json!(got), w);
        }
    }
}

#[test]
fn choice_and_choice_by_display_id_match_python() {
    let all = specs();
    let spec_of = |n: &str| all.iter().find(|(k, _)| *k == n).expect("spec").1.clone();
    for (key, cases) in obj(g(&["spec", "choice"])) {
        let (sname, fname) = key.split_once('.').expect("spec.field");
        let s = spec_of(sname);
        let f = s.get(fname).expect("field");
        for (raw, want) in obj(cases) {
            let r: Json = serde_json::from_str(raw).expect("raw");
            let got = run(f.choice(&r), |c| {
                c.map(|c| json!(c.value)).unwrap_or(Json::Null)
            });
            check(&format!("{key}.choice({raw})"), &got, want);
        }
    }
    for (key, cases) in obj(g(&["spec", "choice_by_display_id"])) {
        let (sname, fname) = key.split_once('.').expect("spec.field");
        let s = spec_of(sname);
        let f = s.get(fname).expect("field");
        for (raw, want) in obj(cases) {
            let r: Json = serde_json::from_str(raw).expect("raw");
            let got = f
                .choice_by_display_id(&r)
                .map(|c| json!(c.value))
                .unwrap_or(Json::Null);
            check(&format!("{key}.choice_by_display_id({raw})"), &got, want);
        }
    }
}

#[test]
fn render_subject_matches_python() {
    let s = sr_spec();
    let item = s.item.as_ref().expect("item");
    for (key, want) in obj(g(&["spec", "render_subject"])) {
        if key == "template2" {
            continue;
        }
        let raw: Json = serde_json::from_str(key).expect("values");
        let values = crate::value::values_from_json(raw.as_object().expect("obj"));
        check(key, &json!(item.render_subject(&values)), want);
    }
    let tmpl = ServiceItem {
        id: json!(1),
        display_id: json!(2),
        name: "x".into(),
        short_description: String::new(),
        category_name: String::new(),
        description_html: String::new(),
        allow_attachments: true,
        attachment_mandatory: false,
        allow_quantity: false,
        subject_template: "  {{ custom_field.cf_a }} - {{custom_field.b}} {{custom_field.cf_}} "
            .into(),
    };
    let mut values = Values::new();
    values.insert("a".into(), "A".into());
    values.insert("b".into(), "B".into());
    check(
        "template2",
        &json!(tmpl.render_subject(&values)),
        g(&["spec", "render_subject", "template2"]),
    );
}

#[test]
fn from_service_item_matches_python() {
    for (name, c) in obj(g(&["spec", "from_service_item"])) {
        let got = run(FormSpec::from_service_item(&c["in"]), |s| pg::spec(&s));
        check(name, &got, &c["out"]);
    }
}

#[test]
fn link_sections_match_python() {
    let raw = pg::fixture("ticket_fields")["ticket_fields"]
        .as_array()
        .expect("ticket_fields");
    let mut w = Vec::new();
    let mut flds = Vec::new();
    for f in raw {
        flds.extend(fields_from_form_field(f, &mut w).expect("fields"));
    }
    let mut s1 = FormSpec::new("ticket", flds.clone(), None, w.clone(), None);
    link_sections(&mut s1, raw).expect("sections");
    check(
        "link_sections",
        &pg::spec(&s1),
        g(&["spec", "link_sections"]),
    );
    let mut s2 = FormSpec::new("ticket", flds, None, w, None);
    link_ticket_sections(&mut s2, raw).expect("sections");
    check(
        "link_ticket_sections",
        &pg::spec(&s2),
        g(&["spec", "link_ticket_sections"]),
    );
}

/// Python `AttributeError` has no Rust class of its own: it surfaces as a
/// `TypeError` carrying Python's message.
fn rust_class(want: &Json) -> Json {
    match want["err"][0].as_str() {
        Some("AttributeError" | "KeyError") => json!({"err": ["TypeError", want["err"][1]]}),
        _ => want.clone(),
    }
}

/// What differs by design between Python's spec and `FormSpec`:
/// * `position` is a number: bools count as ints, other falsy values as 0;
/// * `lookup_link` is text: a falsy non-string is "no link", others `str()`.
fn typed(spec: &Json) -> Json {
    let mut spec = spec.clone();
    for f in spec["fields"].as_array_mut().unwrap() {
        let pos = match &f["position"] {
            Json::Bool(b) => json!(i64::from(*b)),
            p @ (Json::String(_) | Json::Array(_) | Json::Object(_)) if !fsv_py::truthy(p) => {
                json!(0)
            }
            p => p.clone(),
        };
        f["position"] = pos;
        let link = match &f["lookup_link"] {
            Json::String(_) | Json::Null => f["lookup_link"].clone(),
            l if !fsv_py::truthy(l) => Json::Null,
            l => json!(fsv_py::py_str(l)),
        };
        f["lookup_link"] = link;
    }
    spec
}

#[test]
fn malformed_form_fields_match_python() {
    let cases = g(&["spec", "malformed"]).as_array().unwrap();
    assert!(cases.len() > 150);
    let mut errors = 0;
    for c in cases {
        let label = format!("{} [{}] {}", c["name"], c["module"], c["data"]);
        let got = run(
            FormSpec::from_form_fields(&c["data"], c["module"].as_str().unwrap()),
            |s| crate::py_golden::spec(&s),
        );
        let want = match c["out"].get("ok") {
            // Python builds the spec, then every use of it fails on the
            // position: Rust refuses at construction with that error.
            Some(_) if c["engine"].get("err").is_some() => {
                let e = &c["engine"]["err"];
                assert!(
                    got["err"][0] == "TypeError",
                    "{label}: rust {got}, python engine {e}"
                );
                if e[1].as_str().unwrap().starts_with("unsupported operand") {
                    assert_eq!(got["err"][1], e[1], "{label}");
                }
                errors += 1;
                continue;
            }
            Some(ok) => json!({"ok": typed(ok)}),
            None => rust_class(&c["out"]),
        };
        if want.get("err").is_some() {
            errors += 1;
        }
        check(&label, &got, &want);
    }
    assert!(errors > 60, "{errors} error cases");
}
