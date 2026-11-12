//! Read-side loaders behind the site-like list and detail views
//! (`views.py`: `optional_get`, `load_views`, `view_settings`,
//! `load_edit_rules`, `load_properties`; `cli.py`: `_load_views`,
//! `_saved_views`, `_resolve_view_name`).
//!
//! Every optional section degrades to a [`Missing`] or a note; only an
//! expired session (`Error::Session`) is returned as an error.

use std::collections::HashSet;
use std::sync::Arc;

use jiff::tz::TimeZone;
use serde_json::{Map, Value as Json, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr;
use fsv_core::resource::Resource;
use fsv_forms::fetch::BusinessRulesOptions;
use fsv_forms::rules::EngineOptions;
use fsv_forms::{Field, FormSpec, FormState, Kind, RulesEngine, StateOptions, Value, Values};
use fsv_py::{py_str, truthy};

use crate::formmeta::{
    ClientResolver, MetaStore, RULES_UNAVAILABLE_KEY, existing_from_record, load_rules,
    name_existing, workspace_from_fields,
};
use crate::service::Api;
use crate::site::{self, Missing, Prop, SavedView};

/// `MODULE_OF`: resource name -> form module.
pub const MODULE_OF: [(&str, &str); 3] = [
    ("changes", "change"),
    ("tickets", "ticket"),
    ("problems", "problem"),
];

const SKIP_PROPS: [&str; 2] = ["subject", "description"];

/// `MODULE_OF.get(name, name.rstrip("s"))`.
pub fn module_of(resource_name: &str) -> String {
    MODULE_OF
        .iter()
        .find(|(r, _)| *r == resource_name)
        .map_or_else(
            || resource_name.trim_end_matches('s').to_string(),
            |(_, m)| (*m).to_string(),
        )
}

fn blank(v: Option<&Json>) -> bool {
    matches!(v, None | Some(Json::Null)) || v.and_then(Json::as_str) == Some("")
}

/// `optional_get`: `GET /api/_/<path>`. An expired session is the error;
/// any other failure is a [`Missing`] named `what` (else `path`).
pub fn optional_get(
    c: &dyn Api,
    path: &str,
    params: Option<&Map<String, Json>>,
    what: &str,
) -> Result<std::result::Result<Json, Missing>> {
    let label = if what.is_empty() { path } else { what };
    let params = params.filter(|p| !p.is_empty()).cloned();
    match c.int_get(path, params) {
        Ok(v) => Ok(Ok(v)),
        Err(e @ Error::Session(_)) => Err(e),
        Err(Error::Api(e)) => Ok(Err(Missing::new(label, Some(i64::from(e.status))))),
        Err(_) => Ok(Err(Missing::new(label, None))),
    }
}

// ---------------------------------------------------------------------------
// saved views

/// `load_views`: the web's saved views for `res` in server order.
pub fn load_views(c: &dyn Api, res: &Resource) -> Result<(Vec<SavedView>, Option<Missing>)> {
    let Some(path) = res.filters_path.filter(|p| !p.is_empty()) else {
        return Ok((Vec::new(), None));
    };
    Ok(match optional_get(c, path, None, "saved views")? {
        Ok(data) => (site::parse_saved_views(&data), None),
        Err(miss) => (Vec::new(), Some(miss)),
    })
}

/// `view_settings`: the view's own sort and conditions (`<filters>/<id>`):
/// the first object inside the answer, else the answer; `{}` if hidden.
pub fn view_settings(c: &dyn Api, res: &Resource, view: &SavedView) -> Result<Json> {
    let Some(path) = res.filters_path.filter(|p| !p.is_empty()) else {
        return Ok(json!({}));
    };
    Ok(
        match optional_get(c, &format!("{path}/{}", view.id), None, "view settings")? {
            Ok(Json::Object(data)) => data
                .values()
                .find(|v| v.is_object())
                .cloned()
                .unwrap_or(Json::Object(data)),
            _ => json!({}),
        },
    )
}

/// `cli._load_views`: the value under the first key ending in `_filters`
/// (`[]` when there is none). A non-object answer is an error, as iterating
/// it is in Python, except an empty list or a string (no such key).
pub fn load_view_rows(c: &dyn Api, res: &Resource) -> Result<Json> {
    let data = c.int_get(res.filters_path.unwrap_or(""), None)?;
    match &data {
        Json::Object(m) => Ok(m
            .iter()
            .find(|(k, _)| k.ends_with("_filters"))
            .map_or_else(|| json!([]), |(_, v)| v.clone())),
        Json::String(_) => Ok(json!([])),
        Json::Array(items) if items.is_empty() => Ok(json!([])),
        Json::Array(_) => Err(Error::other("'list' object has no attribute 'get'")),
        other => {
            let kind = match other {
                Json::Bool(_) => "bool",
                Json::Number(n) if n.is_f64() => "float",
                Json::Number(_) => "int",
                _ => "NoneType",
            };
            Err(Error::other(format!("'{kind}' object is not iterable")))
        }
    }
}

/// `cli._saved_views`: the saved views, `[]` unless the session failed.
pub fn saved_views(c: &dyn Api, res: &Resource) -> Result<Vec<SavedView>> {
    match load_view_rows(c, res) {
        Ok(rows @ Json::Array(_)) => Ok(site::parse_saved_views(&rows)),
        Ok(_) => Ok(Vec::new()),
        Err(e @ Error::Session(_)) => Err(e),
        Err(_) => Ok(Vec::new()),
    }
}

/// `cli._resolve_view_name`: a view id by id or name (case-insensitive),
/// else `view` unchanged. Errors from the listing propagate.
pub fn resolve_view_name(
    c: &dyn Api,
    res: &Resource,
    view: Option<&str>,
) -> Result<Option<String>> {
    let Some(text) = view.filter(|v| !v.is_empty()) else {
        return Ok(view.map(str::to_string));
    };
    let rows = load_view_rows(c, res)?;
    let items: Vec<&Map<String, Json>> = rows
        .as_array()
        .map(|a| a.iter().filter_map(Json::as_object).collect())
        .unwrap_or_default();
    let wanted = pystr::casefold(pystr::strip(text));
    let or_empty = |v: Option<&Json>| v.filter(|v| truthy(v)).map(py_str).unwrap_or_default();
    for item in &items {
        let id = or_empty(item.get("id"));
        if pystr::casefold(&id) == wanted {
            return Ok(Some(id));
        }
    }
    for item in &items {
        if pystr::casefold(&or_empty(item.get("name"))) == wanted {
            let id = item
                .get("id")
                .filter(|v| truthy(v))
                .map_or_else(|| text.to_string(), py_str);
            return Ok(Some(id));
        }
    }
    Ok(Some(text.to_string()))
}

// ---------------------------------------------------------------------------
// properties

/// `_record_for_form`: a ticket record under the names its form uses
/// (`agent_id` for `responder_id`, `ticket_type` for `type`).
pub fn record_for_form(res: &Resource, record: &Map<String, Json>) -> Map<String, Json> {
    let mut rec = record.clone();
    if res.name == "tickets" {
        if !rec.contains_key("agent_id") {
            let responder = rec.get("responder_id").cloned().unwrap_or(Json::Null);
            rec.insert("agent_id".into(), responder);
        }
        if rec.get("ticket_type").is_none_or(Json::is_null)
            && let Some(t) = rec.get("type").filter(|v| truthy(v)).cloned()
        {
            rec.insert("ticket_type".into(), t);
        }
    }
    rec
}

/// `load_edit_rules`: the edit form's business rules for one record, cached
/// in `store` per record (the server filters them by `module_id`).
///
/// `(None, note)` when the account has none or will not show them;
/// `(None, None)` when it answered with no rules. Only a session failure is
/// an error.
pub fn load_edit_rules(
    c: &dyn Api,
    store: &dyn MetaStore,
    module: &str,
    record: &Json,
    raw_fields: &[Json],
) -> Result<(Option<Json>, Option<String>)> {
    let ws = record.get("workspace_id").cloned().unwrap_or(Json::Null);
    let fws = workspace_from_fields(raw_fields);
    let mid = record
        .get("display_id")
        .filter(|v| truthy(v))
        .or_else(|| record.get("id"))
        .cloned()
        .unwrap_or(Json::Null);
    let configured = store.configured_workspace();
    let place = if truthy(&configured) {
        configured
    } else if !blank(Some(&ws)) {
        ws.clone()
    } else {
        fws.clone()
    };
    let place = if blank(Some(&place)) {
        "none".to_string()
    } else {
        py_str(&place)
    };
    let key = format!("{module}-rules-edit_form-ws{place}-{}", py_str(&mid));
    if let Some(cached @ Json::Object(_)) = store.cached(&key) {
        return Ok((Some(cached), None));
    }
    let query = BusinessRulesOptions {
        form_type: "edit".into(),
        module_id: mid,
        ..Default::default()
    };
    let rules = match load_rules(c, store, module, &[ws, fws], &query) {
        Ok(r) => r,
        Err(e @ Error::Session(_)) => return Err(e),
        Err(Error::Api(e)) => {
            return Ok((
                None,
                Some(format!(
                    "static field visibility (business rules: unavailable ({}))",
                    e.status
                )),
            ));
        }
        Err(_) => {
            return Ok((
                None,
                Some("static field visibility (business rules unreadable)".into()),
            ));
        }
    };
    if rules.get(RULES_UNAVAILABLE_KEY).is_some_and(truthy) {
        let tail = match rules.get("unavailable_status").filter(|s| truthy(s)) {
            Some(status) => format!(" ({}))", py_str(status)),
            None => ")".into(),
        };
        return Ok((
            None,
            Some(format!(
                "static field visibility (business rules: not available on this account{tail}"
            )),
        ));
    }
    if !rules.get("business_rules").is_some_and(truthy) {
        return Ok((None, None));
    }
    store.save(&key, "form_rules", &rules);
    Ok((Some(rules), None))
}

fn empty_value(v: &Value) -> bool {
    match v {
        Value::Json(Json::Object(m)) => m.is_empty(),
        other => other.is_empty(),
    }
}

/// `_prop_value`: the display text of one field in the evaluated form.
pub fn prop_value(state: &FormState, f: &Field, tz: Option<&TimeZone>) -> String {
    let Some(v) = state
        .evaluated
        .values
        .get(&f.api_name)
        .filter(|v| !empty_value(v))
    else {
        return String::new();
    };
    if matches!(f.kind, Kind::Date | Kind::Datetime)
        && let Some(s) = v.as_str()
    {
        if f.kind == Kind::Datetime {
            return site::fmt_when(&json!(s), tz, true);
        }
        let head: String = s.chars().take(10).collect();
        let shown = if site::is_date_only(&json!(head)) {
            head
        } else {
            s.to_string()
        };
        return site::fmt_when(&json!(shown), tz, true);
    }
    let lookup_text = |lv: &fsv_forms::LookupValue| {
        if lv.display.is_empty() {
            py_str(&lv.id)
        } else {
            lv.display.clone()
        }
    };
    if let Value::List(items) = v
        && !items.is_empty()
        && items.iter().all(|x| matches!(x, Value::Lookup(_)))
    {
        let names: Vec<String> = items
            .iter()
            .filter_map(|x| match x {
                Value::Lookup(lv) => Some(lookup_text(lv)),
                _ => None,
            })
            .collect();
        return names.join(", ");
    }
    if let Value::Lookup(lv) = v {
        return lookup_text(lv);
    }
    match state.display(&f.api_name, None) {
        Ok(text) if matches!(f.kind, Kind::Textarea | Kind::Richtext) => {
            site::one_line(&Json::String(text))
        }
        Ok(text) => text,
        Err(_) => v.py_str(),
    }
}

/// `load_properties`: form fields in web order, visible per the edit-form
/// rules, with display values (names, not ids), plus a note for the page.
///
/// Falls back to [`site::flat_properties`] with a note when the form cannot
/// be read or its rules cannot be applied. Only a session failure (while
/// loading the rules) is an error. The result is what
/// `fsv_views::detail::Properties` wraps in `Some`.
pub fn load_properties(
    c: &Arc<dyn Api + Send + Sync>,
    store: &dyn MetaStore,
    res: &Resource,
    record: &Json,
    schema: Option<&Json>,
    tz: Option<&TimeZone>,
    user: Option<&Json>,
) -> Result<(Vec<Prop>, Option<String>)> {
    let raw_fields: Vec<Json> = schema
        .and_then(|s| s.get("fields"))
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    if raw_fields.is_empty() {
        return Ok((Vec::new(), Some("no form fields".into())));
    }
    let module = module_of(res.name);
    let spec =
        match FormSpec::from_form_fields(&json!({ res.form_fields_key: raw_fields }), &module) {
            Ok(s) => Arc::new(s),
            Err(_) => {
                return Ok((
                    site::flat_properties(record, schema),
                    Some("form metadata unreadable; fields shown as stored".into()),
                ));
            }
        };
    let (rules, note) = load_edit_rules(c.as_ref(), store, &module, record, &raw_fields)?;
    let empty = Map::new();
    let rec = record_for_form(res, record.as_object().unwrap_or(&empty));
    let rules = rules.unwrap_or_else(|| json!({"business_rules": []}));
    let engine = || {
        let opts = EngineOptions {
            module: Some(module.clone()),
            workspace_id: record.get("workspace_id").filter(|w| !w.is_null()).cloned(),
            ..EngineOptions::default()
        };
        RulesEngine::new(spec.clone(), Some(&rules), opts)
    };
    let existing = existing_from_record(&spec, &rec);
    let existing = name_existing(existing, &spec, &rec, Some(&ClientResolver::new(c.clone())));
    let tzname = user
        .and_then(|u| u.get("time_zone"))
        .filter(|v| truthy(v))
        .map(py_str);
    let state = |tz: Option<String>,
                 existing: Values|
     -> std::result::Result<FormState, fsv_forms::FormError> {
        let opts = StateOptions {
            tz,
            existing: Some(existing),
            ..StateOptions::default()
        };
        FormState::new(spec.clone(), Some(engine()?), "edit", None, opts)
    };
    let state = match state(tzname, existing.clone()) {
        Err(e) if e.is_value_error() => state(None, existing),
        other => other,
    };
    let Ok(state) = state else {
        return Ok((
            site::flat_properties(record, schema),
            Some("form rules could not be applied; fields shown as stored".into()),
        ));
    };
    let visible: HashSet<&str> = state
        .visible_fields()
        .iter()
        .map(|f| f.api_name.as_str())
        .collect();
    let mut out = Vec::new();
    for f in spec.ordered() {
        let name = f.api_name.as_str();
        if !visible.contains(name)
            || SKIP_PROPS.contains(&name)
            || matches!(f.kind, Kind::Notice | Kind::Attachment)
        {
            continue;
        }
        let value = prop_value(&state, f, tz);
        if f.synthetic && value.is_empty() {
            continue;
        }
        out.push(Prop {
            name: name.to_string(),
            label: f.label.clone(),
            value,
            required: state.evaluated.required.contains(name),
        });
    }
    Ok((out, note))
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex};

    use fsv_core::client::{Body, Response};
    use fsv_core::error::ApiError;
    use fsv_core::resource::{CHANGES, TICKETS};

    use super::*;
    use crate::formmeta::MemStore;

    /// Outputs of `port/golden/oracles/fsv_api_views/oracle.py` (the Python functions over a
    /// recording fake client). `{"$ref": name}` stands for `$fixtures[name]`.
    static ORACLE: LazyLock<Json> = LazyLock::new(|| {
        let raw: Json = serde_json::from_str(include_str!("testdata/views_oracle.json")).unwrap();
        let fixtures = raw["$fixtures"].clone();
        fn resolve(v: &Json, fx: &Json) -> Json {
            match v {
                Json::Object(m) if m.len() == 1 && m.contains_key("$ref") => {
                    fx[m["$ref"].as_str().unwrap()].clone()
                }
                Json::Object(m) => {
                    Json::Object(m.iter().map(|(k, x)| (k.clone(), resolve(x, fx))).collect())
                }
                Json::Array(a) => Json::Array(a.iter().map(|x| resolve(x, fx)).collect()),
                other => other.clone(),
            }
        }
        resolve(&raw, &fixtures)
    });

    /// The oracle's fake: `path -> JSON | {"__status": n} | "__session" | "__other"`;
    /// an unrouted path is a 404.
    struct Fake {
        routes: Map<String, Json>,
        calls: Mutex<Vec<Json>>,
    }

    impl Fake {
        fn new(routes: &Json) -> Fake {
            Fake {
                routes: routes.as_object().cloned().unwrap_or_default(),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn with(path: &str, reply: &Json) -> Fake {
            Fake::new(&json!({ path: reply }))
        }
        fn calls(&self) -> Json {
            Json::Array(self.calls.lock().unwrap().clone())
        }
    }

    fn unexpected<T>() -> Result<T> {
        Err(Error::other("unexpected call"))
    }

    impl Api for Fake {
        fn int_get(&self, path: &str, params: Option<Map<String, Json>>) -> Result<Json> {
            self.calls.lock().unwrap().push(json!([path, params]));
            match self.routes.get(path) {
                None => Err(ApiError::new(404, json!({"description": "nope"})).into()),
                Some(Json::String(s)) if s == "__session" => Err(Error::session("expired")),
                Some(Json::String(s)) if s == "__other" => Err(Error::other("boom")),
                Some(Json::Object(m)) if m.contains_key("__status") => Err(ApiError::new(
                    m["__status"].as_u64().unwrap() as u16,
                    json!({"description": "nope"}),
                )
                .into()),
                Some(v) => Ok(v.clone()),
            }
        }
        fn int_post(&self, _: &str, _: &Json) -> Result<Json> {
            unexpected()
        }
        fn int_put(&self, _: &str, _: &Json) -> Result<Json> {
            unexpected()
        }
        fn int_patch(&self, _: &str, _: &Json) -> Result<Json> {
            unexpected()
        }
        fn int_delete(
            &self,
            _: &str,
            _: Option<&Json>,
            _: Option<Map<String, Json>>,
        ) -> Result<Json> {
            unexpected()
        }
        fn autocomplete(
            &self,
            _: &str,
            _: &str,
            _: Option<Map<String, Json>>,
        ) -> Result<Vec<Json>> {
            unexpected()
        }
        fn lookup_choices(&self, _: &str, _: &str) -> Result<Vec<Json>> {
            unexpected()
        }
        fn fulltext_search(&self, _: &str, _: &str, _: i64, _: Option<&str>) -> Result<Json> {
            unexpected()
        }
        fn get_csrf(&self) -> Result<String> {
            unexpected()
        }
        fn send_once(
            &self,
            _: &str,
            _: &str,
            _: Body,
            _: Option<Map<String, Json>>,
            _: &str,
        ) -> Result<Json> {
            unexpected()
        }
        fn fetch(&self, _: &str) -> Result<Response> {
            unexpected()
        }
        fn domain(&self) -> String {
            "acme.freshservice.com".into()
        }
    }

    /// The oracle's `run`: `{"ok": ...}` or `{"session": msg}`.
    fn run<T>(got: Result<T>, ok: impl FnOnce(T) -> Json) -> Json {
        match got {
            Ok(v) => json!({"ok": ok(v)}),
            Err(Error::Session(msg)) => json!({"session": msg}),
            Err(e) => json!({"error": e.to_string()}),
        }
    }

    fn miss(m: Option<Missing>) -> Json {
        m.map_or(
            Json::Null,
            |m| json!({"what": m.what, "status": m.status, "note": m.note()}),
        )
    }

    fn view_json(v: &SavedView) -> Json {
        let mut m = v.to_json().as_object().cloned().unwrap();
        m.insert("raw".into(), v.raw.clone());
        Json::Object(m)
    }

    fn cases(section: &str) -> Vec<(String, Json)> {
        ORACLE[section]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[test]
    fn load_views_matches_python() {
        for (name, case) in cases("load_views") {
            let c = Fake::with("change-filters", &case["payload"]);
            let got = run(
                load_views(&c, &CHANGES),
                |(views, m)| json!({"views": views.iter().map(view_json).collect::<Vec<_>>(), "missing": miss(m)}),
            );
            assert_eq!(got, case["result"], "{name}");
            assert_eq!(c.calls(), case["calls"], "{name}");
        }
    }

    #[test]
    fn views_of_a_resource_without_filters_are_empty() {
        static NO_FILTERS: Resource = Resource {
            filters_path: None,
            ..CHANGES
        };
        let c = Fake::new(&json!({}));
        assert_eq!(load_views(&c, &NO_FILTERS).unwrap(), (Vec::new(), None));
        let view = site::parse_saved_views(&json!([{"id": "x"}])).remove(0);
        assert_eq!(view_settings(&c, &NO_FILTERS, &view).unwrap(), json!({}));
        assert_eq!(c.calls(), json!([]));
    }

    #[test]
    fn view_settings_matches_python() {
        let view = site::parse_saved_views(&json!([{"id": "my_open", "name": "Mine"}])).remove(0);
        for (name, case) in cases("view_settings") {
            let c = Fake::with("change-filters/my_open", &case["payload"]);
            assert_eq!(
                run(view_settings(&c, &CHANGES, &view), |v| v),
                case["result"],
                "{name}"
            );
            assert_eq!(c.calls(), case["calls"], "{name}");
        }
    }

    #[test]
    fn saved_views_matches_python() {
        for (name, case) in cases("saved_views") {
            let c = Fake::with("change-filters", &case["payload"]);
            let got = run(saved_views(&c, &CHANGES), |views| {
                json!(views.iter().map(view_json).collect::<Vec<_>>())
            });
            assert_eq!(got, case["result"], "{name}");
        }
    }

    #[test]
    fn resolve_view_name_matches_python() {
        let payload = &ORACLE["load_views"]["acme"]["payload"];
        for case in ORACLE["resolve_view_name"].as_array().unwrap() {
            let c = Fake::with("change-filters", payload);
            let text = case["text"].as_str();
            assert_eq!(
                run(resolve_view_name(&c, &CHANGES, text), |v| json!(v)),
                case["result"],
                "{text:?}"
            );
        }
    }

    fn store(pre: &Json, configured: &Json) -> MemStore {
        let entries = pre
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        MemStore {
            entries: Mutex::new(entries),
            workspace: configured.clone(),
        }
    }

    fn store_json(s: &MemStore) -> Json {
        let entries = s.entries.lock().unwrap();
        let mut keys: Vec<&String> = entries.keys().collect();
        keys.sort();
        Json::Object(
            keys.into_iter()
                .map(|k| (k.clone(), entries[k].clone()))
                .collect(),
        )
    }

    #[test]
    fn load_edit_rules_matches_python() {
        for case in ORACLE["edit_rules"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let c = Fake::new(&case["routes"]);
            let s = store(&case["pre"], &case["configured"]);
            let fields = case["fields"].as_array().unwrap();
            let got = run(
                load_edit_rules(
                    &c,
                    &s,
                    case["module"].as_str().unwrap(),
                    &case["record"],
                    fields,
                ),
                |(r, n)| json!([r, n]),
            );
            assert_eq!(got, case["result"], "{name}");
            assert_eq!(store_json(&s), case["store"], "{name}");
            assert_eq!(c.calls(), case["calls"], "{name}");
        }
    }

    #[test]
    fn load_properties_matches_python() {
        for case in ORACLE["properties"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let fake = Arc::new(Fake::new(&case["routes"]));
            let c: Arc<dyn Api + Send + Sync> = fake.clone();
            let s = MemStore::default();
            let res = if case["res"] == "tickets" {
                &TICKETS
            } else {
                &CHANGES
            };
            let tz = case["tz"].as_str().map(|z| TimeZone::get(z).unwrap());
            let user = Some(&case["user"]).filter(|u| !u.is_null());
            let schema = Some(&case["schema"]).filter(|u| !u.is_null());
            let got = load_properties(&c, &s, res, &case["record"], schema, tz.as_ref(), user);
            let got = run(
                got,
                |(props, note)| json!({"props": props.iter().map(Prop::to_json).collect::<Vec<_>>(), "note": note}),
            );
            assert_eq!(got, case["result"], "{name}");
            assert_eq!(fake.calls(), case["calls"], "{name}");
            let keys: Vec<String> = store_json(&s)
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            assert_eq!(json!(keys), case["store_keys"], "{name}");
        }
    }

    /// Python raises `AttributeError` on a non-object field; the port keeps
    /// the page (`FormSpec::from_form_fields` skips it).
    #[test]
    fn non_object_field_does_not_fail_the_page() {
        let c: Arc<dyn Api + Send + Sync> = Arc::new(Fake::new(&json!({})));
        let schema = json!({"fields": ["x"]});
        let got = load_properties(
            &c,
            &MemStore::default(),
            &CHANGES,
            &json!({"id": 1}),
            Some(&schema),
            None,
            None,
        )
        .unwrap();
        assert!(got.0.is_empty());
    }

    #[test]
    fn module_of_follows_python() {
        assert_eq!(module_of("changes"), "change");
        assert_eq!(module_of("releases"), "release");
        assert_eq!(module_of("bass"), "ba");
    }

    #[test]
    fn record_for_form_maps_ticket_names() {
        let rec = json!({"responder_id": 5, "type": "Incident", "ticket_type": null})
            .as_object()
            .unwrap()
            .clone();
        let got = record_for_form(&TICKETS, &rec);
        assert_eq!(got["agent_id"], json!(5));
        assert_eq!(got["ticket_type"], json!("Incident"));
        assert_eq!(record_for_form(&CHANGES, &rec), rec);
    }

    #[test]
    fn optional_get_names_the_section() {
        let c = Fake::new(&json!({"a": {"__status": 403}, "b": "__other", "c": [1]}));
        assert_eq!(
            optional_get(&c, "a", None, "").unwrap().unwrap_err().note(),
            "a: not permitted for your role (403)"
        );
        assert_eq!(
            optional_get(&c, "b", None, "thing")
                .unwrap()
                .unwrap_err()
                .note(),
            "thing: unavailable"
        );
        let empty = Map::new();
        assert_eq!(
            optional_get(&c, "c", Some(&empty), "").unwrap().unwrap(),
            json!([1])
        );
        assert_eq!(c.calls(), json!([["a", null], ["b", null], ["c", null]]));
    }
}
