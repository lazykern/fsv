//! The reads behind a site page (`views.optional_get`, `collect_pages`,
//! conversations, requested items). HTTP is the caller's [`Api`].

use serde_json::{Map, Value, json};

use fsv_api::service::Api;
use fsv_api::site::{self, Collected, Missing, SavedView, oldest_first};
use fsv_core::client::{Body, Response};
use fsv_core::resource::Resource;
use fsv_core::{Error, Result as ApiResult};

/// An expired or rejected session. Every other failure becomes a [`Missing`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(pub String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionError {}

/// Why a read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Raised to the caller, never shown as a missing section.
    Session(String),
    /// HTTP status of an `APIError`; `None` for any other failure.
    Status(Option<i64>),
}

impl From<Error> for FetchError {
    /// `optional_get`'s `except` ladder: a `SessionError` stays a session
    /// error, an `APIError` keeps its status, anything else has none.
    fn from(e: Error) -> Self {
        match e {
            Error::Session(msg) => FetchError::Session(msg),
            Error::Api(api) => FetchError::Status(Some(i64::from(api.status))),
            _ => FetchError::Status(None),
        }
    }
}

/// The client [`load_detail`](crate::load_detail) and
/// [`load_tab`](crate::load_tab) read through: any [`Api`]. Kept for callers
/// that pass `&mut` a client; everything else takes `&dyn Api`.
pub trait Source {
    fn api(&self) -> &dyn Api;
}

impl<T: Api> Source for T {
    fn api(&self) -> &dyn Api {
        self
    }
}

/// `GET /api/_/<path>` (`c.int_get`), classified.
pub(crate) fn int_read(
    src: &dyn Api,
    path: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Value, FetchError> {
    src.int_get(path, params.cloned()).map_err(FetchError::from)
}

/// `GET /api/v2/<path>` (`c.v2_get`), classified.
pub(crate) fn v2_read(src: &dyn Api, path: &str) -> Result<Value, FetchError> {
    src.v2_get(path, None).map_err(FetchError::from)
}

pub use fsv_api::site::MAX_DETAIL_PAGES;
pub const MAX_CONVERSATION_PAGES: i64 = 50;

/// `optional_get`: any failure but an expired session is a [`Missing`].
pub fn optional_get(
    src: &dyn Api,
    path: &str,
    params: Option<&Map<String, Value>>,
    what: &str,
) -> Result<Result<Value, Missing>, SessionError> {
    let params = params.filter(|p| !p.is_empty());
    let label = if what.is_empty() { path } else { what };
    match int_read(src, path, params) {
        Ok(v) => Ok(Ok(v)),
        Err(FetchError::Session(msg)) => Err(SessionError(msg)),
        Err(FetchError::Status(status)) => Ok(Err(Missing::new(label, status))),
    }
}

/// Port of fsv.views.ticket_headers: the object rows of `tickets/headers`,
/// empty when the read fails or the payload has no header list.
pub fn ticket_headers(src: &dyn Api) -> Result<Vec<Value>, SessionError> {
    let Ok(data) = optional_get(src, "tickets/headers", None, "list headers")? else {
        return Ok(Vec::new());
    };
    Ok(match data.get("headers") {
        Some(Value::Array(rows)) => rows.iter().filter(|h| h.is_object()).cloned().collect(),
        _ => Vec::new(),
    })
}

/// `workspace_get`: `ws/<workspace>/<path>` first, the plain path when that 404s.
pub fn workspace_get(
    src: &dyn Api,
    path: &str,
    params: Option<&Map<String, Value>>,
    workspace: Option<&Value>,
    what: &str,
) -> Result<Result<Value, Missing>, SessionError> {
    if let Some(ws) = workspace.filter(|v| !v.is_null() && v.as_str() != Some("")) {
        let got = optional_get(
            src,
            &format!("ws/{}/{path}", fsv_py::py_str(ws)),
            params,
            what,
        )?;
        match &got {
            Err(miss) if miss.status == Some(404) => {}
            _ => return Ok(got),
        }
    }
    optional_get(src, path, params, what)
}

/// `collect_pages` over an [`Api`]; a session failure stops the walk and is returned.
pub fn collect_pages(
    src: &dyn Api,
    path: &str,
    key: &str,
    params: &Map<String, Value>,
    what: &str,
    max_pages: i64,
) -> Result<Collected, SessionError> {
    let label = if what.is_empty() { key } else { what };
    let mut session: Option<SessionError> = None;
    let got = site::collect_pages(path, key, params, what, max_pages, |p, request| {
        if session.is_some() {
            return Err(Missing::new(label, None));
        }
        let request = if request.is_empty() {
            None
        } else {
            Some(request)
        };
        match int_read(src, p, request) {
            Ok(v) => Ok(v),
            Err(FetchError::Session(msg)) => {
                session = Some(SessionError(msg));
                Err(Missing::new(label, None))
            }
            Err(FetchError::Status(status)) => Err(Missing::new(label, status)),
        }
    });
    match session {
        Some(err) => Err(err),
        None => Ok(got),
    }
}

/// `load_views`: the web's saved views for `res` in server order.
pub fn load_views(
    src: &dyn Api,
    res: &Resource,
) -> Result<(Vec<SavedView>, Option<Missing>), SessionError> {
    let Some(path) = res.filters_path.filter(|p| !p.is_empty()) else {
        return Ok((Vec::new(), None));
    };
    Ok(match optional_get(src, path, None, "saved views")? {
        Ok(data) => (site::parse_saved_views(&data), None),
        Err(miss) => (Vec::new(), Some(miss)),
    })
}

/// `_record_for_form`: [`fsv_api::views::record_for_form`].
pub use fsv_api::views::record_for_form;

/// `change_associations.AssociationReader`: a page whose rows lack a positive
/// id fails as a whole instead of being silently trimmed by the paginator.
pub struct AssociationReader<'a>(pub &'a dyn Api);

impl Api for AssociationReader<'_> {
    fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> ApiResult<Value> {
        let data = match self.0.int_get(path, params) {
            Ok(data) => data,
            Err(e @ Error::Api(_)) => return Err(e),
            Err(_) => return Err(Error::other("association page unavailable")),
        };
        let key = path.rsplit('/').next().unwrap_or(path);
        let ok = data.get(key).and_then(Value::as_array).is_some_and(|rows| {
            rows.iter()
                .all(|r| r.is_object() && positive_id(r.get("id")).is_some())
        });
        if !data.is_object() || !ok {
            return Err(Error::other("unreadable association page"));
        }
        Ok(data)
    }
    fn int_post(&self, path: &str, body: &Value) -> ApiResult<Value> {
        self.0.int_post(path, body)
    }
    fn int_put(&self, path: &str, body: &Value) -> ApiResult<Value> {
        self.0.int_put(path, body)
    }
    fn int_patch(&self, path: &str, body: &Value) -> ApiResult<Value> {
        self.0.int_patch(path, body)
    }
    fn int_delete(
        &self,
        path: &str,
        body: Option<&Value>,
        params: Option<Map<String, Value>>,
    ) -> ApiResult<Value> {
        self.0.int_delete(path, body, params)
    }
    fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Option<Map<String, Value>>,
    ) -> ApiResult<Vec<Value>> {
        self.0.autocomplete(kind, query, params)
    }
    fn lookup_choices(&self, link: &str, query: &str) -> ApiResult<Vec<Value>> {
        self.0.lookup_choices(link, query)
    }
    fn fulltext_search(
        &self,
        entity: &str,
        term: &str,
        page: i64,
        sort: Option<&str>,
    ) -> ApiResult<Value> {
        self.0.fulltext_search(entity, term, page, sort)
    }
    fn get_csrf(&self) -> ApiResult<String> {
        self.0.get_csrf()
    }
    fn send_once(
        &self,
        method: &str,
        path: &str,
        body: Body,
        params: Option<Map<String, Value>>,
        csrf: &str,
    ) -> ApiResult<Value> {
        self.0.send_once(method, path, body, params, csrf)
    }
    fn fetch(&self, url: &str) -> ApiResult<Response> {
        self.0.fetch(url)
    }
    fn page(&self, path: &str) -> ApiResult<Response> {
        self.0.page(path)
    }
    fn domain(&self) -> String {
        self.0.domain()
    }
    fn workspace_cookies(&self) -> Vec<String> {
        self.0.workspace_cookies()
    }
    fn me(&self) -> ApiResult<Value> {
        self.0.me()
    }
    fn v2_get(&self, path: &str, params: Option<Map<String, Value>>) -> ApiResult<Value> {
        self.0.v2_get(path, params)
    }
}

fn positive_id(value: Option<&Value>) -> Option<u64> {
    let text = match value? {
        Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok().filter(|n| *n > 0)
}

/// `_rows`: the mappings under `key` (or the list itself).
pub fn rows_of(data: &Value, key: &str) -> Vec<Value> {
    let list = match data {
        Value::Object(map) => map.get(key).and_then(Value::as_array),
        Value::Array(items) => Some(items),
        _ => None,
    };
    list.map(|items| items.iter().filter(|x| x.is_object()).cloned().collect())
        .unwrap_or_default()
}

/// Conversations oldest first with their `meta`, and what could not be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversations {
    pub rows: Vec<Value>,
    pub meta: Map<String, Value>,
    pub missing: Option<Missing>,
}

/// `load_conversations`: the latest `per_page` by default (the web shows 3), every page with `all_pages`.
pub fn load_conversations(
    src: &dyn Api,
    ticket_id: &Value,
    all_pages: bool,
    per_page: i64,
) -> Result<Conversations, SessionError> {
    let path = format!("tickets/{}/conversations", fsv_py::py_str(ticket_id));
    let mut params = obj(json!({"include": "user,phone,feedback", "per_page": per_page}));
    if !all_pages {
        let data = match optional_get(src, &path, Some(&params), "conversations")? {
            Ok(data) => data,
            Err(miss) => {
                return Ok(Conversations {
                    rows: Vec::new(),
                    meta: Map::new(),
                    missing: Some(miss),
                });
            }
        };
        let rows = data
            .get("conversations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let meta = data
            .get("meta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        return Ok(Conversations {
            rows: oldest_first(&rows),
            meta,
            missing: None,
        });
    }
    params.insert("per_page".into(), json!(30));
    params.insert("page".into(), json!(1));
    let got = collect_pages(
        src,
        &path,
        "conversations",
        &params,
        "conversations",
        MAX_CONVERSATION_PAGES,
    )?;
    let mut meta = got.meta;
    let loaded = got.rows.len() as i64;
    let count = match meta.get("count").filter(|v| fsv_py::truthy(v)) {
        None => Some(0),
        Some(v) => py_int(v),
    };
    meta.insert(
        "count".into(),
        json!(count.map_or(loaded, |c| c.max(loaded))),
    );
    let mut partial = got.failure;
    let complete = meta.get("complete").is_some_and(fsv_py::truthy);
    if !complete {
        let pages = meta.get("pages").and_then(Value::as_i64).unwrap_or(0);
        partial = match partial {
            Some(miss) if pages != 0 => Some(Missing::new(
                format!(
                    "conversations after the first {loaded} (page {})",
                    pages + 1
                ),
                miss.status,
            )),
            Some(miss) => Some(miss),
            None => {
                let note = meta
                    .get("partial")
                    .map(fsv_py::py_str)
                    .unwrap_or_else(|| "None".into());
                Some(Missing::new(
                    format!("conversations after page {pages} ({note})"),
                    None,
                ))
            }
        };
    }
    Ok(Conversations {
        rows: oldest_first(&got.rows),
        meta,
        missing: partial,
    })
}

/// `service.get_requested_items`: each row merged with its `more_info` view
/// (a row whose detail cannot be read is kept as listed).
pub fn get_requested_items(src: &dyn Api, ticket_id: &Value) -> Result<Vec<Value>, FetchError> {
    let tid = fsv_py::py_str(ticket_id);
    let data = int_read(src, &format!("tickets/{tid}/requested_items"), None)?;
    let items = data
        .get("requested_items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let more = obj(json!({"view": "more_info"}));
    let mut out = Vec::new();
    for item in items {
        let Some(rid) = item.get("id").filter(|v| fsv_py::truthy(v)) else {
            out.push(item);
            continue;
        };
        let path = format!("tickets/{tid}/requested_items/{}", fsv_py::py_str(rid));
        let detailed = match int_read(src, &path, Some(&more)) {
            Ok(detail) => match detail.get("requested_item") {
                Some(inner) => inner.clone(),
                None => detail,
            },
            Err(_) => {
                out.push(item);
                continue;
            }
        };
        let (Some(base), Some(extra)) = (item.as_object(), detailed.as_object()) else {
            out.push(item);
            continue;
        };
        let mut merged = base.clone();
        for (k, v) in extra {
            merged.insert(k.clone(), v.clone());
        }
        let mut meta = base
            .get("item")
            .filter(|v| fsv_py::truthy(v))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(more_meta) = extra
            .get("item")
            .filter(|v| fsv_py::truthy(v))
            .and_then(Value::as_object)
        {
            for (k, v) in more_meta {
                meta.insert(k.clone(), v.clone());
            }
        }
        merged.insert("item".into(), Value::Object(meta));
        out.push(Value::Object(merged));
    }
    Ok(out)
}

/// `_requested_for`: `Self`, the person's name, or `#<id>`.
pub fn requested_for(src: &dyn Api, record: &Value) -> Result<Option<String>, SessionError> {
    let Some(rf) = record
        .get("requested_for_id")
        .filter(|v| !v.is_null() && v.as_str() != Some(""))
    else {
        return Ok(None);
    };
    let rf_s = fsv_py::py_str(rf);
    if rf_s
        == record
            .get("requester_id")
            .map(fsv_py::py_str)
            .unwrap_or_else(|| "None".into())
    {
        return Ok(Some("Self".into()));
    }
    if let Ok(data) = optional_get(src, &format!("users/{rf_s}"), None, "requested for")?
        && data.is_object()
    {
        let user = data.get("user").unwrap_or(&data);
        let nm = name_of(user);
        if !nm.is_empty() {
            return Ok(Some(nm));
        }
    }
    Ok(Some(format!("#{rf_s}")))
}

/// `views._name`.
pub fn name_of(obj: &Value) -> String {
    let Some(map) = obj.as_object() else {
        return String::new();
    };
    ["name", "value", "email"]
        .iter()
        .find_map(|k| map.get(*k).filter(|v| fsv_py::truthy(v)))
        .map(|v| fsv_core::pystr::strip(&fsv_py::py_str(v)).to_string())
        .unwrap_or_default()
}

pub(crate) fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// `int(v)`: JSON numbers (floats truncate) and base-10 strings.
pub(crate) fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::String(s) => fsv_core::pystr::strip(s).replace('_', "").parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use super::*;

    /// The [`Api`] methods a page never calls.
    macro_rules! read_only_api {
        () => {
            fn int_post(&self, path: &str, _: &Value) -> fsv_core::Result<Value> {
                unreachable!("int_post {path}")
            }
            fn int_put(&self, path: &str, _: &Value) -> fsv_core::Result<Value> {
                unreachable!("int_put {path}")
            }
            fn int_patch(&self, path: &str, _: &Value) -> fsv_core::Result<Value> {
                unreachable!("int_patch {path}")
            }
            fn int_delete(
                &self,
                path: &str,
                _: Option<&Value>,
                _: Option<Map<String, Value>>,
            ) -> fsv_core::Result<Value> {
                unreachable!("int_delete {path}")
            }
            fn autocomplete(
                &self,
                kind: &str,
                _: &str,
                _: Option<Map<String, Value>>,
            ) -> fsv_core::Result<Vec<Value>> {
                unreachable!("autocomplete {kind}")
            }
            fn lookup_choices(&self, link: &str, _: &str) -> fsv_core::Result<Vec<Value>> {
                unreachable!("lookup_choices {link}")
            }
            fn fulltext_search(
                &self,
                entity: &str,
                _: &str,
                _: i64,
                _: Option<&str>,
            ) -> fsv_core::Result<Value> {
                unreachable!("fulltext_search {entity}")
            }
            fn get_csrf(&self) -> fsv_core::Result<String> {
                unreachable!("get_csrf")
            }
            fn send_once(
                &self,
                _: &str,
                path: &str,
                _: fsv_core::client::Body,
                _: Option<Map<String, Value>>,
                _: &str,
            ) -> fsv_core::Result<Value> {
                unreachable!("send_once {path}")
            }
            fn fetch(&self, url: &str) -> fsv_core::Result<fsv_core::client::Response> {
                unreachable!("fetch {url}")
            }
            fn domain(&self) -> String {
                "example.freshservice.com".into()
            }
        };
    }
    pub(crate) use read_only_api;

    /// A scripted reply as the client raises it; [`FetchError::from`] maps it back.
    pub(crate) fn raised(reply: Result<Value, FetchError>) -> fsv_core::Result<Value> {
        reply.map_err(|e| match e {
            FetchError::Session(msg) => Error::session(msg),
            FetchError::Status(Some(status)) => Error::from(fsv_core::ApiError::new(
                u16::try_from(status).expect("HTTP status"),
                json!({}),
            )),
            FetchError::Status(None) => Error::other("boom"),
        })
    }

    /// Canned responses keyed by path; a missing path is a 404.
    #[derive(Default)]
    pub struct Canned {
        pub int: Vec<(String, Result<Value, FetchError>)>,
        pub v2: Vec<(String, Result<Value, FetchError>)>,
        pub calls: Mutex<Vec<String>>,
    }

    impl Canned {
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn reply(
            table: &[(String, Result<Value, FetchError>)],
            path: &str,
        ) -> fsv_core::Result<Value> {
            raised(
                table
                    .iter()
                    .find(|(p, _)| p == path)
                    .map(|(_, r)| r.clone())
                    .unwrap_or(Err(FetchError::Status(Some(404)))),
            )
        }
    }

    impl Api for Canned {
        fn int_get(&self, path: &str, _: Option<Map<String, Value>>) -> fsv_core::Result<Value> {
            self.calls.lock().unwrap().push(path.to_string());
            Canned::reply(&self.int, path)
        }
        fn v2_get(&self, path: &str, _: Option<Map<String, Value>>) -> fsv_core::Result<Value> {
            self.calls.lock().unwrap().push(format!("v2:{path}"));
            Canned::reply(&self.v2, path)
        }
        read_only_api!();
    }
}

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests {
    use super::fake::Canned;
    use super::*;

    #[test]
    fn client_errors_classify_like_optional_get() {
        assert_eq!(
            FetchError::from(Error::session("gone")),
            FetchError::Session("gone".into())
        );
        assert_eq!(
            FetchError::from(Error::from(fsv_core::ApiError::new(0, "x"))),
            FetchError::Status(Some(0))
        );
        assert_eq!(
            FetchError::from(Error::other("boom")),
            FetchError::Status(None)
        );
    }

    #[test]
    fn name_of_matches_python() {
        let got: Vec<String> = [
            json!({"name": " Ann "}),
            json!({"name": "", "value": "v"}),
            json!({"email": "e@x"}),
            json!({"name": 0, "value": 5}),
            json!("str"),
            json!({}),
        ]
        .iter()
        .map(name_of)
        .collect();
        assert_eq!(got, ["Ann", "v", "e@x", "5", "", ""]);
    }

    #[test]
    fn rows_of_matches_python() {
        assert_eq!(
            rows_of(&json!({"a": [{"x": 1}, 2, {"y": 2}]}), "a"),
            [json!({"x": 1}), json!({"y": 2})]
        );
        assert_eq!(rows_of(&json!([{"x": 1}, "z"]), "a"), [json!({"x": 1})]);
        assert!(rows_of(&json!({"a": {"x": 1}}), "a").is_empty());
        assert!(rows_of(&json!("s"), "a").is_empty());
    }

    #[test]
    fn workspace_get_falls_back_on_404() {
        let src = Canned {
            int: vec![("change_management/9".into(), Ok(json!({"ok": 1})))],
            ..Default::default()
        };
        let got = workspace_get(
            &src,
            "change_management/9",
            None,
            Some(&json!(3)),
            "lifecycle",
        )
        .unwrap();
        assert_eq!(got.unwrap(), json!({"ok": 1}));
        assert_eq!(
            src.calls(),
            ["ws/3/change_management/9", "change_management/9"]
        );
    }

    #[test]
    fn session_errors_propagate() {
        let src = Canned {
            int: vec![("x".into(), Err(FetchError::Session("expired".into())))],
            ..Default::default()
        };
        assert_eq!(optional_get(&src, "x", None, "").unwrap_err().0, "expired");
        let miss = optional_get(&src, "y", None, "").unwrap().unwrap_err();
        assert_eq!(miss.note(), "y: not available on this account (404)");
    }

    /// Each payload or failure Python's `ticket_headers` saw in
    /// `gen_fsv_views_extra.py`.
    #[test]
    fn ticket_headers_matches_python() {
        let golden: Value =
            serde_json::from_str(include_str!("../tests/fixtures/views_extra.json")).unwrap();
        for (name, case) in golden["ticket_headers"].as_object().unwrap() {
            let result = match name.as_str() {
                "rows" => Ok(
                    json!({"headers": [{"name": "subject"}, "x", 3, {"name": "status", "label": "Status"}]}),
                ),
                "not_list" => Ok(json!({"headers": "x"})),
                "no_key" => Ok(json!({"other": []})),
                "top_list" => Ok(json!([{"name": "subject"}])),
                "null" => Ok(Value::Null),
                "forbidden" => Err(FetchError::Status(Some(403))),
                "other_error" => Err(FetchError::Status(None)),
                "session" => Err(FetchError::Session("expired".into())),
                other => panic!("unknown case {other}"),
            };
            let src = Canned {
                int: vec![("tickets/headers".into(), result)],
                ..Default::default()
            };
            let got = match ticket_headers(&src) {
                Ok(rows) => Value::Array(rows),
                Err(e) => json!({"session_error": e.0}),
            };
            assert_eq!(got, case["out"], "{name}");
            assert_eq!(json!(src.calls()), case["calls"], "{name}");
        }
    }

    #[test]
    fn association_reader_rejects_bad_rows() {
        let inner = Canned {
            int: vec![
                (
                    "changes/1/tickets".into(),
                    Ok(json!({"tickets": [{"id": 4}, {"id": "x"}]})),
                ),
                (
                    "changes/1/problems".into(),
                    Ok(json!({"problems": [{"id": 4}]})),
                ),
            ],
            ..Default::default()
        };
        let reader = AssociationReader(&inner);
        assert_eq!(
            int_read(&reader, "changes/1/tickets", None),
            Err(FetchError::Status(None))
        );
        assert!(int_read(&reader, "changes/1/problems", None).is_ok());
    }

    #[test]
    fn requested_for_self_name_and_id() {
        let src = Canned {
            int: vec![("users/5".into(), Ok(json!({"user": {"name": " Ann "}})))],
            ..Default::default()
        };
        assert_eq!(
            requested_for(&src, &json!({"requested_for_id": 2, "requester_id": 2}))
                .unwrap()
                .as_deref(),
            Some("Self")
        );
        assert_eq!(
            requested_for(&src, &json!({"requested_for_id": 5, "requester_id": 2}))
                .unwrap()
                .as_deref(),
            Some("Ann")
        );
        assert_eq!(
            requested_for(&src, &json!({"requested_for_id": 6}))
                .unwrap()
                .as_deref(),
            Some("#6")
        );
        assert_eq!(requested_for(&src, &json!({})).unwrap(), None);
    }
}
