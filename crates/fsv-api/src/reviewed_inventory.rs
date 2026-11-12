//! Strict complete read inventories for mutation authorization and readback
//! (`reviewed_inventory.py`).
//!
//! The display paginator drops malformed rows and deduplicates. Writes check
//! each raw page first, before that information is lost. A rejected page
//! becomes the paginator's missing section, so every failure surfaces as
//! one refusal.

use std::cell::Cell;
use std::collections::BTreeSet;

use serde_json::{Map, Value};

use fsv_core::error::{Error, Result};
use fsv_py::py_str;

use crate::reviewed_write::refused;
use crate::service::{Api, MAX_DETAIL_PAGES, collect_pages};

const UNAVAILABLE: &str = "complete, consistent mutation inventory unavailable";

/// Which client read the pages go through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Internal,
    V2,
}

fn is_int(v: &Value) -> bool {
    matches!(v, Value::Number(n) if n.is_i64() || n.is_u64())
}

/// A positive id from an int or an ASCII digit string.
fn row_identity(row: &Value) -> Option<u64> {
    let text = match row.get("id")? {
        Value::Number(n) if n.is_u64() => n.to_string(),
        Value::Number(n) if n.is_i64() => return None,
        Value::String(s) => s.clone(),
        _ => return None,
    };
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok().filter(|n| *n > 0)
}

struct Checker<'a> {
    key: &'a str,
    seen: BTreeSet<u64>,
    totals: BTreeSet<i64>,
    page_totals: BTreeSet<i64>,
    requested: i64,
}

impl Checker<'_> {
    fn page(&mut self, raw: &Value, query: Option<&Map<String, Value>>) -> Result<()> {
        self.requested += 1;
        let Some(rows) = raw
            .get(self.key)
            .and_then(Value::as_array)
            .filter(|_| raw.is_object())
        else {
            return Err(refused("unreadable mutation inventory page"));
        };
        for row in rows {
            let Some(id) = row_identity(row).filter(|id| !self.seen.contains(id)) else {
                return Err(refused("invalid or duplicate mutation inventory identity"));
            };
            self.seen.insert(id);
        }
        let empty = Value::Object(Map::new());
        let meta = raw.get("meta").unwrap_or(&empty);
        if !meta.is_object() {
            return Err(refused("invalid mutation inventory metadata"));
        }
        let mut flags = BTreeSet::new();
        for source in [raw, meta] {
            for name in ["has_more", "has_next"] {
                match source.get(name) {
                    None | Some(Value::Null) => {}
                    Some(Value::Bool(b)) => {
                        flags.insert(*b);
                    }
                    Some(_) => return Err(refused("ambiguous inventory continuation flag")),
                }
            }
            if source.get("next_page").is_some_and(|v| !v.is_null()) {
                return Err(refused("unsupported next_page mutation continuation"));
            }
            for (name, floor) in [("page", 1), ("total_pages", 0)] {
                if let Some(v) = source.get(name)
                    && (!is_int(v) || v.as_i64().is_none_or(|n| n < floor))
                {
                    return Err(refused("invalid inventory page metadata"));
                }
            }
            if let Some(page) = source.get("page") {
                let wanted = match query.and_then(|q| q.get("page")) {
                    Some(v) => py_str(v)
                        .parse::<i64>()
                        .map_err(|e| Error::value(e.to_string()))?,
                    None => self.requested,
                };
                if page.as_i64() != Some(wanted) {
                    return Err(refused("inventory page identity disagrees"));
                }
            }
            if let Some(total_pages) = source.get("total_pages").and_then(Value::as_i64) {
                if total_pages == 0 && !rows.is_empty() {
                    return Err(refused("nonempty inventory with zero pages"));
                }
                self.page_totals.insert(total_pages.max(1));
            }
            for name in ["total", "total_count", "count"] {
                if let Some(v) = source.get(name) {
                    match v.as_i64().filter(|n| is_int(v) && *n >= 0) {
                        Some(n) => {
                            self.totals.insert(n);
                        }
                        None => return Err(refused("invalid mutation inventory total")),
                    }
                }
            }
        }
        if flags.len() > 1 {
            return Err(refused("conflicting inventory continuation flags"));
        }
        let mut continuing = Vec::new();
        for name in ["next_page_url", "next_start_token", "next_cursor"] {
            let values: Vec<&Value> = [raw, meta].iter().filter_map(|s| s.get(name)).collect();
            if values
                .iter()
                .any(|v| !v.is_null() && v.as_str().is_none_or(str::is_empty))
            {
                return Err(refused("invalid inventory continuation token"));
            }
            if values.len() == 2 && values[0] != values[1] {
                return Err(refused("conflicting inventory continuation tokens"));
            }
            if values.first().is_some_and(|v| fsv_py::truthy(v)) {
                continuing.push(name);
            }
        }
        let terminal = flags.len() == 1 && flags.contains(&false);
        let more = flags.len() == 1 && flags.contains(&true);
        if terminal && !continuing.is_empty() {
            return Err(refused("terminal inventory flag contradicts continuation"));
        }
        let empty_link = [raw, meta]
            .iter()
            .any(|s| s.get("next_page_url").is_some_and(|v| !fsv_py::truthy(v)));
        if more && empty_link && continuing.is_empty() {
            return Err(refused("missing claimed inventory continuation"));
        }
        Ok(())
    }
}

/// Port of `reviewed_inventory.complete_inventory`: every row, sorted by
/// id, after each raw page passed the identity, flag, page and total checks.
pub fn complete_inventory(
    c: &dyn Api,
    path: &str,
    key: &str,
    params: Option<&Map<String, Value>>,
    family: Family,
) -> Result<Vec<Value>> {
    complete_with(
        |route, query| match family {
            Family::V2 => c.v2_get(route, query),
            Family::Internal => c.int_get(route, query),
        },
        path,
        key,
        params,
    )
}

fn complete_with<F>(
    mut get: F,
    path: &str,
    key: &str,
    params: Option<&Map<String, Value>>,
) -> Result<Vec<Value>>
where
    F: FnMut(&str, Option<Map<String, Value>>) -> Result<Value>,
{
    let mut checker = Checker {
        key,
        seen: BTreeSet::new(),
        totals: BTreeSet::new(),
        page_totals: BTreeSet::new(),
        requested: 0,
    };
    let pages = collect_pages(
        |route, query| {
            let raw = get(route, query.clone())?;
            checker.page(&raw, query.as_ref())?;
            Ok(raw)
        },
        path,
        key,
        params,
        key,
        MAX_DETAIL_PAGES,
    )?;
    let meta = &pages.meta;
    let page_count = meta.get("pages").and_then(Value::as_i64);
    let totals_ok = checker.totals.is_empty()
        || (checker.totals.len() == 1 && checker.totals.contains(&(pages.rows.len() as i64)));
    let pages_ok = checker.page_totals.is_empty()
        || (checker.page_totals.len() == 1
            && page_count.is_some_and(|n| checker.page_totals.contains(&n)));
    if pages.missing.is_some()
        || !meta.get("complete").is_some_and(fsv_py::truthy)
        || meta.get("partial").is_some_and(fsv_py::truthy)
        || !totals_ok
        || !pages_ok
    {
        return Err(refused(UNAVAILABLE));
    }
    let mut rows = pages.rows;
    rows.sort_by_key(|r| row_identity(r).unwrap_or(0));
    Ok(rows)
}

/// Port of `reviewed_inventory.single_page_inventory`: a captured
/// unpaginated inventory, validated without any further wire request.
pub fn single_page_inventory(raw: &Value, key: &str) -> Result<Vec<Value>> {
    let count = raw
        .get(key)
        .and_then(Value::as_array)
        .filter(|_| raw.is_object())
        .map_or(0, Vec::len);
    let read = Cell::new(false);
    let mut params = Map::new();
    params.insert("page".into(), Value::from(1));
    params.insert("per_page".into(), Value::from(count as i64 + 1));
    complete_with(
        |_, _| {
            if read.replace(true) {
                return Err(refused("single-page inventory claims more rows"));
            }
            Ok(raw.clone())
        },
        "captured-inventory",
        key,
        Some(&params),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, LazyLock, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::service::testing::{Call, FnApi};

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/reviewed_inventory_oracle.json")).unwrap()
    });

    fn outcome(got: Result<Vec<Value>>) -> Value {
        match got {
            Ok(rows) => json!({"rows": rows}),
            Err(e) => json!({"error": e.to_string()}),
        }
    }

    fn paged(pages: Vec<Value>, family: &'static str) -> (FnApi, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let c = FnApi::new(move |call: &Call| {
            let want = if family == "v2" { "V2" } else { "GET" };
            assert_eq!(call.method, want);
            let params = call.params.clone().unwrap_or_default();
            log.lock().unwrap().push(json!([call.path, params]));
            let page = params.get("page").and_then(Value::as_i64).unwrap_or(1);
            Ok(pages[(page - 1) as usize].clone())
        });
        (c, seen)
    }

    #[test]
    fn cases_match_python() {
        for case in ORACLE["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let got = if case["single"] == json!(true) {
                single_page_inventory(&case["raw"], "rows")
            } else {
                let family = if case["api"] == json!("v2") {
                    "v2"
                } else {
                    "internal"
                };
                let pages = case["pages"].as_array().unwrap().clone();
                let (c, seen) = paged(pages, family);
                let params = case["params"].as_object().unwrap().clone();
                let fam = if family == "v2" {
                    Family::V2
                } else {
                    Family::Internal
                };
                let got = complete_inventory(&c, "example", "rows", Some(&params), fam);
                assert_eq!(json!(*seen.lock().unwrap()), case["requests"], "{name}");
                got
            };
            assert_eq!(outcome(got), case["outcome"], "{name}");
        }
    }
}
