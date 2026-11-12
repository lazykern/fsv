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
        json!({"itam_cmdb_request_id": row["itam_cmdb_request_id"]})
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
        let values: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        json!({key: values})
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
                        let text = fsv_py::py_str(id);
                        let n: i64 = text.parse().unwrap();
                        s.rows.push(json!({
                            "item_id": text,
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
}
