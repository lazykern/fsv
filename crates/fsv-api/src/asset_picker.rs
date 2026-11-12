//! Read-only change CI/asset candidate selection; never associates a record
//! (`asset_picker.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, parse_id};

use crate::create::search_assets_for_change;
use crate::reviewed_write::refused;
use crate::service::Api;
use crate::watchers::positive_id;

/// Port of the dict fsv.asset_picker.candidates returns.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidates {
    pub change_id: i64,
    pub page: i64,
    pub scope: String,
    pub assets: Vec<Map<String, Value>>,
    pub mutation: String,
    pub warning: String,
}

impl Candidates {
    /// The result dict, keys in Python's order.
    pub fn to_json(&self) -> Value {
        json!({"change_id": self.change_id, "page": self.page, "scope": self.scope,
               "assets": self.assets, "mutation": self.mutation, "warning": self.warning})
    }
}

/// Port of fsv.asset_picker.candidates: one `assets-to-associate` search
/// page (30 rows) for the change.
pub fn candidates(c: &dyn Api, target: &Value, query: &str, page: i64) -> Result<Candidates> {
    let target = positive_id(&json!(parse_id(&fsv_py::py_str(target), &CHANGES)?))?;
    if page < 1 {
        return Err(Error::value("page must be positive"));
    }
    let raw = search_assets_for_change(c, target, query, page, 30)?;
    let rows = match raw.get("assets") {
        Some(Value::Array(rows)) if rows.iter().all(Value::is_object) => rows,
        _ => return Err(refused("asset candidate response is unavailable")),
    };
    let ids = rows
        .iter()
        .map(|row| positive_id(row.get("display_id").unwrap_or(&Value::Null)))
        .collect::<Result<Vec<_>>>()?;
    if ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id)) {
        return Err(refused(
            "duplicate asset display identities; selection is ambiguous",
        ));
    }
    Ok(Candidates {
        change_id: target,
        page,
        scope: "single search page; not a complete inventory".into(),
        assets: rows.iter().filter_map(|r| r.as_object().cloned()).collect(),
        mutation: "none".into(),
        warning: "Selection only. Asset/CI add-remove variants remain unsupported by this picker."
            .into(),
    })
}

/// Port of fsv.asset_picker.select: the one row with `display_id`.
pub fn select<'a>(result: &'a Candidates, display_id: &Value) -> Result<&'a Map<String, Value>> {
    let display_id = positive_id(display_id)?;
    let mut hits = Vec::new();
    for row in &result.assets {
        if positive_id(row.get("display_id").unwrap_or(&Value::Null))? == display_id {
            hits.push(row);
        }
    }
    match hits.as_slice() {
        [one] => Ok(one),
        _ => Err(refused(
            "exact asset display ID is not unique on this search page",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::*;

    fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
        r.unwrap_err().to_string()
    }

    fn api(assets: Value) -> FakeApi {
        FakeApi::new().ok("GET assets-to-associate", json!({ "assets": assets }))
    }

    /// Every display id is validated before duplicates are checked;
    /// messages from running `asset_picker.candidates` on these rows.
    #[test]
    fn candidates_validates_every_id_before_duplicates() {
        for (rows, want) in [
            (
                json!([{"display_id": 7}, {"display_id": 7}, {"display_id": "x"}]),
                "expected a positive numeric identity",
            ),
            (
                json!([{"display_id": 7}, {"display_id": "7"}, {"display_id": 0}]),
                "expected a positive numeric identity",
            ),
            (
                json!([{"display_id": "x"}, {"display_id": 7}, {"display_id": 7}]),
                "expected a positive numeric identity",
            ),
            (
                json!([{"display_id": 7}, {"display_id": "7"}]),
                "duplicate asset display identities; selection is ambiguous",
            ),
        ] {
            assert_eq!(
                err(candidates(&api(rows.clone()), &json!("CHN-5"), "q", 1)),
                want,
                "{rows}"
            );
        }
    }

    /// Expected values from running `asset_picker.candidates` / `select`
    /// on the same rows.
    #[test]
    fn candidates_search_one_page_read_only() {
        let rows =
            json!([{"display_id": 7, "name": "Acme DB"}, {"display_id": "8", "name": "Globex"}]);
        let c = api(rows.clone());
        let got = candidates(&c, &json!("CHN-5"), "acme", 1).unwrap();
        assert_eq!(
            got.to_json(),
            json!({"change_id": 5, "page": 1, "scope": "single search page; not a complete inventory",
                   "assets": rows, "mutation": "none",
                   "warning": "Selection only. Asset/CI add-remove variants remain unsupported by this picker."})
        );
        let calls = c.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path, "assets-to-associate");
        assert_eq!(
            calls[0].params.clone().map(Value::Object),
            Some(
                json!({"entity": "change", "entity_id": 5, "include_meta": "true", "page": 1,
                        "per_page": 30, "search_term": "acme"})
            )
        );
        drop(calls);
        assert_eq!(select(&got, &json!(8)).unwrap()["name"], json!("Globex"));
        assert_eq!(
            err(select(&got, &json!(9))),
            "exact asset display ID is not unique on this search page"
        );
        assert_eq!(
            err(select(&got, &json!(0))),
            "expected a positive numeric identity"
        );
    }

    #[test]
    fn candidates_refusals() {
        assert_eq!(
            err(candidates(&api(json!([])), &json!(5), "", 0)),
            "page must be positive"
        );
        assert_eq!(
            err(candidates(
                &api(json!([{"display_id": 7}, {"display_id": 7}])),
                &json!(5),
                "",
                1
            )),
            "duplicate asset display identities; selection is ambiguous"
        );
        assert_eq!(
            err(candidates(&api(json!([1])), &json!(5), "", 1)),
            "asset candidate response is unavailable"
        );
        let none = FakeApi::new().ok("GET assets-to-associate", json!({}));
        assert_eq!(
            err(candidates(&none, &json!(5), "", 1)),
            "asset candidate response is unavailable"
        );
        assert_eq!(
            err(candidates(
                &api(json!([{"display_id": null}])),
                &json!(5),
                "",
                1
            )),
            "expected a positive numeric identity"
        );
    }
}
