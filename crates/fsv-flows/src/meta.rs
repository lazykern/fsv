//! The parts of `fsv.formmeta` the write flows call: [`FormMeta`] (fields +
//! rules of one form, with the engine built from them), the
//! [`ClientResolver`] over the web's autocomplete endpoints, and answers →
//! [`FormState`].
//!
//! Fetching and caching (`load_meta`) stay with the caller: flows that
//! re-check metadata take a [`MetaSource`].

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value as Json, json};

use fsv_api::service::Api;
use fsv_core::error::{Error, Result};
use fsv_core::safety::norm;
use fsv_forms::rules::EngineOptions;
use fsv_forms::state::ResolverError;
use fsv_forms::tz::TzEnv;
use fsv_forms::{
    Field, FormError, FormSpec, FormState, Kind, LookupResolver, LookupValue, MetadataFingerprint,
    RulesEngine, StateOptions, Value, Values, metadata_fingerprint,
};
use fsv_py::truthy;

/// `MODULES`: module -> (metadata path, fields key).
pub const MODULES: [(&str, &str, &str); 2] = [
    ("change", "change_form_fields", "change_fields"),
    ("ticket", "ticket_form_fields", "ticket_fields"),
];

/// Default fields whose record key differs from the form field name.
pub const RECORD_KEYS: [(&str, &str); 6] = [
    ("department", "department_id"),
    ("group", "group_id"),
    ("agent", "agent_id"),
    ("change_window", "change_window_id"),
    ("requester", "requester_id"),
    ("requested_for", "requested_for_id"),
];

pub const RULES_UNAVAILABLE_KEY: &str = "unavailable";

/// `RECORD_KEYS.get(name, name)`.
pub fn record_key(name: &str) -> &str {
    RECORD_KEYS
        .iter()
        .find(|(k, _)| *k == name)
        .map_or(name, |(_, v)| v)
}

/// A FormError the CLI's form handlers catch (exit 2) or a plain error.
pub(crate) fn form_err(e: FormError) -> Error {
    if e.is_form_error() || e.is_value_error() {
        Error::Value(e.to_string())
    } else {
        Error::Other(e.to_string())
    }
}

/// Field list + business rules of one form, as fetched.
#[derive(Clone)]
pub struct FormMeta {
    pub module: String,
    /// `new_form` | `edit_form`.
    pub form_type: String,
    pub form_fields: Json,
    pub rules: Json,
    pub workspace_id: Json,
    pub fetched_at: f64,
    /// `"server"` | `"cache"`.
    pub source: String,
    /// Record display id (edit form).
    pub module_id: Json,
    /// Time-zone environment for date-times typed without an offset
    /// (`None`: the process).
    pub tz_env: Option<TzEnv>,
    spec: Arc<FormSpec>,
}

impl FormMeta {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        module: &str,
        form_type: &str,
        form_fields: Json,
        rules: Json,
        workspace_id: Json,
        fetched_at: f64,
        source: &str,
        module_id: Json,
    ) -> Result<FormMeta> {
        let spec = FormSpec::from_form_fields(&form_fields, module).map_err(form_err)?;
        Ok(FormMeta {
            module: module.to_string(),
            form_type: form_type.to_string(),
            form_fields,
            rules,
            workspace_id,
            fetched_at,
            source: source.to_string(),
            module_id,
            tz_env: None,
            spec: Arc::new(spec),
        })
    }

    pub fn with_tz_env(mut self, env: TzEnv) -> FormMeta {
        self.tz_env = Some(env);
        self
    }

    pub fn fields_key(&self) -> &'static str {
        MODULES
            .iter()
            .find(|(m, _, _)| *m == self.module)
            .map_or("change_fields", |(_, _, k)| k)
    }

    /// `form_fields[fields_key] or []`.
    pub fn raw_fields(&self) -> Vec<Json> {
        match self.form_fields.get(self.fields_key()) {
            Some(Json::Array(a)) => a.clone(),
            _ => Vec::new(),
        }
    }

    pub fn user(&self) -> Option<&Json> {
        self.rules.get("user")
    }

    pub fn fingerprint(&self) -> MetadataFingerprint {
        metadata_fingerprint(Some(&self.form_fields), Some(&self.rules), None)
    }

    /// "form metadata as of ..." for reviews.
    pub fn as_of(&self) -> Map<String, Json> {
        let fp = self.fingerprint();
        let fetched = jiff::Timestamp::from_second(self.fetched_at.trunc() as i64)
            .map(|t| {
                t.to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string()
            })
            .unwrap_or_default();
        let mut out = Map::new();
        out.insert("updated_at".into(), json!(fp.updated_at));
        out.insert("fetched_at".into(), json!(fetched));
        out.insert("source".into(), json!(self.source));
        out.insert(
            "fingerprint".into(),
            json!(fp.hash.chars().take(16).collect::<String>()),
        );
        out
    }

    pub fn spec(&self) -> Arc<FormSpec> {
        self.spec.clone()
    }

    pub fn engine(&self) -> Result<RulesEngine> {
        let opts = EngineOptions {
            module: Some(self.module.clone()),
            workspace_id: (!self.workspace_id.is_null()).then(|| self.workspace_id.clone()),
            ..EngineOptions::default()
        };
        RulesEngine::new(self.spec.clone(), Some(&self.rules), opts).map_err(form_err)
    }

    /// `FormState(spec, meta.engine(), mode=..., resolver=..., existing=...)`.
    pub fn state(
        &self,
        mode: &str,
        resolver: Option<Arc<dyn Resolver>>,
        existing: Option<Values>,
    ) -> Result<FormState> {
        let resolver: Option<Arc<dyn LookupResolver>> =
            resolver.map(|r| r as Arc<dyn LookupResolver>);
        let opts = StateOptions {
            resolver,
            existing,
            tz_env: self.tz_env.clone(),
            ..StateOptions::default()
        };
        FormState::new(self.spec.clone(), Some(self.engine()?), mode, None, opts).map_err(form_err)
    }
}

/// `load_meta(client, module, mode, fresh=..., module_id=...)`.
pub trait MetaSource {
    fn load_meta(
        &self,
        module: &str,
        mode: &str,
        fresh: bool,
        module_id: &Json,
    ) -> Result<FormMeta>;
}

/// A lookup resolver that can also name ids it has seen (`name_for`).
pub trait Resolver: LookupResolver + Send + Sync {
    fn name_for(&self, _entity: &str, _id: &Json) -> Option<String> {
        None
    }
}

/// `LookupResolver` over the web's autocomplete endpoints.
pub struct ClientResolver {
    pub client: Arc<dyn Api + Send + Sync>,
    groups: Mutex<Option<Vec<Json>>>,
    boot: Mutex<Option<Json>>,
}

fn s(v: Option<&Json>) -> String {
    fsv_py::py_str(v.unwrap_or(&Json::Null))
}

/// `a or b or c` over JSON values.
fn first_truthy<'a>(vals: &[Option<&'a Json>]) -> Option<&'a Json> {
    vals.iter().flatten().copied().find(|v| truthy(v))
}

fn email_of(r: &Json) -> Option<String> {
    ["email", "details", "primary_email"]
        .iter()
        .filter_map(|k| r.get(*k).and_then(Json::as_str))
        .find(|v| v.contains('@'))
        .map(str::to_string)
}

fn person(r: &Json, id: &Json) -> LookupValue {
    let shown = first_truthy(&[r.get("value"), r.get("name")])
        .map_or_else(|| fsv_py::py_str(id), fsv_py::py_str);
    LookupValue::new(id.clone(), shown, email_of(r).as_deref())
}

impl ClientResolver {
    pub fn new(client: Arc<dyn Api + Send + Sync>) -> ClientResolver {
        ClientResolver {
            client,
            groups: Mutex::new(None),
            boot: Mutex::new(None),
        }
    }

    fn bootstrap(&self) -> std::result::Result<Json, Error> {
        let mut g = self.boot.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            let got = self.client.int_get("bootstrap/agents_groups", None)?;
            *g = Some(if truthy(&got) { got } else { json!({}) });
        }
        Ok(g.clone().unwrap_or_else(|| json!({})))
    }

    fn rows(doc: &Json, key: &str) -> Vec<Json> {
        match doc.get(key) {
            Some(Json::Array(a)) => a.clone(),
            _ => Vec::new(),
        }
    }
}

impl LookupResolver for ClientResolver {
    fn lookup(
        &self,
        entity: &str,
        query: &str,
        field: &Field,
    ) -> std::result::Result<Vec<LookupValue>, ResolverError> {
        let c = &self.client;
        let boxed = |e: Error| -> ResolverError { Box::new(e) };
        if entity == "user" {
            let mut params = Map::new();
            params.insert("all_users".into(), json!("true"));
            let rows = c
                .autocomplete("requesters", query, Some(params))
                .map_err(boxed)?;
            return Ok(rows
                .iter()
                .filter(|r| {
                    r.get("id").is_some_and(|v| !v.is_null())
                        && !r.get("deleted").is_some_and(truthy)
                })
                .map(|r| person(r, &r["id"]))
                .collect());
        }
        if entity == "agent" {
            let rows = c.autocomplete("agents", query, None).map_err(boxed)?;
            return Ok(rows
                .iter()
                .filter(|r| r.get("user_id").is_some_and(|v| !v.is_null()))
                .filter(|r| {
                    let v = first_truthy(&[r.get("value")])
                        .map(fsv_py::py_str)
                        .unwrap_or_default();
                    !v.contains("(Deactivated)")
                })
                .map(|r| person(r, &r["user_id"]))
                .collect());
        }
        if entity == "group" {
            let mut groups = self.groups.lock().unwrap_or_else(|p| p.into_inner());
            if groups.is_none() {
                *groups = Some(Self::rows(&self.bootstrap().map_err(boxed)?, "groups"));
            }
            let q = norm(query);
            return Ok(groups
                .iter()
                .flatten()
                .filter(|g| g.get("id").is_some_and(|v| !v.is_null()))
                .filter(|g| {
                    norm(
                        &first_truthy(&[g.get("name")])
                            .map(fsv_py::py_str)
                            .unwrap_or_default(),
                    )
                    .contains(&q)
                })
                .map(|g| {
                    let shown = first_truthy(&[g.get("name")]).unwrap_or(&g["id"]);
                    LookupValue::new(g["id"].clone(), fsv_py::py_str(shown), None)
                })
                .collect());
        }
        if let Some(link) = field.lookup_link.as_deref().filter(|l| !l.is_empty()) {
            let rows = c.lookup_choices(link, query).map_err(boxed)?;
            return Ok(rows
                .iter()
                .filter(|r| r.get("id").is_some_and(|v| !v.is_null()))
                .map(|r| {
                    let shown = first_truthy(&[r.get("name"), r.get("value")]).unwrap_or(&r["id"]);
                    LookupValue::new(
                        r["id"].clone(),
                        fsv_py::py_str(shown),
                        email_of(r).as_deref(),
                    )
                })
                .collect());
        }
        Ok(Vec::new())
    }
}

impl Resolver for ClientResolver {
    /// Display name of a group/agent by id from the once-fetched bootstrap
    /// lists; `None` when unknown (e.g. requesters).
    fn name_for(&self, entity: &str, id: &Json) -> Option<String> {
        let key = match entity {
            "group" => "groups",
            "agent" => "agents",
            _ => return None,
        };
        let boot = self.bootstrap().ok()?;
        let want = fsv_py::py_str(id);
        for row in Self::rows(&boot, key) {
            if !row.is_object() {
                continue;
            }
            if (s(row.get("id")) == want || s(row.get("user_id")) == want)
                && let Some(shown) = first_truthy(&[row.get("name"), row.get("value")])
            {
                return Some(fsv_py::py_str(shown));
            }
        }
        None
    }
}

/// Field by internal name or label; ambiguous labels are an error.
pub fn find_field<'a>(spec: &'a FormSpec, key: &str) -> std::result::Result<&'a Field, FormError> {
    let k = fsv_core::pystr::strip(key);
    if let Some(f) = spec.get(k) {
        return Ok(f);
    }
    let hits: Vec<&Field> = spec
        .fields()
        .iter()
        .filter(|f| f.on_form && norm(&f.label) == norm(k))
        .collect();
    if hits.len() > 1 {
        let names: Vec<&str> = hits.iter().map(|f| f.api_name.as_str()).collect();
        return Err(FormError::Other(format!(
            "label {} is ambiguous (fields: {}); use the internal name",
            fsv_py::py_repr_str(k),
            names.join(", ")
        )));
    }
    spec.field(k)
}

pub fn is_status(f: &Field) -> bool {
    f.field_type == "default_status"
}

pub fn is_requester(f: &Field) -> bool {
    f.field_type == "default_requester"
}

pub fn is_description(f: &Field) -> bool {
    f.field_type == "default_description"
}

/// Review warning when the tenant gave no business rules.
pub fn rules_warnings(rules: &Json) -> Vec<String> {
    match rules.get(RULES_UNAVAILABLE_KEY).filter(|v| truthy(v)) {
        None => Vec::new(),
        Some(why) => vec![format!(
            "business rules could not be loaded ({}); only the form's static required/visible \
             settings were checked, the server may still ask for more. If the account has workspaces, \
             `fsv config set forms.workspace_id N` picks the right one",
            fsv_py::py_str(why)
        )],
    }
}

fn empty_json(v: &Json) -> bool {
    match v {
        Json::Null => true,
        Json::String(s) => s.is_empty(),
        Json::Array(a) => a.is_empty(),
        Json::Object(m) => m.is_empty(),
        _ => false,
    }
}

/// Map a change record (`GET /api/_/changes/<id>`) to form values.
pub fn existing_from_record(spec: &FormSpec, record: &Map<String, Json>) -> Values {
    let custom = record.get("custom_fields").filter(|v| truthy(v));
    let mut out = Values::new();
    for f in spec.fields() {
        let name = &f.api_name;
        let mut v: Json = if f.is_custom {
            custom
                .and_then(Json::as_object)
                .and_then(|c| c.get(name))
                .cloned()
                .unwrap_or(Json::Null)
        } else {
            let v = record
                .get(record_key(name))
                .or_else(|| {
                    if record.contains_key(record_key(name)) {
                        None
                    } else {
                        record.get(name.as_str())
                    }
                })
                .cloned()
                .unwrap_or(Json::Null);
            match v {
                Json::Object(ref m) if m.contains_key("id") => m["id"].clone(),
                other => other,
            }
        };
        let value = if let Json::Object(m) = &v
            && m.contains_key("id")
        {
            let shown = first_truthy(&[m.get("value"), m.get("name")]).unwrap_or(&m["id"]);
            Value::Lookup(LookupValue::new(
                m["id"].clone(),
                fsv_py::py_str(shown),
                None,
            ))
        } else if f.kind.is_lookup() && !empty_json(&v) {
            match std::mem::take(&mut v) {
                Json::Array(items) => Value::List(
                    items
                        .into_iter()
                        .map(|x| Value::Lookup(LookupValue::id(x)))
                        .collect(),
                ),
                other => Value::Lookup(LookupValue::id(other)),
            }
        } else {
            if empty_json(&v) {
                continue;
            }
            Value::from_json(&v)
        };
        out.insert(name.clone(), value);
    }
    out
}

fn record_objects(name: &str) -> &'static [&'static str] {
    match name {
        "requester" => &["requester"],
        "agent" => &["agent", "responder", "owner"],
        "group" => &["group"],
        _ => &[],
    }
}

fn record_names(name: &str) -> &'static [&'static str] {
    match name {
        "agent" => &["owner_name", "responder_name", "agent_name"],
        "group" => &["group_name"],
        "requester" => &["requester_name"],
        _ => &[],
    }
}

/// (display, email) the record itself carries for a lookup field, if any.
fn hint_name(
    name: &str,
    vid: &Json,
    record: &Map<String, Json>,
) -> (Option<String>, Option<String>) {
    for key in record_objects(name) {
        let Some(obj @ Json::Object(m)) = record.get(*key) else {
            continue;
        };
        let id = m.get("id").unwrap_or(&Json::Null);
        if (id.is_null()
            || id == vid
            || fsv_py::py_str(id) == fsv_py::py_str(vid)
            || m.get("user_id") == Some(vid))
            && let Some(shown) = first_truthy(&[m.get("name"), m.get("value"), m.get("full_name")])
        {
            return (Some(fsv_py::py_str(shown)), email_of(obj));
        }
    }
    for key in record_names(name) {
        if let Some(v) = record.get(*key).filter(|v| truthy(v)) {
            return (Some(fsv_py::py_str(v)), None);
        }
    }
    (None, None)
}

/// Give id-only lookup values (requester, agent, group, ...) a display
/// name: from the record's own nested objects, else the resolver's cached
/// lookups. Ids nobody can name are left as they are.
pub fn name_existing(
    mut existing: Values,
    spec: &FormSpec,
    record: &Map<String, Json>,
    resolver: Option<&dyn Resolver>,
) -> Values {
    let fix = |f: &Field, v: &Value| -> Value {
        let Value::Lookup(lv) = v else {
            return v.clone();
        };
        if !lv.display.is_empty() && lv.display != fsv_py::py_str(&lv.id) {
            return v.clone();
        }
        let (mut display, email) = hint_name(&f.api_name, &lv.id, record);
        if display.is_none()
            && let Some(r) = resolver
        {
            let entity = match f.lookup_entity.as_deref().filter(|e| !e.is_empty()) {
                Some(e) => e,
                None => match f.kind {
                    Kind::LookupGroup => "group",
                    Kind::LookupAgent => "agent",
                    Kind::LookupUser => "user",
                    _ => "",
                },
            };
            display = r.name_for(entity, &lv.id);
        }
        match display.filter(|d| !d.is_empty()) {
            Some(d) => Value::Lookup(LookupValue::new(
                lv.id.clone(),
                d,
                email.or(lv.email.clone()).as_deref(),
            )),
            None => v.clone(),
        }
    };
    let names: Vec<String> = existing.keys().cloned().collect();
    for name in names {
        let Some(f) = spec.get(&name) else { continue };
        let Some(v) = existing.get(&name) else {
            continue;
        };
        let fixed = match v {
            Value::List(items) => Value::List(items.iter().map(|x| fix(f, x)).collect()),
            other => fix(f, other),
        };
        existing.insert(name, fixed);
    }
    existing
}

/// Set answers (label or internal name -> raw) in web field order.
/// Returns every form problem instead of stopping at the first; `failed`
/// collects the api names of fields whose answer was rejected. Errors that
/// are not form errors (a failing lookup) are returned as `Err`.
pub fn apply_answers(
    state: &mut FormState,
    answers: &Map<String, Json>,
    mut failed: Option<&mut HashSet<String>>,
) -> Result<Vec<String>> {
    let spec = state.spec.clone();
    let mut errors = Vec::new();
    let mut resolved: Map<String, Json> = Map::new();
    for (key, raw) in answers {
        match find_field(&spec, key) {
            Ok(f) => {
                resolved.insert(f.api_name.clone(), raw.clone());
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    for name in state.rules.position_order(resolved.keys()) {
        match state.set(&name, Value::from_json(&resolved[&name])) {
            Ok(_) => {}
            Err(e) if e.is_form_error() => {
                errors.push(e.to_string());
                if let Some(f) = failed.as_deref_mut() {
                    f.insert(name);
                }
            }
            Err(e) => return Err(form_err(e)),
        }
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{acme_meta, acme_record, fake_resolver};

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
