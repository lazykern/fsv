//! Read-only change CI/asset candidate selection; never associates a record
//! (`asset_picker.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, parse_id};

use crate::integrated_assets::search;
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

/// Port of fsv.asset_picker.candidates: one integrated-device search page.
/// The change id is checked and then unused; search is tenant-wide (`36408b2`).
pub fn candidates(c: &dyn Api, target: &Value, query: &str, page: i64) -> Result<Candidates> {
    let target = positive_id(&json!(parse_id(&fsv_py::py_str(target), &CHANGES)?))?;
    if page < 1 {
        return Err(Error::value("page must be positive"));
    }
    let raw = search(c, query, "devices", page)?;
    let data = raw
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| refused("asset candidate response is unavailable"))?;
    let mut rows = Vec::with_capacity(data.len());
    for row in data {
        let Some(name) = row.get("name").and_then(Value::as_object) else {
            return Err(refused("asset candidate response is unavailable"));
        };
        let display_id = positive_id(name.get("id").unwrap_or(&Value::Null))?;
        rows.push(json!({
            "display_id": display_id,
            "name": name.get("label").cloned().unwrap_or(Value::Null),
            "ci_type_name": row.get("type").cloned().unwrap_or(Value::Null),
            "inventory": row,
        }));
    }
    if rows.iter().any(|row| !row.is_object()) {
        return Err(refused("asset candidate response is unavailable"));
    }
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

    fn itam(data: Value, total: i64) -> FakeApi {
        FakeApi::new().ok(
            "GET https://acme.freshservice.com/itam/core/device/api/",
            json!({"page": 1, "error": null, "total_rows": total, "data": data}),
        )
    }

    fn device(id: i64, label: &str, pk: i64) -> Value {
        json!({"device_pk": pk, "name": {"id": id, "label": label}, "type": "Computer"})
    }

    /// Search rejects a bad identity before a later duplicate is considered.
    #[test]
    fn candidates_validates_every_id_before_duplicates() {
        for (rows, total, want) in [
            (
                json!([device(7, "a", 7), device(7, "b", 7), {"name": {"id": "x"}}]),
                3,
                "expected a positive numeric identity",
            ),
            (
                json!([device(7, "a", 7), {"name": {"id": "7"}}, {"name": {"id": 0}}]),
                3,
                "expected a positive numeric identity",
            ),
            (
                json!([{"name": {"id": "x"}}, device(7, "a", 7), device(7, "b", 7)]),
                3,
                "expected a positive numeric identity",
            ),
            (
                json!([device(7, "a", 7), {"device_pk": 7, "name": {"id": "7", "label": "b"}}]),
                2,
                "invalid inventory totals or duplicate identities",
            ),
        ] {
            assert_eq!(
                err(candidates(
                    &itam(rows.clone(), total),
                    &json!("CHN-5"),
                    "q",
                    1
                )),
                want,
                "{rows}"
            );
        }
    }

    /// Expected values from running `asset_picker.candidates` / `select`
    /// on the same rows.
    #[test]
    fn candidates_search_one_page_read_only() {
        let data = json!([device(7, "Acme DB", 7), device(8, "Globex", 8)]);
        let c = itam(data, 2);
        let got = candidates(&c, &json!("CHN-5"), "acme", 1).unwrap();
        assert_eq!(got.assets[0]["display_id"], json!(7));
        assert_eq!(got.assets[0]["name"], json!("Acme DB"));
        assert_eq!(got.assets[0]["ci_type_name"], json!("Computer"));
        assert_eq!(got.assets[0]["inventory"]["device_pk"], json!(7));
        assert_eq!(got.assets[1]["name"], json!("Globex"));
        assert_eq!(got.change_id, 5);
        assert!(got.scope.contains("not a complete inventory"));
        let calls = c.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].path.ends_with("/itam/core/device/api/"));
        assert_eq!(
            calls[0].params.clone().map(Value::Object),
            Some(json!({"order": "", "page": 1, "page_size": 50, "q": "acme"}))
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
            err(candidates(&itam(json!([]), 0), &json!(5), "", 0)),
            "page must be positive"
        );
        assert_eq!(
            err(candidates(
                &itam(json!([device(7, "a", 7), device(7, "b", 7)]), 2),
                &json!(5),
                "",
                1
            )),
            "invalid inventory totals or duplicate identities"
        );
        assert_eq!(
            err(candidates(&itam(json!([1]), 1), &json!(5), "", 1)),
            "integrated inventory identity unavailable"
        );
        let none = FakeApi::new().ok(
            "GET https://acme.freshservice.com/itam/core/device/api/",
            json!({}),
        );
        assert_eq!(
            err(candidates(&none, &json!(5), "", 1)),
            "integrated inventory unavailable"
        );
        assert_eq!(
            err(candidates(
                &itam(json!([{"name": {"id": null}}]), 1),
                &json!(5),
                "",
                1
            )),
            "expected a positive numeric identity"
        );
    }
}
