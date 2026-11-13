//! Integrated inventory search and reviewed CI/service association contracts
//! (`integrated_assets.py`, Python `36408b2`).

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::resource::{CHANGES, PROBLEMS, TICKETS, parse_id};
use fsv_py::Dumps;

use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::watchers::positive_id;

const FAMILIES: &[(&str, &str)] = &[
    ("devices", "device"),
    ("assets", "asset"),
    ("services", "businessservice"),
];

fn family(kind: &str) -> Option<&'static str> {
    FAMILIES.iter().find(|(k, _)| *k == kind).map(|(_, v)| *v)
}

fn json_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) if n.is_i64() => n.as_i64(),
        Value::Number(n) if n.is_u64() => i64::try_from(n.as_u64()?).ok(),
        _ => None,
    }
}

fn nonneg_int(v: Option<&Value>) -> Option<i64> {
    json_int(v?).filter(|n| *n >= 0)
}

/// `(wire module, numeric id)`. `sr` is the tickets resource.
pub fn target_id(module: &str, target: &Value) -> Result<(&'static str, i64)> {
    let (resource, wire) = match module {
        "tickets" | "sr" => (&TICKETS, "tickets"),
        "changes" => (&CHANGES, "changes"),
        "problems" => (&PROBLEMS, "problems"),
        _ => return Err(Error::value("unsupported association module")),
    };
    let id = positive_id(&json!(parse_id(&fsv_py::py_str(target), resource)?))?;
    Ok((wire, id))
}

/// Port of `integrated_assets.search`.
pub fn search(c: &dyn Api, query: &str, kind: &str, page: i64) -> Result<Value> {
    let Some(family) = family(kind) else {
        return Err(Error::value(
            "choose devices, assets or services and a positive page",
        ));
    };
    if page < 1 {
        return Err(Error::value(
            "choose devices, assets or services and a positive page",
        ));
    }
    let url = format!("https://{}/itam/core/{family}/api/", c.domain());
    let params = obj(json!({"order": "", "page": page, "page_size": 50, "q": query}));
    let raw = c.absolute_get(&url, Some(params))?;
    let Some(obj) = raw.as_object() else {
        return Err(refused("integrated inventory unavailable"));
    };
    let out_of_range = obj
        .get("total_rows")
        .and_then(Value::as_i64)
        .filter(|total| {
            page > 1
                && obj.get("error").is_some_and(Value::is_string)
                && obj.get("data") == Some(&json!([]))
                && (0..=(page - 1) * 50).contains(total)
        });
    if let Some(total) = out_of_range {
        // Native out-of-range answer: {"error": "Requested page is out of range.", "data": [], "page": 1, ...}
        return Err(Error::value(format!(
            "no results on page {page}: the {kind} inventory has {total} row(s) ({} page(s) of 50)",
            std::cmp::max(1, (total + 49) / 50)
        )));
    }
    if obj.get("error").is_some_and(fsv_py::truthy) || !obj.get("data").is_some_and(Value::is_array)
    {
        return Err(refused("integrated inventory unavailable"));
    }
    let total = nonneg_int(obj.get("total_rows"));
    let page_ok = json_int(obj.get("page").unwrap_or(&Value::Null)) == Some(page);
    let Some(total) = total.filter(|_| page_ok) else {
        return Err(refused("integrated inventory pagination unavailable"));
    };
    let data = obj.get("data").and_then(Value::as_array).unwrap();
    let mut ids = Vec::with_capacity(data.len());
    for row in data {
        let Some(row) = row.as_object() else {
            return Err(refused("integrated inventory identity unavailable"));
        };
        let Some(name) = row.get("name").and_then(Value::as_object) else {
            return Err(refused("integrated inventory identity unavailable"));
        };
        let ident = positive_id(name.get("id").unwrap_or(&Value::Null))?;
        if let Some(pk) = row.get(&format!("{family}_pk")).filter(|v| !v.is_null())
            && positive_id(pk)? != ident
        {
            return Err(refused("conflicting inventory identities"));
        }
        ids.push(ident);
    }
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() || total < ids.len() as i64 {
        return Err(refused("invalid inventory totals or duplicate identities"));
    }
    let mut out = obj.clone();
    out.insert("kind".into(), json!(kind));
    out.insert(
        "complete".into(),
        json!(page == 1 && ids.len() as i64 == total),
    );
    Ok(Value::Object(out))
}

/// Port of `integrated_assets.inventory`: every associated device or service,
/// sorted by `item_id`. Physical asset writes are unproven.
pub fn inventory(c: &dyn Api, module: &str, target: &Value, kind: &str) -> Result<Vec<Value>> {
    let (module, target) = target_id(module, target)?;
    if kind != "devices" && kind != "services" {
        return Err(Error::value(
            "association kind must be devices or services; physical asset writes are unproven",
        ));
    }
    let association_type = if kind == "services" {
        "services"
    } else {
        "cis"
    };
    let category = if kind == "services" {
        "Business service"
    } else {
        "Device"
    };
    let mut rows = Vec::new();
    let mut totals = HashSet::new();
    let mut pages = HashSet::new();
    let mut finished = false;
    for page in 1..=100 {
        let params = obj(json!({"page": page, "association_type": association_type}));
        let raw = c.int_get(&format!("{module}/{target}/associated-cis"), Some(params))?;
        let Some(body) = raw.as_object() else {
            return Err(refused("association inventory unavailable"));
        };
        let Some(data) = body.get("data").and_then(Value::as_array) else {
            return Err(refused("association inventory unavailable"));
        };
        let Some(meta) = body.get("meta").and_then(Value::as_object) else {
            return Err(refused("association inventory unavailable"));
        };
        let Some(total_count) = nonneg_int(meta.get("total_count")) else {
            return Err(refused("association pagination unavailable"));
        };
        let Some(total_pages) = nonneg_int(meta.get("total_pages")) else {
            return Err(refused("association pagination unavailable"));
        };
        if meta.get("partial").is_some_and(fsv_py::truthy)
            || meta.get("complete") == Some(&Value::Bool(false))
        {
            return Err(refused("association inventory incomplete"));
        }
        totals.insert(total_count);
        pages.insert(total_pages);
        for row in data {
            let Some(row) = row.as_object() else {
                return Err(refused("invalid association row"));
            };
            positive_id(row.get("item_id").unwrap_or(&Value::Null))?;
            positive_id(row.get("itam_cmdb_request_id").unwrap_or(&Value::Null))?;
            if row.get("category").and_then(Value::as_str) != Some(category) {
                return Err(refused("unproven association category"));
            }
            rows.push(Value::Object(row.clone()));
        }
        if page >= total_pages {
            finished = true;
            break;
        }
        if data.is_empty() {
            return Err(refused("association page missing"));
        }
    }
    if !finished {
        return Err(refused("association page limit exceeded"));
    }
    for key in ["item_id", "itam_cmdb_request_id"] {
        let mut seen = HashSet::new();
        for row in &rows {
            let id = positive_id(row.get(key).unwrap_or(&Value::Null))?;
            if !seen.insert(id) {
                return Err(refused("duplicate association identities"));
            }
        }
    }
    if totals.len() != 1 || !totals.contains(&(rows.len() as i64)) || pages.len() != 1 {
        return Err(refused("association inventory total mismatch"));
    }
    rows.sort_by_key(|row| positive_id(row.get("item_id").unwrap_or(&Value::Null)).unwrap_or(0));
    Ok(rows)
}

pub const INTEGRATED: &str = "integrated";
pub const LEGACY: &str = "legacy";

fn legacy_read_path(module: &str, id: i64) -> Option<String> {
    match module {
        "changes" => Some(format!("changes/{id}/assets")),
        "tickets" => Some(format!("tickets/{id}/associated_assets")),
        _ => None,
    }
}

fn legacy_write_path(module: &str, id: i64) -> Option<String> {
    match module {
        "changes" => Some(format!("changes/{id}/assets/associate")),
        "tickets" => Some(format!("tickets/{id}/associate_assets")),
        _ => None,
    }
}

fn present(c: &dyn Api, path: &str, params: Option<Map<String, Value>>) -> Result<bool> {
    match c.int_get(path, params) {
        Ok(_) => Ok(true),
        Err(err) if err.status() == Some(404) => Ok(false),
        Err(err) => Err(err),
    }
}

/// Port of `integrated_assets.driver`. Read-only probes: 404 means absent.
pub fn driver(c: &dyn Api, module: &str, target: &Value, kind: &str) -> Result<&'static str> {
    let (module, target) = target_id(module, target)?;
    let association_type = if kind == "services" {
        "services"
    } else {
        "cis"
    };
    let params = obj(json!({"page": 1, "association_type": association_type}));
    if present(
        c,
        &format!("{module}/{target}/associated-cis"),
        Some(params),
    )? {
        return Ok(INTEGRATED);
    }
    if let Some(path) = legacy_read_path(module, target)
        && present(c, &path, None)?
    {
        return Ok(LEGACY);
    }
    Err(refused(format!(
        "no asset association endpoint found for {module} {target} (associated-cis and the legacy asset list both answered 404); nothing read or sent"
    )))
}

fn config_display_id(row: &Value) -> Result<i64> {
    let Some(item) = row.get("config_item").filter(|v| v.is_object()) else {
        return Err(refused("legacy asset association row unreadable"));
    };
    positive_id(item.get("display_id").unwrap_or(&Value::Null))
}

/// Port of `integrated_assets.legacy_inventory`.
pub fn legacy_inventory(c: &dyn Api, module: &str, target: &Value) -> Result<Vec<Value>> {
    let (module, target) = target_id(module, target)?;
    let Some(path) = legacy_read_path(module, target) else {
        return Err(refused("no legacy asset association for this module"));
    };
    let raw = c.int_get(&path, None)?;
    let Some(assets) = raw.get("associated_assets").and_then(Value::as_array) else {
        return Err(refused("legacy asset association list unreadable"));
    };
    if let Some(meta) = raw.get("meta").and_then(Value::as_object)
        && (meta.get("partial").is_some_and(fsv_py::truthy)
            || meta.get("has_more").is_some_and(fsv_py::truthy)
            || meta.get("next_page").is_some_and(fsv_py::truthy))
    {
        return Err(refused("legacy asset association list incomplete"));
    }
    let mut rows = Vec::with_capacity(assets.len());
    for row in assets {
        config_display_id(row)?;
        rows.push(row.clone());
    }
    let mut seen = HashSet::new();
    for row in &rows {
        if !seen.insert(config_display_id(row)?) {
            return Err(refused("duplicate legacy asset associations"));
        }
    }
    rows.sort_by_key(|row| config_display_id(row).unwrap_or(0));
    Ok(rows)
}

fn legacy_entity(module: &str) -> Option<&'static str> {
    match module {
        "changes" => Some("change"),
        "tickets" => Some("ticket"),
        _ => None,
    }
}

/// Port of `integrated_assets.legacy_candidates`.
pub fn legacy_candidates(c: &dyn Api, module: &str, target: &Value) -> Result<Vec<Value>> {
    let (module, target) = target_id(module, target)?;
    let Some(entity) = legacy_entity(module) else {
        return Err(refused("no legacy asset association for this module"));
    };
    let raw = c.int_get(&format!("{module}/{target}"), None)?;
    let row = raw
        .as_object()
        .and_then(|obj| obj.get(entity).cloned())
        .unwrap_or(raw);
    let same = row
        .get("id")
        .is_some_and(|id| fsv_py::py_str(id) == target.to_string());
    if !row.is_object() || !same {
        return Err(refused("exact record identity unavailable"));
    }
    let requester = positive_id(row.get("requester_id").unwrap_or(&Value::Null))?;
    let mut rows = Vec::new();
    let mut total = 0_i64;
    let mut finished = false;
    for page in 1..=50 {
        let params = obj(json!({
            "entity": entity,
            "per_page": 30,
            "page": page,
            "user_id": requester,
            "entity_id": target,
            "include_meta": "true",
        }));
        let data = c.int_get("assets-to-associate", Some(params))?;
        let Some(assets) = data.get("assets").and_then(Value::as_array) else {
            return Err(refused("asset candidates unreadable"));
        };
        let Some(page_total) = data
            .get("meta")
            .and_then(|m| m.get("total_count"))
            .and_then(json_int)
            .filter(|n| *n >= 0)
        else {
            return Err(refused("asset candidate total unavailable"));
        };
        total = page_total;
        rows.extend(assets.iter().cloned());
        if rows.len() as i64 >= total {
            finished = true;
            break;
        }
        if assets.is_empty() {
            return Err(refused("asset candidates incomplete"));
        }
    }
    if !finished {
        return Err(refused("asset candidate page limit exceeded"));
    }
    let mut seen = HashSet::new();
    for row in &rows {
        let id = if row.is_object() {
            positive_id(row.get("display_id").unwrap_or(&Value::Null))?
        } else {
            positive_id(&Value::Null)?
        };
        if !seen.insert(id) {
            return Err(refused("asset candidates inconsistent"));
        }
    }
    if rows.len() as i64 != total {
        return Err(refused("asset candidates inconsistent"));
    }
    Ok(rows)
}

/// Port of `integrated_assets.legacy_plan`.
pub fn legacy_plan(
    c: &dyn Api,
    module: &str,
    target: i64,
    ids: &[i64],
    kind: &str,
    remove: bool,
) -> Result<WritePlan> {
    if remove {
        return Err(refused(
            "removing a legacy asset association has no captured wire; use the web UI; nothing sent",
        ));
    }
    let (module, target) = target_id(module, &json!(target))?;
    let Some(path) = (kind == "devices")
        .then(|| legacy_write_path(module, target))
        .flatten()
    else {
        return Err(refused(
            "legacy association writes cover assets on changes and tickets only; nothing sent",
        ));
    };
    let before = legacy_inventory(c, module, &json!(target))?;
    let current: HashSet<i64> = before
        .iter()
        .map(config_display_id)
        .collect::<Result<_>>()?;
    if ids.iter().any(|id| current.contains(id)) {
        return Err(refused("requested asset is already associated; omit it"));
    }
    let offered: HashSet<i64> = legacy_candidates(c, module, &json!(target))?
        .iter()
        .map(|row| positive_id(row.get("display_id").unwrap_or(&Value::Null)))
        .collect::<Result<_>>()?;
    if ids.iter().any(|id| !offered.contains(id)) {
        return Err(refused(
            "asset display ID is not among the assets the web offers for this record (assets-to-associate); nothing sent",
        ));
    }
    Ok(WritePlan {
        action: "legacy_asset_add".into(),
        target,
        tenant: c.domain(),
        method: "PUT".into(),
        path,
        payload_json: Dumps::default().dump(&json!({"item_ids": ids})),
        context_hash: snapshot_hash(&json!(before))?,
    })
}

/// Port of `integrated_assets.listing`.
pub fn listing(c: &dyn Api, module: &str, target: &Value, kind: &str) -> Result<Value> {
    if driver(c, module, target, kind)? == INTEGRATED {
        return Ok(json!({
            "data": inventory(c, module, target, kind)?,
            "complete": true,
            "driver": INTEGRATED,
        }));
    }
    if kind != "devices" {
        return Err(refused(
            "the legacy asset association list has no business services; use the web UI",
        ));
    }
    Ok(json!({
        "data": legacy_inventory(c, module, target)?,
        "complete": true,
        "driver": LEGACY,
    }))
}

fn dumps(v: &Value) -> String {
    Dumps::default().sort_keys(true).dump(v)
}

/// Port of `integrated_assets.build_plan`.
pub fn build_plan(
    c: &dyn Api,
    module: &str,
    target: &Value,
    ids: &[Value],
    kind: &str,
    remove: bool,
) -> Result<WritePlan> {
    let (wire, target_id) = target_id(module, target)?;
    let mut ids: Vec<i64> = ids.iter().map(positive_id).collect::<Result<_>>()?;
    ids.sort_unstable();
    if ids.is_empty()
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
        || (remove && ids.len() != 1)
    {
        return Err(Error::value(
            "provide distinct inventory IDs; removal accepts exactly one ID",
        ));
    }
    // Python probes associated-cis, then reads the inventory page. A 404
    // probe falls through to the legacy list inside `driver`.
    if driver(c, module, &json!(target_id), kind)? == LEGACY {
        return legacy_plan(c, module, target_id, &ids, kind, remove);
    }
    let before = inventory(c, module, &json!(target_id), kind)?;
    let payload = if remove {
        let row = before.iter().find(|row| {
            positive_id(row.get("item_id").unwrap_or(&Value::Null)).ok() == Some(ids[0])
        });
        let Some(row) = row else {
            return Err(refused(
                "inventory ID is not associated with this exact record",
            ));
        };
        json!({"itam_cmdb_request_id": positive_id(row.get("itam_cmdb_request_id").unwrap_or(&Value::Null))?})
    } else {
        let current: HashSet<i64> = before
            .iter()
            .map(|row| positive_id(row.get("item_id").unwrap_or(&Value::Null)))
            .collect::<Result<_>>()?;
        if ids.iter().any(|id| current.contains(id)) {
            return Err(refused(
                "requested inventory ID is already associated; omit it",
            ));
        }
        let key = if kind == "services" {
            "businessapp_ids"
        } else {
            "device_ids"
        };
        json!({key: ids})
    };
    let context = snapshot_hash(&json!(before))?;
    Ok(WritePlan {
        action: if remove {
            "integrated_asset_remove".into()
        } else {
            "integrated_asset_add".into()
        },
        target: target_id,
        tenant: c.domain(),
        method: "PUT".into(),
        path: format!(
            "{wire}/{target_id}/{}",
            if remove {
                "disassociate-ci"
            } else {
                "associate-ci"
            }
        ),
        payload_json: dumps(&payload),
        context_hash: context,
    })
}

/// Port of `integrated_assets.send`.
#[allow(clippy::too_many_arguments)]
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    module: &str,
    ids: &[Value],
    kind: &str,
    remove: bool,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let module = module.to_string();
    let ids = ids.to_vec();
    let kind = kind.to_string();
    let target = plan.target;
    let want: HashSet<i64> = ids.iter().map(positive_id).collect::<Result<_>>()?;
    if plan.action == "legacy_asset_add" {
        let verify = |_: &Value| {
            let rows = legacy_inventory(c, &module, &json!(target))?;
            let actual: HashSet<i64> = rows.iter().map(config_display_id).collect::<Result<_>>()?;
            Ok(obj(json!({
                "verified": want.is_subset(&actual),
                "readback": rows,
                "driver": LEGACY,
                "provenance": "legacy asset association: capture-based, not yet live-proven on any tenant",
            })))
        };
        let rebuild = || build_plan(c, &module, &json!(target), &ids, &kind, remove);
        return execute(plan, c, journal, reviewed_hash, &rebuild, &verify);
    }
    let verify = |_: &Value| {
        let rows = inventory(c, &module, &json!(target), &kind)?;
        let actual: HashSet<i64> = rows
            .iter()
            .map(|row| positive_id(row.get("item_id").unwrap_or(&Value::Null)))
            .collect::<Result<_>>()?;
        let verified = if remove {
            want.is_disjoint(&actual)
        } else {
            want.is_subset(&actual)
        };
        Ok(obj(json!({"verified": verified, "readback": rows})))
    };
    let rebuild = || build_plan(c, &module, &json!(target), &ids, &kind, remove);
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

/// `{verified, readback, warning?}` from an inventory attempt. Any failure
/// is unverified and must not be resent.
pub fn verification(rows: Result<Vec<Value>>, devices: &[Value]) -> Map<String, Value> {
    let (readback, verified) = match rows {
        Ok(rows) => match covered(&rows, devices) {
            Ok(ok) => (Value::Array(rows), ok),
            Err(_) => (Value::Null, false),
        },
        Err(_) => (Value::Null, false),
    };
    let mut out = obj(json!({"verified": verified, "readback": readback}));
    if !verified {
        out.insert(
            "warning".into(),
            json!("Create-time devices not independently verified; do not resend."),
        );
    }
    out
}

fn covered(rows: &[Value], devices: &[Value]) -> Result<bool> {
    let want: HashSet<i64> = devices.iter().map(positive_id).collect::<Result<_>>()?;
    let actual: HashSet<i64> = rows
        .iter()
        .map(|row| positive_id(row.get("item_id").unwrap_or(&Value::Null)))
        .collect::<Result<_>>()?;
    Ok(want.is_subset(&actual))
}

/// Port of `integrated_assets.verify_devices`.
pub fn verify_devices(
    c: &dyn Api,
    module: &str,
    target: &Value,
    devices: &[Value],
) -> Map<String, Value> {
    verification(inventory(c, module, target, "devices"), devices)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::service::testing::*;
    use fsv_core::error::ApiError;

    struct World {
        rows: Vec<Value>,
        unlink: bool,
        search: Value,
        inventory: Option<Value>,
    }

    fn world() -> World {
        World {
            rows: Vec::new(),
            unlink: false,
            search: json!({"page": 1, "error": null, "total_rows": 1,
                "data": [{"device_pk": 81, "name": {"id": 81, "label": "Acme"}}]}),
            inventory: None,
        }
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.path.contains("/itam/core/") {
                return Ok(s.search.clone());
            }
            if call.method == "GET" {
                if let Some(inv) = &s.inventory {
                    return Ok(inv.clone());
                }
                let n = s.rows.len();
                return Ok(json!({
                    "data": s.rows,
                    "meta": {"total_count": n, "total_pages": if n == 0 { 0 } else { 1 }}
                }));
            }
            if call.method == "PUT" {
                let payload = call.body.clone().unwrap_or(Value::Null);
                if payload.get("itam_cmdb_request_id").is_some() {
                    if s.unlink {
                        s.rows.retain(|row| {
                            row.get("item_id").map(fsv_py::py_str).as_deref() != Some("81")
                        });
                    } else {
                        s.rows.clear();
                    }
                } else {
                    let key = if payload.get("businessapp_ids").is_some() {
                        "businessapp_ids"
                    } else {
                        "device_ids"
                    };
                    let cat = if key == "businessapp_ids" {
                        "Business service"
                    } else {
                        "Device"
                    };
                    for id in payload
                        .get(key)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let n = id.as_i64().expect("inventory id");
                        s.rows.push(json!({
                            "item_id": n,
                            "itam_cmdb_request_id": n + 900,
                            "category": cat
                        }));
                    }
                }
                return Ok(json!({}));
            }
            Err(Error::other(format!(
                "unexpected {} {}",
                call.method, call.path
            )))
        })
    }

    #[test]
    fn cases_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/integrated_assets_oracle.json")).unwrap();
        let cases = oracle["cases"].as_object().unwrap();
        assert_eq!(cases.len(), 26);
        for (name, case) in cases {
            let state = Arc::new(Mutex::new(world()));
            let c = api(state.clone());
            let journal = MemJournal::default();
            let mut plan: Option<WritePlan> = None;
            for (i, (op, want)) in case["ops"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["outputs"].as_array().unwrap())
                .enumerate()
            {
                let ctx = format!("{name} op {i}");
                let module = op.get("module").and_then(Value::as_str).unwrap_or("");
                let kind = op.get("kind").and_then(Value::as_str).unwrap_or("devices");
                let remove = op.get("remove").and_then(Value::as_bool).unwrap_or(false);
                let ids = op
                    .get("ids")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let got = match op["op"].as_str().unwrap() {
                    "set" => {
                        let mut s = state.lock().unwrap();
                        if let Some(v) = op.get("unlink") {
                            s.unlink = v.as_bool().unwrap();
                        }
                        if let Some(v) = op.get("search") {
                            s.search = v.clone();
                        }
                        if let Some(v) = op.get("inventory") {
                            s.inventory = Some(v.clone());
                        }
                        Ok(json!({}))
                    }
                    "search" => search(
                        &c,
                        op.get("q").and_then(Value::as_str).unwrap_or(""),
                        kind,
                        op.get("page").and_then(Value::as_i64).unwrap_or(1),
                    )
                    .map(|r| json!({"result": r})),
                    "build" => {
                        let target = op.get("target").cloned().unwrap_or(json!(23));
                        build_plan(&c, module, &target, &ids, kind, remove).map(|p| {
                            let out = json!({"plan": {
                                "action": p.action, "target": p.target, "tenant": p.tenant,
                                "method": p.method, "path": p.path, "payload": p.payload(),
                                "context_hash": p.context_hash, "hash": p.hash(),
                                "review": p.review(),
                            }});
                            plan = Some(p);
                            out
                        })
                    }
                    "send" => {
                        let p = plan.as_ref().unwrap();
                        let full = p.hash();
                        let reviewed =
                            match op.get("hash").and_then(Value::as_str).unwrap_or("full") {
                                "none" => None,
                                "wrong" => Some("wrong"),
                                _ => Some(full.as_str()),
                            };
                        send(p, &c, &journal, module, &ids, kind, remove, reviewed)
                            .map(|r| json!({"result": r}))
                    }
                    "verify" => {
                        let target = op.get("target").cloned().unwrap_or(json!(23));
                        let devices = op["devices"].as_array().unwrap();
                        Ok(
                            json!({"result": Value::Object(verify_devices(&c, module, &target, devices))}),
                        )
                    }
                    "rows" => Ok(json!({"rows": state.lock().unwrap().rows.clone()})),
                    other => panic!("{ctx} {other}"),
                };
                match got {
                    Ok(v) => assert_eq!(v, *want, "{ctx}"),
                    Err(e) => {
                        assert_eq!(
                            e.to_string(),
                            want["error"]["msg"].as_str().unwrap(),
                            "{ctx}"
                        );
                        match want["error"]["type"].as_str().unwrap() {
                            "ValueError" => assert!(matches!(e, Error::Value(_)), "{ctx}"),
                            "ReviewedWriteError" => {
                                assert!(matches!(e, Error::Exit { code: 2, .. }), "{ctx} {e:?}")
                            }
                            other => panic!("{ctx} {other}"),
                        }
                    }
                }
            }
        }
    }

    fn not_found() -> Error {
        ApiError::new(404, json!({})).into()
    }

    #[test]
    fn legacy_driver_lists_plans_and_verifies_by_readback() {
        let added = Arc::new(Mutex::new(Vec::<i64>::new()));
        let added_probe = added.clone();
        let c = FnApi::new(move |call: &Call| {
            if call.path.ends_with("associated-cis") {
                return Err(not_found());
            }
            if call.method == "GET" && call.path.ends_with("/assets")
                || call.path.ends_with("/associated_assets")
            {
                let rows: Vec<Value> = added_probe
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|id| json!({"config_item": {"display_id": id, "name": "x"}}))
                    .collect();
                return Ok(json!({"associated_assets": rows}));
            }
            if call.method == "GET" && (call.path == "changes/41" || call.path == "tickets/41") {
                let key = if call.path.starts_with("changes") {
                    "change"
                } else {
                    "ticket"
                };
                return Ok(json!({key: {"id": 41, "requester_id": 9000004001_i64}}));
            }
            if call.path == "assets-to-associate" {
                return Ok(json!({
                    "assets": [
                        {"display_id": 3811, "name": "Acme laptop"},
                        {"display_id": 3812, "name": "Globex phone"}
                    ],
                    "meta": {"total_count": 2}
                }));
            }
            if call.method == "PUT" {
                let ids = call
                    .body
                    .as_ref()
                    .and_then(|b| b.get("item_ids"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                added_probe
                    .lock()
                    .unwrap()
                    .extend(ids.iter().filter_map(Value::as_i64));
                return Ok(json!({"untrusted": true}));
            }
            Err(not_found())
        });
        let listed = listing(&c, "changes", &json!("CHN-41"), "devices").unwrap();
        assert_eq!(listed["driver"], "legacy");
        assert_eq!(listed["data"], json!([]));
        let err = listing(&c, "tickets", &json!(41), "services").unwrap_err();
        assert!(err.to_string().contains("business services"));
        let missing = FnApi::new(|call: &Call| {
            if call.path.ends_with("associated-cis") {
                return Err(ApiError::new(403, json!({})).into());
            }
            Err(not_found())
        });
        let probe = driver(&missing, "tickets", &json!(41), "devices").unwrap_err();
        assert_eq!(probe.status(), Some(403));
        let none = FnApi::new(|_: &Call| Err(not_found()));
        let closed = listing(&none, "tickets", &json!(41), "devices").unwrap_err();
        assert!(closed.to_string().contains("nothing read or sent"));

        let plan = build_plan(
            &c,
            "changes",
            &json!("CHN-41"),
            &[json!(3812), json!(3811)],
            "devices",
            false,
        )
        .unwrap();
        assert_eq!(plan.method, "PUT");
        assert_eq!(plan.path, "changes/41/assets/associate");
        assert_eq!(plan.action, "legacy_asset_add");
        assert_eq!(plan.payload(), json!({"item_ids": [3811, 3812]}));
        let removed = build_plan(&c, "changes", &json!(41), &[json!(3811)], "devices", true);
        assert!(
            removed
                .unwrap_err()
                .to_string()
                .contains("no captured wire")
        );
        let journal = MemJournal::default();
        let sent = send(
            &plan,
            &c,
            &journal,
            "changes",
            &[json!(3811), json!(3812)],
            "devices",
            false,
            Some(&plan.hash()),
        )
        .unwrap();
        assert_eq!(sent["verified"], json!(true));
        assert!(
            sent["provenance"]
                .as_str()
                .unwrap()
                .contains("not yet live-proven")
        );
        assert_eq!(c.writes().len(), 1);
    }

    /// `test_sweep1_messages.py::test_out_of_range_inventory_page_says_no_results` (c692ce2).
    #[test]
    fn out_of_range_inventory_page_says_no_results() {
        let paged = |answer: fn(i64) -> Value| {
            FnApi::new(move |call: &Call| {
                assert_eq!(call.method, "GET");
                let page = call.params.as_ref().unwrap()["page"].as_i64().unwrap();
                Ok(answer(page))
            })
        };
        let native = paged(|page| {
            if page == 1 {
                json!({"page": 1, "error": null, "total_rows": 0, "data": []})
            } else {
                json!({"page": 1, "error": "Requested page is out of range.", "total_rows": 0, "data": []})
            }
        });
        assert_eq!(
            search(&native, "", "devices", 1).unwrap()["complete"],
            json!(true)
        );
        let e = search(&native, "", "devices", 2).unwrap_err();
        assert!(matches!(e, Error::Value(_)));
        assert_eq!(
            e.to_string(),
            "no results on page 2: the devices inventory has 0 row(s) (1 page(s) of 50)"
        );
        let broken =
            paged(|page| json!({"page": page, "error": "boom", "total_rows": 500, "data": []}));
        assert!(
            search(&broken, "", "devices", 2)
                .unwrap_err()
                .to_string()
                .contains("integrated inventory unavailable")
        );
        let first = paged(|_| json!({"page": 1, "error": "x", "total_rows": 0, "data": []}));
        assert!(
            search(&first, "", "devices", 1)
                .unwrap_err()
                .to_string()
                .contains("integrated inventory unavailable")
        );
    }
}
