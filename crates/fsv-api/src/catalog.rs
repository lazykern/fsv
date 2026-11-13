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
use fsv_forms::{
    Conflict, EvaluatedForm, Field, FormSpec, FormState, Kind, LookupResolver, LookupValue,
    MetadataFingerprint, RulesEngine, StateOptions, metadata_fingerprint, plan_hash,
};
use fsv_py::{py_repr, py_repr_str, py_str, truthy};

use crate::formmeta::{
    MetaStore, RULES_UNAVAILABLE_KEY, RULES_WORKSPACE_KEY, apply_answers, find_field, form_err,
    load_rules, rules_warnings,
};
use crate::recipients::plain_email;
use crate::service::{Api, Journal, definite_rejection, obj};

pub const PER_PAGE: i64 = 30;
pub const MAX_PAGES: i64 = 50;
pub const SAFETY: &str = "This sends a real service request to the catalog item's fulfilment team.";
pub const REQUESTER_MISSING: &str = "requester is not set: pass --requester EMAIL, add `requester:` to the draft, \
                                     or set defaults.sr_requester in the fsv config";
pub const REQUESTED_FOR_NOTE: &str =
    "Separate requested-for is available via --requested-for; requester remains unchanged.";
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
    list.map(|a| a.iter().filter(|r| r.is_object()).cloned().collect())
        .unwrap_or_default()
}

/// `list_categories`: flattened; nested sub-categories get `parent_id`.
pub fn list_categories(c: &dyn Api) -> Result<Vec<Json>> {
    let data = c.int_get(
        "catalog/service-categories",
        Some(obj(json!({"category_hierarchy": "true"}))),
    )?;
    fn walk(rows: &[Json], parent: &Json, out: &mut Vec<Json>) {
        for r in rows {
            let Some(m) = r.as_object() else { continue };
            let mut row: Map<String, Json> = m
                .iter()
                .filter(|(k, _)| *k != "sub_categories" && *k != "children")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !parent.is_null() && row.get("parent_id").is_none_or(Json::is_null) {
                row.insert("parent_id".into(), parent.clone());
            }
            out.push(Json::Object(row));
            let kids = [m.get("sub_categories"), m.get("children")]
                .into_iter()
                .flatten()
                .find(|v| truthy(v));
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
    list_items_info(c, popular, max_pages, None)
}

/// `list_items(..., info=...)`: reading stops after `max_pages` pages; then
/// `info` gets `truncated: true` with the pages read and per-page size (and
/// `total_pages` when the server said), so callers can say the list is
/// incomplete instead of silently cutting it.
pub fn list_items_info(
    c: &dyn Api,
    popular: bool,
    max_pages: i64,
    mut info: Option<&mut Map<String, Json>>,
) -> Result<Vec<Json>> {
    if let Some(info) = info.as_deref_mut() {
        info.insert("truncated".into(), json!(false));
        info.insert("pages_read".into(), json!(0));
        info.insert("per_page".into(), json!(PER_PAGE));
        info.insert("max_pages".into(), json!(max_pages));
    }
    if popular {
        return Ok(rows(
            &c.int_get(
                "catalog/service-items",
                Some(obj(json!({"filter": "frequently_requested"}))),
            )?,
            "service_items",
        ));
    }
    let mut out = Vec::new();
    let mut seen: Vec<Json> = Vec::new();
    let mut complete = false;
    let mut total = Json::Null;
    for page in 1..=max_pages {
        let data = c.int_get(
            "catalog/service-items",
            Some(obj(
                json!({"aurora_search": "true", "page": page, "per_page": PER_PAGE}),
            )),
        )?;
        let rs = rows(&data, "service_items");
        for r in &rs {
            let key = r
                .get("id")
                .or_else(|| r.get("display_id"))
                .cloned()
                .unwrap_or(Json::Null);
            if !seen.contains(&key) && !r.get("deleted").is_some_and(truthy) {
                seen.push(key);
                out.push(r.clone());
            }
        }
        total = data
            .get("meta")
            .filter(|m| truthy(m))
            .and_then(|m| m.get("total_pages"))
            .cloned()
            .unwrap_or(Json::Null);
        if let Some(info) = info.as_deref_mut() {
            info.insert("pages_read".into(), json!(page));
        }
        if rs.is_empty()
            || total.as_i64().is_some_and(|t| page >= t)
            || (total.is_null() && (rs.len() as i64) < PER_PAGE)
        {
            complete = true;
            break;
        }
    }
    if !complete && let Some(info) = info {
        info.insert("truncated".into(), json!(true));
        if total.is_i64() {
            info.insert("total_pages".into(), total);
        }
    }
    Ok(out)
}

/// `truncation_notice`: one line when [`list_items_info`] stopped before
/// the last page.
pub fn truncation_notice(info: &Map<String, Json>) -> Option<String> {
    if !info.get("truncated").is_some_and(truthy) {
        return None;
    }
    let field = |k: &str| py_str(info.get(k).unwrap_or(&Json::Null));
    let more = match info.get("total_pages") {
        Some(t) if t.is_i64() => format!(" of {}", py_str(t)),
        _ => String::new(),
    };
    Some(format!(
        "catalog list truncated: read {}{more} pages of {} items (limit {} pages); more items exist \
         and are not shown or searched",
        field("pages_read"),
        field("per_page"),
        field("max_pages")
    ))
}

fn name_of(c: &Json) -> String {
    c.get("name").map(py_str).unwrap_or_default()
}

/// `resolve_category`: a category by id or name plus its sub-categories.
pub fn resolve_category(categories: &[Json], text: &str) -> Result<Vec<Json>> {
    let q = fsv_core::pystr::strip(text);
    let mut hits: Vec<&Json> = categories
        .iter()
        .filter(|c| py_str(c.get("id").unwrap_or(&Json::Null)) == q)
        .collect();
    if hits.is_empty() {
        hits = categories
            .iter()
            .filter(|c| norm_s(&name_of(c)) == norm_s(q))
            .collect();
    }
    if hits.is_empty() {
        hits = categories
            .iter()
            .filter(|c| norm_s(&name_of(c)).contains(&norm_s(q)))
            .collect();
    }
    if hits.len() != 1 {
        let pool: Vec<&Json> = if hits.is_empty() {
            categories.iter().collect()
        } else {
            hits.clone()
        };
        let names: Vec<String> = pool
            .iter()
            .take(15)
            .map(|c| py_str(c.get("name").unwrap_or(&Json::Null)))
            .collect();
        let what = if hits.is_empty() {
            "no category matches"
        } else {
            "several categories match"
        };
        return Err(Error::value(format!(
            "{what} {}; categories: {}",
            py_repr_str(q),
            names.join(", ")
        )));
    }
    let mut picked = vec![hits[0].clone()];
    let mut ids = vec![hits[0].get("id").cloned().unwrap_or(Json::Null)];
    let mut grew = true;
    while grew {
        grew = false;
        for c in categories {
            let (pid, id) = (
                c.get("parent_id").cloned().unwrap_or(Json::Null),
                c.get("id").cloned().unwrap_or(Json::Null),
            );
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
pub fn filter_items(
    items: &[Json],
    category_ids: Option<&[Json]>,
    search: Option<&str>,
    categories: &[Json],
) -> Vec<Json> {
    let names: Vec<(Json, String)> = categories
        .iter()
        .map(|c| {
            (
                c.get("id").cloned().unwrap_or(Json::Null),
                or_empty(c.get("name")),
            )
        })
        .collect();
    let ids: Option<HashSet<String>> = category_ids.map(|ids| ids.iter().map(py_str).collect());
    let words: Vec<String> = search
        .filter(|s| !s.is_empty())
        .map(|s| {
            norm_s(s)
                .split(' ')
                .filter(|w| !w.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for it in items {
        if let Some(ids) = &ids
            && !ids.contains(&py_str(it.get("category_id").unwrap_or(&Json::Null)))
        {
            continue;
        }
        let cat = it
            .get("category_name")
            .filter(|v| truthy(v))
            .map(py_str)
            .unwrap_or_else(|| {
                let cid = it.get("category_id").unwrap_or(&Json::Null);
                names
                    .iter()
                    .find(|(k, _)| k == cid)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_default()
            });
        let parts = [
            or_empty(it.get("name")),
            or_empty(it.get("short_description")),
            cat,
            or_empty(it.get("display_id")),
        ];
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
    let item = data.as_object().map(|m| {
        m.get("service_item")
            .cloned()
            .unwrap_or_else(|| data.clone())
    });
    match item {
        Some(i @ Json::Object(_))
            if i.get("custom_fields").is_some_and(truthy)
                || i.get("display_id").is_some_and(|v| !v.is_null()) =>
        {
            Ok(i)
        }
        _ => Err(ApiError::with_request(
            200,
            Json::String(format!(
                "no catalog item {} in the response",
                py_str(display_id)
            )),
            "GET",
            path,
        )
        .into()),
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
    if !s.is_empty()
        && s.chars()
            .all(|c| fsv_core::pystr::decimal_value(c).is_some())
    {
        let id = fsv_core::pystr::parse_decimal(&s).unwrap_or(0);
        return get_item(c, &json!(id));
    }
    let mut info = Map::new();
    let items = list_items_info(c, false, MAX_PAGES, Some(&mut info))?;
    let exact: Vec<Json> = items
        .iter()
        .filter(|i| norm_s(&name_of(i)) == norm_s(&s))
        .cloned()
        .collect();
    let hits = if exact.is_empty() {
        filter_items(&items, None, Some(&s), &[])
    } else {
        exact
    };
    match hits.len() {
        1 => get_item(c, hits[0].get("display_id").unwrap_or(&Json::Null)),
        0 => Err(Error::value(format!(
            "no catalog item matches {}; list them with `fsv sr catalog --search TEXT`{}",
            py_repr_str(&s),
            truncation_notice(&info)
                .map(|cut| format!(" ({cut}; use the item's display id)"))
                .unwrap_or_default()
        ))),
        n => {
            let shown: Vec<String> = hits
                .iter()
                .take(10)
                .map(|i| {
                    format!(
                        "#{} {}",
                        i.get("display_id").map_or("None".into(), py_str),
                        i.get("name").map_or("None".into(), py_str)
                    )
                })
                .collect();
            Err(Error::value(format!(
                "{n} catalog items match {}: {}. Use the item id.",
                py_repr_str(&s),
                shown.join("; ")
            )))
        }
    }
}

pub fn place_request(c: &dyn Api, display_id: &Json, body: &Json) -> Result<Json> {
    c.int_post(
        &format!("catalog/service-items/{}/place-request", py_str(display_id)),
        body,
    )
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
            let id = c
                .get("id")
                .or_else(|| c.get("display_id"))
                .cloned()
                .unwrap_or(Json::Null);
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
    pub fn new(
        item: Json,
        rules: Json,
        workspace_id: Json,
        fetched_at: f64,
        source: &str,
    ) -> Result<ItemMeta> {
        let spec = FormSpec::from_service_item(&item).map_err(form_err)?;
        Ok(ItemMeta {
            item,
            rules,
            workspace_id,
            fetched_at,
            source: source.into(),
            spec: Arc::new(spec),
        })
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

    /// `raw_fields`: the item's fields flattened in web order; children name
    /// their parent (the `FormMeta` surface `fsv form` reads).
    pub fn raw_fields(&self) -> Vec<Json> {
        fn walk(rows: Option<&Json>, parent: Option<&Json>, out: &mut Vec<Json>) {
            for row in rows.and_then(Json::as_array).into_iter().flatten() {
                let Some(m) = row.as_object() else { continue };
                if m.get("deleted").is_some_and(truthy) {
                    continue;
                }
                let mut flat: Map<String, Json> = m
                    .iter()
                    .filter(|(k, _)| *k != "fields")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if let Some(p) = parent.filter(|p| !p.is_null()) {
                    flat.insert("parent".into(), p.clone());
                }
                out.push(Json::Object(flat));
                walk(m.get("fields"), m.get("name"), out);
            }
        }
        let si = self.item.get("service_item").unwrap_or(&self.item);
        let mut out = Vec::new();
        walk(si.get("custom_fields"), None, &mut out);
        out
    }

    /// `FormState(meta.spec, meta.engine(), mode=..., resolver=..., existing=...)`.
    pub fn state(
        &self,
        mode: &str,
        resolver: Option<Arc<dyn fsv_forms::LookupResolver>>,
        existing: Option<fsv_forms::Values>,
    ) -> Result<FormState> {
        let opts = fsv_forms::StateOptions {
            resolver,
            existing,
            ..fsv_forms::StateOptions::default()
        };
        FormState::new(self.spec.clone(), Some(self.engine()?), mode, None, opts).map_err(form_err)
    }

    pub fn as_of(&self) -> Map<String, Json> {
        let fp = self.fingerprint();
        let fetched = jiff::Timestamp::from_second(self.fetched_at.trunc() as i64)
            .map(|t| {
                t.to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string()
            })
            .unwrap_or_default();
        obj(
            json!({"updated_at": fp.updated_at, "fetched_at": fetched, "source": self.source,
                   "fingerprint": fp.hash.chars().take(16).collect::<String>()}),
        )
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `load_item_meta`: item form + business rules, always from the server.
pub fn load_item_meta(
    c: &dyn Api,
    store: &dyn MetaStore,
    r: &str,
    item: Option<&Json>,
) -> Result<ItemMeta> {
    let raw = match item {
        Some(i) => i.clone(),
        None => find_item(c, r)?,
    };
    let configured = store.configured_workspace();
    let ws = if truthy(&configured) {
        configured
    } else {
        raw.get("workspace_id").cloned().unwrap_or(Json::Null)
    };
    let query = BusinessRulesOptions {
        item_id: raw.get("id").cloned().unwrap_or(Json::Null),
        ..Default::default()
    };
    let rules = load_rules(
        c,
        store,
        "service_request",
        std::slice::from_ref(&ws),
        &query,
    )?;
    let ws = rules.get(RULES_WORKSPACE_KEY).cloned().unwrap_or(ws);
    ItemMeta::new(raw, rules, ws, now(), "server")
}

// ---------------------------------------------------------------------------
// answers

/// `_match_choice`: exact (NFC, whitespace, case), else a unique prefix.
pub fn match_choice(f: &Field, raw: &Json) -> (Json, Option<String>) {
    let Some(text) = raw.as_str() else {
        return (raw.clone(), None);
    };
    if f.choice(text).ok().flatten().is_some() {
        return (raw.clone(), None);
    }
    let key = norm_s(text);
    let pool: Vec<_> = f.choices.iter().filter(|c| c.visible).collect();
    let exact: Vec<_> = pool
        .iter()
        .filter(|c| key == norm_s(&c.label) || key == norm_s(&c.value))
        .collect();
    if exact.len() == 1 {
        return (json!(exact[0].value), None);
    }
    let pre: Vec<_> = pool
        .iter()
        .filter(|c| {
            !key.is_empty()
                && (norm_s(&c.label).starts_with(&key) || norm_s(&c.value).starts_with(&key))
        })
        .collect();
    if pre.len() == 1 {
        return (
            json!(pre[0].value),
            Some(format!(
                "{}: {} matched the choice {}",
                f.label,
                py_repr_str(text),
                py_repr_str(&pre[0].label)
            )),
        );
    }
    (raw.clone(), None)
}

/// `lenient_answers`: dropdown/multi-select answers matched leniently.
pub fn lenient_answers(
    spec: &FormSpec,
    answers: &Map<String, Json>,
) -> (Map<String, Json>, Vec<String>) {
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

/// `os.path.expanduser` (POSIX). `~` and `~/...` use `$HOME`, or the current
/// uid's directory when `HOME` is unset. `~user` and `~user/...` use that
/// account's home. An unknown name is left unchanged.
fn expanduser(s: &str) -> String {
    if !s.starts_with('~') {
        return s.to_string();
    }
    let i = s[1..].find('/').map(|n| n + 1).unwrap_or(s.len());
    let userhome = if i == 1 {
        match std::env::var("HOME") {
            Ok(h) => h,
            Err(_) => match pw_dir_uid() {
                Some(h) => h,
                None => return s.to_string(),
            },
        }
    } else {
        match pw_dir_name(&s[1..i]) {
            Some(h) => h,
            None => return s.to_string(),
        }
    };
    let joined = format!("{}{}", userhome.trim_end_matches('/'), &s[i..]);
    if joined.is_empty() {
        "/".to_string()
    } else {
        joined
    }
}

fn pw_dir_name(name: &str) -> Option<String> {
    let c = std::ffi::CString::new(name).ok()?;
    // SAFETY: `name` is a NUL-terminated C string. The returned `passwd` is
    // copied before this returns.
    unsafe { pw_dir(libc::getpwnam(c.as_ptr())) }
}

fn pw_dir_uid() -> Option<String> {
    // SAFETY: the returned `passwd` is copied before this returns.
    unsafe { pw_dir(libc::getpwuid(libc::getuid())) }
}

/// Copy `pw_dir` from a `getpwnam` / `getpwuid` result. Null means no entry.
///
/// # Safety
/// `pwd` is null or a valid `passwd` from `getpwnam` / `getpwuid`.
unsafe fn pw_dir(pwd: *mut libc::passwd) -> Option<String> {
    if pwd.is_null() {
        return None;
    }
    let dir = unsafe { (*pwd).pw_dir };
    if dir.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(dir) };
    s.to_str().ok().map(str::to_string)
}

/// `file_answers`: file-field answers as paths (`~` expanded, relative to `base`).
pub fn file_answers(
    spec: &FormSpec,
    answers: &Map<String, Json>,
    base: Option<&str>,
) -> Map<String, Json> {
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
                    Some(b) if !s.starts_with('/') => {
                        json!(Path::new(b).join(&s).to_string_lossy())
                    }
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
    if !deps.is_empty()
        || any(&raw, &|w| {
            w.contains("unknown field") || w.contains("none was given")
        })
    {
        errors.push(
            "business rules reference unavailable fields or context; use the web form".into(),
        );
    }
    if any(&raw, &|w| {
        w.contains("populate")
            && (w.contains("ignored") || w.contains("not a choice") || w.contains("unstable"))
    }) {
        errors.push("business rule choice validation is incomplete; use the web form".into());
    }
    if !engine.unsupported.is_empty() {
        errors.push("business rules contain unsupported validation; use the web form".into());
    }
    if any(&deps, &|w| w.contains("unknown condition field"))
        || any(&raw, &|w| w.contains("condition on unknown field"))
    {
        warnings.push("A business rule condition references an unknown field.".into());
    }
    if any(&deps, &|w| w.contains("unknown action field"))
        || any(&raw, &|w| {
            w.contains("unknown field") && !w.contains("condition on unknown field")
        })
    {
        warnings.push("A business rule action references an unknown target.".into());
    }
    if any(&deps, &|w| w.contains(" context field")) || any(&raw, &|w| w.contains("none was given"))
    {
        warnings.push("Business rule source context is missing or incomplete.".into());
    }
    if any(&raw, &|w| {
        !(w.contains("unknown field") || w.contains("none was given"))
    }) {
        warnings.push("A business rule diagnostic was reported; details are withheld to protect context data.".into());
    }
    if meta.spec.warnings.iter().any(|w| {
        w.contains("unknown type")
            || w.contains("no section choice reveals")
            || w.contains("no choice reveals")
    }) {
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
        .filter(|f| {
            f.kind == Kind::Lookup
                && !f.sent
                && (!is_empty(state.values.get(&f.api_name))
                    || !is_empty(state.evaluated.values.get(&f.api_name)))
        })
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
    state
        .spec
        .ordered()
        .into_iter()
        .filter(|f| answered.contains(&f.api_name))
        .map(|f| {
            format!(
                "{}: catalog lookup submission is unverified; use the web form",
                f.label
            )
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    pub placeholder: String,
    pub path: String,
    /// File field api name, or `attachments`.
    pub target: String,
    pub label: String,
}

#[derive(Clone)]
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
            out.entry(u.target.clone())
                .or_default()
                .push(u.path.clone());
        }
        out
    }

    fn plan_files(&self) -> PlanFiles {
        PlanFiles::Fields(
            self.files()
                .into_iter()
                .map(|(k, v)| (k, v.into_iter().map(PathBuf::from).collect()))
                .collect(),
        )
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
        let requester = self
            .requester
            .as_ref()
            .map(|r| json!({"id": r.id, "name": r.display, "email": r.email}));
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
            hashed.insert(
                "recipient_identity".into(),
                Json::Object(self.identity.clone()),
            );
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
    Ok(obj(
        json!({"name": d.name, "size": d.size, "sha256": d.sha256}),
    ))
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
        if state
            .acknowledge(&[fsv_forms::Ack::Text(a.clone())])
            .is_err()
        {
            warnings.push(format!(
                "ack {}: no blocking message matches (ignored)",
                py_repr_str(a)
            ));
        }
    }
    warnings
}

fn real_conflicts(ev: &EvaluatedForm) -> Vec<Conflict> {
    let current: HashSet<String> = ev.matched.iter().map(py_str).collect();
    let mut out = Vec::new();
    for c in &ev.conflicts {
        let live: Vec<_> = c
            .decisions
            .iter()
            .filter(|d| current.contains(&py_str(&d.rule_id)))
            .cloned()
            .collect();
        let ids: HashSet<String> = live.iter().map(|d| py_str(&d.rule_id)).collect();
        if ids.len() < 2 {
            continue;
        }
        let positive: HashSet<bool> = live
            .iter()
            .map(|d| ["show", "mandate", "disable"].contains(&d.action.as_str()))
            .collect();
        if positive.len() < 2 {
            continue;
        }
        let winner = if live.contains(&c.winner) {
            c.winner.clone()
        } else {
            live[live.len() - 1].clone()
        };
        out.push(Conflict {
            field: c.field.clone(),
            property: c.property.clone(),
            decisions: live,
            winner,
        });
    }
    out
}

fn rule_of(r: &ChoiceRestriction) -> String {
    if r.rule_name.is_empty() {
        py_str(&r.rule_id)
    } else {
        r.rule_name.clone()
    }
}

fn label_of(state: &FormState, name: &str) -> String {
    state
        .spec
        .get(name)
        .map_or_else(|| name.to_string(), |f| f.label.clone())
}

fn review_notes(state: &FormState) -> Result<Vec<String>> {
    let ev = &state.evaluated;
    let mut out = Vec::new();
    for (name, rid) in &ev.populated {
        if ev.is_visible(name) {
            out.push(format!(
                "{} = {} (filled in by rule {})",
                label_of(state, name),
                state.display(name, None).map_err(form_err)?,
                py_str(rid)
            ));
        }
    }
    for (name, r) in &ev.restricted {
        if ev.is_visible(name) {
            let offered: Vec<String> = state
                .choices(name)
                .map_err(form_err)?
                .into_iter()
                .filter(|c| c.visible)
                .map(|c| c.label)
                .collect();
            let shown = if offered.is_empty() {
                "(none)".to_string()
            } else {
                offered.join(", ")
            };
            out.push(format!(
                "{}: choices limited to {shown} by rule {}",
                label_of(state, name),
                py_repr_str(&rule_of(r))
            ));
        }
    }
    let pos = |n: &String| ev.visible.iter().position(|v| v == n).unwrap_or(1 << 20);
    let mut disabled: Vec<&String> = ev.disabled.iter().collect();
    disabled.sort_by_key(|n| pos(n));
    for name in disabled {
        let d = ev.decision(name, "disabled");
        out.push(format!(
            "{}: read-only{}",
            label_of(state, name),
            d.map(|d| format!(" ({})", d.describe()))
                .unwrap_or_default()
        ));
    }
    for (name, (old, rid)) in &ev.overridden {
        out.push(format!(
            "{}: your value {} was replaced by rule {}",
            label_of(state, name),
            py_repr_str(&state.display(name, Some(old)).map_err(form_err)?),
            py_str(rid)
        ));
    }
    out.extend(
        real_conflicts(ev)
            .iter()
            .map(|c| format!("conflict: {}", c.describe())),
    );
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
            rule: if m.rule_name.is_empty() {
                m.rule_id.clone()
            } else {
                json!(m.rule_name)
            },
        })
        .collect()
}

fn review_dropped(state: &FormState) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (f, v, d) in state.dropped() {
        let why = match &d {
            Some(d) if d.action == "hide" => {
                format!("hidden by rule {}", py_repr_str(&d.rule_name))
            }
            _ => "hidden".to_string(),
        };
        out.push(format!(
            "{} = {} ({why})",
            f.label,
            state.display(&f.api_name, Some(&v)).map_err(form_err)?
        ));
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
    let item = spec
        .item
        .clone()
        .ok_or_else(|| Error::other("not a service-item form"))?;
    let mut engine = meta.engine()?;
    if who.requested_for.is_some() {
        // rules needing requested-for attributes fail closed until reloaded for that identity
        engine.context.requested_for = None;
    }
    let mut state = FormState::new(
        spec.clone(),
        Some(engine),
        "new",
        None,
        StateOptions {
            resolver,
            ..StateOptions::default()
        },
    )
    .map_err(form_err)?;
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
                errors.push(format!(
                    "{}: give file paths (attachment ids are assigned after upload)",
                    f.label
                ));
                continue;
            };
            let ph = format!("<upload {}: {}>", uploads.len() + 1, basename(&p));
            uploads.push(Upload {
                placeholder: ph.clone(),
                path: p,
                target: name.clone(),
                label: f.label.clone(),
            });
            field_ph.entry(name.clone()).or_default().push(json!(ph));
        }
    }
    let mut general_ph: Vec<Json> = Vec::new();
    for p in &draft.attachments {
        let ph = format!("<upload {}: {}>", uploads.len() + 1, basename(p));
        uploads.push(Upload {
            placeholder: ph.clone(),
            path: p.clone(),
            target: "attachments".into(),
            label: "Other attachments".into(),
        });
        general_ph.push(json!(ph));
    }
    errors.extend(
        uploads
            .iter()
            .filter(|u| !is_file(&u.path))
            .map(|u| format!("file not found: {}", u.path)),
    );

    let skip: HashSet<String> = failed
        .iter()
        .filter_map(|n| spec.get(n))
        .map(|f| format!("{} is required", f.label))
        .collect();
    let mut problems = errors.clone();
    problems.extend(
        state
            .errors()
            .map_err(form_err)?
            .into_iter()
            .filter(|p| !skip.contains(p)),
    );
    let (rule_warnings, rule_errors) = rule_diagnostics(meta, &state);
    warnings.extend(rule_warnings);
    problems.extend(rule_errors);
    problems.extend(unsupported_lookup_errors(&state, draft));
    if ev.attachments_required.unwrap_or(item.attachment_mandatory) && draft.attachments.is_empty()
    {
        problems.push("this item requires at least one general attachment: add `attachments: [FILE]` to the draft or pass --attach FILE".into());
    }
    if !draft.attachments.is_empty() && !item.allow_attachments {
        problems.push("this item does not accept general attachments".into());
    }
    let req_email = requester
        .and_then(|r| r.email.clone())
        .filter(|e| !e.is_empty());
    match requester {
        None => problems.insert(0, REQUESTER_MISSING.into()),
        Some(r) if req_email.is_none() => {
            problems.insert(0, format!("requester {r} has no email address"))
        }
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
        let modes: &[bool] = if problems.is_empty() {
            &[false, true]
        } else {
            &[true]
        };
        for &incomplete in modes {
            let opts = SrOptions {
                field_attachments: field_ph.clone(),
                attachments: if item.allow_attachments {
                    general_ph.clone()
                } else {
                    vec![]
                },
                account_id: Json::Null,
                allow_incomplete: incomplete,
            };
            match build_sr_payload(Some(&item), &state, email, &opts) {
                Ok(mut p) => {
                    if let Some(e) = who
                        .requested_for
                        .and_then(|r| r.email.clone())
                        .filter(|e| !e.is_empty())
                    {
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

    let mut review = Review {
        title: format!("New service request: {}", item.name),
        ..Review::default()
    };
    let category = if item.category_name.is_empty() {
        String::new()
    } else {
        format!(" · {}", item.category_name)
    };
    review.rows.push((
        "Catalog item".into(),
        format!("{} (#{}){category}", item.name, py_str(&item.display_id)),
    ));
    review.rows.push((
        "Requester".into(),
        requester.map_or_else(|| "(not set)".to_string(), ToString::to_string),
    ));
    if let Some(rf) = who.requested_for {
        review.rows.push((
            "Requested for".into(),
            format!("{rf} (user ID {})", py_str(&rf.id)),
        ));
    }
    if valid_cc && !who.cc_emails.is_empty() {
        let shown = if !who.cc_ids.is_empty() && who.cc_ids.len() == who.cc_emails.len() {
            who.cc_emails
                .iter()
                .zip(who.cc_ids)
                .map(|(e, i)| format!("{e} (user ID {})", py_str(i)))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            who.cc_emails.join(", ")
        };
        review.rows.push(("CC".into(), shown));
    }
    let subject = if item.subject_template.is_empty() {
        String::new()
    } else {
        item.render_subject(&ev.values)
    };
    if !subject.is_empty() {
        review
            .rows
            .push(("Subject (item template)".into(), subject));
    }
    for name in state.payload_values().keys() {
        let Some(f) = spec.get(name) else { continue };
        let text = if f.kind == Kind::Attachment {
            match ev.values.get(name) {
                Some(fsv_forms::Value::List(items)) => items
                    .iter()
                    .map(|p| basename(&p.py_str()).to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => String::new(),
            }
        } else {
            state.display(name, None).map_err(form_err)?
                + if ev.populated.contains_key(name) {
                    " (auto)"
                } else {
                    ""
                }
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
            Some(ReviewAttachment {
                target: u.label.clone(),
                name: d.name,
                size: d.size,
                sha256: d.sha256,
                path: u.path.clone(),
            })
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
        if let Some(requester) = requester {
            let id = crate::watchers::positive_id(&requester.id)?;
            identity.insert("requester_id".into(), json!(id));
        }
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

/// `SendFailure.__init__`: uploads or the place-request failed; lists every
/// attachment id already on the server.
pub fn send_failure(step: &str, uploaded: &[Json], error: &dyn std::fmt::Display) -> Error {
    Error::other(failure_text(step, uploaded, error))
}

fn uploaded_ids(uploaded: &[Json]) -> String {
    uploaded
        .iter()
        .map(|u| format!("{} ({})", py_str(&u["id"]), py_str(&u["name"])))
        .collect::<Vec<_>>()
        .join(", ")
}

fn failure_text(step: &str, uploaded: &[Json], error: &dyn std::fmt::Display) -> String {
    let ids = uploaded_ids(uploaded);
    let ids = if ids.is_empty() { "none" } else { ids.as_str() };
    format!("{step} failed: {error}. Uploaded attachment ids: {ids}")
}

/// Port of fsv.catalog.SendFailure: the failed step, the attachments
/// already on the server and the underlying error.
#[derive(Debug)]
pub struct SendFailure {
    pub step: String,
    pub uploaded: Vec<Json>,
    pub cause: Error,
}

impl std::fmt::Display for SendFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&failure_text(&self.step, &self.uploaded, &self.cause))
    }
}

impl std::error::Error for SendFailure {}

impl SendFailure {
    /// Port of fsv.cli_sr.place's `except cat.SendFailure` note: printed
    /// before the error when attachments were uploaded, else `None`.
    pub fn uploaded_note(&self) -> Option<String> {
        (!self.uploaded.is_empty()).then(|| {
            format!(
                "uploaded attachment ids (not attached to any request): {}",
                uploaded_ids(&self.uploaded)
            )
        })
    }

    /// Port of fsv.cli_sr.place's `except cat.SendFailure` conversion:
    /// `FlowExit(exit_code_for(cause), "{step} failed: {detail}; {what}")`.
    pub fn into_error(self) -> Error {
        let what = if self.step == "upload" {
            "nothing was placed"
        } else if definite_rejection(&self.cause).is_some() {
            "the server refused it; nothing was placed"
        } else {
            "the outcome is unknown: check `fsv journal tail` and the requester's requests before retrying"
        };
        let detail = match &self.cause {
            Error::Api(e) => format!("HTTP {}", e.status),
            Error::Http { kind, .. } => kind.clone(),
            Error::Session(_) => "SessionError".into(),
            Error::Value(_) => "ValueError".into(),
            Error::Exit { .. } => "FlowExit".into(),
            Error::EditorAbort(_) => "EditorAbort".into(),
            Error::Io(_) => "OSError".into(),
            Error::Aborted => "Abort".into(),
            Error::Other(_) | Error::Context { .. } => "RuntimeError".into(),
        };
        let code = match &self.cause {
            Error::Exit { .. } => fsv_core::error::EXIT_OTHER,
            other => other.exit_code(),
        };
        Error::exit(code, format!("{} failed: {detail}; {what}", self.step))
    }
}

impl From<SendFailure> for Error {
    fn from(f: SendFailure) -> Error {
        Error::other(f.to_string())
    }
}

/// Placeholders that still occur in `str(body)` after substitution.
///
/// Substitution replaces a string only when it equals a placeholder, so a
/// placeholder that is only a substring of another field stays in the body.
fn unreplaced_placeholders(body: &Json, ids: &Map<String, Json>) -> Vec<String> {
    let rendered = py_str(body);
    ids.keys()
        .filter(|p| rendered.contains(p.as_str()))
        .cloned()
        .collect()
}

/// `_substitute`: placeholders -> attachment ids, recursively.
pub fn substitute(value: &Json, ids: &Map<String, Json>) -> Json {
    match value {
        Json::String(s) if ids.contains_key(s) => ids[s].clone(),
        Json::Array(a) => Json::Array(a.iter().map(|v| substitute(v, ids)).collect()),
        Json::Object(m) => Json::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), substitute(v, ids)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `execute`: upload the files, then place the request (journaled under
/// the plan hash). Call only after the review was confirmed. A
/// [`SendFailure`] is flattened to its `Display` text; use
/// [`execute_detailed`] to keep the step, uploads and cause.
pub fn execute(
    plan: &SRPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    upload_fn: &dyn Fn(&dyn Api, &str) -> Result<i64>,
    allow_duplicate: bool,
) -> Result<Json> {
    execute_detailed(plan, c, journal, upload_fn, allow_duplicate).map_err(|e| match e {
        Executed::Failed(f) => f.into(),
        Executed::Refused(e) => e,
    })
}

/// Port of the two outcomes of fsv.catalog.execute raising: a refusal
/// before anything was sent, or a [`SendFailure`] once a step was tried.
#[derive(Debug)]
pub enum Executed {
    Refused(Error),
    Failed(SendFailure),
}

impl From<Error> for Executed {
    fn from(e: Error) -> Executed {
        Executed::Refused(e)
    }
}

impl From<Executed> for Error {
    /// Port of fsv.cli_sr.place's conversion: a send failure becomes the
    /// structured `FlowExit`; refusals pass through.
    fn from(e: Executed) -> Error {
        match e {
            Executed::Refused(e) => e,
            Executed::Failed(f) => f.into_error(),
        }
    }
}

/// Port of fsv.catalog.execute keeping [`SendFailure`] structured.
pub fn execute_detailed(
    plan: &SRPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    upload_fn: &dyn Fn(&dyn Api, &str) -> Result<i64>,
    allow_duplicate: bool,
) -> std::result::Result<Json, Executed> {
    fn flow_exit(code: i32, msg: impl Into<String>) -> Executed {
        Executed::Refused(Error::exit(code, msg))
    }
    let failed = |step: &str, uploaded: &[Json], cause: Error| {
        Executed::Failed(SendFailure {
            step: step.into(),
            uploaded: uploaded.to_vec(),
            cause,
        })
    };
    let Some(payload) = plan
        .payload
        .as_ref()
        .filter(|_| plan.ok() && !plan.hash.is_empty())
    else {
        return Err(flow_exit(
            2,
            format!("plan has problems: {}", plan.errors.join("; ")),
        ));
    };
    if !plan.identity.is_empty() && plan.identity.get("tenant") != Some(&json!(c.domain())) {
        return Err(flow_exit(
            2,
            "tenant changed since review; nothing was sent",
        ));
    }
    duplicate_check(plan, allow_duplicate, journal)?;
    let (_, mut preflight) = rule_diagnostics(&plan.meta, &plan.state);
    preflight.extend(unsupported_lookup_errors(&plan.state, &plan.draft));
    preflight.extend(plan.state.errors().map_err(form_err)?);
    match &plan.meta.spec.item {
        None => preflight.push("not a service-item form".into()),
        Some(item) => {
            if plan
                .state
                .evaluated
                .attachments_required
                .unwrap_or(item.attachment_mandatory)
                && plan.draft.attachments.is_empty()
            {
                preflight.push("this item requires at least one general attachment".into());
            }
            if !plan.draft.attachments.is_empty() && !item.allow_attachments {
                preflight.push("this item does not accept general attachments".into());
            }
        }
    }
    if !preflight.is_empty() {
        return Err(flow_exit(
            2,
            format!("plan has problems: {}", preflight.join("; ")),
        ));
    }
    let current = plan_hash(&plan.hashed(), Some(&plan.plan_files()))
        .map_err(|e| flow_exit(2, format!("cannot verify reviewed files: {e}")))?;
    if current != plan.hash {
        return Err(flow_exit(
            2,
            "the reviewed payload or files changed; review again",
        ));
    }
    let target = json!(plan.target());
    let before = journal.read().len();
    journal.record_plan("started", &plan.hash, &plan.action, Some(&target), None);
    let entries = journal.read();
    let last = entries.last();
    let field = |k: &str| {
        last.and_then(|e| e.get(k))
            .and_then(Json::as_str)
            .map(str::to_string)
    };
    if entries.len() <= before
        || field("kind").as_deref() != Some(format!("plan:{}", plan.action).as_str())
        || field("plan").as_deref() != Some(plan.hash.as_str())
        || field("event").as_deref() != Some("started")
    {
        return Err(flow_exit(
            5,
            "could not persist the request journal; nothing was uploaded or placed",
        ));
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
                journal.record_plan_steps(
                    "failed",
                    &plan.hash,
                    &plan.action,
                    Some(&target),
                    None,
                    &[],
                    true,
                );
                return Err(failed("upload", &uploaded, e));
            }
        }
    }
    let body = substitute(&Json::Object(payload.clone()), &ids);
    // `p in str(body)`: a placeholder left inside a longer field still counts.
    // This is outside the place-request try, so a hit does not mark the journal failed.
    let missing = unreplaced_placeholders(&body, &ids);
    if !missing.is_empty() {
        let shown = py_repr(&Json::Array(
            missing.into_iter().map(Json::String).collect(),
        ));
        return Err(failed(
            "place-request",
            &uploaded,
            Error::other(format!("unreplaced upload placeholders: {shown}")),
        ));
    }
    let result = match place_request(c, &plan.meta.display_id(), &body) {
        Ok(r) => r,
        Err(e) => {
            if definite_rejection(&e).is_some() {
                journal.record_plan_steps(
                    "failed",
                    &plan.hash,
                    &plan.action,
                    Some(&target),
                    None,
                    &[],
                    true,
                );
            }
            return Err(failed("place-request", &uploaded, e));
        }
    };
    let created = created_ref(&result);
    if created["id"].is_null() {
        return Err(failed(
            "place-request",
            &uploaded,
            Error::other("response has no exact request ID; outcome unknown"),
        ));
    }
    let steps = json!([
        {"step": "upload", "ok": true, "ids": uploaded.iter().map(|u| u["id"].clone()).collect::<Vec<_>>()},
        {"step": "place-request", "ok": true, "id": created["id"]},
    ]);
    let jsteps = [
        PlanStep {
            step: "upload".into(),
            ok: true,
        },
        PlanStep {
            step: "place-request".into(),
            ok: true,
        },
    ];
    journal.record_plan_steps(
        "done",
        &plan.hash,
        &plan.action,
        Some(&target),
        Some(&created["id"]),
        &jsteps,
        false,
    );
    let mut verification = json!({"verified": false, "fields": {}});
    if !plan.identity.is_empty() {
        let readback = || -> Result<Json> {
            let raw = c.int_get(&format!("tickets/{}", py_str(&created["id"])), None)?;
            let row = if raw.is_object() {
                raw.get("ticket").cloned().unwrap_or_else(|| raw.clone())
            } else {
                Json::Null
            };
            if !row.is_object()
                || py_str(row.get("id").unwrap_or(&Json::Null)) != py_str(&created["id"])
            {
                return Err(Error::value("identity mismatch"));
            }
            let Some(body) = body.as_object() else {
                return Err(Error::value("identity mismatch"));
            };
            let fields = crate::sr_recipient_readback::recipient_fields(
                row.as_object().unwrap(),
                body,
                &plan.identity,
            )?;
            let verified = !fields.is_empty() && fields.values().all(|v| v == "matched");
            Ok(json!({"verified": verified, "fields": fields}))
        };
        if let Ok(v) = readback() {
            verification = v;
        }
        if verification["verified"] != Json::Bool(true) {
            verification["warning"] =
                json!("Request recipient fields were not fully verified; do not resend.");
        }
    }
    Ok(
        json!({"created": created, "result": result, "uploaded": uploaded, "body": body, "plan_hash": plan.hash,
              "steps": steps, "verification": verification}),
    )
}

/// `review_diff`: what changed between two plans (rows, payload keys).
fn review_diff(old: &SRPlan, new: &SRPlan) -> Vec<String> {
    let to_map =
        |rows: &[(String, String)]| -> IndexMap<String, String> { rows.iter().cloned().collect() };
    let (a, b) = (to_map(&old.review.rows), to_map(&new.review.rows));
    let mut keys: Vec<&String> = a.keys().collect();
    keys.extend(b.keys().filter(|k| !a.contains_key(*k)));
    let dash = "—".to_string();
    let mut out: Vec<String> = keys
        .into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .map(|k| {
            format!(
                "{k}: {} → {}",
                a.get(k).unwrap_or(&dash),
                b.get(k).unwrap_or(&dash)
            )
        })
        .collect();
    if out.is_empty() && old.payload != new.payload {
        let empty = Map::new();
        let (op, np) = (
            old.payload.as_ref().unwrap_or(&empty),
            new.payload.as_ref().unwrap_or(&empty),
        );
        let mut keys: Vec<&String> = op.keys().chain(np.keys()).collect();
        keys.sort();
        keys.dedup();
        out.extend(
            keys.into_iter()
                .filter(|k| op.get(*k) != np.get(*k))
                .map(|k| format!("payload {k} changed")),
        );
    }
    if old.errors != new.errors {
        out.extend(
            new.errors
                .iter()
                .filter(|e| !old.errors.contains(e))
                .map(|e| format!("problem: {e}")),
        );
    }
    out
}

/// `freshen`: re-fetch the item and its rules right before sending.
/// -> (plan to send, diff lines when the plan changed).
pub fn freshen(
    plan: SRPlan,
    c: &dyn Api,
    store: &dyn MetaStore,
    rebuild: &mut dyn FnMut(ItemMeta) -> Result<SRPlan>,
) -> Result<(SRPlan, Option<Vec<String>>)> {
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
    let mut hashes = vec![plan.hash.clone()];
    if plan.identity.contains_key("requester_id") {
        // A write reviewed before requester ids were bound stays unresolved.
        let mut previous = plan.identity.clone();
        previous.shift_remove("requester_id");
        let mut hashed = obj(json!({"request": plan.payload, "target": plan.target()}));
        hashed.insert("recipient_identity".into(), Json::Object(previous));
        hashes.push(
            plan_hash(&Json::Object(hashed), Some(&plan.plan_files()))
                .map_err(|e| flow_exit(2, format!("cannot verify reviewed files: {e}")))?,
        );
    }
    if hashes.iter().any(|hash| {
        crate::service::last_event(journal, "create", hash).as_deref() == Some("started")
    }) {
        return Err(flow_exit(
            4,
            "previous request outcome is unknown; inspect the server and journal before retry",
        ));
    }
    if plan.action != "create" || allow_duplicate {
        return Ok(());
    }
    let Some(hit) = journal.recent_plan(&plan.hash, "create", DUPLICATE_WINDOW_HOURS) else {
        return Ok(());
    };
    let ts = py_str(hit.get("ts").unwrap_or(&Json::Null));
    let what = if hit.get("event").and_then(Json::as_str) == Some("done") {
        format!(
            "was already sent at {ts} (created {})",
            py_str(hit.get("created_id").unwrap_or(&Json::Null))
        )
    } else {
        format!("was started at {ts} with no recorded outcome (it may have been created)")
    };
    Err(flow_exit(
        4,
        format!(
            "this exact plan {what}. Check `fsv journal tail`; pass --allow-duplicate to send it again."
        ),
    ))
}

// ---------------------------------------------------------------------------
// requested items (read side)

/// `label_map`: name -> label over item field definitions (`fields`,
/// `nested_fields`, `sections[].fields`).
pub fn label_map(defs: &[Json]) -> IndexMap<String, String> {
    fn walk(rows: &[Json], out: &mut IndexMap<String, String>) {
        for r in rows {
            let Some(m) = r.as_object() else { continue };
            let (name, label) = (
                m.get("name").filter(|v| truthy(v)),
                m.get("label").filter(|v| truthy(v)),
            );
            if let (Some(name), Some(label)) = (name, label)
                && py_str(m.get("type").unwrap_or(&Json::Null)) != "1024"
                && !is_content_type(&or_empty(m.get("field_type")))
            {
                out.entry(py_str(name))
                    .or_insert_with(|| fsv_forms::pytext::html_unescape(&py_str(label)));
            }
            let list = |k: &str| {
                m.get(k)
                    .filter(|v| truthy(v))
                    .and_then(Json::as_array)
                    .cloned()
                    .unwrap_or_default()
            };
            walk(&list("fields"), out);
            walk(&list("nested_fields"), out);
            for s in list("sections") {
                if let Some(fields) = s
                    .get("fields")
                    .filter(|v| truthy(v))
                    .and_then(Json::as_array)
                {
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
    let meta = requested
        .get("item")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut labels = label_map(
        meta.get("custom_fields")
            .filter(|v| truthy(v))
            .and_then(Json::as_array)
            .map_or(&[][..], Vec::as_slice),
    );
    let values = requested.obj_or_empty_map("custom_fields");
    let did = meta
        .get("display_id")
        .filter(|v| truthy(v))
        .or_else(|| requested.get("service_item_display_id"))
        .cloned()
        .unwrap_or(Json::Null);
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
                labels
                    .entry(f.api_name.clone())
                    .or_insert_with(|| f.label.clone());
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
        self.get(key)
            .filter(|v| truthy(v))
            .and_then(Json::as_object)
            .cloned()
            .unwrap_or_default()
    }
}

/// `value_text`: one requested-item value as display text.
pub fn value_text(value: &Json) -> String {
    match value {
        Json::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        Json::Object(m) => ["name", "value", "label", "email"]
            .iter()
            .find_map(|k| m.get(*k).filter(|v| truthy(v)).map(py_str))
            .unwrap_or_else(|| py_str(value)),
        Json::Bool(b) => (if *b { "Yes" } else { "No" }).into(),
        Json::String(s) if s.contains('<') => {
            fsv_core::pystr::collapse_ws(&fsv_forms::spec::plain_text(s))
        }
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
                t("attachment_url")
                    || t("canonical_url")
                    || (t("name") && m.get("id").is_some_and(|v| !v.is_null()))
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
        return Err(Error::value(format!(
            "unsafe file name {r}: contains a path separator"
        )));
    }
    let b = name.as_bytes();
    if name.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
        return Err(Error::value(format!("unsafe file name {r}: absolute path")));
    }
    if crate::service::has_control(name, "") {
        return Err(Error::value(format!(
            "unsafe file name {r}: control characters"
        )));
    }
    let dir = PathBuf::from(expanduser(directory));
    let root = std::fs::canonicalize(&dir).unwrap_or(dir);
    let joined = root.join(name);
    let target = std::fs::canonicalize(&joined).unwrap_or(joined);
    if target.parent() != Some(root.as_path()) {
        return Err(Error::value(format!(
            "unsafe file name {r}: resolves outside {}",
            root.display()
        )));
    }
    Ok(target)
}

/// `domain_url`.
pub fn domain_url(path: &str, domain: &str) -> String {
    format!("https://{domain}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formmeta::MemStore;
    use crate::service::testing::{FakeApi, MemJournal, Reply};
    use std::sync::LazyLock;

    /// Outputs of `port/golden/oracles/fsv_oracle/o_catalog.py` (the Python functions).
    static ORACLE: LazyLock<Json> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/catalog_oracle.json")).unwrap()
    });
    static FILES: LazyLock<String> = LazyLock::new(|| {
        let dir = std::env::temp_dir().join("fsv_api_catalog_files");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("deck.pptx"), b"deck-bytes").unwrap();
        std::fs::write(dir.join("notes.txt"), b"notes\n").unwrap();
        dir.to_string_lossy().into_owned()
    });

    fn want(key: &str) -> Json {
        let text = ORACLE[key]
            .to_string()
            .replace("/tmp/fsv_oracle/files", &FILES);
        serde_json::from_str(&text).unwrap()
    }

    fn want_err(key: &str) -> String {
        ORACLE[key]["msg"].as_str().unwrap().to_string()
    }

    fn err<T>(r: Result<T>) -> String {
        r.err().expect("an error").to_string()
    }

    fn item_with(name: &str, updated: &str, lookup: bool) -> Json {
        let mut fields = json!([
            {"id": 9000000003i64, "name": "answer", "label": "Answer", "type": 1001, "required": false, "account_id": 9000000004i64},
            {"id": 9000000005i64, "name": "deck", "label": "Deck", "type": 1014, "required": false, "account_id": 9000000004i64,
             "field_options": {"allowed_file_types": "[\".pptx\"]", "max_attachment_limit": "1"}},
            {"id": 9000000008i64, "name": "size", "label": "Size", "type": 1003, "required": false, "account_id": 9000000004i64,
             "choices": [{"id": "s1", "value": "Small", "label": "Small", "position": 1},
                         {"id": "s2", "value": "Medium  Large", "label": "Medium  Large", "position": 2},
                         {"id": "s3", "value": "Mega", "label": "Mega", "position": 3}]},
        ]);
        if lookup {
            fields.as_array_mut().unwrap().push(json!({"id": 9000000009i64, "name": "who", "label": "Who", "type": 501, "required": false, "account_id": 9000000004i64}));
        }
        json!({"id": 9000000001i64, "display_id": 9000000002i64, "name": name, "category_name": "Eng", "workspace_id": 3,
               "updated_at": updated, "allow_attachments": true, "configs": {"attachment_mandatory": false}, "custom_fields": fields})
    }

    fn item() -> Json {
        item_with("Acme request", "2030-01-15T10:00:00+09:00", false)
    }

    fn meta() -> ItemMeta {
        ItemMeta::new(
            item(),
            json!({"business_rules": []}),
            json!(3),
            0.0,
            "server",
        )
        .unwrap()
    }

    fn answers(v: Json) -> Map<String, Json> {
        obj(v)
    }

    fn ann() -> LookupValue {
        LookupValue::new(json!(9000000007i64), "Acme", Some("a@example.com"))
    }

    fn state(m: &ItemMeta) -> FormState {
        FormState::new(
            m.spec(),
            Some(m.engine().unwrap()),
            "new",
            None,
            StateOptions::default(),
        )
        .unwrap()
    }

    fn file(name: &str) -> String {
        format!("{}/{name}", *FILES)
    }

    fn full_plan() -> SRPlan {
        let draft = Draft {
            answers: answers(json!({"Answer": "ok", "Size": "meg", "Deck": file("deck.pptx")})),
            attachments: vec![file("notes.txt")],
            ..Draft::default()
        };
        build_sr_plan(
            &meta(),
            &draft,
            None,
            Some(&ann()),
            false,
            &Recipients::default(),
            "acme.freshservice.com",
        )
        .unwrap()
    }

    fn recipients_plan() -> SRPlan {
        let bob = LookupValue::new(json!(55), "Bob", Some("bob@example.com"));
        let cc = ["c@example.com".to_string()];
        let ids = [json!(66)];
        let who = Recipients {
            requested_for: Some(&bob),
            cc_emails: &cc,
            cc_ids: &ids,
        };
        build_sr_plan(
            &meta(),
            &Draft {
                answers: answers(json!({"Answer": "ok"})),
                ..Draft::default()
            },
            None,
            Some(&ann()),
            false,
            &who,
            "acme.freshservice.com",
        )
        .unwrap()
    }

    fn rows_json(p: &SRPlan) -> Json {
        json!(
            p.review
                .rows
                .iter()
                .map(|(a, b)| json!([a, b]))
                .collect::<Vec<_>>()
        )
    }

    #[test]
    fn norm_and_rows() {
        assert_eq!(
            json!([
                norm(&json!("  A\u{301}b  C ")),
                norm(&json!(5)),
                norm(&Json::Null)
            ]),
            want("norm")
        );
        let got = json!([
            rows(&json!([{"a": 1}, 2]), "k"),
            rows(&json!({"k": [{"b": 1}, "x"]}), "k"),
            rows(&json!({"k": 1}), "k"),
            rows(&json!("x"), "k")
        ]);
        assert_eq!(got, want("rows"));
    }

    fn cats() -> Json {
        json!({"service_categories": [{"id": 1, "name": "Engineering", "sub_categories": [{"id": 2, "name": "Eng Tools"}, "x"]},
                                      {"id": 3, "name": "Access", "children": [{"id": 4, "name": "VPN", "parent_id": 9}]}]})
    }

    fn flat() -> Vec<Json> {
        list_categories(&FakeApi::new().ok("GET catalog/service-categories", cats())).unwrap()
    }

    fn summ(id: i64, did: i64, name: &str, cat: i64, extra: Json) -> Json {
        let mut m = obj(
            json!({"id": id, "display_id": did, "name": name, "short_description": format!("{name} desc"), "category_id": cat}),
        );
        m.extend(obj(extra));
        Json::Object(m)
    }

    fn params(a: &FakeApi) -> Json {
        json!(
            a.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| json!(c.params))
                .collect::<Vec<_>>()
        )
    }

    #[test]
    fn categories_and_items() {
        let a = FakeApi::new().ok("GET catalog/service-categories", cats());
        let got = list_categories(&a).unwrap();
        assert_eq!(json!([got, params(&a)[0]]), want("list_categories"));

        let page1 = json!({"service_items": [summ(1, 11, "Laptop", 1, json!({})), summ(2, 12, "VPN Access", 3, json!({})), summ(1, 11, "Laptop", 1, json!({}))], "meta": {"total_pages": 2}});
        let page2 = json!({"service_items": [summ(3, 13, "Old", 1, json!({"deleted": true})), summ(4, 14, "Build Agent", 2, json!({}))], "meta": {"total_pages": 2}});
        let a = FakeApi::new()
            .ok("GET catalog/service-items", page1)
            .ok("GET catalog/service-items", page2);
        let ids: Vec<Json> = list_items(&a, false, MAX_PAGES)
            .unwrap()
            .iter()
            .map(|i| i["id"].clone())
            .collect();
        assert_eq!(json!([ids, params(&a)]), want("list_items"));

        let a = FakeApi::new().ok(
            "GET catalog/service-items",
            json!({"service_items": [summ(5, 15, "Pop", 1, json!({}))]}),
        );
        let got = list_items(&a, true, MAX_PAGES).unwrap();
        assert_eq!(json!([got, params(&a)[0]]), want("list_popular"));

        let a = FakeApi::new()
            .ok(
                "GET catalog/service-items",
                json!({"service_items": [summ(1, 11, "A", 1, json!({}))]}),
            )
            .ok("GET catalog/service-items", json!({"service_items": []}));
        let ids: Vec<Json> = list_items(&a, false, MAX_PAGES)
            .unwrap()
            .iter()
            .map(|i| i["id"].clone())
            .collect();
        assert_eq!(
            json!([ids, a.calls.lock().unwrap().len()]),
            want("list_short_page")
        );
    }

    #[test]
    fn resolve_and_filter() {
        let flat = flat();
        let ids = |q: &str| {
            json!(
                resolve_category(&flat, q)
                    .unwrap()
                    .iter()
                    .map(|c| c["id"].clone())
                    .collect::<Vec<_>>()
            )
        };
        assert_eq!(
            json!([ids("engineering"), ids("3"), ids("vp")]),
            want("resolve_category")
        );
        assert_eq!(
            err(resolve_category(&flat, "e")),
            want_err("resolve_category_many")
        );
        assert_eq!(
            err(resolve_category(&flat, "zzz")),
            want_err("resolve_category_none")
        );

        let items = [
            summ(1, 11, "Laptop", 1, json!({})),
            summ(2, 12, "VPN Access", 3, json!({"category_name": "Access"})),
            summ(4, 14, "Build Agent", 2, json!({})),
        ];
        let ids = |v: Vec<Json>| json!(v.iter().map(|i| i["id"].clone()).collect::<Vec<_>>());
        let got = json!([
            ids(filter_items(
                &items,
                Some(&[json!(1), json!("2")]),
                None,
                &[]
            )),
            ids(filter_items(&items, None, Some("access"), &[])),
            ids(filter_items(&items, None, Some("eng tools"), &flat)),
            ids(filter_items(&items, None, Some("14"), &[])),
        ]);
        assert_eq!(got, want("filter_items"));
    }

    #[test]
    fn get_find_place_and_created_ref() {
        let a = FakeApi::new()
            .ok(
                "GET catalog/service-items/9000000002",
                json!({"service_item": item()}),
            )
            .ok("GET catalog/service-items/5", json!({"x": 1}));
        assert_eq!(
            json!(get_item(&a, &json!(9000000002i64)).unwrap()["name"]),
            want("get_item")
        );
        assert_eq!(err(get_item(&a, &json!(5))), want_err("get_item_bad"));
        assert_eq!(
            json!([item_ref(" #77 "), item_ref("Laptop")]),
            want("item_ref")
        );

        let items = json!([
            summ(1, 11, "Laptop", 1, json!({})),
            summ(2, 12, "VPN Access", 3, json!({"category_name": "Access"})),
            summ(4, 14, "Build Agent", 2, json!({}))
        ]);
        let a = FakeApi::new()
            .ok("GET catalog/service-items", json!({"service_items": items}))
            .ok(
                "GET catalog/service-items/11",
                json!({"service_item": {"display_id": 11, "name": "Laptop"}}),
            )
            .ok(
                "GET catalog/service-items/77",
                json!({"service_item": {"display_id": 77}}),
            )
            .ok(
                "GET catalog/service-items/12",
                json!({"service_item": {"display_id": 12, "name": "By digits"}}),
            );
        assert_eq!(
            json!([
                find_item(&a, "#77").unwrap()["display_id"],
                find_item(&a, "laptop").unwrap()["display_id"]
            ]),
            want("find_item")
        );
        assert_eq!(find_item(&a, "१२").unwrap()["display_id"], json!(12));
        assert_eq!(err(find_item(&a, " # ")), want_err("find_item_empty"));
        assert_eq!(err(find_item(&a, "printer")), want_err("find_item_none"));
        assert_eq!(err(find_item(&a, "a")), want_err("find_item_many"));

        let a = FakeApi::new().ok(
            "POST catalog/service-items/77/place-request",
            json!({"service_request": {"id": 5, "display_id": 6}}),
        );
        let got = place_request(&a, &json!(77), &json!({"a": 1})).unwrap();
        let w = &a.writes()[0];
        assert_eq!(
            json!([got, [{"method": w.method, "path": w.path, "params": w.params, "body": w.body}]]),
            want("place_request")
        );

        let cases = [
            json!({"id": 1}),
            json!({"x": {"display_id": 6}}),
            json!({"x": {"id": 2, "human_display_id": "SR-9"}}),
            json!({"x": 1}),
            json!("s"),
        ];
        assert_eq!(
            json!(cases.iter().map(created_ref).collect::<Vec<_>>()),
            want("created_ref")
        );
    }

    #[test]
    fn upload_posts_the_file() {
        // `catalog.upload` is `create.upload_file`; same oracle case (`o_create.py` "upload_file").
        let want: Json = serde_json::from_str::<Json>(include_str!("testdata/create_oracle.json"))
            .unwrap()["upload_file"]
            .clone();
        let path = std::env::temp_dir()
            .join("fsv_api_catalog_upload")
            .join("a.txt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"hello").unwrap();
        let a = FakeApi::new().ok("POST attachments", json!({"id": 33}));
        let got = upload(&a, &path.to_string_lossy()).unwrap();
        assert_eq!(json!([got, a.writes()[0].body]), want);
    }

    #[test]
    fn item_meta_and_load() {
        let m = meta();
        let names: Vec<String> = m
            .spec()
            .fields()
            .iter()
            .map(|f| f.api_name.clone())
            .collect();
        let w = want("item_meta");
        let mut as_of = m.as_of();
        assert_eq!(
            as_of
                .shift_remove("fetched_at")
                .unwrap()
                .as_str()
                .unwrap()
                .len(),
            w[4]["fetched_at"].as_str().unwrap().len()
        );
        let mut w_as_of = w[4].as_object().unwrap().clone();
        w_as_of.shift_remove("fetched_at");
        assert_eq!(
            json!([m.display_id(), m.name(), names, m.fingerprint().hash]),
            json!([w[0], w[1], w[2], w[3]])
        );
        assert_eq!(as_of, w_as_of);

        let a = FakeApi::new().ok(
            "GET ws/3/business_rules",
            json!({"business_rules": [], "user": {"id": 1}}),
        );
        let mm = load_item_meta(&a, &MemStore::default(), "", Some(&item())).unwrap();
        let calls = a.calls.lock().unwrap();
        let got = json!([
            mm.rules,
            mm.workspace_id,
            mm.source,
            calls.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
            calls[0].params.as_ref().unwrap()["item_id"]
        ]);
        assert_eq!(got, want("load_item_meta"));
    }

    #[test]
    fn answers_helpers() {
        let spec = meta().spec();
        let size = find_field(&spec, "size").unwrap();
        let got: Vec<Json> = [
            json!("Small"),
            json!("medium   large"),
            json!("me"),
            json!("meg"),
            json!("x"),
            json!(5),
        ]
        .iter()
        .map(|v| json!(match_choice(size, v)))
        .collect();
        assert_eq!(json!(got), want("match_choice"));
        assert_eq!(
            json!(lenient_answers(
                &spec,
                &answers(json!({"Size": "meg", "answer": "hi", "unknown": 1}))
            )),
            want("lenient")
        );
        assert_eq!(
            json!(file_answers(
                &spec,
                &answers(json!({"Deck": "deck.pptx", "answer": "x", "deck2": "y"})),
                Some("/base")
            )),
            want("file_answers")
        );
        let got = file_answers(
            &spec,
            &answers(json!({"deck": ["/abs/a.pptx", "~/b.pptx"], "unknown": "z"})),
            None,
        );
        assert_eq!(got["deck"][0], want("file_answers_abs"));
    }

    /// `os.path.expanduser`: `~user` uses the account home; an unknown name stays.
    #[test]
    fn file_answers_expands_tilde_user() {
        let home = std::env::var("HOME").unwrap_or_default();
        let trimmed = home.trim_end_matches('/');
        let expect_home = if trimmed.is_empty() {
            "/".to_string()
        } else {
            trimmed.to_string()
        };
        assert_eq!(expanduser("~"), expect_home);
        assert_eq!(expanduser("~/b.pptx"), format!("{expect_home}/b.pptx"));
        assert_eq!(expanduser("~nosuchuser-fsv-g6c"), "~nosuchuser-fsv-g6c");
        assert_eq!(
            expanduser("~nosuchuser-fsv-g6c/c.pptx"),
            "~nosuchuser-fsv-g6c/c.pptx"
        );

        let user = std::env::var("USER").expect("USER");
        let named = expanduser(&format!("~{user}"));
        assert!(named.starts_with('/'), "{named}");
        assert_ne!(named, format!("~{user}"));
        assert_eq!(
            expanduser(&format!("~{user}/b.pptx")),
            format!("{named}/b.pptx")
        );

        let spec = meta().spec();
        let got = file_answers(
            &spec,
            &answers(json!({
                "deck": [format!("~{user}/b.pptx"), "~nosuchuser-fsv-g6c/c.pptx"]
            })),
            Some("/base"),
        );
        assert_eq!(got["deck"][0], json!(format!("{named}/b.pptx")));
        assert_eq!(got["deck"][1], json!("/base/~nosuchuser-fsv-g6c/c.pptx"));
    }

    #[test]
    fn diagnostics_and_lookup_errors() {
        let m = meta();
        assert_eq!(json!(rule_diagnostics(&m, &state(&m))), want("rule_diag"));
        let mu = ItemMeta::new(
            item(),
            json!({"business_rules": [], "unavailable": "why"}),
            json!(3),
            0.0,
            "server",
        )
        .unwrap();
        assert_eq!(
            json!(rule_diagnostics(&mu, &state(&mu))),
            want("rule_diag_unavailable")
        );
        let ml = ItemMeta::new(
            item_with("Acme request", "2030-01-15T10:00:00+09:00", true),
            json!({"business_rules": []}),
            json!(3),
            0.0,
            "server",
        )
        .unwrap();
        let stl = state(&ml);
        let d = |v: Json| Draft {
            answers: answers(v),
            ..Draft::default()
        };
        let got = json!([
            unsupported_lookup_errors(&stl, &d(json!({"Who": "Ann"}))),
            unsupported_lookup_errors(&stl, &d(json!({"who": ""}))),
            unsupported_lookup_errors(&state(&m), &d(json!({"answer": "x"}))),
        ]);
        assert_eq!(got, want("lookup_errors"));
    }

    #[test]
    fn digest_and_requester_line() {
        assert_eq!(json!(digest(&file("deck.pptx")).unwrap()), want("digest"));
        let got = [
            requester_line(&LookupValue::new(json!(1), "Ann", Some("a@b.co"))),
            requester_line(&LookupValue::new(json!(2), String::new(), Some("b@c.co"))),
            requester_line(&LookupValue::id(3)),
        ];
        assert_eq!(json!(got), want("requester_line"));
    }

    #[test]
    fn build_sr_plan_matches_python() {
        let p = full_plan();
        let mut j = p.to_json();
        j["review"]["metadata"]
            .as_object_mut()
            .unwrap()
            .shift_remove("fetched_at");
        assert_eq!(
            json!([p.ok(), p.target(), p.files(), p.hash, j]),
            want("plan")
        );

        let p2 = recipients_plan();
        assert_eq!(
            json!([p2.ok(), p2.payload, p2.identity, p2.hash, rows_json(&p2)]),
            want("plan_recipients")
        );

        let draft = Draft {
            answers: answers(json!({"Nope": 1})),
            description_html: Some("<p>x</p>".into()),
            planning: answers(json!({"a": {}})),
            attachments: vec!["/missing.txt".into()],
            ..Draft::default()
        };
        let p3 = build_sr_plan(
            &meta(),
            &draft,
            None,
            None,
            false,
            &Recipients::default(),
            "acme.freshservice.com",
        )
        .unwrap();
        assert_eq!(
            json!([p3.ok(), p3.errors, p3.payload, p3.hash]),
            want("plan_errors")
        );

        let cc = ["bad".to_string()];
        let who = Recipients {
            cc_emails: &cc,
            ..Recipients::default()
        };
        let p4 = build_sr_plan(
            &meta(),
            &Draft::default(),
            None,
            Some(&LookupValue::new(json!(1), "NoMail", None)),
            false,
            &who,
            "acme.freshservice.com",
        )
        .unwrap();
        assert_eq!(json!([p4.errors, p4.payload]), want("plan_errors2"));
    }

    #[test]
    fn send_failure_and_substitute() {
        let got = [
            send_failure("upload", &[json!({"id": 1, "name": "a.txt"})], &"boom").to_string(),
            send_failure("place-request", &[], &"x").to_string(),
        ];
        assert_eq!(json!(got), want("send_failure"));
        let ids = obj(json!({"<u1>": 7}));
        assert_eq!(
            substitute(
                &json!({"a": ["<u1>", {"b": "<u1>"}], "c": "<u2>", "d": 1}),
                &ids
            ),
            want("substitute")
        );
    }

    const PLACE: &str = "POST catalog/service-items/9000000002/place-request";

    #[test]
    fn execute_uploads_then_places() {
        let p = full_plan();
        let j = MemJournal::default();
        let a = FakeApi::new().ok(
            PLACE,
            json!({"service_request": {"id": 500, "display_id": 600}}),
        );
        let next = std::sync::atomic::AtomicI64::new(101);
        let up = |_: &dyn Api, _: &str| Ok(next.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        assert_eq!(execute(&p, &a, &j, &up, false).unwrap(), want("execute"));
        assert_eq!(json!(j.events()), want("execute_events"));
        let w = &a.writes()[0];
        assert_eq!(
            json!([{"method": w.method, "path": w.path, "params": w.params, "body": w.body}]),
            want("execute_writes")
        );
        let msg = err(execute(&p, &a, &j, &|_: &dyn Api, _: &str| Ok(1), false));
        assert_eq!(
            msg,
            want_err("execute_dup")
                .replace(" at None ", &format!(" at {} ", py_str(&j.read()[1]["ts"])))
        );
    }

    #[test]
    fn execute_failures() {
        let p = full_plan();
        let seven = |_: &dyn Api, _: &str| Ok(7);
        let j = MemJournal::default();
        let a = FakeApi::new().on(PLACE, Reply::Status(422));
        assert_eq!(
            err(execute(&p, &a, &j, &seven, false)),
            want_err("execute_422")
        );
        assert_eq!(json!(j.events()), want("execute_422_events"));

        let j = MemJournal::default();
        assert_eq!(
            err(execute(
                &p,
                &FakeApi::new(),
                &j,
                &|_: &dyn Api, _: &str| Err(Error::other("disk")),
                false
            )),
            want_err("execute_upload_fail")
        );
        assert_eq!(json!(j.events()), want("execute_upload_fail_events"));

        let draft = Draft {
            answers: answers(json!({"Nope": 1})),
            description_html: Some("<p>x</p>".into()),
            planning: answers(json!({"a": {}})),
            attachments: vec!["/missing.txt".into()],
            ..Draft::default()
        };
        let p3 = build_sr_plan(
            &meta(),
            &draft,
            None,
            None,
            false,
            &Recipients::default(),
            "acme.freshservice.com",
        )
        .unwrap();
        assert_eq!(
            err(execute(
                &p3,
                &FakeApi::new(),
                &MemJournal::default(),
                &seven,
                false
            )),
            want_err("execute_bad_plan")
        );

        let j = MemJournal::default();
        let three = |_: &dyn Api, _: &str| Ok(3);
        let a = FakeApi::new().ok(PLACE, json!({"ok": true}));
        assert_eq!(
            err(execute(&p, &a, &j, &three, false)),
            want_err("execute_noid")
        );
        assert_eq!(json!(j.events()), want("execute_noid_events"));

        let a = FakeApi::new().ok(PLACE, json!({"service_request": {"id": 500}})).ok(
            "GET tickets/500",
            json!({"ticket": {"id": 500, "email": "a@example.com", "requested_for": "bob@example.com", "cc_emails": ["x@example.com"]}}),
        );
        assert_eq!(
            execute(
                &recipients_plan(),
                &a,
                &MemJournal::default(),
                &three,
                false
            )
            .unwrap(),
            want("execute_identity")
        );

        let deck2 = file("deck2.pptx");
        std::fs::write(&deck2, b"v1").unwrap();
        let pd = build_sr_plan(
            &meta(),
            &Draft {
                answers: answers(json!({"Answer": "ok", "Deck": deck2})),
                ..Draft::default()
            },
            None,
            Some(&ann()),
            false,
            &Recipients::default(),
            "acme.freshservice.com",
        )
        .unwrap();
        std::fs::write(&deck2, b"v2").unwrap();
        assert_eq!(
            err(execute(
                &pd,
                &FakeApi::new(),
                &MemJournal::default(),
                &three,
                false
            )),
            want_err("execute_changed_file")
        );
    }

    /// `catalog.execute` raising: `SendFailure(step, uploaded, error)` once a
    /// step was tried, a `FlowExit` before; messages from the Python oracle.
    #[test]
    fn execute_detailed_keeps_the_failed_step() {
        let p = full_plan();
        let seven = |_: &dyn Api, _: &str| Ok(7);
        let a = FakeApi::new().on(PLACE, Reply::Status(422));
        let Err(Executed::Failed(f)) =
            execute_detailed(&p, &a, &MemJournal::default(), &seven, false)
        else {
            panic!("place-request should fail");
        };
        assert_eq!(f.step, "place-request");
        assert_eq!(f.uploaded.len(), p.uploads.len());
        assert!(f.uploaded.iter().all(|u| u["id"] == json!(7)));
        assert_eq!(f.cause.status(), Some(422));
        assert_eq!(f.to_string(), want_err("execute_422"));

        let disk = |_: &dyn Api, _: &str| Err(Error::other("disk"));
        let Err(Executed::Failed(f)) =
            execute_detailed(&p, &FakeApi::new(), &MemJournal::default(), &disk, false)
        else {
            panic!("upload should fail");
        };
        assert_eq!((f.step.as_str(), f.uploaded.len()), ("upload", 0));
        assert_eq!(f.to_string(), want_err("execute_upload_fail"));

        let j = MemJournal::default();
        j.record_plan("done", &p.hash, "create", None, Some(&json!(5)));
        let Err(Executed::Refused(e)) = execute_detailed(&p, &FakeApi::new(), &j, &seven, false)
        else {
            panic!("a placed plan is refused");
        };
        assert_eq!(e.to_string(), want_err("dup_done"));
    }

    /// A placeholder that is only a substring of another field is not
    /// substituted (`value in ids` is exact) and `p in str(body)` still fails
    /// the send, without a failed journal mark and without place-request.
    #[test]
    fn execute_rejects_placeholder_substring() {
        let ids = obj(json!({"<upload 1: a.txt>": 9}));
        let body = substitute(
            &json!({"note": "see <upload 1: a.txt> please", "file": "<upload 1: a.txt>"}),
            &ids,
        );
        assert_eq!(body["file"], json!(9));
        assert_eq!(body["note"], json!("see <upload 1: a.txt> please"));
        assert_eq!(
            unreplaced_placeholders(&body, &ids),
            vec!["<upload 1: a.txt>".to_string()]
        );

        let mut p = full_plan();
        let ph = p.uploads[0].placeholder.clone();
        let mut payload = p.payload.clone().unwrap();
        payload.insert("subject".into(), json!(format!("see {ph} please")));
        p.payload = Some(payload);
        p.hash = plan_hash(&p.hashed(), Some(&p.plan_files())).unwrap();
        let j = MemJournal::default();
        let a = FakeApi::new().ok(PLACE, json!({"service_request": {"id": 1}}));
        let Err(Executed::Failed(f)) =
            execute_detailed(&p, &a, &j, &|_: &dyn Api, _: &str| Ok(7), false)
        else {
            panic!("a leftover placeholder must fail the send");
        };
        let shown = py_repr(&json!([ph]));
        assert_eq!(f.step, "place-request");
        assert_eq!(
            f.cause.to_string(),
            format!("unreplaced upload placeholders: {shown}")
        );
        assert_eq!(
            f.to_string(),
            format!(
                "place-request failed: unreplaced upload placeholders: {shown}. Uploaded attachment ids: 7 (deck.pptx), 7 (notes.txt)"
            )
        );
        assert_eq!(j.events(), vec!["started".to_string()]);
        assert!(a.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn duplicate_check_journal() {
        let p = full_plan();
        let j = MemJournal::default();
        j.record_plan("started", &p.hash, "create", None, None);
        assert_eq!(err(duplicate_check(&p, true, &j)), want_err("dup_started"));
        let j = MemJournal::default();
        j.record_plan("done", &p.hash, "create", None, Some(&json!(5)));
        assert_eq!(err(duplicate_check(&p, false, &j)), want_err("dup_done"));
        assert!(duplicate_check(&p, true, &j).is_ok());
        assert_eq!(ORACLE["dup_allowed"], Json::Null);

        let recipients = recipients_plan();
        assert_eq!(recipients.identity["requester_id"], json!(9000000007i64));
        let mut previous = recipients.identity.clone();
        previous.shift_remove("requester_id");
        let mut hashed = obj(json!({"request": recipients.payload, "target": recipients.target()}));
        hashed.insert("recipient_identity".into(), Json::Object(previous));
        let old = plan_hash(&Json::Object(hashed), Some(&recipients.plan_files())).unwrap();
        assert_eq!(
            old,
            "ab09e59e340830bc1e26b2ba6b0f89f1c4360defee3340d4e239bc7b80a5f85e"
        );
        assert_ne!(old, recipients.hash);
        let j = MemJournal::default();
        j.record_plan("started", &old, "create", None, None);
        assert_eq!(
            err(duplicate_check(&recipients, true, &j)),
            want_err("dup_started")
        );
    }

    #[test]
    fn freshen_rebuilds_on_change() {
        let store = MemStore::default();
        let a = FakeApi::new()
            .ok(
                "GET catalog/service-items/9000000002",
                json!({"service_item": item()}),
            )
            .ok("GET ws/3/business_rules", json!({"business_rules": []}));
        let ok = || Draft {
            answers: answers(json!({"Answer": "ok"})),
            ..Draft::default()
        };
        let build = |m: &ItemMeta, d: Draft| {
            build_sr_plan(
                m,
                &d,
                None,
                Some(&ann()),
                false,
                &Recipients::default(),
                "acme.freshservice.com",
            )
        };
        let lm = load_item_meta(&a, &store, "9000000002", None).unwrap();
        let pf = build(&lm, ok()).unwrap();
        let hash = pf.hash.clone();
        let (same, diff) = freshen(pf, &a, &store, &mut |_| {
            Err(Error::other("rebuild must not run"))
        })
        .unwrap();
        assert_eq!(json!([same.hash == hash, diff]), want("freshen_same"));

        let a = FakeApi::new()
            .ok("GET catalog/service-items/9000000002", json!({"service_item": item_with("Acme request", "2030-02-01T00:00:00+09:00", false)}))
            .ok("GET ws/3/business_rules", json!({"business_rules": []}));
        let (new, diff) = freshen(same, &a, &store, &mut |m| build(&m, ok())).unwrap();
        assert_eq!(json!([new.hash == hash, diff]), want("freshen_same_hash"));
        let pf = build(&lm, ok()).unwrap();
        let (new, diff) = freshen(pf, &a, &store, &mut |m| {
            build(
                &m,
                Draft {
                    answers: answers(json!({"Answer": "new"})),
                    ..Draft::default()
                },
            )
        })
        .unwrap();
        assert_eq!(json!([new.hash, diff]), want("freshen_changed"));
    }

    #[test]
    fn labels_and_values() {
        let defs = [
            json!({"name": "a", "label": "A &amp; B"}),
            json!({"name": "n", "label": "Notice", "type": 1024}),
            json!({"name": "c", "label": "C", "field_type": "custom_static_rich_text"}),
            json!({"name": "s", "label": "S", "fields": [{"name": "s1", "label": "S1"}], "nested_fields": [{"name": "s2", "label": "S2"}],
                   "sections": [{"fields": [{"name": "s3", "label": "S3"}]}, "x"]}),
            json!({"name": "a", "label": "dup"}),
            json!("junk"),
        ];
        assert_eq!(json!(label_map(&defs)), want("label_map"));
        let a = FakeApi::new().ok(
            "GET catalog/service-items/9000000002",
            json!({"service_item": item()}),
        );
        let req = json!({"item": {"display_id": 9000000002i64, "custom_fields": [{"name": "answer", "label": "Ans"}]}, "custom_fields": {"answer": "x", "size": "Small", "zzz": 1}});
        assert_eq!(json!(item_labels(&a, &req).unwrap()), want("item_labels"));
        assert_eq!(
            json!(
                item_labels(
                    &FakeApi::new(),
                    &json!({"service_item_display_id": 5, "custom_fields": {"q": 1}})
                )
                .unwrap()
            ),
            want("item_labels_404")
        );
        let req = json!({"item": {"custom_fields": [{"name": "q", "label": "Q"}]}, "custom_fields": {"q": 1}});
        assert_eq!(
            json!(item_labels(&FakeApi::new(), &req).unwrap()),
            want("item_labels_complete")
        );

        let vals = [
            json!(["a", {"name": "N"}]),
            json!({"value": "V"}),
            json!({"x": 1}),
            json!(true),
            json!(false),
            json!("<p>a  <b>b</b></p>"),
            json!(5),
            Json::Null,
            json!("a < b"),
        ];
        assert_eq!(
            json!(vals.iter().map(value_text).collect::<Vec<_>>()),
            want("value_text")
        );
        let vals = [
            json!([1, 2]),
            json!([{"attachment_url": "u"}]),
            json!([{"name": "n", "id": 1}]),
            json!([{"name": "n"}]),
            json!([]),
            json!("x"),
            json!([1, "a"]),
        ];
        assert_eq!(
            json!(vals.iter().map(is_file_value).collect::<Vec<_>>()),
            want("is_file_value")
        );
    }

    #[test]
    fn safe_target_and_domain_url() {
        let dir = std::env::temp_dir().join("fsv_api_catalog_dl");
        std::fs::create_dir_all(&dir).unwrap();
        let got = safe_target(&dir.to_string_lossy(), "a.txt").unwrap();
        let w = want("safe_target");
        assert_eq!(got.file_name(), Path::new(w.as_str().unwrap()).file_name());
        assert_eq!(got.parent().unwrap(), dir.canonicalize().unwrap());
        for (key, bad) in [
            ("st_empty", " "),
            ("st_dots", "a..b"),
            ("st_sep", "a/b"),
            ("st_bs", "a\\b"),
            ("st_drive", "C:x"),
            ("st_ctl", "a\u{1}"),
        ] {
            assert_eq!(
                err(safe_target(&dir.to_string_lossy(), bad)),
                want_err(key),
                "{key}"
            );
        }
        assert_eq!(
            json!(domain_url("/a/b", "acme.freshservice.com")),
            want("domain_url")
        );
    }

    /// `catalog.SendFailure` and `cli_sr.place`'s structured conversion.
    #[test]
    fn send_failure_structured_conversion() {
        let oracle: Json =
            serde_json::from_str(include_str!("testdata/reviewed_oracle.json")).unwrap();
        let file = || vec![json!({"id": 9, "name": "a.txt"})];
        let api = |s: u16, b: &str| Error::from(fsv_core::ApiError::new(s, json!(b)));
        let cases = [
            ("upload", vec![], Error::other("boom")),
            ("upload", file(), Error::value("bad")),
            ("place-request", file(), api(422, "no")),
            ("place-request", vec![], api(409, "dup")),
            ("place-request", vec![], api(403, "denied")),
            ("place-request", vec![], api(500, "x")),
            ("place-request", vec![], Error::session("expired")),
            (
                "place-request",
                vec![],
                Error::other("response has no exact request ID; outcome unknown"),
            ),
            ("place-request", vec![], Error::exit(4, "changed")),
        ];
        for ((step, uploaded, cause), want) in cases
            .into_iter()
            .zip(oracle["send_failure"].as_array().unwrap())
        {
            let f = SendFailure {
                step: step.into(),
                uploaded,
                cause,
            };
            assert_eq!(f.to_string(), want["display"]);
            assert_eq!(json!(f.uploaded_note()), want["note"]);
            let e = f.into_error();
            assert_eq!(
                (json!(e.exit_code()), e.to_string()),
                (
                    want["code"].clone(),
                    want["message"].as_str().unwrap().to_string()
                )
            );
        }
    }

    /// `ItemMeta.raw_fields` (cc81f93): web order, children name their
    /// parent, deleted rows and their children are skipped.
    #[test]
    fn item_raw_fields_flatten_like_python() {
        let item = json!({"service_item": {"display_id": 5, "custom_fields": [
            {"name": "a", "label": "A", "fields": [{"name": "a1", "label": "A1"},
                                                   {"name": "gone", "deleted": true}, "junk"]},
            {"name": "d", "deleted": true, "fields": [{"name": "hidden"}]},
            {"label": "nameless", "parent": "x", "fields": [{"name": "orphan"}]}]}});
        let m = ItemMeta {
            item,
            rules: json!({}),
            workspace_id: Json::Null,
            fetched_at: 0.0,
            source: "fixture".into(),
            spec: Arc::new(FormSpec::from_service_item(&json!({"custom_fields": []})).unwrap()),
        };
        assert_eq!(
            Json::Array(m.raw_fields()),
            json!([{"name": "a", "label": "A"}, {"name": "a1", "label": "A1", "parent": "a"},
                   {"label": "nameless", "parent": "x"}, {"name": "orphan"}])
        );
    }
}
