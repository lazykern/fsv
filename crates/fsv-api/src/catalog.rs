//! Service catalog (`catalog.py`): browse categories and items, load an
//! item's form, build a reviewed service-request plan, and place it.
//!
//! A plan is the engine state, the exact request body (uploads shown as
//! `<upload N: name>` placeholders, replaced by the attachment ids after the
//! files are uploaded), the files, the review, and a plan hash over all of
//! it. Nothing is uploaded or sent while building a plan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::{Map, Value as Json, json};

use fsv_core::error::{ApiError, Error, Result};
use fsv_core::journal::PlanStep;
use fsv_core::review::{Review, ReviewAttachment, ReviewMessage};
use fsv_forms::fetch::BusinessRulesOptions;
use fsv_forms::payload::{SrOptions, build_sr_payload};
use fsv_forms::plan::PlanFiles;
use fsv_forms::rules::{ChoiceRestriction, EngineOptions, is_empty};
use fsv_forms::spec::is_content_type;
use fsv_forms::{Conflict, EvaluatedForm, Field, FormSpec, FormState, Kind, LookupResolver, LookupValue, MetadataFingerprint, RulesEngine, StateOptions, metadata_fingerprint, plan_hash};
use fsv_py::{py_repr_str, py_str, truthy};

use crate::formmeta::{MetaStore, RULES_UNAVAILABLE_KEY, RULES_WORKSPACE_KEY, apply_answers, find_field, form_err, load_rules, rules_warnings};
use crate::notes::{EMAIL, valid_recipient};
use crate::service::{Api, Journal, definite_rejection, obj};

pub const PER_PAGE: i64 = 30;
pub const MAX_PAGES: i64 = 50;
pub const SAFETY: &str = "This sends a real service request to the catalog item's fulfilment team.";
pub const REQUESTER_MISSING: &str = "requester is not set: pass --requester EMAIL, add `requester:` to the draft, \
                                     or set defaults.sr_requester in the fsv config";
pub const REQUESTED_FOR_NOTE: &str = "Separate requested-for is available via --requested-for; requester remains unchanged.";
const DUPLICATE_WINDOW_HOURS: f64 = 24.0;

/// `FlowExit(code, message)`.
fn flow_exit(code: i32, msg: impl Into<String>) -> Error {
    Error::exit(code, msg)
}

/// `norm`: NFC, collapsed whitespace, casefolded `str(value)`.
pub fn norm(value: &Json) -> String {
    fsv_core::safety::norm(&py_str(value))
}

fn norm_s(s: &str) -> String {
    fsv_core::safety::norm(s)
}

// ---------------------------------------------------------------------------
// reads

/// `_rows`: the dict rows of a list, or of `data[key]`.
pub fn rows(data: &Json, key: &str) -> Vec<Json> {
    let list = match data {
        Json::Array(a) => Some(a),
        Json::Object(m) => m.get(key).and_then(Json::as_array),
        _ => None,
    };
    list.map(|a| a.iter().filter(|r| r.is_object()).cloned().collect()).unwrap_or_default()
}

/// `list_categories`: flattened; nested sub-categories get `parent_id`.
pub fn list_categories(c: &dyn Api) -> Result<Vec<Json>> {
    let data = c.int_get("catalog/service-categories", Some(obj(json!({"category_hierarchy": "true"}))))?;
    fn walk(rows: &[Json], parent: &Json, out: &mut Vec<Json>) {
        for r in rows {
            let Some(m) = r.as_object() else { continue };
            let mut row: Map<String, Json> = m.iter().filter(|(k, _)| *k != "sub_categories" && *k != "children").map(|(k, v)| (k.clone(), v.clone())).collect();
            if !parent.is_null() && row.get("parent_id").is_none_or(Json::is_null) {
                row.insert("parent_id".into(), parent.clone());
            }
            out.push(Json::Object(row));
            let kids = [m.get("sub_categories"), m.get("children")].into_iter().flatten().find(|v| truthy(v));
            if let Some(Json::Array(kids)) = kids {
                let kids: Vec<Json> = kids.iter().filter(|k| k.is_object()).cloned().collect();
                walk(&kids, m.get("id").unwrap_or(&Json::Null), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&rows(&data, "service_categories"), &Json::Null, &mut out);
    Ok(out)
}

/// `list_items`: every requestable item (all pages), or the popular ones.
pub fn list_items(c: &dyn Api, popular: bool, max_pages: i64) -> Result<Vec<Json>> {
    if popular {
        return Ok(rows(&c.int_get("catalog/service-items", Some(obj(json!({"filter": "frequently_requested"}))))?, "service_items"));
    }
    let mut out = Vec::new();
    let mut seen: Vec<Json> = Vec::new();
    for page in 1..=max_pages {
        let data = c.int_get("catalog/service-items", Some(obj(json!({"aurora_search": "true", "page": page, "per_page": PER_PAGE}))))?;
        let rs = rows(&data, "service_items");
        for r in &rs {
            let key = r.get("id").or_else(|| r.get("display_id")).cloned().unwrap_or(Json::Null);
            if !seen.contains(&key) && !r.get("deleted").is_some_and(truthy) {
                seen.push(key);
                out.push(r.clone());
            }
        }
        let total = data.get("meta").filter(|m| truthy(m)).and_then(|m| m.get("total_pages")).cloned().unwrap_or(Json::Null);
        if rs.is_empty() || total.as_i64().is_some_and(|t| page >= t) || (total.is_null() && (rs.len() as i64) < PER_PAGE) {
            break;
        }
    }
    Ok(out)
}

fn name_of(c: &Json) -> String {
    c.get("name").map(py_str).unwrap_or_default()
}

/// `resolve_category`: a category by id or name plus its sub-categories.
pub fn resolve_category(categories: &[Json], text: &str) -> Result<Vec<Json>> {
    let q = fsv_core::pystr::strip(text);
    let mut hits: Vec<&Json> = categories.iter().filter(|c| py_str(c.get("id").unwrap_or(&Json::Null)) == q).collect();
    if hits.is_empty() {
        hits = categories.iter().filter(|c| norm_s(&name_of(c)) == norm_s(q)).collect();
    }
    if hits.is_empty() {
        hits = categories.iter().filter(|c| norm_s(&name_of(c)).contains(&norm_s(q))).collect();
    }
    if hits.len() != 1 {
        let pool: Vec<&Json> = if hits.is_empty() { categories.iter().collect() } else { hits.clone() };
        let names: Vec<String> = pool.iter().take(15).map(|c| py_str(c.get("name").unwrap_or(&Json::Null))).collect();
        let what = if hits.is_empty() { "no category matches" } else { "several categories match" };
        return Err(Error::value(format!("{what} {}; categories: {}", py_repr_str(q), names.join(", "))));
    }
    let mut picked = vec![hits[0].clone()];
    let mut ids = vec![hits[0].get("id").cloned().unwrap_or(Json::Null)];
    let mut grew = true;
    while grew {
        grew = false;
        for c in categories {
            let (pid, id) = (c.get("parent_id").cloned().unwrap_or(Json::Null), c.get("id").cloned().unwrap_or(Json::Null));
            if ids.contains(&pid) && !ids.contains(&id) {
                ids.push(id);
                picked.push(c.clone());
                grew = true;
            }
        }
    }
    Ok(picked)
}

/// `str(x or "")`.
fn or_empty(v: Option<&Json>) -> String {
    v.filter(|v| truthy(v)).map(py_str).unwrap_or_default()
}

/// `filter_items`: by category ids and/or every search word.
pub fn filter_items(items: &[Json], category_ids: Option<&[Json]>, search: Option<&str>, categories: &[Json]) -> Vec<Json> {
    let names: Vec<(Json, String)> = categories.iter().map(|c| (c.get("id").cloned().unwrap_or(Json::Null), or_empty(c.get("name")))).collect();
    let ids: Option<HashSet<String>> = category_ids.map(|ids| ids.iter().map(py_str).collect());
    let words: Vec<String> = search.filter(|s| !s.is_empty()).map(|s| norm_s(s).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for it in items {
        if let Some(ids) = &ids
            && !ids.contains(&py_str(it.get("category_id").unwrap_or(&Json::Null)))
        {
            continue;
        }
        let cat = it.get("category_name").filter(|v| truthy(v)).map(py_str).unwrap_or_else(|| {
            let cid = it.get("category_id").unwrap_or(&Json::Null);
            names.iter().find(|(k, _)| k == cid).map(|(_, n)| n.clone()).unwrap_or_default()
        });
        let parts = [or_empty(it.get("name")), or_empty(it.get("short_description")), cat, or_empty(it.get("display_id"))];
        let hay = norm_s(&parts.join(" "));
        if !words.iter().all(|w| hay.contains(w.as_str())) {
            continue;
        }
        out.push(it.clone());
    }
    out
}

/// `get_item`: the item form.
pub fn get_item(c: &dyn Api, display_id: &Json) -> Result<Json> {
    let path = format!("catalog/service-items/{}", py_str(display_id));
    let data = c.int_get(&path, None)?;
    let item = data.as_object().map(|m| m.get("service_item").cloned().unwrap_or_else(|| data.clone()));
    match item {
        Some(i @ Json::Object(_)) if i.get("custom_fields").is_some_and(truthy) || i.get("display_id").is_some_and(|v| !v.is_null()) => Ok(i),
        _ => Err(ApiError::with_request(200, Json::String(format!("no catalog item {} in the response", py_str(display_id))), "GET", path).into()),
    }
}

/// `_item_ref`: `#77` -> `77`.
pub fn item_ref(r: &str) -> String {
    let s = fsv_core::pystr::strip(r);
    s.strip_prefix('#').unwrap_or(s).to_string()
}

/// `find_item`: by display id, else an exact or unique partial name.
pub fn find_item(c: &dyn Api, r: &str) -> Result<Json> {
    let s = item_ref(r);
    if s.is_empty() {
        return Err(Error::value("catalog item is empty"));
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return get_item(c, &json!(s.parse::<i64>().unwrap_or(0)));
    }
    let items = list_items(c, false, MAX_PAGES)?;
    let exact: Vec<Json> = items.iter().filter(|i| norm_s(&name_of(i)) == norm_s(&s)).cloned().collect();
    let hits = if exact.is_empty() { filter_items(&items, None, Some(&s), &[]) } else { exact };
    match hits.len() {
        1 => get_item(c, hits[0].get("display_id").unwrap_or(&Json::Null)),
        0 => Err(Error::value(format!("no catalog item matches {}; list them with `fsv sr catalog --search TEXT`", py_repr_str(&s)))),
        n => {
            let shown: Vec<String> = hits.iter().take(10).map(|i| format!("#{} {}", i.get("display_id").map_or("None".into(), py_str), i.get("name").map_or("None".into(), py_str))).collect();
            Err(Error::value(format!("{n} catalog items match {}: {}. Use the item id.", py_repr_str(&s), shown.join("; "))))
        }
    }
}

pub fn place_request(c: &dyn Api, display_id: &Json, body: &Json) -> Result<Json> {
    c.int_post(&format!("catalog/service-items/{}/place-request", py_str(display_id)), body)
}

/// `upload`: [`crate::create::upload_file`].
pub fn upload(c: &dyn Api, path: &str) -> Result<i64> {
    crate::create::upload_file(c, path)
}

/// `created_ref`: `{"id", "display"}` of the created request.
pub fn created_ref(result: &Json) -> Json {
    let mut cands: Vec<&Map<String, Json>> = Vec::new();
    if let Json::Object(m) = result {
        cands.push(m);
        cands.extend(m.values().filter_map(Json::as_object));
    }
    let has = |m: &Map<String, Json>, k: &str| m.get(k).is_some_and(|v| !v.is_null());
    for c in cands {
        if has(c, "id") || has(c, "display_id") {
            let shown = match c.get("human_display_id").filter(|v| truthy(v)) {
                Some(h) => py_str(h),
                None if has(c, "display_id") => format!("SR-{}", py_str(&c["display_id"])),
                None => format!("SR-{}", py_str(&c["id"])),
            };
            let id = c.get("id").or_else(|| c.get("display_id")).cloned().unwrap_or(Json::Null);
            return json!({"id": id, "display": shown});
        }
    }
    json!({"id": null, "display": "(no id in the response)"})
}

// ---------------------------------------------------------------------------
// item metadata

/// An item form plus the business rules the web loads with it.
#[derive(Clone)]
pub struct ItemMeta {
    pub item: Json,
    pub rules: Json,
    pub workspace_id: Json,
    pub fetched_at: f64,
    pub source: String,
    spec: Arc<FormSpec>,
}

impl ItemMeta {
    pub fn new(item: Json, rules: Json, workspace_id: Json, fetched_at: f64, source: &str) -> Result<ItemMeta> {
        let spec = FormSpec::from_service_item(&item).map_err(form_err)?;
        Ok(ItemMeta { item, rules, workspace_id, fetched_at, source: source.into(), spec: Arc::new(spec) })
    }

    pub fn display_id(&self) -> Json {
        self.item.get("display_id").cloned().unwrap_or(Json::Null)
    }

    pub fn name(&self) -> String {
        or_empty(self.item.get("name"))
    }

    pub fn spec(&self) -> Arc<FormSpec> {
        self.spec.clone()
    }

    pub fn engine(&self) -> Result<RulesEngine> {
        let opts = EngineOptions {
            module: Some("service_request".into()),
            item_id: self.item.get("id").cloned().unwrap_or(Json::Null),
            workspace_id: (!self.workspace_id.is_null()).then(|| self.workspace_id.clone()),
            ..EngineOptions::default()
        };
        RulesEngine::new(self.spec.clone(), Some(&self.rules), opts).map_err(form_err)
    }

    pub fn fingerprint(&self) -> MetadataFingerprint {
        metadata_fingerprint(None, Some(&self.rules), Some(&self.item))
    }

    pub fn as_of(&self) -> Map<String, Json> {
        let fp = self.fingerprint();
        let fetched = jiff::Timestamp::from_second(self.fetched_at.trunc() as i64)
            .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
            .unwrap_or_default();
        obj(json!({"updated_at": fp.updated_at, "fetched_at": fetched, "source": self.source,
                   "fingerprint": fp.hash.chars().take(16).collect::<String>()}))
    }
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// `load_item_meta`: item form + business rules, always from the server.
pub fn load_item_meta(c: &dyn Api, store: &dyn MetaStore, r: &str, item: Option<&Json>) -> Result<ItemMeta> {
    let raw = match item {
        Some(i) => i.clone(),
        None => find_item(c, r)?,
    };
    let configured = store.configured_workspace();
    let ws = if truthy(&configured) { configured } else { raw.get("workspace_id").cloned().unwrap_or(Json::Null) };
    let query = BusinessRulesOptions { item_id: raw.get("id").cloned().unwrap_or(Json::Null), ..Default::default() };
    let rules = load_rules(c, store, "service_request", std::slice::from_ref(&ws), &query)?;
    let ws = rules.get(RULES_WORKSPACE_KEY).cloned().unwrap_or(ws);
    ItemMeta::new(raw, rules, ws, now(), "server")
}

// ---------------------------------------------------------------------------
// answers

/// `_match_choice`: exact (NFC, whitespace, case), else a unique prefix.
pub fn match_choice(f: &Field, raw: &Json) -> (Json, Option<String>) {
    let Some(text) = raw.as_str() else { return (raw.clone(), None) };
    if f.choice(text).ok().flatten().is_some() {
        return (raw.clone(), None);
    }
    let key = norm_s(text);
    let pool: Vec<_> = f.choices.iter().filter(|c| c.visible).collect();
    let exact: Vec<_> = pool.iter().filter(|c| key == norm_s(&c.label) || key == norm_s(&c.value)).collect();
    if exact.len() == 1 {
        return (json!(exact[0].value), None);
    }
    let pre: Vec<_> = pool.iter().filter(|c| !key.is_empty() && (norm_s(&c.label).starts_with(&key) || norm_s(&c.value).starts_with(&key))).collect();
    if pre.len() == 1 {
        return (json!(pre[0].value), Some(format!("{}: {} matched the choice {}", f.label, py_repr_str(text), py_repr_str(&pre[0].label))));
    }
    (raw.clone(), None)
}

/// `lenient_answers`: dropdown/multi-select answers matched leniently.
pub fn lenient_answers(spec: &FormSpec, answers: &Map<String, Json>) -> (Map<String, Json>, Vec<String>) {
    let mut out = Map::new();
    let mut notes = Vec::new();
    for (k, v) in answers {
        let Ok(f) = find_field(spec, k) else {
            out.insert(k.clone(), v.clone());
            continue;
        };
        let v = match (f.kind, v) {
            (Kind::Dropdown, _) => {
                let (v, note) = match_choice(f, v);
                notes.extend(note);
                v
            }
            (Kind::MultiSelect, Json::Array(items)) => Json::Array(
                items
                    .iter()
                    .map(|x| {
                        let (x2, note) = match_choice(f, x);
                        notes.extend(note);
                        x2
                    })
                    .collect(),
            ),
            _ => v.clone(),
        };
        out.insert(k.clone(), v);
    }
    (out, notes)
}

/// `os.path.expanduser` for `~` and `~/...`.
fn expanduser(s: &str) -> String {
    let home = || std::env::var("HOME").unwrap_or_default();
    if s == "~" {
        home()
    } else if let Some(rest) = s.strip_prefix("~/") {
        format!("{}/{rest}", home().trim_end_matches('/'))
    } else {
        s.to_string()
    }
}

/// `file_answers`: file-field answers as paths (`~` expanded, relative to `base`).
pub fn file_answers(spec: &FormSpec, answers: &Map<String, Json>, base: Option<&str>) -> Map<String, Json> {
    let mut out = answers.clone();
    for (k, v) in answers {
        let Ok(f) = find_field(spec, k) else { continue };
        if f.kind != Kind::Attachment || v.is_null() || *v == json!("") || *v == json!([]) {
            continue;
        }
        let items = match v {
            Json::Array(a) => a.clone(),
            other => vec![other.clone()],
        };
        let paths: Vec<Json> = items
            .iter()
            .map(|p| {
                let s = expanduser(&py_str(p));
                match base {
                    Some(b) if !s.starts_with('/') => json!(Path::new(b).join(&s).to_string_lossy()),
                    _ => json!(s),
                }
            })
            .collect();
        out.insert(k.clone(), Json::Array(paths));
    }
    out
}

// ---------------------------------------------------------------------------
// plans

/// `rule_diagnostics`: safe-to-display (warnings, blockers), without rule
/// operands or user data.
pub fn rule_diagnostics(meta: &ItemMeta, state: &FormState) -> (Vec<String>, Vec<String>) {
    let engine = &state.rules;
    let mut warnings = rules_warnings(&meta.rules);
    let mut errors = Vec::new();
    if meta.rules.get(RULES_UNAVAILABLE_KEY).is_some_and(truthy) && meta.source != "static" {
        errors.push("business rules are unavailable; use the web form".to_string());
    }
    let deps = engine.unavailable_dependencies("new");
    let raw = engine.warnings();
    let any = |v: &[String], f: &dyn Fn(&str) -> bool| v.iter().any(|w| f(w));
    if !deps.is_empty() || any(&raw, &|w| w.contains("unknown field") || w.contains("none was given")) {
        errors.push("business rules reference unavailable fields or context; use the web form".into());
    }
    if any(&raw, &|w| w.contains("populate") && (w.contains("ignored") || w.contains("not a choice") || w.contains("unstable"))) {
        errors.push("business rule choice validation is incomplete; use the web form".into());
    }
    if !engine.unsupported.is_empty() {
        errors.push("business rules contain unsupported validation; use the web form".into());
    }
    if any(&deps, &|w| w.contains("unknown condition field")) || any(&raw, &|w| w.contains("condition on unknown field")) {
        warnings.push("A business rule condition references an unknown field.".into());
    }
    if any(&deps, &|w| w.contains("unknown action field")) || any(&raw, &|w| w.contains("unknown field") && !w.contains("condition on unknown field")) {
        warnings.push("A business rule action references an unknown target.".into());
    }
    if any(&deps, &|w| w.contains(" context field")) || any(&raw, &|w| w.contains("none was given")) {
        warnings.push("Business rule source context is missing or incomplete.".into());
    }
    if any(&raw, &|w| !(w.contains("unknown field") || w.contains("none was given"))) {
        warnings.push("A business rule diagnostic was reported; details are withheld to protect context data.".into());
    }
    if meta.spec.warnings.iter().any(|w| w.contains("unknown type") || w.contains("no section choice reveals") || w.contains("no choice reveals")) {
        errors.push("catalog field validation is incomplete; use the web form".into());
    }
    (warnings, errors)
}

/// The draft parts a service request reads (`fsv.drafts.Draft`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub answers: Map<String, Json>,
    pub description_html: Option<String>,
    pub attachments: Vec<String>,
    /// Planning entries (changes only; any entry is an error here).
    pub planning: Map<String, Json>,
    pub ack: Vec<String>,
}

fn json_empty(v: &Json) -> bool {
    match v {
        Json::Null => true,
        Json::String(s) => s.is_empty(),
        Json::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// `unsupported_lookup_errors`: never silently discard an answered lookup.
pub fn unsupported_lookup_errors(state: &FormState, draft: &Draft) -> Vec<String> {
    let mut answered: HashSet<String> = state
        .spec
        .fields()
        .iter()
        .filter(|f| f.kind == Kind::Lookup && !f.sent && (!is_empty(state.values.get(&f.api_name)) || !is_empty(state.evaluated.values.get(&f.api_name))))
        .map(|f| f.api_name.clone())
        .collect();
    for (key, value) in &draft.answers {
        if json_empty(value) {
            continue;
        }
        if let Ok(f) = find_field(&state.spec, key)
            && f.kind == Kind::Lookup
            && !f.sent
        {
            answered.insert(f.api_name.clone());
        }
    }
    state.spec.ordered().into_iter().filter(|f| answered.contains(&f.api_name)).map(|f| format!("{}: catalog lookup submission is unverified; use the web form", f.label)).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    pub placeholder: String,
    pub path: String,
    /// File field api name, or `attachments`.
    pub target: String,
    pub label: String,
}

pub struct SRPlan {
    pub meta: ItemMeta,
    pub state: FormState,
    pub draft: Draft,
    pub payload: Option<Map<String, Json>>,
    pub uploads: Vec<Upload>,
    pub review: Review,
    pub errors: Vec<String>,
    pub requester: Option<LookupValue>,
    pub identity: Map<String, Json>,
    pub hash: String,
    /// Journal / duplicate-check compatible with changes.
    pub action: String,
}

fn is_file(p: &str) -> bool {
    Path::new(p).is_file()
}

/// `os.path.basename`.
fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

impl SRPlan {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn target(&self) -> String {
        format!("catalog-item/{}", py_str(&self.meta.display_id()))
    }

    pub fn files(&self) -> IndexMap<String, Vec<String>> {
        let mut out: IndexMap<String, Vec<String>> = IndexMap::new();
        for u in &self.uploads {
            out.entry(u.target.clone()).or_default().push(u.path.clone());
        }
        out
    }

    fn plan_files(&self) -> PlanFiles {
        PlanFiles::Fields(self.files().into_iter().map(|(k, v)| (k, v.into_iter().map(PathBuf::from).collect())).collect())
    }

    pub fn to_json(&self) -> Json {
        let uploads: Vec<Json> = self
            .uploads
            .iter()
            .filter(|u| is_file(&u.path))
            .filter_map(|u| {
                let mut row = obj(json!({"placeholder": u.placeholder, "target": u.label}));
                row.extend(digest(&u.path).ok()?);
                Some(Json::Object(row))
            })
            .collect();
        let requester = self.requester.as_ref().map(|r| json!({"id": r.id, "name": r.display, "email": r.email}));
        json!({
            "plan_hash": if self.hash.is_empty() { Json::Null } else { json!(self.hash) },
            "action": "place-request",
            "item": {"id": self.meta.item.get("id"), "display_id": self.meta.display_id(), "name": self.meta.name()},
            "endpoint": format!("POST /api/_/catalog/service-items/{}/place-request", py_str(&self.meta.display_id())),
            "requester": requester,
            "recipient_identity": self.identity,
            "payload": if self.ok() { self.payload.clone().map(Json::Object).unwrap_or(Json::Null) } else { Json::Null },
            "uploads": uploads,
            "safety": SAFETY,
            "review": self.review.to_dict(),
            "warnings": self.review.warnings,
            "errors": self.errors,
        })
    }

    fn hashed(&self) -> Json {
        let mut hashed = obj(json!({"request": self.payload, "target": self.target()}));
        if !self.identity.is_empty() {
            hashed.insert("recipient_identity".into(), Json::Object(self.identity.clone()));
        }
        Json::Object(hashed)
    }
}

/// `sha256_file`: `{"name", "size", "sha256", "path"}`.
fn sha256_file(path: &str) -> std::io::Result<fsv_forms::plan::FileDigest> {
    fsv_forms::plan::file_digest(path)
}

/// `_digest`: name, size and sha256 of one file.
pub fn digest(path: &str) -> std::io::Result<Map<String, Json>> {
    let d = sha256_file(path)?;
    Ok(obj(json!({"name": d.name, "size": d.size, "sha256": d.sha256})))
}

/// `requester_line`.
pub fn requester_line(req: &LookupValue) -> String {
    let who = if !req.display.is_empty() {
        req.display.clone()
    } else if let Some(e) = req.email.as_deref().filter(|e| !e.is_empty()) {
        e.to_string()
    } else {
        py_str(&req.id)
    };
    format!("Requester: {who} (approvals route via {who}’s manager)")
}

// change_flow review helpers (`_acknowledge`, `_notes`, `_messages`, `_dropped`)

fn acknowledge(state: &mut FormState, acks: &[String], ack_all: bool) -> Vec<String> {
    let mut warnings = Vec::new();
    if ack_all || acks.iter().any(|a| norm_s(a) == "all") {
        state.acknowledge_all();
        return warnings;
    }
    for a in acks {
        if state.acknowledge(&[fsv_forms::Ack::Text(a.clone())]).is_err() {
            warnings.push(format!("ack {}: no blocking message matches (ignored)", py_repr_str(a)));
        }
    }
    warnings
}

fn real_conflicts(ev: &EvaluatedForm) -> Vec<Conflict> {
    let current: HashSet<String> = ev.matched.iter().map(py_str).collect();
    let mut out = Vec::new();
    for c in &ev.conflicts {
        let live: Vec<_> = c.decisions.iter().filter(|d| current.contains(&py_str(&d.rule_id))).cloned().collect();
        let ids: HashSet<String> = live.iter().map(|d| py_str(&d.rule_id)).collect();
        if ids.len() < 2 {
            continue;
        }
        let positive: HashSet<bool> = live.iter().map(|d| ["show", "mandate", "disable"].contains(&d.action.as_str())).collect();
        if positive.len() < 2 {
            continue;
        }
        let winner = if live.contains(&c.winner) { c.winner.clone() } else { live[live.len() - 1].clone() };
        out.push(Conflict { field: c.field.clone(), property: c.property.clone(), decisions: live, winner });
    }
    out
}

fn rule_of(r: &ChoiceRestriction) -> String {
    if r.rule_name.is_empty() { py_str(&r.rule_id) } else { r.rule_name.clone() }
}

fn label_of(state: &FormState, name: &str) -> String {
    state.spec.get(name).map_or_else(|| name.to_string(), |f| f.label.clone())
}

fn review_notes(state: &FormState) -> Result<Vec<String>> {
    let ev = &state.evaluated;
    let mut out = Vec::new();
    for (name, rid) in &ev.populated {
        if ev.is_visible(name) {
            out.push(format!("{} = {} (filled in by rule {})", label_of(state, name), state.display(name, None).map_err(form_err)?, py_str(rid)));
        }
    }
    for (name, r) in &ev.restricted {
        if ev.is_visible(name) {
            let offered: Vec<String> = state.choices(name).map_err(form_err)?.into_iter().filter(|c| c.visible).map(|c| c.label).collect();
            let shown = if offered.is_empty() { "(none)".to_string() } else { offered.join(", ") };
            out.push(format!("{}: choices limited to {shown} by rule {}", label_of(state, name), py_repr_str(&rule_of(r))));
        }
    }
    let pos = |n: &String| ev.visible.iter().position(|v| v == n).unwrap_or(1 << 20);
    let mut disabled: Vec<&String> = ev.disabled.iter().collect();
    disabled.sort_by_key(|n| pos(n));
    for name in disabled {
        let d = ev.decision(name, "disabled");
        out.push(format!("{}: read-only{}", label_of(state, name), d.map(|d| format!(" ({})", d.describe())).unwrap_or_default()));
    }
    for (name, (old, rid)) in &ev.overridden {
        out.push(format!(
            "{}: your value {} was replaced by rule {}",
            label_of(state, name),
            py_repr_str(&state.display(name, Some(old)).map_err(form_err)?),
            py_str(rid)
        ));
    }
    out.extend(real_conflicts(ev).iter().map(|c| format!("conflict: {}", c.describe())));
    Ok(out)
}

fn review_messages(state: &FormState) -> Vec<ReviewMessage> {
    state
        .evaluated
        .messages
        .iter()
        .map(|m| ReviewMessage {
            text: m.text.clone(),
            blocking: m.blocking,
            acknowledged: m.blocking && state.acknowledged.contains(&m.key()),
            rule: if m.rule_name.is_empty() { m.rule_id.clone() } else { json!(m.rule_name) },
        })
        .collect()
}

fn review_dropped(state: &FormState) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (f, v, d) in state.dropped() {
        let why = match &d {
            Some(d) if d.action == "hide" => format!("hidden by rule {}", py_repr_str(&d.rule_name)),
            _ => "hidden".to_string(),
        };
        out.push(format!("{} = {} ({why})", f.label, state.display(&f.api_name, Some(&v)).map_err(form_err)?));
    }
    Ok(out)
}

/// Who receives the request besides the requester ([`build_sr_plan`]).
#[derive(Clone, Debug, Default)]
pub struct Recipients<'a> {
    pub requested_for: Option<&'a LookupValue>,
    pub cc_emails: &'a [String],
    pub cc_ids: &'a [Json],
}

fn plain_email(e: &str) -> bool {
    valid_recipient(e) && EMAIL.is_match(e)
}

/// `build_sr_plan`: engine state, payload, uploads, review and plan hash.
pub fn build_sr_plan(
    meta: &ItemMeta,
    draft: &Draft,
    resolver: Option<Arc<dyn LookupResolver>>,
    requester: Option<&LookupValue>,
    ack_all: bool,
    who: &Recipients<'_>,
    tenant: &str,
) -> Result<SRPlan> {
    let spec = meta.spec();
    let item = spec.item.clone().ok_or_else(|| Error::other("not a service-item form"))?;
    let mut engine = meta.engine()?;
    if who.requested_for.is_some() {
        // rules needing requested-for attributes fail closed until reloaded for that identity
        engine.context.requested_for = None;
    }
    let mut state = FormState::new(spec.clone(), Some(engine), "new", None, StateOptions { resolver, ..StateOptions::default() }).map_err(form_err)?;
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = spec.warnings.clone();
    warnings.push(REQUESTED_FOR_NOTE.into());
    if draft.description_html.is_some() {
        errors.push("description_md/description_html: catalog items have no description field; answer the item's own fields".into());
    }
    if !draft.planning.is_empty() {
        errors.push("planning: is for changes; service requests have no planning fields".into());
    }
    let (answers, notes) = lenient_answers(&spec, &draft.answers);
    warnings.extend(notes);
    let mut failed = HashSet::new();
    errors.extend(apply_answers(&mut state, &answers, Some(&mut failed))?);
    warnings.extend(acknowledge(&mut state, &draft.ack, ack_all));

    let mut uploads: Vec<Upload> = Vec::new();
    let mut field_ph: IndexMap<String, Vec<Json>> = IndexMap::new();
    let ev = state.evaluated.clone();
    for name in &ev.visible {
        let Some(f) = spec.get(name) else { continue };
        let v = ev.values.get(name);
        if f.kind != Kind::Attachment || is_empty(v) {
            continue;
        }
        let items = match v {
            Some(fsv_forms::Value::List(items)) => items.clone(),
            Some(other) => vec![other.clone()],
            None => vec![],
        };
        for p in items {
            let fsv_forms::Value::Str(p) = p else {
                errors.push(format!("{}: give file paths (attachment ids are assigned after upload)", f.label));
                continue;
            };
            let ph = format!("<upload {}: {}>", uploads.len() + 1, basename(&p));
            uploads.push(Upload { placeholder: ph.clone(), path: p, target: name.clone(), label: f.label.clone() });
            field_ph.entry(name.clone()).or_default().push(json!(ph));
        }
    }
    let mut general_ph: Vec<Json> = Vec::new();
    for p in &draft.attachments {
        let ph = format!("<upload {}: {}>", uploads.len() + 1, basename(p));
        uploads.push(Upload { placeholder: ph.clone(), path: p.clone(), target: "attachments".into(), label: "Other attachments".into() });
        general_ph.push(json!(ph));
    }
    errors.extend(uploads.iter().filter(|u| !is_file(&u.path)).map(|u| format!("file not found: {}", u.path)));

    let skip: HashSet<String> = failed.iter().filter_map(|n| spec.get(n)).map(|f| format!("{} is required", f.label)).collect();
    let mut problems = errors.clone();
    problems.extend(state.errors().map_err(form_err)?.into_iter().filter(|p| !skip.contains(p)));
    let (rule_warnings, rule_errors) = rule_diagnostics(meta, &state);
    warnings.extend(rule_warnings);
    problems.extend(rule_errors);
    problems.extend(unsupported_lookup_errors(&state, draft));
    if ev.attachments_required.unwrap_or(item.attachment_mandatory) && draft.attachments.is_empty() {
        problems.push("this item requires at least one general attachment: add `attachments: [FILE]` to the draft or pass --attach FILE".into());
    }
    if !draft.attachments.is_empty() && !item.allow_attachments {
        problems.push("this item does not accept general attachments".into());
    }
    let req_email = requester.and_then(|r| r.email.clone()).filter(|e| !e.is_empty());
    match requester {
        None => problems.insert(0, REQUESTER_MISSING.into()),
        Some(r) if req_email.is_none() => problems.insert(0, format!("requester {r} has no email address")),
        _ => {}
    }
    if let Some(rf) = who.requested_for
        && !rf.email.as_deref().is_some_and(plain_email)
    {
        problems.push("requested-for must resolve to a user with a plain email address".into());
    }
    let valid_cc = who.cc_emails.iter().all(|v| plain_email(v));
    if !valid_cc {
        problems.push("CC recipients must be plain email addresses".into());
    }

    let mut payload = None;
    if let Some(email) = &req_email {
        let modes: &[bool] = if problems.is_empty() { &[false, true] } else { &[true] };
        for &incomplete in modes {
            let opts = SrOptions {
                field_attachments: field_ph.clone(),
                attachments: if item.allow_attachments { general_ph.clone() } else { vec![] },
                account_id: Json::Null,
                allow_incomplete: incomplete,
            };
            match build_sr_payload(Some(&item), &state, email, &opts) {
                Ok(mut p) => {
                    if let Some(e) = who.requested_for.and_then(|r| r.email.clone()).filter(|e| !e.is_empty()) {
                        p.insert("requested_for".into(), json!(e));
                    }
                    if !who.cc_emails.is_empty() && valid_cc {
                        p.insert("cc_emails".into(), json!(who.cc_emails));
                    }
                    payload = Some(p);
                    break;
                }
                Err(fsv_forms::FormError::Payload { problems: found }) => {
                    for p in found {
                        if !problems.contains(&p) {
                            problems.push(p);
                        }
                    }
                }
                Err(e) if e.is_form_error() => {
                    problems.push(e.to_string());
                    break;
                }
                Err(e) => return Err(form_err(e)),
            }
        }
    }

    let mut review = Review { title: format!("New service request: {}", item.name), ..Review::default() };
    let category = if item.category_name.is_empty() { String::new() } else { format!(" · {}", item.category_name) };
    review.rows.push(("Catalog item".into(), format!("{} (#{}){category}", item.name, py_str(&item.display_id))));
    review.rows.push(("Requester".into(), requester.map_or_else(|| "(not set)".to_string(), ToString::to_string)));
    if let Some(rf) = who.requested_for {
        review.rows.push(("Requested for".into(), format!("{rf} (user ID {})", py_str(&rf.id))));
    }
    if valid_cc && !who.cc_emails.is_empty() {
        let shown = if !who.cc_ids.is_empty() && who.cc_ids.len() == who.cc_emails.len() {
            who.cc_emails.iter().zip(who.cc_ids).map(|(e, i)| format!("{e} (user ID {})", py_str(i))).collect::<Vec<_>>().join(", ")
        } else {
            who.cc_emails.join(", ")
        };
        review.rows.push(("CC".into(), shown));
    }
    let subject = if item.subject_template.is_empty() { String::new() } else { item.render_subject(&ev.values) };
    if !subject.is_empty() {
        review.rows.push(("Subject (item template)".into(), subject));
    }
    for name in state.payload_values().keys() {
        let Some(f) = spec.get(name) else { continue };
        let text = if f.kind == Kind::Attachment {
            match ev.values.get(name) {
                Some(fsv_forms::Value::List(items)) => items.iter().map(|p| basename(&p.py_str()).to_string()).collect::<Vec<_>>().join(", "),
                _ => String::new(),
            }
        } else {
            state.display(name, None).map_err(form_err)? + if ev.populated.contains_key(name) { " (auto)" } else { "" }
        };
        review.rows.push((f.label.clone(), text));
    }
    review.notify = vec![SAFETY.to_string()];
    review.notify.extend(requester.map(requester_line));
    review.messages = review_messages(&state);
    review.dropped = review_dropped(&state)?;
    review.notes = review_notes(&state)?;
    review.attachments = uploads
        .iter()
        .filter(|u| is_file(&u.path))
        .filter_map(|u| {
            let d = sha256_file(&u.path).ok()?;
            Some(ReviewAttachment { target: u.label.clone(), name: d.name, size: d.size, sha256: d.sha256, path: u.path.clone() })
        })
        .collect();
    review.warnings = warnings;
    review.metadata = meta.as_of();
    let mut identity = Map::new();
    if let Some(rf) = who.requested_for {
        identity.insert("requested_for_id".into(), rf.id.clone());
    }
    if !who.cc_ids.is_empty() {
        if who.cc_ids.len() != who.cc_emails.len() {
            problems.push("CC identity count differs from addresses".into());
        }
        identity.insert("cc_ids".into(), json!(who.cc_ids));
    }
    if !identity.is_empty() {
        identity.insert("tenant".into(), json!(tenant));
    }
    review.errors = problems.clone();
    let mut plan = SRPlan {
        meta: meta.clone(),
        state,
        draft: draft.clone(),
        payload,
        uploads,
        review,
        errors: problems,
        requester: requester.cloned(),
        identity,
        hash: String::new(),
        action: "create".into(),
    };
    if plan.payload.is_some() && plan.errors.is_empty() {
        plan.hash = plan_hash(&plan.hashed(), Some(&plan.plan_files()))?;
        plan.review.plan_hash = plan.hash.clone();
    }
    Ok(plan)
}

// ---------------------------------------------------------------------------
// sending

/// `SendFailure`: uploads or the place-request failed; lists every
/// attachment id already on the server.
pub fn send_failure(step: &str, uploaded: &[Json], error: &dyn std::fmt::Display) -> Error {
    let ids: Vec<String> = uploaded.iter().map(|u| format!("{} ({})", py_str(&u["id"]), py_str(&u["name"]))).collect();
    let ids = if ids.is_empty() { "none".to_string() } else { ids.join(", ") };
    Error::other(format!("{step} failed: {error}. Uploaded attachment ids: {ids}"))
}

/// `_substitute`: placeholders -> attachment ids, recursively.
pub fn substitute(value: &Json, ids: &Map<String, Json>) -> Json {
    match value {
        Json::String(s) if ids.contains_key(s) => ids[s].clone(),
        Json::Array(a) => Json::Array(a.iter().map(|v| substitute(v, ids)).collect()),
        Json::Object(m) => Json::Object(m.iter().map(|(k, v)| (k.clone(), substitute(v, ids))).collect()),
        other => other.clone(),
    }
}

/// `execute`: upload the files, then place the request (journaled under
/// the plan hash). Call only after the review was confirmed.
pub fn execute(plan: &SRPlan, c: &dyn Api, journal: &dyn Journal, upload_fn: &dyn Fn(&dyn Api, &str) -> Result<i64>, allow_duplicate: bool) -> Result<Json> {
    let Some(payload) = plan.payload.as_ref().filter(|_| plan.ok() && !plan.hash.is_empty()) else {
        return Err(flow_exit(2, format!("plan has problems: {}", plan.errors.join("; "))));
    };
    if !plan.identity.is_empty() && plan.identity.get("tenant") != Some(&json!(c.domain())) {
        return Err(flow_exit(2, "tenant changed since review; nothing was sent"));
    }
    duplicate_check(plan, allow_duplicate, journal)?;
    let (_, mut preflight) = rule_diagnostics(&plan.meta, &plan.state);
    preflight.extend(unsupported_lookup_errors(&plan.state, &plan.draft));
    preflight.extend(plan.state.errors().map_err(form_err)?);
    match &plan.meta.spec.item {
        None => preflight.push("not a service-item form".into()),
        Some(item) => {
            if plan.state.evaluated.attachments_required.unwrap_or(item.attachment_mandatory) && plan.draft.attachments.is_empty() {
                preflight.push("this item requires at least one general attachment".into());
            }
            if !plan.draft.attachments.is_empty() && !item.allow_attachments {
                preflight.push("this item does not accept general attachments".into());
            }
        }
    }
    if !preflight.is_empty() {
        return Err(flow_exit(2, format!("plan has problems: {}", preflight.join("; "))));
    }
    let current = plan_hash(&plan.hashed(), Some(&plan.plan_files())).map_err(|e| flow_exit(2, format!("cannot verify reviewed files: {e}")))?;
    if current != plan.hash {
        return Err(flow_exit(2, "the reviewed payload or files changed; review again"));
    }
    let target = json!(plan.target());
    let before = journal.read().len();
    journal.record_plan("started", &plan.hash, &plan.action, Some(&target), None);
    let entries = journal.read();
    let last = entries.last();
    let field = |k: &str| last.and_then(|e| e.get(k)).and_then(Json::as_str).map(str::to_string);
    if entries.len() <= before
        || field("kind").as_deref() != Some(format!("plan:{}", plan.action).as_str())
        || field("plan").as_deref() != Some(plan.hash.as_str())
        || field("event").as_deref() != Some("started")
    {
        return Err(flow_exit(5, "could not persist the request journal; nothing was uploaded or placed"));
    }
    let mut uploaded: Vec<Json> = Vec::new();
    let mut ids = Map::new();
    for u in &plan.uploads {
        match upload_fn(c, &u.path) {
            Ok(att_id) => {
                ids.insert(u.placeholder.clone(), json!(att_id));
                uploaded.push(json!({"id": att_id, "name": basename(&u.path), "target": u.label}));
            }
            Err(e) => {
                journal.record_plan_steps("failed", &plan.hash, &plan.action, Some(&target), None, &[], true);
                return Err(send_failure("upload", &uploaded, &e));
            }
        }
    }
    let body = substitute(&Json::Object(payload.clone()), &ids);
    let result = match place_request(c, &plan.meta.display_id(), &body) {
        Ok(r) => r,
        Err(e) => {
            if definite_rejection(&e).is_some() {
                journal.record_plan_steps("failed", &plan.hash, &plan.action, Some(&target), None, &[], true);
            }
            return Err(send_failure("place-request", &uploaded, &e));
        }
    };
    let created = created_ref(&result);
    if created["id"].is_null() {
        return Err(send_failure("place-request", &uploaded, &"response has no exact request ID; outcome unknown"));
    }
    let steps = json!([
        {"step": "upload", "ok": true, "ids": uploaded.iter().map(|u| u["id"].clone()).collect::<Vec<_>>()},
        {"step": "place-request", "ok": true, "id": created["id"]},
    ]);
    let jsteps = [PlanStep { step: "upload".into(), ok: true }, PlanStep { step: "place-request".into(), ok: true }];
    journal.record_plan_steps("done", &plan.hash, &plan.action, Some(&target), Some(&created["id"]), &jsteps, false);
    let mut verification = json!({"verified": false, "fields": {}});
    if !plan.identity.is_empty() {
        let readback = || -> Result<Json> {
            let raw = c.int_get(&format!("tickets/{}", py_str(&created["id"])), None)?;
            let row = if raw.is_object() { raw.get("ticket").cloned().unwrap_or_else(|| raw.clone()) } else { Json::Null };
            if !row.is_object() || py_str(row.get("id").unwrap_or(&Json::Null)) != py_str(&created["id"]) {
                return Err(Error::value("identity mismatch"));
            }
            let mut fields = Map::new();
            for key in ["email", "requested_for", "cc_emails"] {
                if let Some(want) = body.get(key) {
                    let state = match row.get(key) {
                        None => "unavailable",
                        Some(got) if got == want => "matched",
                        Some(_) => "mismatched",
                    };
                    fields.insert(key.into(), json!(state));
                }
            }
            let verified = !fields.is_empty() && fields.values().all(|v| v == "matched");
            Ok(json!({"verified": verified, "fields": fields}))
        };
        if let Ok(v) = readback() {
            verification = v;
        }
        if verification["verified"] != Json::Bool(true) {
            verification["warning"] = json!("Request recipient fields were not fully verified; do not resend.");
        }
    }
    Ok(json!({"created": created, "result": result, "uploaded": uploaded, "body": body, "plan_hash": plan.hash,
              "steps": steps, "verification": verification}))
}

/// `review_diff`: what changed between two plans (rows, payload keys).
fn review_diff(old: &SRPlan, new: &SRPlan) -> Vec<String> {
    let to_map = |rows: &[(String, String)]| -> IndexMap<String, String> { rows.iter().cloned().collect() };
    let (a, b) = (to_map(&old.review.rows), to_map(&new.review.rows));
    let mut keys: Vec<&String> = a.keys().collect();
    keys.extend(b.keys().filter(|k| !a.contains_key(*k)));
    let dash = "—".to_string();
    let mut out: Vec<String> = keys.into_iter().filter(|k| a.get(*k) != b.get(*k)).map(|k| format!("{k}: {} → {}", a.get(k).unwrap_or(&dash), b.get(k).unwrap_or(&dash))).collect();
    if out.is_empty() && old.payload != new.payload {
        let empty = Map::new();
        let (op, np) = (old.payload.as_ref().unwrap_or(&empty), new.payload.as_ref().unwrap_or(&empty));
        let mut keys: Vec<&String> = op.keys().chain(np.keys()).collect();
        keys.sort();
        keys.dedup();
        out.extend(keys.into_iter().filter(|k| op.get(*k) != np.get(*k)).map(|k| format!("payload {k} changed")));
    }
    if old.errors != new.errors {
        out.extend(new.errors.iter().filter(|e| !old.errors.contains(e)).map(|e| format!("problem: {e}")));
    }
    out
}

/// `freshen`: re-fetch the item and its rules right before sending.
/// -> (plan to send, diff lines when the plan changed).
pub fn freshen(plan: SRPlan, c: &dyn Api, store: &dyn MetaStore, rebuild: &mut dyn FnMut(ItemMeta) -> Result<SRPlan>) -> Result<(SRPlan, Option<Vec<String>>)> {
    let fresh = load_item_meta(c, store, &py_str(&plan.meta.display_id()), None)?;
    if !fresh.fingerprint().changed(&plan.meta.fingerprint()) {
        return Ok((plan, None));
    }
    let new = rebuild(fresh)?;
    if !new.hash.is_empty() && new.hash == plan.hash {
        return Ok((new, None));
    }
    let mut diff = review_diff(&plan, &new);
    if diff.is_empty() {
        diff.push("catalog item or rules changed; plan hash differs".into());
    }
    Ok((new, Some(diff)))
}

/// `duplicate_check`: an unknown earlier outcome, then the 24-hour
/// duplicate window (`change_flow.check_duplicate`).
pub fn duplicate_check(plan: &SRPlan, allow_duplicate: bool, journal: &dyn Journal) -> Result<()> {
    if crate::service::last_event(journal, "create", &plan.hash).as_deref() == Some("started") {
        return Err(flow_exit(4, "previous request outcome is unknown; inspect the server and journal before retry"));
    }
    if plan.action != "create" || allow_duplicate {
        return Ok(());
    }
    let Some(hit) = journal.recent_plan(&plan.hash, "create", DUPLICATE_WINDOW_HOURS) else { return Ok(()) };
    let ts = py_str(hit.get("ts").unwrap_or(&Json::Null));
    let what = if hit.get("event").and_then(Json::as_str) == Some("done") {
        format!("was already sent at {ts} (created {})", py_str(hit.get("created_id").unwrap_or(&Json::Null)))
    } else {
        format!("was started at {ts} with no recorded outcome (it may have been created)")
    };
    Err(flow_exit(4, format!("this exact plan {what}. Check `fsv journal tail`; pass --allow-duplicate to send it again.")))
}

// ---------------------------------------------------------------------------
// requested items (read side)

/// `label_map`: name -> label over item field definitions (`fields`,
/// `nested_fields`, `sections[].fields`).
pub fn label_map(defs: &[Json]) -> IndexMap<String, String> {
    fn walk(rows: &[Json], out: &mut IndexMap<String, String>) {
        for r in rows {
            let Some(m) = r.as_object() else { continue };
            let (name, label) = (m.get("name").filter(|v| truthy(v)), m.get("label").filter(|v| truthy(v)));
            if let (Some(name), Some(label)) = (name, label)
                && py_str(m.get("type").unwrap_or(&Json::Null)) != "1024"
                && !is_content_type(&or_empty(m.get("field_type")))
            {
                out.entry(py_str(name)).or_insert_with(|| fsv_forms::pytext::html_unescape(&py_str(label)));
            }
            let list = |k: &str| m.get(k).filter(|v| truthy(v)).and_then(Json::as_array).cloned().unwrap_or_default();
            walk(&list("fields"), out);
            walk(&list("nested_fields"), out);
            for s in list("sections") {
                if let Some(fields) = s.get("fields").filter(|v| truthy(v)).and_then(Json::as_array) {
                    walk(fields, out);
                }
            }
        }
    }
    let mut out = IndexMap::new();
    walk(defs, &mut out);
    out
}

/// `item_labels`: labels for a requested item's values, from its embedded
/// field definitions, else the catalog item form.
pub fn item_labels(c: &dyn Api, requested: &Json) -> Result<IndexMap<String, String>> {
    let meta = requested.get("item").filter(|v| truthy(v)).cloned().unwrap_or_else(|| json!({}));
    let mut labels = label_map(meta.get("custom_fields").filter(|v| truthy(v)).and_then(Json::as_array).map_or(&[][..], Vec::as_slice));
    let values = requested.obj_or_empty_map("custom_fields");
    let did = meta.get("display_id").filter(|v| truthy(v)).or_else(|| requested.get("service_item_display_id")).cloned().unwrap_or(Json::Null);
    if !did.is_null() && values.keys().any(|k| !labels.contains_key(k)) {
        let spec = match get_item(c, &did) {
            Ok(item) => match FormSpec::from_service_item(&item) {
                Ok(s) => Some(s),
                Err(e) if e.is_value_error() => None,
                Err(e) => return Err(form_err(e)),
            },
            Err(Error::Api(_) | Error::Value(_)) => None,
            Err(e) => return Err(e),
        };
        for f in spec.iter().flat_map(|s| s.fields()) {
            if f.kind != Kind::Notice {
                labels.entry(f.api_name.clone()).or_insert_with(|| f.label.clone());
            }
        }
    }
    Ok(labels)
}

trait ObjMap {
    fn obj_or_empty_map(&self, key: &str) -> Map<String, Json>;
}

impl ObjMap for Json {
    fn obj_or_empty_map(&self, key: &str) -> Map<String, Json> {
        self.get(key).filter(|v| truthy(v)).and_then(Json::as_object).cloned().unwrap_or_default()
    }
}

/// `value_text`: one requested-item value as display text.
pub fn value_text(value: &Json) -> String {
    match value {
        Json::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Json::Object(m) => ["name", "value", "label", "email"].iter().find_map(|k| m.get(*k).filter(|v| truthy(v)).map(py_str)).unwrap_or_else(|| py_str(value)),
        Json::Bool(b) => (if *b { "Yes" } else { "No" }).into(),
        Json::String(s) if s.contains('<') => fsv_core::pystr::collapse_ws(&fsv_forms::spec::plain_text(s)),
        other => py_str(other),
    }
}

/// `is_file_value`: a non-empty list of attachment ids or attachment rows.
pub fn is_file_value(value: &Json) -> bool {
    let Json::Array(a) = value else { return false };
    !a.is_empty()
        && a.iter().all(|v| match v {
            Json::Bool(_) => true,
            Json::Number(n) => n.is_i64() || n.is_u64(),
            Json::Object(m) => {
                let t = |k: &str| m.get(k).is_some_and(truthy);
                t("attachment_url") || t("canonical_url") || (t("name") && m.get("id").is_some_and(|v| !v.is_null()))
            }
            _ => false,
        })
}

/// `safe_target`: where to write a server-supplied file name inside
/// `directory`; rejects anything that could escape it.
pub fn safe_target(directory: &str, name: &str) -> Result<PathBuf> {
    let r = py_repr_str(name);
    if fsv_core::pystr::strip(name).is_empty() {
        return Err(Error::value("empty file name"));
    }
    if name.contains("..") {
        return Err(Error::value(format!("unsafe file name {r}: contains '..'")));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(Error::value(format!("unsafe file name {r}: contains a path separator")));
    }
    let b = name.as_bytes();
    if name.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
        return Err(Error::value(format!("unsafe file name {r}: absolute path")));
    }
    if crate::service::has_control(name, "") {
        return Err(Error::value(format!("unsafe file name {r}: control characters")));
    }
    let dir = PathBuf::from(expanduser(directory));
    let root = std::fs::canonicalize(&dir).unwrap_or(dir);
    let joined = root.join(name);
    let target = std::fs::canonicalize(&joined).unwrap_or(joined);
    if target.parent() != Some(root.as_path()) {
        return Err(Error::value(format!("unsafe file name {r}: resolves outside {}", root.display())));
    }
    Ok(target)
}

/// `domain_url`.
pub fn domain_url(path: &str, domain: &str) -> String {
    format!("https://{domain}{path}")
}
