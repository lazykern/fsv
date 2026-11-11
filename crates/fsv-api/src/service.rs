//! Reusable data-fetching layer for the CLI and TUI (`service.py`).
//!
//! Every call goes through [`Api`], implemented for [`fsv_core::Client`]
//! and by test fakes, so nothing here needs the network to be exercised.
//! Rows tagged by Python with `item["_resource"] = resource` carry the
//! resource name as a string under the same key.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::client::{Body, RequestOpts, Response};
use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, PROBLEMS, Resource, TICKETS};
use fsv_py::{ValueExt, truthy};

use crate::site::Missing;

/// The client operations the ported modules use (`fsv.client.Client`).
pub trait Api: Sync {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value>;
    fn int_post(&self, path: &str, body: &Value) -> Result<Value>;
    fn int_put(&self, path: &str, body: &Value) -> Result<Value>;
    fn int_patch(&self, path: &str, body: &Value) -> Result<Value>;
    fn int_delete(
        &self,
        path: &str,
        body: Option<&Value>,
        params: Option<Map<String, Value>>,
    ) -> Result<Value>;
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Option<Map<String, Value>>,
    ) -> Result<Vec<Value>>;
    fn lookup_choices(&self, link: &str, query: &str) -> Result<Vec<Value>>;
    fn fulltext_search(
        &self,
        entity: &str,
        term: &str,
        page: i64,
        sort: Option<&str>,
    ) -> Result<Value>;
    /// `_get_csrf`.
    fn get_csrf(&self) -> Result<String>;
    /// `_request` on the internal API: exactly one request with the given
    /// CSRF token, never refreshed or replayed.
    fn send_once(
        &self,
        method: &str,
        path: &str,
        body: Body,
        params: Option<Map<String, Value>>,
        csrf: &str,
    ) -> Result<Value>;
    /// `_client.get(url, follow_redirects=True)`: raw download, no status mapping.
    fn fetch(&self, url: &str) -> Result<Response>;
    /// Tenant host (`config.DOMAIN`).
    fn domain(&self) -> String;
    /// Values of session cookies whose name contains `workspace`.
    fn workspace_cookies(&self) -> Vec<String> {
        Vec::new()
    }
}

impl Api for fsv_core::Client {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        fsv_core::Client::int_get(self, path, params)
    }
    fn int_post(&self, path: &str, body: &Value) -> Result<Value> {
        fsv_core::Client::int_post(self, path, body)
    }
    fn int_put(&self, path: &str, body: &Value) -> Result<Value> {
        fsv_core::Client::int_put(self, path, body)
    }
    fn int_patch(&self, path: &str, body: &Value) -> Result<Value> {
        fsv_core::Client::int_patch(self, path, body)
    }
    fn int_delete(
        &self,
        path: &str,
        body: Option<&Value>,
        params: Option<Map<String, Value>>,
    ) -> Result<Value> {
        fsv_core::Client::int_delete(self, path, body, params)
    }
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Option<Map<String, Value>>,
    ) -> Result<Vec<Value>> {
        fsv_core::Client::autocomplete(self, kind, query, params)
    }
    fn lookup_choices(&self, link: &str, query: &str) -> Result<Vec<Value>> {
        fsv_core::Client::lookup_choices(self, link, query)
    }
    fn fulltext_search(
        &self,
        entity: &str,
        term: &str,
        page: i64,
        sort: Option<&str>,
    ) -> Result<Value> {
        fsv_core::Client::fulltext_search(self, entity, term, page, sort)
    }
    fn get_csrf(&self) -> Result<String> {
        fsv_core::Client::get_csrf(self)
    }
    fn send_once(
        &self,
        method: &str,
        path: &str,
        body: Body,
        params: Option<Map<String, Value>>,
        csrf: &str,
    ) -> Result<Value> {
        let url = format!("{}/{}", self.tenant().api_int, path.trim_start_matches('/'));
        let opts = RequestOpts {
            params,
            body,
            ..Default::default()
        }
        .header("Content-Type", "application/json")
        .header("X-CSRF-Token", csrf);
        self.request(method, &url, opts)
    }
    fn fetch(&self, url: &str) -> Result<Response> {
        self.fetch_raw(url, true)
    }
    fn domain(&self) -> String {
        self.tenant().domain.clone()
    }
}

/// Plan events of the write journal (`journal.read` / `journal.record_plan`).
pub trait Journal {
    fn read(&self) -> Vec<Value>;
    fn record_plan(
        &self,
        event: &str,
        plan: &str,
        action: &str,
        target: Option<&Value>,
        created_id: Option<&Value>,
    );
    /// `record_plan(..., steps=..., error=...)`; the journal keeps only step
    /// names, ok flags and whether there was an error.
    #[allow(clippy::too_many_arguments)]
    fn record_plan_steps(
        &self,
        event: &str,
        plan: &str,
        action: &str,
        target: Option<&Value>,
        created_id: Option<&Value>,
        steps: &[fsv_core::journal::PlanStep],
        error: bool,
    ) {
        let _ = (steps, error);
        self.record_plan(event, plan, action, target, created_id);
    }
    /// `journal.recent_plan`: the latest `done`, or a `started` with no
    /// outcome, within `hours`.
    fn recent_plan(&self, plan: &str, action: &str, hours: f64) -> Option<Value> {
        let _ = hours;
        let kind = format!("plan:{action}");
        let rows: Vec<Value> = self
            .read()
            .into_iter()
            .filter(|r| {
                r.get("kind").and_then(Value::as_str) == Some(kind.as_str())
                    && r.get("plan").and_then(Value::as_str) == Some(plan)
            })
            .collect();
        if let Some(done) = rows
            .iter()
            .rev()
            .find(|r| r.get("event").and_then(Value::as_str) == Some("done"))
        {
            return Some(done.clone());
        }
        rows.last()
            .filter(|r| r.get("event").and_then(Value::as_str) == Some("started"))
            .cloned()
    }
}

/// The on-disk journal of the current profile and tenant.
pub struct DiskJournal;

impl Journal for DiskJournal {
    fn read(&self) -> Vec<Value> {
        fsv_core::journal::read(None)
    }
    fn record_plan(
        &self,
        event: &str,
        plan: &str,
        action: &str,
        target: Option<&Value>,
        created_id: Option<&Value>,
    ) {
        let extra = fsv_core::journal::PlanEvent {
            target,
            created_id,
            ..Default::default()
        };
        fsv_core::journal::record_plan(event, plan, action, extra);
    }
    fn record_plan_steps(
        &self,
        event: &str,
        plan: &str,
        action: &str,
        target: Option<&Value>,
        created_id: Option<&Value>,
        steps: &[fsv_core::journal::PlanStep],
        error: bool,
    ) {
        let extra = fsv_core::journal::PlanEvent {
            target,
            created_id,
            steps: Some(steps),
            error,
        };
        fsv_core::journal::record_plan(event, plan, action, extra);
    }
    fn recent_plan(&self, plan: &str, action: &str, hours: f64) -> Option<Value> {
        fsv_core::journal::recent_plan(plan, action, hours)
    }
}

/// `[r for r in journal.read() if kind == plan:<action> and plan == hash]`.
pub(crate) fn plan_events(journal: &dyn Journal, action: &str, hash: &str) -> Vec<Value> {
    let kind = format!("plan:{action}");
    journal
        .read()
        .into_iter()
        .filter(|r| {
            r.get("kind").and_then(Value::as_str) == Some(kind.as_str())
                && r.get("plan").and_then(Value::as_str) == Some(hash)
        })
        .collect()
}

/// `events[-1].get("event")` of [`plan_events`].
pub(crate) fn last_event(journal: &dyn Journal, action: &str, hash: &str) -> Option<String> {
    plan_events(journal, action, hash)
        .last()
        .and_then(|r| r.get("event"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// `journal.plan_hash(body) or ""`.
pub(crate) fn plan_hash(body: &Value) -> String {
    fsv_core::journal::plan_hash(Some(body)).unwrap_or_default()
}

/// `_id`: a positive decimal int or digit string, never a bool or float.
pub(crate) fn positive_id(value: &Value) -> Option<i64> {
    let text = match value {
        Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>().ok().filter(|n| *n > 0)
}

/// Any C0 control character or DEL; `allow` lists permitted ones.
pub(crate) fn has_control(text: &str, allow: &str) -> bool {
    text.chars()
        .any(|c| ((c as u32) < 32 && !allow.contains(c)) || c as u32 == 127)
}

/// `html.escape(text.replace("\r\n", "\n").replace("\r", "\n")).replace("\n", "<br>")`.
pub(crate) fn escape_lines(text: &str) -> String {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    fsv_core::pystr::html_escape(&unified, true).replace('\n', "<br>")
}

/// `json.dumps(payload, ensure_ascii=False)` as bytes.
pub(crate) fn dumps_bytes(payload: &Value, ensure_ascii: bool) -> Vec<u8> {
    fsv_py::Dumps::default()
        .ensure_ascii(ensure_ascii)
        .dump(payload)
        .into_bytes()
}

/// Python's `400 <= status < 500 and status not in (403, 408, 429)`: the
/// server definitely refused, so the plan may be marked failed.
pub(crate) fn definite_rejection(err: &Error) -> Option<u16> {
    let status = err.as_api()?.status;
    ((400..500).contains(&status) && ![403, 408, 429].contains(&status)).then_some(status)
}

/// The value of a `Result` or `None`, keeping a session error (Python
/// re-raises `SessionError` from `optional_get`).
fn session_or_missing(err: Error, what: &str) -> std::result::Result<Missing, Error> {
    match err {
        Error::Session(_) => Err(err),
        Error::Api(e) => Ok(Missing::new(what, Some(i64::from(e.status)))),
        _ => Ok(Missing::new(what, None)),
    }
}

/// `views.MAX_DETAIL_PAGES`.
pub const MAX_DETAIL_PAGES: usize = 50;

/// Rows, merged metadata and the failure of [`collect_pages`].
#[derive(Debug, Clone, PartialEq)]
pub struct Pages {
    pub rows: Vec<Value>,
    pub meta: Map<String, Value>,
    pub missing: Option<Missing>,
}

impl Pages {
    pub fn complete(&self) -> bool {
        self.meta.get("complete") == Some(&Value::Bool(true))
    }
}

/// `int(x)` for JSON scalars.
pub(crate) fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite())
                .map(|f| f.trunc() as i64)
        }),
        Value::String(s) => {
            let t = fsv_core::pystr::strip(s).replace('_', "");
            t.parse::<i64>().ok()
        }
        _ => None,
    }
}

fn int_param(map: &Map<String, Value>, key: &str, default: i64) -> i64 {
    map.get(key).and_then(py_int).unwrap_or(default)
}

/// `views.collect_pages`: bounded internal reads that keep rows and the
/// continuation of an interrupted history. Explicit server pagination wins
/// over page length; continuation URLs may only add query parameters for
/// the same path. `get` is the client's `int_get`; any error but an expired
/// session becomes [`Pages::missing`].
pub fn collect_pages<F>(
    mut get: F,
    path: &str,
    key: &str,
    params: Option<&Map<String, Value>>,
    what: &str,
    max_pages: usize,
) -> Result<Pages>
where
    F: FnMut(&str, Option<Map<String, Value>>) -> Result<Value>,
{
    let origin = params.cloned().unwrap_or_default();
    let label = if what.is_empty() { key } else { what };
    let mut request = origin.clone();
    let mut rows: Vec<Value> = Vec::new();
    let mut meta = Map::new();
    let mut seen_requests: Vec<String> = Vec::new();
    let mut seen_ids: Vec<String> = Vec::new();
    let mut failure = None;
    let mut reason = String::new();
    let mut complete = false;
    let mut pages = 0i64;
    let mut exhausted = true;
    let empty = Map::new();
    for _ in 0..max_pages {
        let mut sig: Vec<(String, String)> = request
            .iter()
            .map(|(k, v)| (k.clone(), fsv_py::py_str(v)))
            .collect();
        sig.sort();
        let signature = format!("{sig:?}");
        if seen_requests.contains(&signature) {
            reason = "continuation cycle".into();
            exhausted = false;
            break;
        }
        seen_requests.push(signature);
        let sent = if request.is_empty() {
            None
        } else {
            Some(request.clone())
        };
        let data = match get(path, sent) {
            Ok(d) => d,
            Err(e) => {
                let miss = session_or_missing(e, label)?;
                reason = miss.note();
                failure = Some(miss);
                exhausted = false;
                break;
            }
        };
        let readable = match &data {
            Value::Array(_) => true,
            Value::Object(m) => m.get(key).is_some_and(Value::is_array),
            _ => false,
        };
        if !readable {
            reason = "unreadable page response".into();
            failure = Some(Missing::new(label, None));
            exhausted = false;
            break;
        }
        let batch: Vec<&Value> = match &data {
            Value::Array(a) => a.iter(),
            _ => data[key].as_array().map(|a| a.iter()).unwrap_or_default(),
        }
        .filter(|r| r.is_object())
        .collect();
        pages += 1;
        for row in &batch {
            let identity = row.get("id").filter(|v| !v.is_null()).map(fsv_py::py_str);
            match identity {
                Some(id) if seen_ids.contains(&id) => {}
                Some(id) => {
                    seen_ids.push(id);
                    rows.push((*row).clone());
                }
                None => rows.push((*row).clone()),
            }
        }
        let raw = data.as_object().unwrap_or(&empty);
        let page_meta = raw.get("meta").and_then(Value::as_object).unwrap_or(&empty);
        for (k, v) in page_meta {
            meta.insert(k.clone(), v.clone());
        }
        let mut flag: Option<Value> = [page_meta, raw]
            .iter()
            .flat_map(|s| ["has_next", "has_more"].map(|k| s.get(k)))
            .flatten()
            .find(|v| !v.is_null())
            .cloned();
        let either = |k: &str| {
            if raw.contains_key(k) {
                raw.get(k)
            } else {
                page_meta.get(k)
            }
        };
        let link_present =
            raw.contains_key("next_page_url") || page_meta.contains_key("next_page_url");
        let link = either("next_page_url").filter(|v| truthy(v)).cloned();
        let start = either("next_start_token").filter(|v| truthy(v)).cloned();
        let cursor = start
            .clone()
            .or_else(|| either("next_cursor").filter(|v| truthy(v)).cloned());
        if flag.is_none() && !link_present && cursor.is_none() {
            let total = if page_meta.contains_key("total_count") {
                page_meta.get("total_count")
            } else {
                page_meta.get("count")
            };
            if let Some(total) = total.and_then(py_int) {
                let offset =
                    (int_param(&origin, "page", 1) - 1) * int_param(&origin, "per_page", 0);
                let more = offset + (rows.len() as i64) < total;
                if more && batch.is_empty() {
                    reason = "empty page before reported total".into();
                    exhausted = false;
                    break;
                }
                flag = Some(Value::Bool(more));
            }
        }
        if flag == Some(Value::Bool(false)) || (link_present && link.is_none() && cursor.is_none())
        {
            complete = true;
            exhausted = false;
            break;
        }
        if let Some(link) = link {
            let Ok(parsed) = fsv_core::pystr::urlsplit(&fsv_py::py_str(&link)) else {
                reason = "unreadable continuation".into();
                exhausted = false;
                break;
            };
            let got = parsed.path.trim_end_matches('/');
            if !parsed.path.is_empty()
                && got != path.trim_end_matches('/')
                && got != format!("/api/_/{}", path.trim_matches('/'))
            {
                reason = "unsupported continuation endpoint".into();
                exhausted = false;
                break;
            }
            let following = fsv_core::pystr::parse_qsl(&parsed.query, true);
            if following.is_empty() {
                reason = "unreadable continuation".into();
                exhausted = false;
                break;
            }
            request.retain(|k, _| !matches!(k.as_str(), "page" | "start_token" | "cursor"));
            for (k, v) in following {
                request.insert(k, Value::String(v));
            }
        } else if let Some(cursor) = cursor {
            let cursor_key = if start.is_some() {
                "start_token"
            } else {
                "cursor"
            };
            request.remove("page");
            request.insert(cursor_key.into(), cursor);
        } else if flag == Some(Value::Bool(true))
            || request
                .get("per_page")
                .is_some_and(|v| truthy(v) && py_int(v).is_some_and(|n| batch.len() as i64 >= n))
        {
            if request.contains_key("start_token") || request.contains_key("cursor") {
                reason = "missing continuation cursor".into();
                exhausted = false;
                break;
            }
            let next = int_param(&request, "page", 1) + 1;
            request.insert("page".into(), json!(next));
        } else {
            complete = true;
            exhausted = false;
            break;
        }
    }
    if exhausted {
        reason = format!("page limit ({max_pages})");
    }
    meta.insert("loaded_count".into(), json!(rows.len()));
    meta.insert("pages".into(), json!(pages));
    meta.insert("complete".into(), json!(complete));
    meta.insert("has_more".into(), json!(!complete));
    meta.insert("has_next".into(), json!(!complete));
    meta.insert(
        "continuation".into(),
        if complete {
            Value::Null
        } else {
            Value::Object(request)
        },
    );
    if complete {
        meta.remove("partial");
    } else {
        meta.insert(
            "partial".into(),
            json!(format!(
                "{label}: incomplete ({} loaded; {reason})",
                rows.len()
            )),
        );
    }
    Ok(Pages {
        rows,
        meta,
        missing: failure,
    })
}

/// `views.load_tab(c, CHANGES, {"display_id": id}, name)` for the list tabs
/// the write verifiers read (`tasks`, `approvals`).
pub(crate) fn change_tab<F>(get: F, change_id: i64, name: &str) -> Result<Pages>
where
    F: FnMut(&str, Option<Map<String, Value>>) -> Result<Value>,
{
    let label = crate::site::tab_label(name);
    let base = format!("changes/{change_id}");
    match name {
        "tasks" => {
            let params = obj(json!({"page": 1, "per_page": 100}));
            collect_pages(
                get,
                &format!("{base}/tasks"),
                "tasks",
                Some(&params),
                &label,
                MAX_DETAIL_PAGES,
            )
        }
        _ => collect_pages(
            get,
            &format!("{base}/approvals"),
            "approvals",
            None,
            &label,
            MAX_DETAIL_PAGES,
        ),
    }
}

/// `views.load_conversations(c, ticket_id, all_pages=True)` rows and meta
/// (rows deduplicated and oldest first).
pub fn load_conversations(c: &dyn Api, ticket_id: i64) -> Result<Pages> {
    let params = obj(json!({"include": "user,phone,feedback", "per_page": 30, "page": 1}));
    let mut pages = collect_pages(
        |p, q| c.int_get(p, q),
        &format!("tickets/{ticket_id}/conversations"),
        "conversations",
        Some(&params),
        "conversations",
        MAX_DETAIL_PAGES,
    )?;
    let count = pages
        .meta
        .get("count")
        .filter(|v| truthy(v))
        .map_or(Some(0), py_int);
    let n = pages.rows.len() as i64;
    pages
        .meta
        .insert("count".into(), json!(count.map_or(n, |c| c.max(n))));
    if !pages.complete() {
        let loaded = pages.meta.get("pages").and_then(Value::as_i64).unwrap_or(0);
        pages.missing = Some(match pages.missing.take() {
            Some(m) if loaded > 0 => Missing::new(
                format!("conversations after the first {n} (page {})", loaded + 1),
                m.status,
            ),
            Some(m) => m,
            None => Missing::new(
                format!(
                    "conversations after page {loaded} ({})",
                    pages
                        .meta
                        .get("partial")
                        .map(fsv_py::py_str)
                        .unwrap_or_default()
                ),
                None,
            ),
        });
    }
    pages.rows = crate::site::oldest_first(&pages.rows);
    Ok(pages)
}

pub(crate) fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// `data.get(key, data)` (Python leaves non-dict data as is).
pub(crate) fn unwrap_key(data: Value, key: &str) -> Value {
    match data {
        Value::Object(mut m) if m.contains_key(key) => m.remove(key).unwrap_or(Value::Null),
        other => other,
    }
}

/// `data.get(key, [])` as rows (non-lists give no rows).
pub(crate) fn rows_at(data: &Value, key: &str) -> Vec<Value> {
    data.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn tag(mut item: Value, resource: &Resource) -> Value {
    if let Value::Object(m) = &mut item {
        m.insert("_resource".into(), json!(resource.name));
    }
    item
}

// ---------------------------------------------------------------------------
// lists and search

/// Arguments of a list request.
#[derive(Debug, Clone)]
pub struct ListArgs<'a> {
    pub page: i64,
    pub per_page: i64,
    pub filter_name: Option<&'a str>,
    pub order_by: Option<&'a str>,
    pub order_type: Option<&'a str>,
    pub query_hash: Option<&'a str>,
    pub or_grouping: bool,
}

impl Default for ListArgs<'_> {
    fn default() -> Self {
        ListArgs {
            page: 1,
            per_page: 30,
            filter_name: None,
            order_by: None,
            order_type: Some("desc"),
            query_hash: None,
            or_grouping: false,
        }
    }
}

/// `_list_params` (`fallback_desc`: `list_page` sends `order_type or "desc"`).
fn list_params(resource: &Resource, a: &ListArgs<'_>, fallback_desc: bool) -> Map<String, Value> {
    let mut p = Map::new();
    p.insert("per_page".into(), json!(a.per_page));
    p.insert("page".into(), json!(a.page));
    p.insert("include".into(), json!(resource.list_include));
    p.insert("cache".into(), json!("true"));
    if let Some(f) = a.filter_name.filter(|s| !s.is_empty()) {
        p.insert("filter".into(), json!(f));
    }
    if let Some(o) = a.order_by.filter(|s| !s.is_empty()) {
        p.insert("order_by".into(), json!(o));
        let ot = match a.order_type {
            Some(t) if !t.is_empty() || !fallback_desc => json!(t),
            _ if fallback_desc => json!("desc"),
            _ => Value::Null,
        };
        p.insert("order_type".into(), ot);
    }
    if let Some(q) = a.query_hash.filter(|s| !s.is_empty()) {
        if a.or_grouping {
            p.insert("advanced_query_hash".into(), json!(q));
            p.insert("query_hash".into(), json!(""));
        } else {
            p.insert("query_hash".into(), json!(q));
            if matches!(resource.name, "changes" | "tickets") {
                p.insert("advanced_query_hash".into(), json!(""));
            }
        }
    }
    p
}

/// `list_page`: one list page as the web requests it, `(items, raw response)`.
pub fn list_page(
    c: &dyn Api,
    resource: &Resource,
    args: &ListArgs<'_>,
) -> Result<(Vec<Value>, Value)> {
    let data = c.int_get(resource.api_path, Some(list_params(resource, args, true)))?;
    let data = if data.is_object() { data } else { json!({}) };
    let items = data
        .list_or_empty(resource.list_key)
        .iter()
        .map(|i| tag(i.clone(), resource))
        .collect();
    Ok((items, data))
}

/// `list_items`: `(items, total)`.
pub fn list_items(
    c: &dyn Api,
    resource: &Resource,
    args: &ListArgs<'_>,
) -> Result<(Vec<Value>, i64)> {
    let data = c.int_get(resource.api_path, Some(list_params(resource, args, false)))?;
    let items: Vec<Value> = rows_at(&data, resource.list_key)
        .into_iter()
        .map(|i| tag(i, resource))
        .collect();
    let total = data
        .get("meta")
        .filter(|v| truthy(v))
        .and_then(|m| m.get("total_count"))
        .filter(|v| truthy(v))
        .or_else(|| data.get("total").filter(|v| truthy(v)))
        .and_then(Value::as_i64)
        .unwrap_or(items.len() as i64);
    Ok((items, total))
}

fn search_resource(entity: &str) -> Option<&'static Resource> {
    match entity {
        "tickets" => Some(&TICKETS),
        "problems" => Some(&PROBLEMS),
        "changes" => Some(&CHANGES),
        _ => None,
    }
}

/// `_normalize_search_result`.
pub fn normalize_search_result(row: &Value, entity: &str) -> Option<Value> {
    static TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(\d+)$").expect("regex"));
    let resource = search_resource(entity)?;
    let (prefix, owner_field) = if row.get_truthy("ticket_path").is_some() {
        ("ticket", "responder_name")
    } else {
        ("itil_module", "owner_name")
    };
    let s = |k: &str| row.get_truthy(k).map(fsv_py::py_str).unwrap_or_default();
    let path = s(&format!("{prefix}_path"));
    let Some(id) = TAIL
        .captures(&path)
        .and_then(|m| fsv_core::pystr::parse_decimal(&m[1]))
    else {
        let mut out = row.as_object().cloned().unwrap_or_default();
        out.insert("_resource".into(), json!(resource.name));
        out.insert("_search_result".into(), json!(true));
        return Some(Value::Object(out));
    };
    let group_key = if prefix == "ticket" {
        "ticket_group"
    } else {
        "itil_module_group"
    };
    let subject = s("subject");
    Some(json!({
        "id": id,
        "display_id": id,
        "human_display_id": s(&format!("{prefix}_display_id")),
        "subject": fsv_core::output::strip_html(Some(&subject)),
        "status": s(&format!("{prefix}_status")),
        "priority_label": s(&format!("{prefix}_priority")),
        "requester": {"name": ""},
        "_agent": s(owner_field),
        "_group": s(group_key),
        "created_at": s("created_at"),
        "_resource": resource.name,
        "_search_result": true,
    }))
}

fn search_one(
    c: &dyn Api,
    entity: &str,
    term: &str,
    sort: Option<&str>,
    page: i64,
) -> Result<(Vec<Value>, Value)> {
    let data = c.fulltext_search(entity, term, page, sort)?;
    let items: Vec<Value> = rows_at(&data, "results")
        .iter()
        .filter_map(|r| normalize_search_result(r, entity))
        .collect();
    let total = data
        .get("total_entries")
        .cloned()
        .unwrap_or_else(|| json!(items.len()));
    Ok((items, total))
}

/// `search_items`: one entity, or tickets/problems/changes in parallel
/// (a failing entity is skipped). Totals keep server values as given.
pub fn search_items(
    c: &dyn Api,
    term: &str,
    entity: Option<&str>,
    sort: Option<&str>,
    page: i64,
) -> Result<(Vec<Value>, Map<String, Value>)> {
    if let Some(entity) = entity.filter(|e| search_resource(e).is_some()) {
        let (items, total) = search_one(c, entity, term, sort, page)?;
        let mut totals = Map::new();
        totals.insert(entity.into(), total);
        return Ok((items, totals));
    }
    let entities = ["tickets", "problems", "changes"];
    let results: Vec<Option<(Vec<Value>, Value)>> = std::thread::scope(|s| {
        let handles: Vec<_> = entities
            .iter()
            .map(|e| s.spawn(move || search_one(c, e, term, sort, page).ok()))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect()
    });
    let mut items = Vec::new();
    let mut totals = Map::new();
    for (entity, r) in entities.iter().zip(results) {
        if let Some((rows, total)) = r {
            items.extend(rows);
            totals.insert((*entity).into(), total);
        }
    }
    Ok((items, totals))
}

/// `list_work_items`: tickets, changes and problems merged, newest update first.
pub fn list_work_items(c: &dyn Api, per_page: i64, page: i64) -> (Vec<Value>, usize) {
    let args = ListArgs {
        page,
        per_page,
        ..Default::default()
    };
    let buckets: Vec<Vec<Value>> = std::thread::scope(|s| {
        let handles: Vec<_> = [&TICKETS, &CHANGES, &PROBLEMS]
            .into_iter()
            .map(|r| s.spawn(|| list_items(c, r, &args).map(|x| x.0).unwrap_or_default()))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let mut all: Vec<Value> = buckets.into_iter().flatten().collect();
    let key = |x: &Value| {
        x.get_truthy("updated_at")
            .map(fsv_py::py_str)
            .unwrap_or_default()
    };
    all.sort_by_key(|x| std::cmp::Reverse(key(x)));
    let n = all.len();
    (all, n)
}

// ---------------------------------------------------------------------------
// records

/// `get_item`; `includes` overrides the per-resource default.
pub fn get_item(
    c: &dyn Api,
    resource: &Resource,
    item_id: i64,
    includes: Option<&str>,
) -> Result<Value> {
    let include = match includes {
        Some(i) => Some(i),
        None if resource.name == "tickets" => Some("requester,stats,phone,feedback,ticket_status"),
        None if matches!(resource.name, "changes" | "problems") => Some("requester,stats"),
        None => None,
    };
    let params = include
        .filter(|i| !i.is_empty())
        .map(|i| obj(json!({"include": i})));
    let data = c.int_get(&format!("{}/{item_id}", resource.api_path), params)?;
    Ok(tag(unwrap_key(data, resource.item_key), resource))
}

pub fn get_activities(
    c: &dyn Api,
    resource: &Resource,
    item_id: i64,
    page: i64,
) -> Result<Vec<Value>> {
    let data = c.int_get(
        &format!("{}/{item_id}/activities", resource.api_path),
        Some(obj(json!({"page": page}))),
    )?;
    Ok(rows_at(&data, "activities"))
}

pub fn get_notes(
    c: &dyn Api,
    resource: &Resource,
    item_id: i64,
    page: i64,
    per_page: i64,
) -> Result<Vec<Value>> {
    if resource.name == "tickets" {
        let params =
            obj(json!({"page": page, "per_page": per_page, "include": "user,phone,feedback"}));
        let data = c.int_get(&format!("tickets/{item_id}/conversations"), Some(params))?;
        return Ok(rows_at(&data, "conversations"));
    }
    let params = obj(json!({"page": page, "per_page": per_page, "include": "user"}));
    let data = c.int_get(
        &format!("{}/{item_id}/notes", resource.api_path),
        Some(params),
    )?;
    Ok(rows_at(&data, "notes"))
}

pub fn get_tasks(c: &dyn Api, resource: &Resource, item_id: i64, page: i64) -> Result<Vec<Value>> {
    let data = c.int_get(
        &format!("{}/{item_id}/tasks", resource.api_path),
        Some(obj(json!({"page": page}))),
    )?;
    Ok(rows_at(&data, "tasks"))
}

pub fn get_approvals(c: &dyn Api, resource: &Resource, item_id: i64) -> Result<Vec<Value>> {
    let data = c.int_get(&format!("{}/{item_id}/approvals", resource.api_path), None)?;
    Ok(rows_at(&data, "approvals"))
}

pub fn get_ticket_approvals(c: &dyn Api, resource: &Resource, item_id: i64) -> Result<Vec<Value>> {
    get_approvals(c, resource, item_id)
}

pub fn get_assets(c: &dyn Api, resource: &Resource, item_id: i64) -> Result<Vec<Value>> {
    match resource.name {
        "changes" => crate::create::get_change_assets(c, item_id),
        "tickets" => Ok(rows_at(
            &c.int_get(
                &format!("{}/{item_id}/associated_assets", resource.api_path),
                None,
            )?,
            "associated_assets",
        )),
        _ => Ok(rows_at(
            &c.int_get(&format!("{}/{item_id}/assets", resource.api_path), None)?,
            "assets",
        )),
    }
}

fn rows_or_empty(r: Result<Value>, key: &str) -> Value {
    Value::Array(r.map(|d| rows_at(&d, key)).unwrap_or_default())
}

/// `get_associations`; tickets and problems report a failed part as empty.
pub fn get_associations(
    c: &dyn Api,
    resource: &Resource,
    item_id: i64,
) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    match resource.name {
        "tickets" => {
            out.insert(
                "changes".into(),
                rows_or_empty(
                    c.int_get(
                        &format!("tickets/{item_id}/changes"),
                        Some(obj(json!({"change_type": "change"}))),
                    ),
                    "changes",
                ),
            );
            out.insert(
                "problems".into(),
                rows_or_empty(
                    c.int_get(&format!("tickets/{item_id}/problems"), None),
                    "problems",
                ),
            );
        }
        "changes" => {
            return crate::associations::inventory(c, item_id, crate::associations::KINDS).map(obj);
        }
        "problems" => {
            out.insert(
                "changes".into(),
                rows_or_empty(
                    c.int_get(&format!("problems/{item_id}/changes"), None),
                    "changes",
                ),
            );
            out.insert(
                "incidents".into(),
                rows_or_empty(
                    c.int_get(&format!("problems/{item_id}/tickets"), None),
                    "tickets",
                ),
            );
        }
        _ => {}
    }
    Ok(out)
}

/// `get_requested_items`: each row merged with its `more_info` detail.
pub fn get_requested_items(c: &dyn Api, item_id: i64) -> Result<Vec<Value>> {
    let data = c.int_get(&format!("tickets/{item_id}/requested_items"), None)?;
    let mut result = Vec::new();
    for item in rows_at(&data, "requested_items") {
        let Some(rid) = item.get_truthy("id").map(fsv_py::py_str) else {
            result.push(item);
            continue;
        };
        let detail = c.int_get(
            &format!("tickets/{item_id}/requested_items/{rid}"),
            Some(obj(json!({"view": "more_info"}))),
        );
        match detail {
            Ok(detail) => {
                let detailed = unwrap_key(detail, "requested_item");
                let mut merged = item.as_object().cloned().unwrap_or_default();
                for (k, v) in detailed.as_object().into_iter().flatten() {
                    merged.insert(k.clone(), v.clone());
                }
                let mut inner = item.obj_or_empty("item").clone();
                for (k, v) in detailed.obj_or_empty("item") {
                    inner.insert(k.clone(), v.clone());
                }
                merged.insert("item".into(), Value::Object(inner));
                result.push(Value::Object(merged));
            }
            Err(_) => result.push(item),
        }
    }
    Ok(result)
}

pub const PLANNING_FIELD_TYPES: [&str; 5] = [
    "planning_field",
    "default_change_reason",
    "default_change_impact",
    "default_change_plan",
    "default_backout_plan",
];

/// `get_change_planning_field_definitions` over an already loaded schema.
pub fn get_change_planning_field_definitions(schema: &Value) -> Vec<Value> {
    schema
        .list_or_empty("fields")
        .iter()
        .filter(|f| {
            let t = f
                .get_truthy("field_type")
                .map(fsv_py::py_str)
                .unwrap_or_default();
            PLANNING_FIELD_TYPES.contains(&t.as_str())
        })
        .cloned()
        .collect()
}

pub fn get_change_planning_fields(c: &dyn Api, change_id: i64) -> Result<Vec<Value>> {
    Ok(rows_at(
        &c.int_get(&format!("changes/{change_id}/planning-fields"), None)?,
        "change_planning_fields",
    ))
}

pub fn planning_fields_by_name(fields: &[Value]) -> Map<String, Value> {
    let mut out = Map::new();
    for f in fields {
        if let Some(name) = f.get_truthy("name") {
            out.insert(fsv_py::py_str(name), f.clone());
        }
    }
    out
}

/// `extract_description_attachment_urls`: attachment links in a description,
/// relative ones made absolute on `domain`.
pub fn extract_description_attachment_urls(html: Option<&str>, domain: &str) -> Vec<String> {
    static URL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:https?://[^"'<>\s]+)?/helpdesk/attachments/(\d+)(?:\?[^"'<>\s]+)?"#)
            .expect("regex")
    });
    let text = fsv_core::pystr::html_unescape(html.unwrap_or(""));
    let mut urls: Vec<String> = Vec::new();
    for m in URL.find_iter(&text) {
        let url = m.as_str();
        let url = if url.starts_with('/') {
            format!("https://{domain}{url}")
        } else {
            url.to_string()
        };
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

/// `get_change_evidence`.
pub fn get_change_evidence(c: &dyn Api, change_id: i64) -> Result<Value> {
    let change = get_item(c, &CHANGES, change_id, None)?;
    let planning = get_change_planning_fields(c, change_id)?;
    let description = change.get_truthy("description").map(fsv_py::py_str);
    Ok(json!({
        "change": change,
        "planning_fields": planning,
        "planning_fields_by_name": planning_fields_by_name(&planning),
        "main_attachments": change.list_or_empty("attachments"),
        "description_attachment_urls": extract_description_attachment_urls(description.as_deref(), &c.domain()),
    }))
}

// ---------------------------------------------------------------------------
// writes

pub fn update_item(c: &dyn Api, resource: &Resource, item_id: i64, body: &Value) -> Result<Value> {
    Ok(unwrap_key(
        c.int_put(&format!("{}/{item_id}", resource.api_path), body)?,
        resource.item_key,
    ))
}

/// `add_note`: changes and problems carry `private`; tickets do not.
pub fn add_note(
    c: &dyn Api,
    resource: &Resource,
    item_id: i64,
    body: &str,
    public: bool,
) -> Result<Value> {
    let mut payload = obj(json!({"body": body}));
    if matches!(resource.name, "changes" | "problems") {
        payload.insert("private".into(), json!(!public));
    }
    Ok(unwrap_key(
        c.int_post(
            &format!("{}/{item_id}/notes", resource.api_path),
            &Value::Object(payload),
        )?,
        "note",
    ))
}

pub fn add_reply(c: &dyn Api, resource: &Resource, item_id: i64, body: &str) -> Result<Value> {
    Ok(unwrap_key(
        c.int_post(
            &format!("{}/{item_id}/reply", resource.api_path),
            &json!({"body": body}),
        )?,
        "conversation",
    ))
}

/// `resolve_status`: embedded status names, else the schema label, else `str(status)`.
pub fn resolve_status(item: &Value, schema: &Value) -> String {
    let sid = item.get("status").cloned().unwrap_or(Value::Null);
    if let Some(v) = item.get_truthy("status_name") {
        return fsv_py::py_str(v);
    }
    for key in ["change_status", "ticket_status", "problem_status"] {
        if let Some(v) = item.get_truthy(key).and_then(|o| o.get_truthy("name")) {
            return fsv_py::py_str(v);
        }
    }
    let label = crate::choice_label("status", &sid, schema);
    if label.is_empty() {
        fsv_py::py_str(&sid)
    } else {
        label
    }
}

/// `PRIORITY.get(item.get("priority") or 0, "-")`.
pub fn resolve_priority(item: &Value) -> String {
    let key = match item.get("priority").filter(|v| truthy(v)) {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        Some(Value::Bool(true)) => Some(1),
        Some(_) => None,
        None => Some(0),
    };
    key.and_then(|k| crate::PRIORITY.iter().find(|(id, _)| *id == k))
        .map_or("-", |(_, l)| l)
        .to_string()
}

pub fn item_url(resource: &Resource, item_id: i64, domain: &str) -> String {
    format!("https://{domain}/a/{}/{item_id}", resource.api_path)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use fsv_core::ApiError;

    /// A scripted response.
    #[derive(Clone, Debug)]
    pub enum Reply {
        Ok(Value),
        Status(u16),
        Fail(u16, Value),
        Session,
        Raw(u16, Vec<u8>),
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct Call {
        pub method: String,
        pub path: String,
        pub params: Option<Map<String, Value>>,
        pub body: Option<Value>,
        pub raw: Option<Vec<u8>>,
    }

    /// Routes `"METHOD path"` to queued replies; the last reply repeats.
    #[derive(Default)]
    pub struct FakeApi {
        routes: Mutex<Vec<(String, VecDeque<Reply>)>>,
        pub calls: Mutex<Vec<Call>>,
        pub cookies: Vec<String>,
    }

    impl FakeApi {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn on(self, route: &str, reply: Reply) -> Self {
            {
                let mut r = self.routes.lock().unwrap();
                if let Some((_, q)) = r.iter_mut().find(|(k, _)| k == route) {
                    q.push_back(reply);
                } else {
                    r.push((route.to_string(), VecDeque::from([reply])));
                }
            }
            self
        }

        pub fn ok(self, route: &str, v: Value) -> Self {
            self.on(route, Reply::Ok(v))
        }

        pub fn writes(&self) -> Vec<Call> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.method != "GET")
                .cloned()
                .collect()
        }

        pub fn gets(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.method == "GET")
                .map(|c| c.path.clone())
                .collect()
        }

        fn reply(&self, method: &str, path: &str) -> Reply {
            let key = format!("{method} {path}");
            let mut r = self.routes.lock().unwrap();
            match r.iter_mut().find(|(k, _)| *k == key) {
                Some((_, q)) if q.len() > 1 => q.pop_front().unwrap(),
                Some((_, q)) => q.front().cloned().unwrap(),
                None => Reply::Status(404),
            }
        }

        fn call(
            &self,
            method: &str,
            path: &str,
            params: Option<Map<String, Value>>,
            body: Option<Value>,
            raw: Option<Vec<u8>>,
        ) -> Result<Value> {
            self.calls.lock().unwrap().push(Call {
                method: method.into(),
                path: path.into(),
                params,
                body,
                raw,
            });
            match self.reply(method, path) {
                Reply::Ok(v) => Ok(v),
                Reply::Status(s) => Err(ApiError::new(s, json!({"description": "nope"})).into()),
                Reply::Fail(s, body) => Err(ApiError::new(s, body).into()),
                Reply::Session => Err(Error::session("expired")),
                Reply::Raw(_, b) => Ok(Value::String(String::from_utf8_lossy(&b).into_owned())),
            }
        }
    }

    impl Api for FakeApi {
        fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
            self.call("GET", path, params, None, None)
        }
        fn int_post(&self, path: &str, body: &Value) -> Result<Value> {
            self.call("POST", path, None, Some(body.clone()), None)
        }
        fn int_put(&self, path: &str, body: &Value) -> Result<Value> {
            self.call("PUT", path, None, Some(body.clone()), None)
        }
        fn int_patch(&self, path: &str, body: &Value) -> Result<Value> {
            self.call("PATCH", path, None, Some(body.clone()), None)
        }
        fn int_delete(
            &self,
            path: &str,
            body: Option<&Value>,
            params: Option<Map<String, Value>>,
        ) -> Result<Value> {
            self.call("DELETE", path, params, body.cloned(), None)
        }
        fn autocomplete(
            &self,
            kind: &str,
            query: &str,
            _params: Option<Map<String, Value>>,
        ) -> Result<Vec<Value>> {
            let v = self.call(
                "GET",
                &format!("autocomplete/{kind}"),
                Some(obj(json!({"q": query}))),
                None,
                None,
            )?;
            Ok(v.as_array().cloned().unwrap_or_default())
        }
        fn lookup_choices(&self, link: &str, query: &str) -> Result<Vec<Value>> {
            let v = self.call("GET", link, Some(obj(json!({"q": query}))), None, None)?;
            Ok(v.as_array().cloned().unwrap_or_default())
        }
        fn fulltext_search(
            &self,
            entity: &str,
            term: &str,
            _page: i64,
            _sort: Option<&str>,
        ) -> Result<Value> {
            self.call(
                "GET",
                &format!("search/{entity}"),
                Some(obj(json!({"term": term}))),
                None,
                None,
            )
        }
        fn get_csrf(&self) -> Result<String> {
            Ok("tok".into())
        }
        fn send_once(
            &self,
            method: &str,
            path: &str,
            body: Body,
            params: Option<Map<String, Value>>,
            _csrf: &str,
        ) -> Result<Value> {
            match body {
                Body::Json(v) => self.call(method, path, params, Some(v), None),
                Body::Raw(b) => self.call(
                    method,
                    path,
                    params,
                    serde_json::from_slice(&b).ok(),
                    Some(b),
                ),
                _ => self.call(method, path, params, None, None),
            }
        }
        fn fetch(&self, url: &str) -> Result<Response> {
            self.calls.lock().unwrap().push(Call {
                method: "FETCH".into(),
                path: url.into(),
                params: None,
                body: None,
                raw: None,
            });
            Ok(match self.reply("FETCH", url) {
                Reply::Raw(s, b) => Response::new(s).with_body(b),
                Reply::Status(s) => Response::new(s),
                _ => Response::new(404),
            })
        }
        fn domain(&self) -> String {
            "acme.freshservice.com".into()
        }
        fn workspace_cookies(&self) -> Vec<String> {
            self.cookies.clone()
        }
    }

    /// In-memory journal.
    #[derive(Default)]
    pub struct MemJournal {
        pub rows: Mutex<Vec<Value>>,
    }

    impl Journal for MemJournal {
        fn read(&self) -> Vec<Value> {
            self.rows.lock().unwrap().clone()
        }
        fn record_plan(
            &self,
            event: &str,
            plan: &str,
            action: &str,
            target: Option<&Value>,
            created_id: Option<&Value>,
        ) {
            self.rows.lock().unwrap().push(json!({
                "kind": format!("plan:{action}"), "event": event, "plan": plan,
                "target": target.cloned(), "created_id": created_id.cloned(),
            }));
        }
    }

    impl MemJournal {
        pub fn events(&self) -> Vec<String> {
            self.read()
                .iter()
                .filter_map(|r| r["event"].as_str().map(str::to_string))
                .collect()
        }
    }

    /// One field of the ticket form fixture (mirrors the oracle's `ticket_meta.f`).
    pub fn form_field(
        id: i64,
        name: &str,
        label: &str,
        field_type: &str,
        position: i64,
        extra: Value,
    ) -> Value {
        let default = field_type.starts_with("default_");
        let mut f = json!({"id": id, "name": name, "label": label, "description": "", "field_type": field_type,
                           "required": false, "required_for_closure": false, "position": position,
                           "default_field": default, "choices": [], "nested_fields": [],
                           "placeholder": "", "helptext": "", "hint": false,
                           "content": null, "displayed_to_agents": true, "data_source": 0,
                           "lookup_config": null, "workspace_id": if default { 1 } else { 2 },
                           "updated_at": "2030-01-02T03:04:05Z"});
        if let (Some(m), Value::Object(e)) = (f.as_object_mut(), extra) {
            m.extend(e);
        }
        f
    }

    /// The ticket edit form: subject, status (Open / Pending), planned dates.
    pub fn ticket_form(status_id: i64) -> Value {
        json!({"ticket_fields": [
            form_field(1, "subject", "Subject", "default_subject", 1, json!({})),
            form_field(2, "status", "Status", "default_status", 2, json!({"choices": [{"id": 2, "value": "Open"}, {"id": status_id, "value": "Pending"}]})),
            form_field(3, "planned_start_date", "Start", "default_planned_start_date", 3, json!({})),
            form_field(4, "planned_end_date", "End", "default_planned_end_date", 4, json!({})),
        ]})
    }

    /// A client with the ticket form, empty workspace-1 rules and ticket 123.
    pub fn ticket_api(ticket: Value) -> FakeApi {
        FakeApi::new()
            .ok("GET ticket_form_fields", ticket_form(3))
            .ok("GET ws/1/business_rules", json!({}))
            .ok("GET tickets/123", ticket)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    /// `change_approvals._id`, `change_associations._id` and `change_tasks._id`
    /// agree on every sample.
    #[test]
    fn positive_id_matches_python_id() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/coverage_oracle.json")).unwrap();
        for row in oracle["misc"]["ids"].as_array().unwrap() {
            let got = json!(positive_id(&row[0]));
            for want in &row.as_array().unwrap()[1..] {
                assert_eq!(got, *want, "{}", row[0]);
            }
        }
    }

    #[test]
    fn list_params_match_python() {
        let a = ListArgs {
            order_by: Some("created_at"),
            order_type: None,
            query_hash: Some("[]"),
            ..Default::default()
        };
        let p = list_params(&CHANGES, &a, true);
        let keys: Vec<&str> = p.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "per_page",
                "page",
                "include",
                "cache",
                "order_by",
                "order_type",
                "query_hash",
                "advanced_query_hash"
            ]
        );
        assert_eq!(p["order_type"], "desc");
        let a = ListArgs {
            query_hash: Some("[x]"),
            or_grouping: true,
            ..Default::default()
        };
        let p = list_params(&PROBLEMS, &a, false);
        assert_eq!(p["advanced_query_hash"], "[x]");
        assert_eq!(p["query_hash"], "");
    }

    #[test]
    fn search_rows_normalize() {
        let row = json!({"ticket_path": "/a/tickets/42", "ticket_display_id": "INC-42", "subject": "<b>Disk</b> &amp; CPU",
                         "ticket_status": "Open", "responder_name": "Ann", "ticket_group": "Ops"});
        let n = normalize_search_result(&row, "tickets").unwrap();
        assert_eq!(n["id"], 42);
        assert_eq!(n["subject"], "Disk  & CPU");
        assert_eq!(n["_agent"], "Ann");
        assert_eq!(n["_resource"], "tickets");
        let odd = normalize_search_result(&json!({"itil_module_path": "/x"}), "changes").unwrap();
        assert_eq!(odd["_search_result"], true);
        assert!(normalize_search_result(&row, "assets").is_none());
    }

    #[test]
    fn description_urls_absolute_and_unique() {
        let html = r#"<a href="/helpdesk/attachments/12?download=true">a</a> <img src="https://x.io/helpdesk/attachments/13"> /helpdesk/attachments/12?download=true"#;
        assert_eq!(
            extract_description_attachment_urls(Some(html), "acme.freshservice.com"),
            [
                "https://acme.freshservice.com/helpdesk/attachments/12?download=true",
                "https://x.io/helpdesk/attachments/13"
            ]
        );
    }

    #[test]
    fn status_and_priority_fallbacks() {
        assert_eq!(
            resolve_status(
                &json!({"status": 3, "change_status": {"name": "Planning"}}),
                &json!({})
            ),
            "Planning"
        );
        assert_eq!(resolve_status(&json!({"status": 9}), &json!({})), "9");
        assert_eq!(resolve_priority(&json!({"priority": 2})), "Medium");
        assert_eq!(resolve_priority(&json!({})), "-");
    }

    #[test]
    fn collect_pages_follows_page_length_and_flags() {
        let api = FakeApi::new()
            .ok("GET x/tasks", json!({"tasks": [{"id": 1}, {"id": 2}]}))
            .ok(
                "GET x/tasks",
                json!({"tasks": [{"id": 2}, {"id": 3}], "meta": {"has_more": false}}),
            );
        let params = obj(json!({"page": 1, "per_page": 2}));
        let p = collect_pages(
            |a, b| api.int_get(a, b),
            "x/tasks",
            "tasks",
            Some(&params),
            "",
            5,
        )
        .unwrap();
        assert!(p.complete());
        assert_eq!(p.rows.len(), 3);
        assert_eq!(p.meta["pages"], 2);

        let api = FakeApi::new().on("GET y", Reply::Status(403));
        let p = collect_pages(|a, b| api.int_get(a, b), "y", "rows", None, "Rows", 5).unwrap();
        assert!(!p.complete());
        assert_eq!(
            p.meta["partial"],
            "Rows: incomplete (0 loaded; Rows: not permitted for your role (403))"
        );
        assert_eq!(p.missing.unwrap().status, Some(403));

        let api = FakeApi::new().on("GET y", Reply::Session);
        assert!(collect_pages(|a, b| api.int_get(a, b), "y", "rows", None, "", 5).is_err());
    }

    #[test]
    fn collect_pages_total_and_links() {
        let api = FakeApi::new()
            .ok(
                "GET p",
                json!({"rows": [{"id": 1}], "next_page_url": "/api/_/p?page=2&per_page=1"}),
            )
            .ok("GET p", json!({"rows": [{"id": 2}], "next_page_url": null}));
        let p = collect_pages(|a, b| api.int_get(a, b), "p", "rows", None, "", 5).unwrap();
        assert!(p.complete());
        assert_eq!(
            api.calls.lock().unwrap()[1].params.as_ref().unwrap()["page"],
            "2"
        );

        let api = FakeApi::new().ok("GET q", json!({"rows": [], "meta": {"total_count": 3}}));
        let p = collect_pages(|a, b| api.int_get(a, b), "q", "rows", None, "", 5).unwrap();
        assert!(
            p.meta["partial"]
                .as_str()
                .unwrap()
                .ends_with("empty page before reported total)")
        );

        let api = FakeApi::new().ok(
            "GET r",
            json!({"rows": [{"id": 1}], "next_page_url": "/other?page=2"}),
        );
        let p = collect_pages(|a, b| api.int_get(a, b), "r", "rows", None, "", 5).unwrap();
        assert!(
            p.meta["partial"]
                .as_str()
                .unwrap()
                .contains("unsupported continuation endpoint")
        );
    }

    #[test]
    fn requested_items_merge_detail() {
        let api = FakeApi::new()
            .ok(
                "GET tickets/5/requested_items",
                json!({"requested_items": [{"id": 9, "item": {"a": 1}}, {"name": "x"}]}),
            )
            .ok(
                "GET tickets/5/requested_items/9",
                json!({"requested_item": {"quantity": 2, "item": {"b": 2}}}),
            );
        let rows = get_requested_items(&api, 5).unwrap();
        assert_eq!(
            rows[0],
            json!({"id": 9, "item": {"a": 1, "b": 2}, "quantity": 2})
        );
        assert_eq!(rows[1], json!({"name": "x"}));
    }

    fn get_calls(api: &FakeApi) -> Vec<Value> {
        api.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| json!([c.method, c.path, c.params]))
            .collect()
    }

    #[test]
    fn list_page_params_and_tags() {
        let api = FakeApi::new().ok(
            "GET changes",
            json!({"changes": [{"id": 1}], "meta": {"total_count": 1}}),
        );
        let args = ListArgs {
            filter_name: Some("mine"),
            order_by: Some("created_at"),
            order_type: Some(""),
            ..Default::default()
        };
        let (items, data) = list_page(&api, &CHANGES, &args).unwrap();
        assert_eq!(items, [json!({"id": 1, "_resource": "changes"})]);
        assert_eq!(
            data,
            json!({"changes": [{"id": 1}], "meta": {"total_count": 1}})
        );
        assert_eq!(
            get_calls(&api),
            [
                json!(["GET", "changes", {"per_page": 30, "page": 1, "include": "change_status,agent,requester,group,department", "cache": "true", "filter": "mine", "order_by": "created_at", "order_type": "desc"}])
            ]
        );
        let api = FakeApi::new().ok("GET tickets", json!("not a dict"));
        assert_eq!(
            list_page(&api, &TICKETS, &ListArgs::default()).unwrap(),
            (vec![], json!({}))
        );
    }

    #[test]
    fn list_params_ticket_query() {
        let a = ListArgs {
            page: 2,
            per_page: 10,
            filter_name: Some("f"),
            order_by: Some("updated_at"),
            order_type: None,
            query_hash: Some("[q]"),
            or_grouping: false,
        };
        assert_eq!(
            Value::Object(list_params(&TICKETS, &a, true)),
            json!({"per_page": 10, "page": 2, "include": "stats,responder,requester,ticket_states,ticket_status,group", "cache": "true", "filter": "f", "order_by": "updated_at", "order_type": "desc", "query_hash": "[q]", "advanced_query_hash": ""})
        );
    }

    #[test]
    fn list_items_totals() {
        let api = FakeApi::new().ok(
            "GET problems",
            json!({"problems": [{"id": 1}, {"id": 2}], "meta": {"total_count": 0}, "total": 7}),
        );
        let (items, total) = list_items(
            &api,
            &PROBLEMS,
            &ListArgs {
                query_hash: Some("[x]"),
                or_grouping: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((items.len(), total), (2, 7));
        assert_eq!(items[0], json!({"id": 1, "_resource": "problems"}));
        assert_eq!(
            get_calls(&api)[0][2],
            json!({"per_page": 30, "page": 1, "include": "problem_status,agent,requester,group", "cache": "true", "advanced_query_hash": "[x]", "query_hash": ""})
        );
        let api = FakeApi::new().ok("GET changes", json!({"changes": [{"id": 1}]}));
        let (_, total) = list_items(
            &api,
            &CHANGES,
            &ListArgs {
                order_by: Some("id"),
                order_type: Some("asc"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(total, 1);
        assert_eq!(get_calls(&api)[0][2]["order_type"], "asc");
    }

    #[test]
    fn normalize_change_search_row() {
        let row = json!({"itil_module_path": "/a/changes/7", "itil_module_display_id": "CHN-7", "subject": "Patch",
                         "itil_module_priority": "High", "owner_name": "Bo", "itil_module_group": "Net", "created_at": "2026-01-01"});
        assert_eq!(
            normalize_search_result(&row, "changes").unwrap(),
            json!({"id": 7, "display_id": 7, "human_display_id": "CHN-7", "subject": "Patch", "status": "", "priority_label": "High", "requester": {"name": ""}, "_agent": "Bo", "_group": "Net", "created_at": "2026-01-01", "_resource": "changes", "_search_result": true})
        );
        assert_eq!(
            normalize_search_result(&json!({"itil_module_path": "/x"}), "changes").unwrap(),
            json!({"itil_module_path": "/x", "_resource": "changes", "_search_result": true})
        );
    }

    #[test]
    fn search_items_one_entity_and_all() {
        let api = FakeApi::new().ok("GET search/changes", json!({"results": [{"itil_module_path": "/a/changes/7", "subject": "x"}], "total_entries": 9}));
        let (items, totals) = search_items(&api, "x", Some("changes"), None, 1).unwrap();
        assert_eq!(items[0]["id"], 7);
        assert_eq!(Value::Object(totals), json!({"changes": 9}));
        let api = FakeApi::new()
            .ok(
                "GET search/tickets",
                json!({"results": [{"ticket_path": "/a/tickets/1"}]}),
            )
            .on("GET search/problems", Reply::Status(500))
            .ok("GET search/changes", json!({"results": []}));
        let (items, totals) = search_items(&api, "x", None, None, 1).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["_resource"], "tickets");
        assert_eq!(Value::Object(totals), json!({"tickets": 1, "changes": 0}));
    }

    #[test]
    fn work_items_newest_first() {
        let api = FakeApi::new()
            .ok(
                "GET tickets",
                json!({"tickets": [{"id": 1, "updated_at": "2026-01-02"}]}),
            )
            .ok(
                "GET changes",
                json!({"changes": [{"id": 2, "updated_at": "2026-01-03"}, {"id": 3}]}),
            )
            .on("GET problems", Reply::Status(500));
        let (items, n) = list_work_items(&api, 30, 1);
        assert_eq!(n, 3);
        assert_eq!(
            items,
            [
                json!({"id": 2, "updated_at": "2026-01-03", "_resource": "changes"}),
                json!({"id": 1, "updated_at": "2026-01-02", "_resource": "tickets"}),
                json!({"id": 3, "_resource": "changes"})
            ]
        );
    }

    #[test]
    fn get_item_default_includes() {
        let api = FakeApi::new()
            .ok("GET tickets/5", json!({"ticket": {"id": 5}}))
            .ok("GET changes/6", json!({"id": 6}))
            .ok("GET problems/7", json!({"problem": {"id": 7}}));
        assert_eq!(
            get_item(&api, &TICKETS, 5, None).unwrap(),
            json!({"id": 5, "_resource": "tickets"})
        );
        assert_eq!(
            get_item(&api, &CHANGES, 6, None).unwrap(),
            json!({"id": 6, "_resource": "changes"})
        );
        assert_eq!(
            get_item(&api, &PROBLEMS, 7, Some("")).unwrap(),
            json!({"id": 7, "_resource": "problems"})
        );
        assert_eq!(
            get_calls(&api),
            [
                json!(["GET", "tickets/5", {"include": "requester,stats,phone,feedback,ticket_status"}]),
                json!(["GET", "changes/6", {"include": "requester,stats"}]),
                json!(["GET", "problems/7", null])
            ]
        );
    }

    #[test]
    fn record_tab_reads() {
        let api = FakeApi::new()
            .ok(
                "GET changes/3/activities",
                json!({"activities": [{"id": 1}]}),
            )
            .ok(
                "GET tickets/3/conversations",
                json!({"conversations": [{"id": 2}]}),
            )
            .ok("GET changes/3/notes", json!({"notes": [{"id": 3}]}))
            .ok("GET changes/3/tasks", json!({"tasks": [{"id": 4}]}))
            .ok("GET changes/3/approvals", json!({"approvals": [{"id": 5}]}))
            .ok("GET tickets/3/approvals", json!({"approvals": [{"id": 6}]}));
        assert_eq!(
            get_activities(&api, &CHANGES, 3, 2).unwrap(),
            [json!({"id": 1})]
        );
        assert_eq!(
            get_notes(&api, &TICKETS, 3, 1, 30).unwrap(),
            [json!({"id": 2})]
        );
        assert_eq!(
            get_notes(&api, &CHANGES, 3, 2, 5).unwrap(),
            [json!({"id": 3})]
        );
        assert_eq!(get_tasks(&api, &CHANGES, 3, 1).unwrap(), [json!({"id": 4})]);
        assert_eq!(
            get_approvals(&api, &CHANGES, 3).unwrap(),
            [json!({"id": 5})]
        );
        assert_eq!(
            get_ticket_approvals(&api, &TICKETS, 3).unwrap(),
            [json!({"id": 6})]
        );
        assert_eq!(
            get_calls(&api),
            [
                json!(["GET", "changes/3/activities", {"page": 2}]),
                json!(["GET", "tickets/3/conversations", {"page": 1, "per_page": 30, "include": "user,phone,feedback"}]),
                json!(["GET", "changes/3/notes", {"page": 2, "per_page": 5, "include": "user"}]),
                json!(["GET", "changes/3/tasks", {"page": 1}]),
                json!(["GET", "changes/3/approvals", null]),
                json!(["GET", "tickets/3/approvals", null])
            ]
        );
    }

    #[test]
    fn assets_per_resource() {
        let api = FakeApi::new()
            .ok(
                "GET tickets/3/associated_assets",
                json!({"associated_assets": [{"id": 1}]}),
            )
            .ok("GET problems/3/assets", json!({"assets": [{"id": 2}]}))
            .ok(
                "GET changes/3/assets",
                json!({"assets": [{"display_id": 9}]}),
            );
        assert_eq!(get_assets(&api, &TICKETS, 3).unwrap(), [json!({"id": 1})]);
        assert_eq!(get_assets(&api, &PROBLEMS, 3).unwrap(), [json!({"id": 2})]);
        assert_eq!(get_assets(&api, &CHANGES, 3).unwrap(), Vec::<Value>::new());
        assert_eq!(
            api.gets(),
            [
                "tickets/3/associated_assets",
                "problems/3/assets",
                "changes/3/assets",
                "changes/3/associated-cis",
                "changes/3/associated-cis",
                "changes/3/associated-cis"
            ]
        );
    }

    #[test]
    fn associations_per_resource() {
        let api = FakeApi::new()
            .ok("GET tickets/3/changes", json!({"changes": [{"id": 1}]}))
            .on("GET tickets/3/problems", Reply::Status(403))
            .ok("GET problems/3/changes", json!({"changes": [{"id": 2}]}))
            .ok("GET problems/3/tickets", json!({"tickets": [{"id": 3}]}));
        assert_eq!(
            Value::Object(get_associations(&api, &TICKETS, 3).unwrap()),
            json!({"changes": [{"id": 1}], "problems": []})
        );
        assert_eq!(
            Value::Object(get_associations(&api, &PROBLEMS, 3).unwrap()),
            json!({"changes": [{"id": 2}], "incidents": [{"id": 3}]})
        );
        let api = FakeApi::new()
            .ok(
                "GET changes/3/tickets",
                json!({"tickets": [{"display_id": 11}]}),
            )
            .ok("GET changes/3/problems", json!({"problems": []}))
            .on("GET changes/3/releases", Reply::Status(404));
        let out = Value::Object(get_associations(&api, &CHANGES, 3).unwrap());
        assert_eq!(out["complete"], false);
        assert_eq!(out["problems"], json!([]));
        assert_eq!(
            out["warnings"],
            json!([
                "tickets: incomplete (0 loaded; tickets: unavailable)",
                "releases: incomplete (0 loaded; releases: not available on this account (404))"
            ])
        );
    }

    #[test]
    fn requested_items_detail_failure_keeps_row() {
        let api = FakeApi::new()
            .ok(
                "GET tickets/5/requested_items",
                json!({"requested_items": [{"id": 9, "item": {"a": 1}}, {"name": "x"}, {"id": 8}]}),
            )
            .ok(
                "GET tickets/5/requested_items/9",
                json!({"requested_item": {"quantity": 2, "item": {"b": 2}}}),
            )
            .on("GET tickets/5/requested_items/8", Reply::Status(500));
        assert_eq!(
            get_requested_items(&api, 5).unwrap(),
            [
                json!({"id": 9, "item": {"a": 1, "b": 2}, "quantity": 2}),
                json!({"name": "x"}),
                json!({"id": 8})
            ]
        );
    }

    #[test]
    fn planning_definitions_fields_and_names() {
        let schema = json!({"fields": [{"name": "a", "field_type": "planning_field"}, {"name": "b", "field_type": "custom_text"},
                                       {"name": "c", "field_type": "default_backout_plan"}, {"name": "d"}]});
        assert_eq!(
            get_change_planning_field_definitions(&schema),
            [
                json!({"name": "a", "field_type": "planning_field"}),
                json!({"name": "c", "field_type": "default_backout_plan"})
            ]
        );
        let api = FakeApi::new().ok(
            "GET changes/4/planning-fields",
            json!({"change_planning_fields": [{"name": "rollout"}, {"id": 2}]}),
        );
        assert_eq!(
            get_change_planning_fields(&api, 4).unwrap(),
            [json!({"name": "rollout"}), json!({"id": 2})]
        );
        let by = planning_fields_by_name(&[
            json!({"name": "a", "x": 1}),
            json!({"name": ""}),
            json!({"name": 5}),
        ]);
        assert_eq!(
            Value::Object(by),
            json!({"a": {"name": "a", "x": 1}, "5": {"name": 5}})
        );
    }

    #[test]
    fn description_urls_unescape_entities() {
        let html = r#"<a href="/helpdesk/attachments/12?download=true">a</a> <img src="https://x.io/helpdesk/attachments/13"> /helpdesk/attachments/12?download=true &lt;/helpdesk/attachments/14&gt;"#;
        assert_eq!(
            extract_description_attachment_urls(Some(html), "acme.freshservice.com"),
            [
                "https://acme.freshservice.com/helpdesk/attachments/12?download=true",
                "https://x.io/helpdesk/attachments/13",
                "https://acme.freshservice.com/helpdesk/attachments/14"
            ]
        );
    }

    #[test]
    fn change_evidence_bundle() {
        let api = FakeApi::new()
            .ok("GET changes/4", json!({"change": {"id": 4, "description": "<a href=\"/helpdesk/attachments/1\">x</a>", "attachments": [{"id": 1}]}}))
            .ok("GET changes/4/planning-fields", json!({"change_planning_fields": [{"name": "rollout", "id": 2}]}));
        assert_eq!(
            get_change_evidence(&api, 4).unwrap(),
            json!({"change": {"id": 4, "description": "<a href=\"/helpdesk/attachments/1\">x</a>", "attachments": [{"id": 1}], "_resource": "changes"},
                   "planning_fields": [{"name": "rollout", "id": 2}], "planning_fields_by_name": {"rollout": {"name": "rollout", "id": 2}},
                   "main_attachments": [{"id": 1}], "description_attachment_urls": ["https://acme.freshservice.com/helpdesk/attachments/1"]})
        );
    }

    #[test]
    fn update_reply_and_public_note() {
        let api = FakeApi::new()
            .ok(
                "PUT changes/3",
                json!({"change": {"id": 3, "subject": "s"}}),
            )
            .ok("POST tickets/3/reply", json!({"conversation": {"id": 8}}))
            .ok("POST problems/3/notes", json!({"id": 1}));
        assert_eq!(
            update_item(&api, &CHANGES, 3, &json!({"subject": "s"})).unwrap(),
            json!({"id": 3, "subject": "s"})
        );
        assert_eq!(
            add_reply(&api, &TICKETS, 3, "hi").unwrap(),
            json!({"id": 8})
        );
        assert_eq!(
            add_note(&api, &PROBLEMS, 3, "n", true).unwrap(),
            json!({"id": 1})
        );
        let bodies: Vec<Value> = api.writes().into_iter().map(|c| c.body.unwrap()).collect();
        assert_eq!(
            bodies,
            [
                json!({"subject": "s"}),
                json!({"body": "hi"}),
                json!({"body": "n", "private": false})
            ]
        );
    }

    #[test]
    fn status_names_schema_and_priority() {
        assert_eq!(
            resolve_status(&json!({"status": 3, "status_name": "Open"}), &json!({})),
            "Open"
        );
        assert_eq!(
            resolve_status(
                &json!({"status": 3, "ticket_status": {"name": "Pending"}}),
                &json!({})
            ),
            "Pending"
        );
        assert_eq!(
            resolve_status(
                &json!({"status": 2}),
                &json!({"fields": [{"name": "status", "choices": [{"id": 2, "value": "Planning"}]}]})
            ),
            "Planning"
        );
        assert_eq!(
            resolve_status(&json!({"status": null}), &json!({"fields": []})),
            "None"
        );
        assert_eq!(resolve_priority(&json!({"priority": 4})), "Urgent");
        assert_eq!(resolve_priority(&json!({"priority": 9})), "-");
        assert_eq!(
            item_url(&PROBLEMS, 12, "acme.freshservice.com"),
            "https://acme.freshservice.com/a/problems/12"
        );
    }

    #[test]
    fn notes_send_private_for_changes_only() {
        let api = FakeApi::new()
            .ok("POST changes/3/notes", json!({"note": {"id": 1}}))
            .ok("POST tickets/3/notes", json!({"id": 2}));
        assert_eq!(
            add_note(&api, &CHANGES, 3, "hi", false).unwrap(),
            json!({"id": 1})
        );
        add_note(&api, &TICKETS, 3, "hi", true).unwrap();
        let w = api.writes();
        assert_eq!(w[0].body, Some(json!({"body": "hi", "private": true})));
        assert_eq!(w[1].body, Some(json!({"body": "hi"})));
    }
}
