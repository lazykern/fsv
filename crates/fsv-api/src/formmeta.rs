//! Form metadata for the CLI faces (`formmeta.py`): fetch, cache and build
//! engine objects for a module form, discover the business-rules workspace,
//! and map records/answers onto a [`FormState`].
//!
//! The cache and the `forms.workspace_id` override come from a
//! [`MetaStore`] ([`DiskStore`] for the real profile).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value as Json, json};

use fsv_core::error::{ApiError, Error, Result};
use fsv_core::safety::norm;
use fsv_forms::fetch::{BusinessRulesOptions, business_rules_query, parse_business_rules};
use fsv_forms::rules::EngineOptions;
use fsv_forms::state::ResolverError;
use fsv_forms::{
    Field, FormError, FormSpec, FormState, Kind, LookupResolver, LookupValue, MetadataFingerprint,
    RulesEngine, StateOptions, Value, Values, metadata_fingerprint,
};
use fsv_py::{py_str, truthy};

use crate::service::Api;

/// `MODULES`: module -> (metadata path, fields key).
pub const MODULES: [(&str, &str, &str); 2] = [
    ("change", "change_form_fields", "change_fields"),
    ("ticket", "ticket_form_fields", "ticket_fields"),
];
pub const FORM_TYPES: [(&str, &str); 2] = [("new", "new_form"), ("edit", "edit_form")];
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
pub const RULES_WORKSPACE_KEY: &str = "workspace";
pub const ACCOUNT_WORKSPACE_PATHS: [&str; 2] = ["bootstrap/me", "workspaces"];
const CURRENT_KEYS: [&str; 4] = [
    "current_workspace_id",
    "current_workspace",
    "selected_workspace_id",
    "active_workspace_id",
];
const PRIMARY_KEYS: [&str; 3] = [
    "primary_workspace_id",
    "default_workspace_id",
    "workspace_id",
];
const PRIMARY_FLAGS: [&str; 6] = [
    "primary",
    "is_primary",
    "default",
    "is_default",
    "current",
    "selected",
];

fn record_key(name: &str) -> &str {
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

/// `module_name`: `changes`/`Tickets` -> `change`/`ticket`.
pub fn module_name(text: &str) -> Result<String> {
    let t = fsv_core::pystr::strip(text).to_lowercase();
    let t = t.trim_end_matches('s');
    if !MODULES.iter().any(|(m, _, _)| *m == t) {
        return Err(Error::value(format!(
            "unsupported form {}; supported: changes, tickets",
            fsv_py::py_repr_str(text)
        )));
    }
    Ok(t.to_string())
}

// ---------------------------------------------------------------------------
// cache + config

/// Where form metadata is cached and the `forms.workspace_id` override.
pub trait MetaStore {
    /// `_cached(name)`: fresh, non-empty data or `None`.
    fn cached(&self, name: &str) -> Option<Json>;
    /// `_save(name, kind, data)`: never an empty answer; write errors ignored.
    fn save(&self, name: &str, kind: &str, data: &Json);
    /// `configured_workspace()`.
    fn configured_workspace(&self) -> Json;
}

/// `_cache_path`: `<cache>/forms/<domain key>--<name>.json`.
pub fn cache_path(cache_dir: &Path, domain_key: &str, name: &str) -> PathBuf {
    cache_dir
        .join("forms")
        .join(format!("{domain_key}--{name}.json"))
}

/// `_non_empty`: a mapping needs one non-empty list value.
pub fn non_empty(data: &Json) -> bool {
    match data {
        Json::Object(m) => m
            .values()
            .any(|v| v.as_array().is_some_and(|a| !a.is_empty())),
        other => truthy(other),
    }
}

/// `configured_workspace` over a loaded config: digit strings become ints.
pub fn configured_workspace_in(config: &Map<String, Json>) -> Json {
    let v = config
        .get("forms")
        .and_then(Json::as_object)
        .and_then(|f| f.get("workspace_id"))
        .cloned()
        .unwrap_or(Json::Null);
    match &v {
        Json::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse::<i64>().map(Json::from).unwrap_or(v)
        }
        _ => v,
    }
}

/// The cache directory and config of the current profile.
pub struct DiskStore {
    pub cache_dir: PathBuf,
    pub domain_key: String,
    pub config: Map<String, Json>,
}

impl DiskStore {
    pub fn current() -> DiskStore {
        DiskStore {
            cache_dir: fsv_core::config::cache_dir(),
            domain_key: fsv_core::config::cache_domain_key(None),
            config: fsv_core::config::load_config(),
        }
    }
}

impl MetaStore for DiskStore {
    fn cached(&self, name: &str) -> Option<Json> {
        let (doc, stale) =
            fsv_core::cache::load(&cache_path(&self.cache_dir, &self.domain_key, name));
        let doc = doc.filter(|_| !stale)?;
        doc.get("data").filter(|d| non_empty(d)).cloned()
    }
    fn save(&self, name: &str, kind: &str, data: &Json) {
        if non_empty(data) {
            let _ = fsv_core::cache::save(
                &cache_path(&self.cache_dir, &self.domain_key, name),
                kind,
                data,
                "",
                0,
            );
        }
    }
    fn configured_workspace(&self) -> Json {
        configured_workspace_in(&self.config)
    }
}

/// An in-memory store (tests, `--no-cache` callers).
#[derive(Default)]
pub struct MemStore {
    pub entries: Mutex<HashMap<String, Json>>,
    pub workspace: Json,
}

impl MetaStore for MemStore {
    fn cached(&self, name: &str) -> Option<Json> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(name)
            .filter(|d| non_empty(d))
            .cloned()
    }
    fn save(&self, name: &str, _kind: &str, data: &Json) {
        if non_empty(data) {
            self.entries
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(name.to_string(), data.clone());
        }
    }
    fn configured_workspace(&self) -> Json {
        self.workspace.clone()
    }
}

// ---------------------------------------------------------------------------
// rules + workspaces

/// `normalize_rules`: `{"business_rules": [...], "user": {...}}` from either shape.
pub fn normalize_rules(raw: &Json) -> Result<Json> {
    let bad = |msg: String| {
        Error::from(ApiError::with_request(
            200,
            Json::String(msg),
            "GET",
            "business_rules",
        ))
    };
    let parsed = match parse_business_rules(raw) {
        Ok(p) => p,
        Err(FormError::Value(msg)) => return Err(bad(msg)),
        Err(e) => return Err(form_err(e)),
    };
    if !parsed.evaluable {
        return Err(bad(
            "the server returned the business-rule listing without conditions/actions; \
                        fsv cannot evaluate the form from it (use the web)"
                .into(),
        ));
    }
    Ok(json!({"business_rules": parsed.rules, "user": parsed.user}))
}

/// `_ws_id`: an int or a digit string, never a bool.
pub fn ws_id(v: &Json) -> Option<i64> {
    match v {
        Json::Number(n) => n.as_i64(),
        Json::String(s) => {
            let t = fsv_core::pystr::strip(s);
            (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
                .then(|| t.parse().ok())
                .flatten()
        }
        _ => None,
    }
}

fn add(out: &mut Vec<i64>, v: Option<&Json>) {
    if let Some(w) = v.and_then(ws_id)
        && !out.contains(&w)
    {
        out.push(w);
    }
}

/// `workspaces_in`: workspace ids an account document names, current/primary first.
pub fn workspaces_in(doc: &Json) -> Vec<i64> {
    fn walk(
        o: &Json,
        depth: usize,
        current: &mut Vec<i64>,
        primary: &mut Vec<i64>,
        rest: &mut Vec<i64>,
    ) {
        let Some(m) = o.as_object().filter(|_| depth <= 3) else {
            return;
        };
        for k in CURRENT_KEYS {
            let v = m.get(k);
            add(
                current,
                if let Some(Json::Object(inner)) = v {
                    inner.get("id")
                } else {
                    v
                },
            );
        }
        for k in PRIMARY_KEYS {
            add(primary, m.get(k));
        }
        for k in ["workspaces", "workspace_list"] {
            if let Some(Json::Array(rows)) = m.get(k) {
                for r in rows {
                    if let Json::Object(row) = r {
                        let flagged = PRIMARY_FLAGS
                            .iter()
                            .any(|f| row.get(*f) == Some(&Json::Bool(true)));
                        add(
                            if flagged { &mut *primary } else { &mut *rest },
                            row.get("id"),
                        );
                    } else {
                        add(rest, Some(r));
                    }
                }
            }
        }
        if let Some(Json::Array(ids)) = m.get("workspace_ids") {
            for v in ids {
                add(rest, Some(v));
            }
        }
        for v in m.values() {
            if v.is_object() {
                walk(v, depth + 1, current, primary, rest);
            }
        }
    }
    let (mut current, mut primary, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    walk(doc, 0, &mut current, &mut primary, &mut rest);
    let mut out = Vec::new();
    for w in current.into_iter().chain(primary).chain(rest) {
        add(&mut out, Some(&json!(w)));
    }
    out
}

/// `account_workspaces`: the session's workspace cookie, `bootstrap/me`,
/// then `workspaces`. Optional endpoints; a session error still raises.
pub fn account_workspaces(c: &dyn Api) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for v in c.workspace_cookies() {
        add(&mut out, Some(&Json::String(v)));
    }
    for path in ACCOUNT_WORKSPACE_PATHS {
        let doc = match c.int_get(path, None) {
            Ok(d) => d,
            Err(e @ Error::Session(_)) => return Err(e),
            Err(_) => continue,
        };
        for w in workspaces_in(&doc) {
            add(&mut out, Some(&json!(w)));
        }
        if !out.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// `fetch_rules`: business rules for a form, portable across tenants.
/// `fallback` is asked only when every earlier candidate failed.
pub fn fetch_rules(
    c: &dyn Api,
    module: &str,
    workspace_ids: &[Json],
    fallback: Option<&dyn Fn() -> Result<Vec<i64>>>,
    query: &BusinessRulesOptions,
) -> Result<Json> {
    let mut cands: Vec<Json> = Vec::new();
    for w in workspace_ids {
        if !w.is_null() && *w != json!("") && !cands.contains(w) {
            cands.push(w.clone());
        }
    }
    let mut empty: Option<Json> = None;
    let mut last: Option<u16> = None;
    let template = query;
    let query = |ws: Json| -> Result<(String, Map<String, Json>)> {
        let opts = BusinessRulesOptions {
            workspace_id: ws,
            ..template.clone()
        };
        let q = business_rules_query(module, &opts).map_err(form_err)?;
        let params = q
            .params
            .iter()
            .map(|(k, v)| (k.clone(), Json::String(v.clone())))
            .collect();
        Ok((q.path().trim_start_matches("/api/_/").to_string(), params))
    };
    fn attempt(
        c: &dyn Api,
        path: &str,
        params: Map<String, Json>,
        ws: Option<&Json>,
        empty: &mut Option<Json>,
        last: &mut Option<u16>,
    ) -> Result<Option<Json>> {
        let raw = match c.int_get(path, Some(params)) {
            Ok(r) => r,
            Err(Error::Api(e)) if matches!(e.status, 403 | 404) => {
                *last = Some(e.status);
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let mut rules = if !truthy(&raw) && !matches!(raw, Json::Bool(_) | Json::Number(_)) {
            json!({"business_rules": [], "user": null})
        } else {
            normalize_rules(&raw)?
        };
        if let (Some(ws), Some(m)) = (ws, rules.as_object_mut()) {
            m.insert(RULES_WORKSPACE_KEY.into(), ws.clone());
        }
        if rules.get("business_rules").is_some_and(truthy) {
            return Ok(Some(rules));
        }
        if empty.is_none() {
            *empty = Some(rules);
        }
        Ok(None)
    }
    for ws in cands.clone() {
        let (path, params) = query(ws.clone())?;
        if let Some(got) = attempt(c, &path, params, Some(&ws), &mut empty, &mut last)? {
            return Ok(got);
        }
    }
    if let Some(e) = &empty {
        return Ok(e.clone());
    }
    if let Some(fallback) = fallback {
        let extra: Vec<Json> = match fallback() {
            Ok(ws) => ws
                .into_iter()
                .map(Json::from)
                .filter(|w| !cands.contains(w))
                .collect(),
            Err(Error::Api(_)) => vec![],
            Err(e) => return Err(e),
        };
        for ws in extra {
            cands.push(ws.clone());
            let (path, params) = query(ws.clone())?;
            if let Some(got) = attempt(c, &path, params, Some(&ws), &mut empty, &mut last)? {
                return Ok(got);
            }
        }
    }
    if let Some(e) = &empty {
        return Ok(e.clone());
    }
    let (_, params) = query(json!(0))?;
    if let Some(got) = attempt(c, "business_rules", params, None, &mut empty, &mut last)? {
        return Ok(got);
    }
    if let Some(e) = empty {
        return Ok(e);
    }
    let status = last.map_or_else(|| "?".to_string(), |s| s.to_string());
    let why = if cands.is_empty() {
        format!(
            "no workspace found in the form metadata or the account, and /api/_/business_rules answered HTTP {status}"
        )
    } else {
        let names: Vec<String> = cands.iter().map(py_str).collect();
        format!(
            "workspace {} and the workspace-less path answered HTTP {status}",
            names.join(", ")
        )
    };
    Ok(
        json!({"business_rules": [], "user": null, RULES_UNAVAILABLE_KEY: why, "unavailable_status": last}),
    )
}

fn unavailable(rules: &Json) -> Option<String> {
    rules
        .get(RULES_UNAVAILABLE_KEY)
        .filter(|v| truthy(v))
        .map(py_str)
}

/// Review warning when the tenant gave no business rules.
pub fn rules_warnings(rules: &Json) -> Vec<String> {
    match unavailable(rules) {
        None => Vec::new(),
        Some(why) => vec![format!(
            "business rules could not be loaded ({why}); only the form's static required/visible \
             settings were checked, the server may still ask for more. If the account has workspaces, \
             `fsv config set forms.workspace_id N` picks the right one"
        )],
    }
}

/// One line for stderr when the rules could not be loaded.
pub fn rules_note(rules: &Json) -> Option<String> {
    unavailable(rules).map(|why| {
        format!(
            "note: business rules not loaded ({why}); checking static required/visible flags only \
             (override the workspace with `fsv config set forms.workspace_id N`)"
        )
    })
}

/// The workspace the custom fields belong to, else the most common one.
pub fn workspace_from_fields(fields: &[Json]) -> Json {
    let ws = |f: &&Json| f.get("workspace_id").filter(|v| !v.is_null()).cloned();
    let custom: Vec<Json> = fields
        .iter()
        .filter(|f| !f.get("default_field").is_some_and(truthy))
        .filter_map(|f| ws(&f))
        .collect();
    let pool = if custom.is_empty() {
        fields.iter().filter_map(|f| ws(&f)).collect()
    } else {
        custom
    };
    // Counter.most_common(1): the highest count, first seen on ties
    let mut counts: Vec<(Json, usize)> = Vec::new();
    for w in pool {
        match counts.iter_mut().find(|(k, _)| *k == w) {
            Some(e) => e.1 += 1,
            None => counts.push((w, 1)),
        }
    }
    let best = counts.iter().map(|(_, n)| *n).max().unwrap_or(0);
    counts
        .into_iter()
        .find(|(_, n)| *n == best)
        .map_or(Json::Null, |(k, _)| k)
}

/// `workspace_candidates`: the override alone when set, else `preferred`.
pub fn workspace_candidates(configured: &Json, preferred: &[Json]) -> Vec<Json> {
    if !configured.is_null() && *configured != json!("") {
        return vec![configured.clone()];
    }
    let mut out = Vec::new();
    for w in preferred {
        if !w.is_null() && *w != json!("") && !out.contains(w) {
            out.push(w.clone());
        }
    }
    out
}

/// `load_rules`: [`fetch_rules`] with the workspace defaulted portably.
pub fn load_rules(
    c: &dyn Api,
    store: &dyn MetaStore,
    module: &str,
    preferred: &[Json],
    query: &BusinessRulesOptions,
) -> Result<Json> {
    let configured = store.configured_workspace();
    let cands = workspace_candidates(&configured, preferred);
    let fallback = || account_workspaces(c);
    let use_account = configured.is_null() || configured == json!("");
    fetch_rules(
        c,
        module,
        &cands,
        if use_account { Some(&fallback) } else { None },
        query,
    )
}

// ---------------------------------------------------------------------------
// FormMeta

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
    pub module_id: Json,
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
            module: module.into(),
            form_type: form_type.into(),
            form_fields,
            rules,
            workspace_id,
            fetched_at,
            source: source.into(),
            module_id,
            spec: Arc::new(spec),
        })
    }

    pub fn fields_key(&self) -> &'static str {
        MODULES
            .iter()
            .find(|(m, _, _)| *m == self.module)
            .map_or("change_fields", |(_, _, k)| k)
    }

    pub fn raw_fields(&self) -> Vec<Json> {
        self.form_fields
            .get(self.fields_key())
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default()
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

    /// `FormState(meta.spec, meta.engine(), mode=..., existing=...)`.
    pub fn state(
        &self,
        mode: &str,
        resolver: Option<Arc<dyn LookupResolver>>,
        existing: Option<Values>,
    ) -> Result<FormState> {
        let opts = StateOptions {
            resolver,
            existing,
            ..StateOptions::default()
        };
        FormState::new(self.spec.clone(), Some(self.engine()?), mode, None, opts).map_err(form_err)
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `load_meta`: form fields + rules; `fresh` bypasses the cache (writes).
pub fn load_meta(
    c: &dyn Api,
    store: &dyn MetaStore,
    module: &str,
    mode: &str,
    fresh: bool,
    module_id: &Json,
) -> Result<FormMeta> {
    let module = module_name(module)?;
    let form_type = FORM_TYPES
        .iter()
        .find(|(m, _)| *m == mode)
        .map(|(_, t)| *t)
        .ok_or_else(|| Error::other(format!("KeyError: {}", fsv_py::py_repr_str(mode))))?;
    let (_, path, key) = *MODULES
        .iter()
        .find(|(m, _, _)| *m == module)
        .expect("module_name validated");
    let mut source = "cache";
    let cached = if fresh {
        None
    } else {
        store.cached(&format!("{module}-fields"))
    };
    let fields = match cached {
        Some(f) => f,
        None => {
            source = "server";
            let raw = c.int_get(path, None)?;
            let Some(rows) = raw
                .as_object()
                .and_then(|m| m.get(key))
                .filter(|v| truthy(v))
            else {
                return Err(ApiError::with_request(
                    200,
                    Json::String(format!("no form fields returned for {module}")),
                    "GET",
                    path,
                )
                .into());
            };
            let fields = json!({ key: rows });
            store.save(&format!("{module}-fields"), "schema", &fields);
            fields
        }
    };
    let configured = store.configured_workspace();
    let ws = if truthy(&configured) {
        configured
    } else {
        workspace_from_fields(
            fields
                .get(key)
                .and_then(Json::as_array)
                .map_or(&[][..], Vec::as_slice),
        )
    };
    let ckey = format!(
        "{module}-rules-{form_type}-ws{}{}",
        py_str(&ws),
        if module_id.is_null() {
            String::new()
        } else {
            format!("-{}", py_str(module_id))
        }
    );
    let cached = if fresh { None } else { store.cached(&ckey) };
    let rules = match cached {
        Some(r) => r,
        None => {
            source = "server";
            let query = BusinessRulesOptions {
                form_type: form_type.to_string(),
                module_id: module_id.clone(),
                ..Default::default()
            };
            let rules = load_rules(c, store, &module, std::slice::from_ref(&ws), &query)?;
            store.save(&ckey, "form_rules", &rules);
            rules
        }
    };
    let ws = rules.get(RULES_WORKSPACE_KEY).cloned().unwrap_or(ws);
    FormMeta::new(
        &module,
        form_type,
        fields,
        rules,
        ws,
        now(),
        source,
        module_id.clone(),
    )
}

// ---------------------------------------------------------------------------
// lookups

/// `LookupResolver` over the web's autocomplete endpoints.
pub struct ClientResolver {
    pub client: Arc<dyn Api + Send + Sync>,
    groups: Mutex<Option<Vec<Json>>>,
    boot: Mutex<Option<Json>>,
}

/// `a or b or c` over JSON values.
fn first_truthy<'a>(vals: &[Option<&'a Json>]) -> Option<&'a Json> {
    vals.iter().flatten().copied().find(|v| truthy(v))
}

/// `_email`: the first `email`/`details`/`primary_email` holding an `@`.
pub fn email_of(r: &Json) -> Option<String> {
    ["email", "details", "primary_email"]
        .iter()
        .filter_map(|k| r.get(*k).and_then(Json::as_str))
        .find(|v| v.contains('@'))
        .map(str::to_string)
}

/// `_person`: a lookup value named by `value`/`name`, else the id.
pub fn person(r: &Json, id: &Json) -> LookupValue {
    let shown = first_truthy(&[r.get("value"), r.get("name")]).map_or_else(|| py_str(id), py_str);
    LookupValue::new(id.clone(), shown, email_of(r).as_deref())
}

fn rows_of(doc: &Json, key: &str) -> Vec<Json> {
    doc.get(key)
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default()
}

impl ClientResolver {
    pub fn new(client: Arc<dyn Api + Send + Sync>) -> ClientResolver {
        ClientResolver {
            client,
            groups: Mutex::new(None),
            boot: Mutex::new(None),
        }
    }

    fn bootstrap(&self) -> Result<Json> {
        let mut g = self.boot.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            let got = self.client.int_get("bootstrap/agents_groups", None)?;
            *g = Some(if truthy(&got) { got } else { json!({}) });
        }
        Ok(g.clone().unwrap_or_else(|| json!({})))
    }

    /// Display name of a group/agent by id from the once-fetched bootstrap
    /// lists; `None` when unknown (e.g. requesters).
    pub fn name_for(&self, entity: &str, id: &Json) -> Option<String> {
        let key = match entity {
            "group" => "groups",
            "agent" => "agents",
            _ => return None,
        };
        let boot = self.bootstrap().ok()?;
        let want = py_str(id);
        for row in rows_of(&boot, key) {
            if row.is_object()
                && (py_str(row.get("id").unwrap_or(&Json::Null)) == want
                    || py_str(row.get("user_id").unwrap_or(&Json::Null)) == want)
                && let Some(shown) = first_truthy(&[row.get("name"), row.get("value")])
            {
                return Some(py_str(shown));
            }
        }
        None
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
        let has = |r: &Json, k: &str| r.get(k).is_some_and(|v| !v.is_null());
        if matches!(
            field.field_type.as_str(),
            "custom_lookup" | "custom_multi_lookup"
        ) && let Some(link) = field.lookup_link.as_deref().filter(|l| !l.is_empty())
        {
            let rows = c.lookup_choices(link, query).map_err(boxed)?;
            return Ok(rows
                .iter()
                .filter(|r| has(r, "id") && !r.get("deleted").is_some_and(truthy))
                .map(|r| {
                    LookupValue::new(
                        r["id"].clone(),
                        py_str(first_truthy(&[r.get("name"), r.get("value")]).unwrap_or(&r["id"])),
                        email_of(r).as_deref(),
                    )
                })
                .collect());
        }
        match entity {
            "user" => {
                let params = crate::service::obj(json!({"all_users": "true"}));
                let rows = c
                    .autocomplete("requesters", query, Some(params))
                    .map_err(boxed)?;
                Ok(rows
                    .iter()
                    .filter(|r| has(r, "id") && !r.get("deleted").is_some_and(truthy))
                    .map(|r| person(r, &r["id"]))
                    .collect())
            }
            "agent" => {
                let rows = c.autocomplete("agents", query, None).map_err(boxed)?;
                Ok(rows
                    .iter()
                    .filter(|r| {
                        has(r, "user_id")
                            && !first_truthy(&[r.get("value")])
                                .map(py_str)
                                .unwrap_or_default()
                                .contains("(Deactivated)")
                    })
                    .map(|r| person(r, &r["user_id"]))
                    .collect())
            }
            "group" => {
                let mut groups = self.groups.lock().unwrap_or_else(|p| p.into_inner());
                if groups.is_none() {
                    *groups = Some(rows_of(&self.bootstrap().map_err(boxed)?, "groups"));
                }
                let q = norm(query);
                Ok(groups
                    .iter()
                    .flatten()
                    .filter(|g| {
                        has(g, "id")
                            && norm(
                                &g.get("name")
                                    .filter(|v| !v.is_null())
                                    .map(py_str)
                                    .unwrap_or_default(),
                            )
                            .contains(&q)
                    })
                    .map(|g| {
                        LookupValue::new(
                            g["id"].clone(),
                            py_str(first_truthy(&[g.get("name")]).unwrap_or(&g["id"])),
                            None,
                        )
                    })
                    .collect())
            }
            _ => match field.lookup_link.as_deref().filter(|l| !l.is_empty()) {
                Some(link) => {
                    let rows = c.lookup_choices(link, query).map_err(boxed)?;
                    Ok(rows
                        .iter()
                        .filter(|r| has(r, "id"))
                        .map(|r| {
                            LookupValue::new(
                                r["id"].clone(),
                                py_str(
                                    first_truthy(&[r.get("name"), r.get("value")])
                                        .unwrap_or(&r["id"]),
                                ),
                                email_of(r).as_deref(),
                            )
                        })
                        .collect())
                }
                None => Ok(Vec::new()),
            },
        }
    }
}

/// `resolve_requester`: exact requester match (email or full name). Anything
/// else is an error listing candidates, unless `pick` chooses.
#[allow(clippy::type_complexity)]
pub fn resolve_requester(
    c: &dyn Api,
    value: &str,
    pick: Option<&dyn Fn(&[LookupValue]) -> LookupValue>,
) -> Result<LookupValue> {
    let q = fsv_core::pystr::strip(value).to_string();
    if q.is_empty() {
        return Err(Error::value("requester is empty"));
    }
    let rows = c.autocomplete(
        "requesters",
        &q,
        Some(crate::service::obj(json!({"all_users": "true"}))),
    )?;
    let found: Vec<LookupValue> = rows
        .iter()
        .filter(|r| {
            r.get("id").is_some_and(|v| !v.is_null()) && !r.get("deleted").is_some_and(truthy)
        })
        .map(|r| person(r, &r["id"]))
        .collect();
    let key = norm(&q);
    let exact: Vec<LookupValue> = found
        .iter()
        .filter(|p| {
            [
                norm(p.email.as_deref().unwrap_or("")),
                norm(&p.display),
                norm(&p.to_string()),
            ]
            .contains(&key)
        })
        .cloned()
        .collect();
    if exact.len() == 1 && exact[0].email.as_deref().is_some_and(|e| !e.is_empty()) {
        return Ok(exact[0].clone());
    }
    let cands = if exact.is_empty() { &found } else { &exact };
    if let Some(pick) = pick
        && !cands.is_empty()
    {
        return Ok(pick(cands));
    }
    let shown_q = fsv_py::py_repr_str(&q);
    if cands.is_empty() {
        return Err(Error::value(format!(
            "no requester matches {shown_q}; use the exact email address"
        )));
    }
    let shown: Vec<String> = cands.iter().take(8).map(ToString::to_string).collect();
    let what = if exact.is_empty() {
        "no exact match"
    } else {
        "several exact matches"
    };
    Err(Error::value(format!(
        "requester {shown_q}: {what}: {}. Use the exact email address.",
        shown.join("; ")
    )))
}

// ---------------------------------------------------------------------------
// answers -> state

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
            Value::Lookup(LookupValue::new(m["id"].clone(), py_str(shown), None))
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

/// `_hint_name`: (display, email) the record itself carries for a lookup field.
pub fn hint_name(
    name: &str,
    vid: &Json,
    record: &Map<String, Json>,
) -> (Option<String>, Option<String>) {
    for key in record_objects(name) {
        let Some(obj @ Json::Object(m)) = record.get(*key) else {
            continue;
        };
        let id = m.get("id").unwrap_or(&Json::Null);
        if (id.is_null() || id == vid || py_str(id) == py_str(vid) || m.get("user_id") == Some(vid))
            && let Some(shown) = first_truthy(&[m.get("name"), m.get("value"), m.get("full_name")])
        {
            return (Some(py_str(shown)), email_of(obj));
        }
    }
    for key in record_names(name) {
        if let Some(v) = record.get(*key).filter(|v| truthy(v)) {
            return (Some(py_str(v)), None);
        }
    }
    (None, None)
}

/// Give id-only lookup values a display name: from the record's own nested
/// objects, else the resolver's cached lookups (`name_for`).
pub fn name_existing(
    mut existing: Values,
    spec: &FormSpec,
    record: &Map<String, Json>,
    resolver: Option<&ClientResolver>,
) -> Values {
    let fix = |f: &Field, v: &Value| -> Value {
        let Value::Lookup(lv) = v else {
            return v.clone();
        };
        if !lv.display.is_empty() && lv.display != py_str(&lv.id) {
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
        let (Some(f), Some(v)) = (spec.get(&name), existing.get(&name)) else {
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

/// Set answers (label or internal name -> raw) in web field order. Returns
/// every form problem; `failed` collects the rejected api names. Errors that
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
    use crate::service::testing::*;

    fn err<T>(r: Result<T>) -> String {
        match r {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        }
    }

    fn rule() -> Json {
        json!({"id": 1, "name": "r", "condition_data": {}, "action_data": []})
    }

    fn change_form() -> Json {
        let f = |id, name, label, ty, pos, extra| form_field(id, name, label, ty, pos, extra);
        json!({"change_fields": [
            f(11, "requester", "Requester", "default_requester", 1, json!({})),
            f(12, "group", "Group", "default_group", 2, json!({})),
            f(13, "agent", "Agent", "default_agent", 3, json!({})),
            f(14, "status", "Status", "default_status", 4, json!({"choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}]})),
            f(15, "description", "Description", "default_description", 5, json!({})),
            f(16, "cf_note", "Dup", "custom_text", 6, json!({})),
            f(17, "cf_other", "Dup", "custom_text", 7, json!({})),
            f(18, "cf_env", "Environment", "custom_dropdown", 8, json!({"choices": [{"id": 1, "value": "Prod"}, {"id": 2, "value": "Test"}]})),
            f(19, "subject", "Subject", "default_subject", 9, json!({"required": true})),
        ]})
    }

    fn spec() -> FormSpec {
        FormSpec::from_form_fields(&change_form(), "change").unwrap()
    }

    fn ser(v: &Value) -> Json {
        match v {
            Value::Lookup(l) => json!({"lookup": [l.id, l.display, l.email]}),
            Value::List(xs) => Json::Array(xs.iter().map(ser).collect()),
            other => other.to_json().unwrap(),
        }
    }

    fn ser_values(v: &Values) -> Json {
        Json::Object(v.iter().map(|(k, x)| (k.clone(), ser(x))).collect())
    }

    fn lv(l: &LookupValue) -> Json {
        ser(&Value::Lookup(l.clone()))
    }

    fn calls_of(api: &FakeApi) -> Vec<(String, Json)> {
        api.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c.path.clone(),
                    c.params
                        .as_ref()
                        .and_then(|p| p.get("form_type").cloned())
                        .unwrap_or(Json::Null),
                )
            })
            .collect()
    }

    fn call(path: &str, form_type: Json) -> (String, Json) {
        (path.to_string(), form_type)
    }

    fn record() -> Map<String, Json> {
        crate::service::obj(
            json!({"requester_id": 7, "requester": {"id": 7, "name": "Ann", "email": "ann@x.io"}, "group_id": {"id": 4, "name": "Ops"},
                                   "agent_id": 21, "status": 2, "description": "", "subject": "Hi", "custom_fields": {"cf_env": "Prod", "cf_note": null}}),
        )
    }

    fn boot() -> Json {
        json!({"groups": [{"id": 4, "name": "Ops"}, {"id": 5, "name": "Ops East"}], "agents": [{"id": 1, "user_id": 21, "name": "Ann"}, {"id": 2, "value": "Bob"}]})
    }

    #[test]
    fn module_name_and_cache_helpers() {
        assert_eq!(
            (
                module_name("Changes").unwrap(),
                module_name(" tickets ").unwrap()
            ),
            ("change".to_string(), "ticket".to_string())
        );
        assert_eq!(
            err(module_name("problems")),
            "unsupported form 'problems'; supported: changes, tickets"
        );
        assert_eq!(
            cache_path(Path::new("/c"), "acme.freshservice.com", "x"),
            PathBuf::from("/c/forms/acme.freshservice.com--x.json")
        );
        let got: Vec<bool> = [
            json!({"a": [1]}),
            json!({"a": []}),
            json!({"a": 1}),
            json!([1]),
            json!([]),
            json!(""),
            json!("x"),
            Json::Null,
            json!({}),
        ]
        .iter()
        .map(non_empty)
        .collect();
        assert_eq!(
            got,
            [true, false, false, true, false, false, true, false, false]
        );
    }

    #[test]
    fn disk_and_mem_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskStore {
            cache_dir: dir.path().to_path_buf(),
            domain_key: "acme.freshservice.com".into(),
            config: Map::new(),
        };
        let mem = MemStore::default();
        for store in [&disk as &dyn MetaStore, &mem] {
            store.save("empty", "schema", &json!({"a": []}));
            store.save("full", "schema", &json!({"a": [1]}));
            assert_eq!(
                (
                    store.cached("empty"),
                    store.cached("full"),
                    store.cached("missing")
                ),
                (None, Some(json!({"a": [1]})), None)
            );
            assert_eq!(store.configured_workspace(), Json::Null);
        }
        assert!(
            dir.path()
                .join("forms/acme.freshservice.com--full.json")
                .exists()
        );
        let set = crate::service::obj(json!({"forms": {"workspace_id": "9"}}));
        assert_eq!(configured_workspace_in(&set), json!(9));
        assert_eq!(configured_workspace_in(&Map::new()), Json::Null);
    }

    #[test]
    fn normalize_rules_shapes() {
        assert_eq!(
            normalize_rules(&json!({"business_rules": [rule()], "user": {"id": 1}})).unwrap(),
            json!({"business_rules": [rule()], "user": {"id": 1}})
        );
        assert_eq!(
            normalize_rules(&json!([rule()])).unwrap(),
            json!({"business_rules": [rule()], "user": null})
        );
        assert_eq!(
            normalize_rules(&json!({"business_rules": []})).unwrap(),
            json!({"business_rules": [], "user": null})
        );
        assert_eq!(
            err(normalize_rules(&json!({"x": 1}))),
            "HTTP 200 listing business rules: unrecognised business_rules response (no business_rules / business_rule key)"
        );
        assert_eq!(
            err(normalize_rules(&json!({"business_rule": [{"id": 1}]}))),
            "HTTP 200 listing business rules: the server returned the business-rule listing without conditions/actions; fsv cannot evaluate the form from it (use the web)"
        );
    }

    #[test]
    fn workspace_ids_and_documents() {
        let got: Vec<Option<i64>> = [
            json!(5),
            json!("7"),
            json!(" 8 "),
            json!(true),
            json!("x"),
            json!(1.5),
            Json::Null,
        ]
        .iter()
        .map(ws_id)
        .collect();
        assert_eq!(got, [Some(5), Some(7), Some(8), None, None, None, None]);
        let doc = json!({"user": {"current_workspace": {"id": 3}, "workspaces": [{"id": 4, "primary": true}, {"id": 5}, 6], "workspace_ids": ["6", 5]}, "workspace_id": 9});
        assert_eq!(workspaces_in(&doc), [3, 9, 4, 5, 6]);
        assert!(workspaces_in(&json!([1])).is_empty());
        assert!(workspaces_in(&json!({"a": {"b": {"c": {"d": {"workspace_id": 1}}}}})).is_empty());
    }

    #[test]
    fn account_workspaces_sources() {
        let mut cookie = FakeApi::new();
        cookie.cookies = vec!["12".into(), "x".into()];
        assert_eq!(account_workspaces(&cookie).unwrap(), [12]);
        assert!(account_workspaces(&FakeApi::new()).unwrap().is_empty());
        assert_eq!(
            account_workspaces(&FakeApi::new().ok("GET bootstrap/me", json!({"workspace_id": 4})))
                .unwrap(),
            [4]
        );
        let ws = FakeApi::new().ok(
            "GET workspaces",
            json!({"workspaces": [{"id": 7}, {"id": 8, "default": true}]}),
        );
        assert_eq!(account_workspaces(&ws).unwrap(), [8, 7]);
        assert!(matches!(
            account_workspaces(&FakeApi::new().on("GET bootstrap/me", Reply::Session)).unwrap_err(),
            Error::Session(_)
        ));
    }

    #[test]
    fn fetch_rules_candidates_fallback_and_unavailable() {
        let q = BusinessRulesOptions::default();
        let api = FakeApi::new().ok(
            "GET ws/3/business_rules",
            json!({"business_rules": [rule()], "user": {"id": 1}}),
        );
        assert_eq!(
            fetch_rules(
                &api,
                "change",
                &[json!(2), json!(3), json!(2), Json::Null],
                None,
                &q
            )
            .unwrap(),
            json!({"business_rules": [rule()], "user": {"id": 1}, "workspace": 3})
        );
        assert_eq!(
            calls_of(&api),
            [
                call("ws/2/business_rules", json!("new_form")),
                call("ws/3/business_rules", json!("new_form"))
            ]
        );
        let api = FakeApi::new().ok("GET ws/2/business_rules", json!({})).ok(
            "GET ws/3/business_rules",
            json!({"business_rules": [rule()]}),
        );
        assert_eq!(
            fetch_rules(&api, "change", &[json!(2), json!(3)], None, &q).unwrap(),
            json!({"business_rules": [rule()], "user": null, "workspace": 3})
        );
        let api = FakeApi::new().ok(
            "GET ws/5/business_rules",
            json!({"business_rules": [rule()]}),
        );
        let fallback = || Ok(vec![2, 5]);
        assert_eq!(
            fetch_rules(&api, "ticket", &[json!(2)], Some(&fallback), &q).unwrap(),
            json!({"business_rules": [rule()], "user": null, "workspace": 5})
        );
        assert_eq!(
            calls_of(&api),
            [
                call("ws/2/business_rules", json!("new_form")),
                call("ws/5/business_rules", json!("new_form"))
            ]
        );
        let api = FakeApi::new().ok("GET business_rules", json!({"business_rules": [rule()]}));
        assert_eq!(
            fetch_rules(&api, "change", &[], None, &q).unwrap(),
            json!({"business_rules": [rule()], "user": null})
        );
        assert_eq!(calls_of(&api), [call("business_rules", json!("new_form"))]);
        assert_eq!(
            fetch_rules(&FakeApi::new(), "change", &[json!(2)], None, &q).unwrap(),
            json!({"business_rules": [], "user": null, "unavailable": "workspace 2 and the workspace-less path answered HTTP 404", "unavailable_status": 404})
        );
        assert_eq!(
            fetch_rules(
                &FakeApi::new().on("GET business_rules", Reply::Status(403)),
                "change",
                &[],
                None,
                &q
            )
            .unwrap(),
            json!({"business_rules": [], "user": null,
                          "unavailable": "no workspace found in the form metadata or the account, and /api/_/business_rules answered HTTP 403", "unavailable_status": 403})
        );
        let down = FakeApi::new().on("GET ws/2/business_rules", Reply::Status(500));
        assert!(
            matches!(fetch_rules(&down, "change", &[json!(2)], None, &q).unwrap_err(), Error::Api(e) if e.status == 500)
        );
    }

    #[test]
    fn rules_messages_and_workspace_choice() {
        assert_eq!(
            rules_warnings(&json!({"unavailable": "x"})),
            [
                "business rules could not be loaded (x); only the form's static required/visible settings were checked, the server may still ask for more. If the account has workspaces, `fsv config set forms.workspace_id N` picks the right one"
            ]
        );
        assert!(rules_warnings(&json!({})).is_empty() && rules_warnings(&Json::Null).is_empty());
        assert_eq!(
            rules_note(&json!({"unavailable": "x"})).as_deref(),
            Some(
                "note: business rules not loaded (x); checking static required/visible flags only (override the workspace with `fsv config set forms.workspace_id N`)"
            )
        );
        assert_eq!(rules_note(&json!({"business_rules": []})), None);
        let rows = |v: Json| v.as_array().unwrap().clone();
        assert_eq!(
            workspace_from_fields(&rows(change_form()["change_fields"].clone())),
            json!(2)
        );
        assert_eq!(
            workspace_from_fields(&rows(ticket_form(3)["ticket_fields"].clone())),
            json!(1)
        );
        assert_eq!(workspace_from_fields(&[]), Json::Null);
        assert_eq!(
            workspace_from_fields(&rows(
                json!([{"workspace_id": 4, "default_field": true}, {"workspace_id": 5}, {"workspace_id": 5}])
            )),
            json!(5)
        );
        assert_eq!(
            workspace_candidates(
                &Json::Null,
                &[json!(1), Json::Null, json!(""), json!(1), json!(2)]
            ),
            [json!(1), json!(2)]
        );
        assert!(workspace_candidates(&Json::Null, &[]).is_empty());
        assert_eq!(
            workspace_candidates(&json!(9), &[json!(1), json!(2)]),
            [json!(9)]
        );
    }

    #[test]
    fn load_rules_override_and_account() {
        let q = BusinessRulesOptions::default();
        let store = MemStore {
            workspace: json!(9),
            ..Default::default()
        };
        let api = FakeApi::new().ok(
            "GET ws/9/business_rules",
            json!({"business_rules": [rule()]}),
        );
        assert_eq!(
            load_rules(&api, &store, "change", &[json!(1)], &q).unwrap(),
            json!({"business_rules": [rule()], "user": null, "workspace": 9})
        );
        assert_eq!(
            calls_of(&api),
            [call("ws/9/business_rules", json!("new_form"))]
        );
        let api = FakeApi::new()
            .ok("GET bootstrap/me", json!({"workspace_id": 4}))
            .ok(
                "GET ws/4/business_rules",
                json!({"business_rules": [rule()]}),
            );
        assert_eq!(
            load_rules(&api, &MemStore::default(), "change", &[Json::Null], &q).unwrap(),
            json!({"business_rules": [rule()], "user": null, "workspace": 4})
        );
        assert_eq!(
            calls_of(&api),
            [
                call("bootstrap/me", Json::Null),
                call("ws/4/business_rules", json!("new_form"))
            ]
        );
    }

    #[test]
    fn form_meta_properties() {
        let m = FormMeta::new(
            "change",
            "new_form",
            change_form(),
            json!({"business_rules": [], "user": {"id": 1}}),
            json!(1),
            0.0,
            "server",
            Json::Null,
        )
        .unwrap();
        assert_eq!(
            (m.fields_key(), m.raw_fields().len(), m.user().cloned()),
            ("change_fields", 9, Some(json!({"id": 1})))
        );
        assert_eq!(
            m.fingerprint().hash,
            "fc9de18c99da72c0b5a2e45c2eba37b1176178d32eb70a61b89124e8926756db"
        );
        let as_of = m.as_of();
        assert_eq!(
            (
                as_of["updated_at"].clone(),
                as_of["source"].clone(),
                as_of["fingerprint"].clone()
            ),
            (
                json!("2030-01-02T03:04:05+00:00"),
                json!("server"),
                json!("fc9de18c99da72c0")
            )
        );
        // Python ran under TZ=UTC: "1970-01-01T00:00:00+00:00"; the offset follows the host zone.
        assert_eq!(
            as_of["fetched_at"].as_str().map(str::len),
            Some("1970-01-01T00:00:00+00:00".len())
        );
        let names: Vec<String> = m
            .spec()
            .fields()
            .iter()
            .map(|f| f.api_name.clone())
            .collect();
        assert_eq!(
            names,
            [
                "requester",
                "group",
                "agent",
                "status",
                "description",
                "cf_note",
                "cf_other",
                "cf_env",
                "subject",
                "requested_for"
            ]
        );
        assert!(m.engine().is_ok() && m.state("new", None, None).is_ok());
    }

    #[test]
    fn load_meta_fresh_then_cached_fields() {
        let api = FakeApi::new()
            .ok("GET ticket_form_fields", ticket_form(3))
            .ok("GET ws/1/business_rules", json!({}));
        let store = MemStore::default();
        let m = load_meta(&api, &store, "tickets", "edit", true, &json!(123)).unwrap();
        assert_eq!(
            (
                m.module.as_str(),
                m.form_type.as_str(),
                m.rules.clone(),
                m.workspace_id.clone(),
                m.source.as_str(),
                m.module_id.clone()
            ),
            (
                "ticket",
                "edit_form",
                json!({"business_rules": [], "user": null, "workspace": 1}),
                json!(1),
                "server",
                json!(123)
            )
        );
        assert_eq!(
            calls_of(&api),
            [
                call("ticket_form_fields", Json::Null),
                call("ws/1/business_rules", json!("edit_form"))
            ]
        );
        let again = load_meta(&api, &store, "ticket", "edit", false, &json!(123)).unwrap();
        assert_eq!(
            (again.source.as_str(), again.rules.clone()),
            (
                "server",
                json!({"business_rules": [], "user": null, "workspace": 1})
            )
        );
        assert_eq!(
            calls_of(&api),
            [
                call("ticket_form_fields", Json::Null),
                call("ws/1/business_rules", json!("edit_form")),
                call("ws/1/business_rules", json!("edit_form"))
            ]
        );
        let none = FakeApi::new().ok("GET change_form_fields", json!({"change_fields": []}));
        assert_eq!(
            err(load_meta(
                &none,
                &MemStore::default(),
                "change",
                "new",
                true,
                &Json::Null
            )),
            "HTTP 200 listing change form fields: no form fields returned for change"
        );
    }

    #[test]
    fn client_resolver_names_and_lookups() {
        let r = ClientResolver::new(Arc::new(
            FakeApi::new().ok("GET bootstrap/agents_groups", boot()),
        ));
        assert_eq!(
            [
                r.name_for("group", &json!(4)),
                r.name_for("agent", &json!("21")),
                r.name_for("agent", &json!(2)),
                r.name_for("user", &json!(1)),
                r.name_for("group", &json!(9))
            ],
            [
                Some("Ops".to_string()),
                Some("Ann".into()),
                Some("Bob".into()),
                None,
                None
            ]
        );
        let api = FakeApi::new()
            .ok("GET bootstrap/agents_groups", boot())
            .ok("GET autocomplete/requesters", json!([{"id": 1, "value": "Ann", "email": "ann@x.io"}, {"id": 2, "value": "Gone", "deleted": true}, {"name": "noid"}]))
            .ok("GET autocomplete/agents", json!([{"user_id": 21, "value": "Ann", "details": "ann@x.io"}, {"user_id": 22, "value": "Old (Deactivated)"}]));
        let r = ClientResolver::new(Arc::new(api));
        let s = spec();
        let look = |entity: &str, q: &str, field: &str| -> Json {
            Json::Array(
                r.lookup(entity, q, s.find(field).unwrap())
                    .unwrap()
                    .iter()
                    .map(lv)
                    .collect(),
            )
        };
        assert_eq!(
            look("user", "a", "requester"),
            json!([{"lookup": [1, "Ann", "ann@x.io"]}])
        );
        assert_eq!(
            look("agent", "a", "agent"),
            json!([{"lookup": [21, "Ann", "ann@x.io"]}])
        );
        assert_eq!(
            look("group", "ops e", "group"),
            json!([{"lookup": [5, "Ops East", null]}])
        );
        assert_eq!(look("other", "x", "subject"), json!([]));
    }

    const CUSTOM_LINK: &str = "/lookup_choices?field_name=reference&module=change";

    /// Sets `reference` on a custom (multi) lookup backed by `rows`; returns
    /// the kind, the stored values and encoded payload (or the error), and
    /// the `(link, q)` calls made.
    fn custom_lookup(rows: Json, answer: Value, multi: bool, source: i64) -> Json {
        let spec = FormSpec::from_form_fields(
            &json!([{"name": "reference", "label": "Reference", "position": 1,
                     "field_type": if multi { "custom_multi_lookup" } else { "custom_lookup" },
                     "lookup_config": {"data_source": source, "link": CUSTOM_LINK}}]),
            "change",
        )
        .unwrap();
        let api = Arc::new(FakeApi::new().ok(&format!("GET {CUSTOM_LINK}"), rows));
        let opts = fsv_forms::state::StateOptions {
            resolver: Some(Arc::new(ClientResolver::new(api.clone()))),
            tz: Some("UTC".into()),
            tz_env: Some(fsv_forms::tz::TzEnv::empty()),
            ..Default::default()
        };
        let mut s =
            fsv_forms::state::FormState::new(Arc::new(spec), None, "new", None, opts).unwrap();
        let f = s.spec.find("reference").unwrap().clone();
        let mut out = json!({"kind": f.kind.as_str()});
        match s.set("reference", answer) {
            Ok(_) => {
                let v = s.get("reference").unwrap().unwrap().clone();
                out["values"] = match ser(&v) {
                    Json::Array(a) => Json::Array(a),
                    one => json!([one]),
                };
                out["encoded"] =
                    fsv_forms::payload::encode_value(&s.spec, &f, &v, &Values::new()).unwrap();
            }
            Err(e) => out["error"] = json!(e.to_string()),
        }
        out["calls"] = api
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| json!([c.path, c.params.as_ref().unwrap()["q"]]))
            .collect();
        out
    }

    #[test]
    fn custom_lookups_resolve_through_their_link() {
        let call = |q: &str| json!([CUSTOM_LINK, q]);
        let acme = || json!([{"id": 901, "value": "Acme"}]);
        let found = |q: &str| {
            json!({"kind": "lookup", "values": [{"lookup": [901, "Acme", null]}], "encoded": 901,
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
            json!({"kind": "lookup",
                   "values": [{"lookup": [901, "Acme", null]}, {"lookup": [902, "Globex", null]}],
                   "encoded": [901, 902], "calls": [call("901"), call("902")]})
        );
        assert_eq!(
            custom_lookup(
                json!([{"id": 901, "name": "Acme", "email": "a@x.io"}]),
                901.into(),
                false,
                5
            ),
            json!({"kind": "lookup_user", "values": [{"lookup": [901, "Acme", "a@x.io"]}],
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
        let api = Arc::new(FakeApi::new().ok(
            &format!("GET {CUSTOM_LINK}"),
            json!([{"id": 1, "name": "A"}, {"id": null, "name": "N"},
                   {"id": 3, "value": "C", "deleted": true}, {"id": 4, "email": "d@x.io"}]),
        ));
        let r = ClientResolver::new(api.clone());
        let got: Vec<Json> = r
            .lookup("user", "q", spec.find("reference").unwrap())
            .unwrap()
            .iter()
            .map(lv)
            .collect();
        assert_eq!(
            got,
            [
                json!({"lookup": [1, "A", null]}),
                json!({"lookup": [4, "4", "d@x.io"]})
            ]
        );
        assert_eq!(api.gets(), [CUSTOM_LINK]);
    }

    #[test]
    fn email_and_person() {
        let got: Vec<Option<String>> = [
            json!({"email": "a@b"}),
            json!({"email": "x", "details": "d@e"}),
            json!({"primary_email": "p@q"}),
            json!({"email": 5}),
        ]
        .iter()
        .map(email_of)
        .collect();
        assert_eq!(
            got,
            [
                Some("a@b".to_string()),
                Some("d@e".into()),
                Some("p@q".into()),
                None
            ]
        );
        assert_eq!(
            lv(&person(&json!({"value": "Ann", "email": "a@b"}), &json!(1))),
            json!({"lookup": [1, "Ann", "a@b"]})
        );
        assert_eq!(
            lv(&person(&json!({"name": "N"}), &json!(2))),
            json!({"lookup": [2, "N", null]})
        );
        assert_eq!(
            lv(&person(&json!({}), &json!(3))),
            json!({"lookup": [3, "3", null]})
        );
    }

    #[test]
    fn resolve_requester_exact_match() {
        let api = FakeApi::new().ok("GET autocomplete/requesters", json!([{"id": 1, "value": "Ann Lee", "email": "ann@x.io"}, {"id": 2, "value": "Ann Lee", "email": "ann2@x.io"},
                                                                          {"id": 3, "value": "Bo", "email": null}]));
        assert_eq!(
            lv(&resolve_requester(&api, "ANN@x.io", None).unwrap()),
            json!({"lookup": [1, "Ann Lee", "ann@x.io"]})
        );
        assert_eq!(
            err(resolve_requester(&api, "Ann Lee", None)),
            "requester 'Ann Lee': several exact matches: Ann Lee <ann@x.io>; Ann Lee <ann2@x.io>. Use the exact email address."
        );
        let last = |c: &[LookupValue]| c[c.len() - 1].clone();
        assert_eq!(
            lv(&resolve_requester(&api, "Ann Lee", Some(&last)).unwrap()),
            json!({"lookup": [2, "Ann Lee", "ann2@x.io"]})
        );
        assert_eq!(
            err(resolve_requester(&api, "Bo", None)),
            "requester 'Bo': several exact matches: Bo. Use the exact email address."
        );
        let none = FakeApi::new().ok("GET autocomplete/requesters", json!([]));
        assert_eq!(
            err(resolve_requester(&none, "zed", None)),
            "no requester matches 'zed'; use the exact email address"
        );
        assert_eq!(
            err(resolve_requester(&api, "  ", None)),
            "requester is empty"
        );
    }

    #[test]
    fn find_field_and_kinds() {
        let s = spec();
        let names: Vec<String> = ["cf_env", " environment ", "Subject"]
            .iter()
            .map(|k| find_field(&s, k).unwrap().api_name.clone())
            .collect();
        assert_eq!(names, ["cf_env", "cf_env", "subject"]);
        assert_eq!(
            find_field(&s, "dup").unwrap_err().to_string(),
            "label 'dup' is ambiguous (fields: cf_note, cf_other); use the internal name"
        );
        assert_eq!(
            find_field(&s, "nope").unwrap_err().to_string(),
            "unknown field: 'nope'"
        );
        let pick = |p: fn(&Field) -> bool| {
            s.fields()
                .iter()
                .filter(|f| p(f))
                .map(|f| f.api_name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            (pick(is_status), pick(is_requester), pick(is_description)),
            (
                vec!["status".to_string()],
                vec!["requester".to_string()],
                vec!["description".to_string()]
            )
        );
    }

    #[test]
    fn existing_values_and_names() {
        let s = spec();
        let rec = record();
        let ex = existing_from_record(&s, &rec);
        assert_eq!(
            ser_values(&ex),
            json!({"requester": {"lookup": [7, "", null]}, "group": {"lookup": [4, "", null]}, "agent": {"lookup": [21, "", null]},
                                           "status": 2, "cf_env": "Prod", "subject": "Hi"})
        );
        let empty = Map::new();
        let owner = crate::service::obj(json!({"owner_name": "Own"}));
        let full = crate::service::obj(json!({"agent": {"user_id": 21, "full_name": "Full"}}));
        assert_eq!(
            [
                hint_name("requester", &json!(7), &rec),
                hint_name("agent", &json!(21), &owner),
                hint_name("group", &json!(4), &empty),
                hint_name("agent", &json!(21), &full)
            ],
            [
                (Some("Ann".to_string()), Some("ann@x.io".to_string())),
                (Some("Own".into()), None),
                (None, None),
                (Some("Full".into()), None)
            ]
        );
        let r = ClientResolver::new(Arc::new(
            FakeApi::new().ok("GET bootstrap/agents_groups", boot()),
        ));
        assert_eq!(
            ser_values(&name_existing(ex.clone(), &s, &rec, Some(&r))),
            json!({"requester": {"lookup": [7, "Ann", "ann@x.io"]}, "group": {"lookup": [4, "Ops", null]}, "agent": {"lookup": [21, "Ann", null]},
                          "status": 2, "cf_env": "Prod", "subject": "Hi"})
        );
        assert_eq!(
            ser_values(&name_existing(ex.clone(), &s, &empty, None)),
            ser_values(&ex)
        );
    }

    /// `name_existing` wraps `namer(...)` in `except Exception`, which catches
    /// `SessionError` too: the value stays id-only after one bootstrap read
    /// (Python: `{'group': LookupValue(id=5, display='5', email=None)}`, 1 call).
    #[test]
    fn name_existing_swallows_a_session_failure_like_python() {
        let s = spec();
        let api = Arc::new(FakeApi::new().on("GET bootstrap/agents_groups", Reply::Session));
        let r = ClientResolver::new(api.clone());
        let mut ex = Values::new();
        ex.insert(
            "group".into(),
            Value::Lookup(LookupValue::new(json!(5), "5", None)),
        );
        assert_eq!(
            ser_values(&name_existing(ex, &s, &Map::new(), Some(&r))),
            json!({"group": {"lookup": [5, "5", null]}})
        );
        assert_eq!(api.gets(), ["bootstrap/agents_groups"]);
    }

    #[test]
    fn apply_answers_collects_problems() {
        let mut state =
            FormState::new(Arc::new(spec()), None, "new", None, StateOptions::default()).unwrap();
        let answers = crate::service::obj(
            json!({"Environment": "Prod", "subject": "S", "dup": "x", "cf_env": "Nope", "Status": "Planning"}),
        );
        let mut failed = HashSet::new();
        assert_eq!(
            apply_answers(&mut state, &answers, Some(&mut failed)).unwrap(),
            [
                "label 'dup' is ambiguous (fields: cf_note, cf_other); use the internal name",
                "Environment: 'Nope' is not a valid choice; choose one of: Prod, Test"
            ]
        );
        assert_eq!(failed.into_iter().collect::<Vec<_>>(), ["cf_env"]);
        assert_eq!(
            (ser(&state.values["subject"]), ser(&state.values["status"])),
            (json!("S"), json!("Planning"))
        );
    }
}
