//! The parts of `fsv.formmeta` the write flows call, re-exported from
//! [`fsv_api::formmeta`]: [`FormMeta`] (fields + rules of one form, with the
//! engine built from them), the [`ClientResolver`] over the web's
//! autocomplete endpoints, and answers → [`fsv_forms::FormState`].
//!
//! Fetching and caching (`load_meta`) stay with the caller: flows that
//! re-check metadata take a [`MetaSource`].

pub use fsv_api::formmeta::{
    ClientResolver, FormMeta, MODULES, MetaSource, RECORD_KEYS, RULES_UNAVAILABLE_KEY, Resolver,
    apply_answers, existing_from_record, find_field, form_err, is_description, is_requester,
    is_status, name_existing, record_key, rules_warnings,
};

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use serde_json::{Map, Value as Json, json};

    use fsv_forms::tz::TzEnv;
    use fsv_forms::{
        FormSpec, FormState, LookupResolver, LookupValue, StateOptions, Value, Values,
    };

    use super::*;
    use crate::testing::{acme_meta, acme_record, fake_resolver};

    const CUSTOM_LINK: &str = "/lookup_choices?field_name=reference&module=change";

    /// Sets `reference` on a custom (multi) lookup backed by `rows`; returns
    /// the kind, `[id, display, email]` per stored value and the encoded
    /// payload (or the error), and the `[link, q]` calls made.
    fn custom_lookup(rows: Json, answer: Value, multi: bool, source: i64) -> Json {
        let spec = FormSpec::from_form_fields(
            &json!([{"name": "reference", "label": "Reference", "position": 1,
                     "field_type": if multi { "custom_multi_lookup" } else { "custom_lookup" },
                     "lookup_config": {"data_source": source, "link": CUSTOM_LINK}}]),
            "change",
        )
        .unwrap();
        let api = crate::testing::fake_client();
        *api.link_rows.lock().unwrap() = Some(rows.as_array().unwrap().clone());
        let opts = StateOptions {
            resolver: Some(Arc::new(ClientResolver::new(api.clone()))),
            tz: Some("UTC".into()),
            tz_env: Some(TzEnv::empty()),
            ..StateOptions::default()
        };
        let mut s = FormState::new(Arc::new(spec), None, "new", None, opts).unwrap();
        let f = s.spec.find("reference").unwrap().clone();
        let mut out = json!({"kind": f.kind.as_str()});
        match s.set("reference", answer) {
            Ok(_) => {
                let v = s.get("reference").unwrap().unwrap().clone();
                let one = |x: &Value| match x {
                    Value::Lookup(l) => json!([l.id, l.display, l.email]),
                    other => panic!("not a lookup: {other:?}"),
                };
                out["values"] = match &v {
                    Value::List(xs) => xs.iter().map(one).collect(),
                    x => json!([one(x)]),
                };
                out["encoded"] =
                    fsv_forms::payload::encode_value(&s.spec, &f, &v, &Values::new()).unwrap();
            }
            Err(e) => out["error"] = json!(e.to_string()),
        }
        out["calls"] = json!(api.link_calls.lock().unwrap().clone());
        out
    }

    #[test]
    fn custom_lookups_resolve_through_their_link() {
        let call = |q: &str| json!([CUSTOM_LINK, q]);
        let acme = || json!([{"id": 901, "value": "Acme"}]);
        let found = |q: &str| {
            json!({"kind": "lookup", "values": [[901, "Acme", null]], "encoded": 901,
                   "calls": [call(q)]})
        };
        assert_eq!(custom_lookup(acme(), 901.into(), false, 99), found("901"));
        assert_eq!(custom_lookup(acme(), "901".into(), false, 99), found("901"));
        assert_eq!(
            custom_lookup(acme(), "Acme".into(), false, 99),
            found("Acme")
        );
        let untrusted = LookupValue::new(901, "untrusted", None);
        assert_eq!(
            custom_lookup(acme(), untrusted.into(), false, 99),
            found("901")
        );
        let globex = || json!([{"id": 902, "name": "Globex"}]);
        let no_901 = json!({"kind": "lookup", "error": "Reference: no match for '901'",
                            "calls": [call("901")]});
        assert_eq!(custom_lookup(globex(), 901.into(), false, 99), no_901);
        assert_eq!(
            custom_lookup(globex(), LookupValue::id(901).into(), false, 99),
            no_901
        );
        assert_eq!(
            custom_lookup(
                json!([{"id": 901, "name": "Acme"}, {"id": 902, "name": "Acme"}]),
                "Acme".into(),
                false,
                99
            ),
            json!({"kind": "lookup", "error": "Reference: 'Acme' matches 2 records: Acme, Acme",
                   "calls": [call("Acme")]})
        );
        assert_eq!(
            custom_lookup(
                json!([{"id": 901, "name": "Acme", "deleted": true}]),
                "Acme".into(),
                false,
                99
            ),
            json!({"kind": "lookup", "error": "Reference: no match for 'Acme'",
                   "calls": [call("Acme")]})
        );
        assert_eq!(
            custom_lookup(
                json!([{"id": 901, "name": "Acme"}, {"id": 902, "name": "Globex"}]),
                Value::list([901.into(), 902.into()]),
                true,
                99
            ),
            json!({"kind": "lookup", "values": [[901, "Acme", null], [902, "Globex", null]],
                   "encoded": [901, 902], "calls": [call("901"), call("902")]})
        );
        assert_eq!(
            custom_lookup(
                json!([{"id": 901, "name": "Acme", "email": "a@x.io"}]),
                901.into(),
                false,
                5
            ),
            json!({"kind": "lookup_user", "values": [[901, "Acme", "a@x.io"]],
                   "encoded": 901, "calls": [call("901")]})
        );
    }

    #[test]
    fn custom_lookup_rows_skip_missing_ids_and_deleted() {
        let spec = FormSpec::from_form_fields(
            &json!([{"name": "reference", "label": "Reference", "position": 1,
                     "field_type": "custom_lookup",
                     "lookup_config": {"data_source": 5, "link": CUSTOM_LINK}}]),
            "change",
        )
        .unwrap();
        let api = crate::testing::fake_client();
        *api.link_rows.lock().unwrap() = Some(vec![
            json!({"id": 1, "name": "A"}),
            json!({"id": null, "name": "N"}),
            json!({"id": 3, "value": "C", "deleted": true}),
            json!({"id": 4, "email": "d@x.io"}),
        ]);
        let r = ClientResolver::new(api.clone());
        let got: Vec<Json> = r
            .lookup("user", "q", spec.find("reference").unwrap())
            .unwrap()
            .iter()
            .map(|l| json!([l.id, l.display, l.email]))
            .collect();
        assert_eq!(got, [json!([1, "A", null]), json!([4, "4", "d@x.io"])]);
        assert_eq!(
            *api.link_calls.lock().unwrap(),
            [(CUSTOM_LINK.to_string(), "q".to_string())]
        );
        assert!(api.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn record_key_maps_default_fields() {
        assert_eq!(record_key("group"), "group_id");
        assert_eq!(record_key("subject"), "subject");
    }

    #[test]
    fn find_field_by_name_label_and_unknown() {
        let m = acme_meta("change", "new_form");
        let spec = m.spec();
        assert_eq!(
            find_field(&spec, " cloud_only ").unwrap().label,
            "Cloud Only"
        );
        assert_eq!(
            find_field(&spec, "cloud  ONLY").unwrap().api_name,
            "cloud_only"
        );
        assert_eq!(
            find_field(&spec, "Colour").unwrap_err().to_string(),
            "unknown field: 'Colour'"
        );
    }

    #[test]
    fn field_type_predicates() {
        let m = acme_meta("change", "new_form");
        let spec = m.spec();
        assert!(is_status(spec.get("status").unwrap()));
        assert!(is_requester(spec.get("requester").unwrap()));
        assert!(is_description(spec.get("description").unwrap()));
        assert!(!is_status(spec.get("subject").unwrap()));
    }

    #[test]
    fn rules_warnings_only_when_unavailable() {
        assert!(rules_warnings(&json!({"business_rules": []})).is_empty());
        let w = rules_warnings(&json!({"unavailable": "HTTP 403"}));
        assert!(w[0].starts_with("business rules could not be loaded (HTTP 403); only"));
    }

    #[test]
    fn existing_and_named_values_from_record() {
        let m = acme_meta("change", "edit_form");
        let spec = m.spec();
        let rec = acme_record();
        let ex = existing_from_record(&spec, rec.as_object().unwrap());
        assert_eq!(ex["subject"], Value::from("Billing: rotate keys"));
        assert_eq!(ex["group"], Value::Lookup(LookupValue::id(9000009001i64)));
        assert!(!ex.contains_key("agent") && !ex.contains_key("reviewer"));
        assert_eq!(ex["legacy_flag"], Value::Bool(false));
        let r = fake_resolver();
        let named = name_existing(ex, &spec, rec.as_object().unwrap(), Some(r.as_ref()));
        match &named["group"] {
            Value::Lookup(lv) => assert_eq!(lv.display, "Platform Ops"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn apply_answers_reports_every_problem() {
        let m = acme_meta("change", "new_form");
        let mut st = m.state("new", None, None).unwrap();
        let mut failed = HashSet::new();
        let mut answers = Map::new();
        answers.insert("Colour".into(), json!("red"));
        answers.insert("Change Type".into(), json!("Sideways"));
        answers.insert("Subject".into(), json!("x"));
        let errs = apply_answers(&mut st, &answers, Some(&mut failed)).unwrap();
        assert_eq!(errs[0], "unknown field: 'Colour'");
        assert!(
            errs[1].starts_with("Change Type: 'Sideways' is not a valid choice"),
            "{errs:?}"
        );
        assert!(failed.contains("change_type"));
        assert_eq!(st.get("subject").unwrap(), Some(&Value::from("x")));
    }

    #[test]
    fn client_resolver_lookups_and_names() {
        let r = fake_resolver();
        let m = acme_meta("change", "new_form");
        let spec = m.spec();
        let f = spec.get("agent").unwrap();
        let got = r.lookup("agent", "alex", f).unwrap();
        assert_eq!(
            got,
            vec![LookupValue::new(
                9000007101i64,
                "Alex Kim",
                Some("alex.kim@example.com")
            )]
        );
        let got = r.lookup("group", "platform", f).unwrap();
        assert_eq!(
            got,
            vec![LookupValue::new(9000009001i64, "Platform Ops", None)]
        );
        let got = r.lookup("user", "jane", f).unwrap();
        assert_eq!(got.len(), 2);
        let rev = spec.get("reviewer").unwrap();
        assert_eq!(
            r.lookup("lookup", "doerr", rev).unwrap()[0].display,
            "Jane Doerr"
        );
        assert_eq!(
            r.name_for("group", &json!(9000009001i64)).as_deref(),
            Some("Platform Ops")
        );
        assert_eq!(r.name_for("user", &json!(1)), None);
    }

    #[test]
    fn meta_as_of_and_fields() {
        let m = acme_meta("change", "new_form");
        let a = m.as_of();
        assert_eq!(a["source"], json!("server"));
        assert_eq!(a["fingerprint"].as_str().unwrap().len(), 16);
        assert_eq!(m.raw_fields().len(), 22);
        assert_eq!(m.fields_key(), "change_fields");
    }
}
