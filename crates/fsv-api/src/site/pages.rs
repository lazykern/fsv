//! Bounded multi-page reads (`views.collect_pages`).
//!
//! The caller performs HTTP. This decides which query to send next and when
//! to stop. Continuation URLs may only add query parameters for the same path.

use serde_json::{Map, Value, json};

use fsv_core::pystr;

use crate::site::list::Missing;

pub const MAX_DETAIL_PAGES: i64 = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct Collected {
    pub rows: Vec<Value>,
    pub meta: Map<String, Value>,
    pub failure: Option<Missing>,
}

/// `collect_pages`. `fetch` is `optional_get`: `Err` is a missing section,
/// not a session failure (the caller still raises those).
pub fn collect_pages<F>(path: &str, key: &str, params: &Map<String, Value>, what: &str, max_pages: i64, mut fetch: F) -> Collected
where
    F: FnMut(&str, &Map<String, Value>) -> Result<Value, Missing>,
{
    let origin = params.clone();
    let mut request = params.clone();
    let mut rows: Vec<Value> = Vec::new();
    let mut meta = Map::new();
    let mut seen_requests = Vec::new();
    let mut seen_ids = Vec::new();
    let mut failure = None;
    let mut reason = String::new();
    let mut complete = false;
    let mut pages = 0i64;
    let mut exhausted = true;
    let label = if what.is_empty() { key } else { what };

    for _ in 0..max_pages.max(0) {
        let signature = request_signature(&request);
        if seen_requests.iter().any(|s| s == &signature) {
            reason = "continuation cycle".to_string();
            exhausted = false;
            break;
        }
        seen_requests.push(signature);
        let data = match fetch(path, &request) {
            Ok(data) => data,
            Err(miss) => {
                reason = miss.note();
                failure = Some(miss);
                exhausted = false;
                break;
            }
        };
        if !readable(&data, key) {
            reason = "unreadable page response".to_string();
            failure = Some(Missing::new(label, None));
            exhausted = false;
            break;
        }
        let batch = rows_of(&data, key);
        pages += 1;
        for row in &batch {
            let identity = row.get("id").filter(|v| !v.is_null()).map(fsv_py::py_str);
            if identity.is_none() || identity.as_ref().is_some_and(|id| !seen_ids.iter().any(|s| s == id)) {
                if let Some(id) = identity {
                    seen_ids.push(id);
                }
                rows.push(Value::Object((*row).clone()));
            }
        }
        let raw = data.as_object();
        let empty = Map::new();
        let raw = raw.unwrap_or(&empty);
        let page_meta = raw.get("meta").and_then(Value::as_object).unwrap_or(&empty);
        for (k, v) in page_meta {
            meta.insert(k.clone(), v.clone());
        }
        let mut flag = first_flag(page_meta, raw).cloned();
        let link_present = raw.contains_key("next_page_url") || page_meta.contains_key("next_page_url");
        let link = get_with_default(raw, page_meta, "next_page_url").filter(|v| fsv_py::truthy(v)).cloned();
        let start = get_with_default(raw, page_meta, "next_start_token");
        let cursor = start
            .filter(|v| fsv_py::truthy(v))
            .or_else(|| get_with_default(raw, page_meta, "next_cursor").filter(|v| fsv_py::truthy(v)))
            .cloned();
        if flag.is_none() && !link_present && cursor.is_none() {
            if let Some(total) = nested_total(page_meta) {
                let offset = (int_key(&origin, "page", 1) - 1) * int_key(&origin, "per_page", 0);
                let more = offset + (rows.len() as i64) < total;
                if more && batch.is_empty() {
                    reason = "empty page before reported total".to_string();
                    exhausted = false;
                    break;
                }
                flag = Some(Value::Bool(more));
            }
        }
        let flag_false = matches!(flag, Some(Value::Bool(false)));
        let flag_true = matches!(flag, Some(Value::Bool(true)));
        if flag_false || (link_present && link.is_none() && cursor.is_none()) {
            complete = true;
            exhausted = false;
            break;
        }
        if let Some(link_val) = &link {
            let link_str = fsv_py::py_str(link_val);
            let parsed = match pystr::urlsplit(&link_str) {
                Ok(parsed) => parsed,
                Err(_) => {
                    reason = "unreadable continuation".to_string();
                    exhausted = false;
                    break;
                }
            };
            if !parsed.path.is_empty() {
                let plain = path.trim_end_matches('/');
                let api = format!("/api/_/{}", path.trim_matches('/'));
                let got = parsed.path.trim_end_matches('/');
                if got != plain && got != api.trim_end_matches('/') {
                    reason = "unsupported continuation endpoint".to_string();
                    exhausted = false;
                    break;
                }
            }
            let following = pystr::parse_qsl(&parsed.query, true);
            if following.is_empty() {
                reason = "unreadable continuation".to_string();
                exhausted = false;
                break;
            }
            request.retain(|k, _| !matches!(k.as_str(), "page" | "start_token" | "cursor"));
            for (k, v) in following {
                request.insert(k, Value::String(v));
            }
        } else if let Some(cursor) = cursor.clone() {
            let cursor_key = if start.is_some_and(|v| fsv_py::truthy(v)) { "start_token" } else { "cursor" };
            request.remove("page");
            request.insert(cursor_key.to_string(), cursor);
        } else if flag_true || per_page_full(&request, batch.len()) {
            if request.contains_key("start_token") || request.contains_key("cursor") {
                reason = "missing continuation cursor".to_string();
                exhausted = false;
                break;
            }
            let next = int_key(&request, "page", 1) + 1;
            request.insert("page".to_string(), json!(next));
        } else {
            complete = true;
            exhausted = false;
            break;
        }
    }
    if exhausted {
        reason = format!("page limit ({max_pages})");
    }
    meta.insert("loaded_count".to_string(), json!(rows.len() as i64));
    meta.insert("pages".to_string(), json!(pages));
    meta.insert("complete".to_string(), json!(complete));
    meta.insert("has_more".to_string(), json!(!complete));
    meta.insert("has_next".to_string(), json!(!complete));
    // Python sets `continuation` in the same update as the counters, then
    // assigns `partial`. Insertion order has to stay that way.
    meta.insert(
        "continuation".to_string(),
        if complete { Value::Null } else { Value::Object(request) },
    );
    if complete {
        meta.remove("partial");
    } else {
        meta.insert(
            "partial".to_string(),
            Value::String(format!("{label}: incomplete ({} loaded; {reason})", rows.len())),
        );
    }
    Collected { rows, meta, failure }
}

fn readable(data: &Value, key: &str) -> bool {
    match data {
        Value::Array(_) => true,
        Value::Object(map) => map.get(key).and_then(Value::as_array).is_some(),
        _ => false,
    }
}

fn rows_of<'a>(data: &'a Value, key: &str) -> Vec<&'a Map<String, Value>> {
    let items = match data {
        Value::Object(map) => map.get(key).and_then(Value::as_array),
        Value::Array(items) => Some(items),
        _ => None,
    };
    items.map(|items| items.iter().filter_map(Value::as_object).collect()).unwrap_or_default()
}

fn request_signature(request: &Map<String, Value>) -> String {
    let mut pairs: Vec<(&str, String)> = request.iter().map(|(k, v)| (k.as_str(), fsv_py::py_str(v))).collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
    let body = pairs.iter().map(|(k, v)| format!("({}, {})", fsv_py::py_repr_str(k), fsv_py::py_repr_str(v))).collect::<Vec<_>>().join(", ");
    format!("[{body}]")
}

fn first_flag<'a>(page_meta: &'a Map<String, Value>, raw: &'a Map<String, Value>) -> Option<&'a Value> {
    for source in [page_meta, raw] {
        for key in ["has_next", "has_more"] {
            if let Some(v) = source.get(key).filter(|v| !v.is_null()) {
                return Some(v);
            }
        }
    }
    None
}

fn get_with_default<'a>(primary: &'a Map<String, Value>, fallback: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    if primary.contains_key(key) { primary.get(key).filter(|v| !v.is_null()) } else { fallback.get(key).filter(|v| !v.is_null()) }
}

fn nested_total(page_meta: &Map<String, Value>) -> Option<i64> {
    let v = if page_meta.contains_key("total_count") { page_meta.get("total_count") } else { page_meta.get("count") };
    v.filter(|v| !v.is_null()).and_then(py_int)
}

fn int_key(map: &Map<String, Value>, key: &str, default: i64) -> i64 {
    match map.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => py_int(v).unwrap_or(default),
    }
}

fn per_page_full(request: &Map<String, Value>, batch: usize) -> bool {
    let Some(raw) = request.get("per_page").filter(|v| fsv_py::truthy(v)) else {
        return false;
    };
    py_int(raw).is_some_and(|n| batch as i64 >= n)
}

fn py_int(v: &Value) -> Option<i64> {
    match v {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok())).or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    fn run(path: &str, key: &str, params: Value, max_pages: i64, pages: Vec<Result<Value, Missing>>) -> Collected {
        let params = params.as_object().cloned().unwrap_or_default();
        let left = RefCell::new(pages);
        collect_pages(path, key, &params, "", max_pages, |_path, _req| {
            left.borrow_mut().pop().ok_or_else(|| Missing::new(key, Some(500))).map_err(|e| e).and_then(|r| r)
        })
    }

    #[test]
    fn page_limit_keeps_the_next_page() {
        let got = run(
            "changes/12/notes",
            "notes",
            json!({"page": 1}),
            1,
            vec![Ok(json!({"notes": [{"id": 1}], "meta": {"has_next": true}}))],
        );
        assert!(got.failure.is_none());
        assert_eq!(got.rows.len(), 1);
        assert_eq!(got.meta.get("complete"), Some(&json!(false)));
        assert_eq!(got.meta.get("continuation").and_then(|v| v.get("page")), Some(&json!(2)));
        assert!(got.meta.get("partial").and_then(Value::as_str).unwrap_or("").contains("page limit"));
    }

    #[test]
    fn count_walks_a_short_page() {
        let got = run(
            "changes/12/notes",
            "notes",
            json!({"per_page": 30}),
            5,
            vec![
                Ok(json!({"notes": [{"id": 2}], "meta": {"count": 2}})),
                Ok(json!({"notes": [{"id": 1}], "meta": {"count": 2}})),
            ],
        );
        assert_eq!(got.rows.iter().filter_map(|r| r.get("id")).collect::<Vec<_>>(), vec![&json!(1), &json!(2)]);
        assert_eq!(got.meta.get("complete"), Some(&json!(true)));
    }

    #[test]
    fn foreign_continuation_url_stops() {
        let got = run(
            "changes/12/activities",
            "activities",
            json!({}),
            5,
            vec![Ok(json!({"activities": [{"id": 1}], "next_page_url": "/api/v2/changes/12/activities?start_token=acme"}))],
        );
        assert_eq!(got.rows.len(), 1);
        assert_eq!(got.meta.get("complete"), Some(&json!(false)));
        assert!(got.meta.get("partial").and_then(Value::as_str).unwrap_or("").contains("unsupported continuation endpoint"));
    }
}
