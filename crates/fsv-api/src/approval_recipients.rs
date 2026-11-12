//! Group and CAB selections expanded to eligible approval users
//! (`approval_recipients.py`).
//!
//! The create body gets user ids only. Group and CAB ids stay in the review
//! hash so a membership change blocks the send.

use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::casefold;
use fsv_py::{py_str, truthy};

use crate::reviewed_write::refused;
use crate::service::{Api, obj};
use crate::watchers::positive_id;

fn exact_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) if n.is_i64() => n.as_i64(),
        Value::Number(n) if n.is_u64() => n.as_u64().and_then(|u| i64::try_from(u).ok()),
        _ => None,
    }
}

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

/// Port of approval_recipients._rows.
fn rows_of(raw: &Value, key: &str) -> Result<Vec<Value>> {
    let Some(obj) = raw.as_object() else {
        return Err(refused("recipient metadata unavailable"));
    };
    let Some(rows) = obj.get(key).and_then(Value::as_array) else {
        return Err(refused("recipient metadata unavailable"));
    };
    if rows.iter().any(|row| !row.is_object()) {
        return Err(refused("recipient metadata unavailable"));
    }
    let meta = match obj.get("meta") {
        None => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(refused("recipient metadata incomplete")),
    };
    if ["has_more", "has_next", "next_page", "partial"]
        .iter()
        .any(|k| meta.get(*k).is_some_and(truthy))
        || meta.get("complete") == Some(&Value::Bool(false))
    {
        return Err(refused("recipient metadata incomplete"));
    }
    if let Some(count) = meta.get("count") {
        match exact_int(count) {
            Some(n) if n == rows.len() as i64 => {}
            _ => return Err(refused("recipient metadata count mismatch")),
        }
    }
    let ids = rows
        .iter()
        .map(|row| positive_id(row.get("id").unwrap_or(&Value::Null)))
        .collect::<Result<Vec<_>>>()?;
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(refused("duplicate recipient metadata identities"));
    }
    Ok(rows.clone())
}

fn inactive(row: &Value) -> bool {
    row.get("deleted").is_some_and(truthy) || row.get("deactivated").is_some_and(truthy)
}

fn sorted_ids(rows: &[Value]) -> Result<Vec<i64>> {
    let mut ids = rows
        .iter()
        .map(|row| positive_id(row.get("id").unwrap_or(&Value::Null)))
        .collect::<Result<Vec<_>>>()?;
    ids.sort_unstable();
    Ok(ids)
}

fn finish(rows: Vec<Value>) -> Result<Vec<i64>> {
    let rows = rows_of(&json!({"approval_users": rows}), "approval_users")?;
    if rows.is_empty() || rows.iter().any(inactive) {
        return Err(refused(
            "empty or inactive group/CAB recipients; no request attempted",
        ));
    }
    let mut ids = rows
        .iter()
        .map(|row| positive_id(row.get("id").unwrap_or(&Value::Null)))
        .collect::<Result<Vec<_>>>()?;
    ids.sort_unstable();
    Ok(ids)
}

/// Port of approval_recipients.members.
pub fn members(c: &dyn Api, target: i64, kind: &str, ident: i64) -> Result<Vec<i64>> {
    if kind == "groups" {
        let mut rows = Vec::new();
        let mut total: Option<i64> = None;
        let mut done = false;
        for page in 1..=100 {
            let raw = c.int_get(
                &format!("changes/{target}/groups/{ident}/members"),
                Some(obj(json!({"page": page, "per_page": 30}))),
            )?;
            let Some(obj) = raw.as_object() else {
                return Err(refused("group members unavailable"));
            };
            let Some(chunk) = obj.get("approval_users").and_then(Value::as_array) else {
                return Err(refused("group members unavailable"));
            };
            let Some(meta) = obj.get("meta").and_then(Value::as_object) else {
                return Err(refused("group members unavailable"));
            };
            let Some(count) = meta.get("count").and_then(exact_int).filter(|n| *n >= 0) else {
                return Err(refused("group members total unavailable/inconsistent"));
            };
            if total.is_some_and(|seen| seen != count) || meta.get("partial").is_some_and(truthy) {
                return Err(refused("group members total unavailable/inconsistent"));
            }
            total = Some(count);
            rows.extend(chunk.iter().cloned());
            if rows.len() as i64 == count {
                done = true;
                break;
            }
            if chunk.is_empty() || rows.len() as i64 > count {
                return Err(refused("group members incomplete"));
            }
        }
        if !done {
            return Err(refused("group members page limit exceeded"));
        }
        return finish(rows);
    }
    if kind == "cabs" {
        let raw = c.int_get(
            &format!("changes/{target}/cab-members"),
            Some(obj(json!({"cab_id": ident}))),
        )?;
        // Native CAB lookups use approval_agents. Agent groups use approval_users.
        let key = if raw.get("approval_agents").is_some() {
            "approval_agents"
        } else {
            "approval_users"
        };
        let rows = rows_of(&raw, key)?;
        if key == "approval_agents" && raw.get("approval_users").is_some() {
            let other = rows_of(&raw, "approval_users")?;
            if sorted_ids(&rows)? != sorted_ids(&other)? {
                return Err(refused("conflicting CAB recipient metadata"));
            }
        }
        return finish(rows);
    }
    Err(Error::value("unsupported recipient source"))
}

fn one_choice<'a>(choices: &'a [Value], value: &str) -> Result<&'a Value> {
    let want = casefold(value);
    let matches: Vec<&Value> = choices
        .iter()
        .filter(|row| {
            row.get("id").is_some_and(|id| py_str(id) == value)
                || casefold(&row.get("name").map(py_str).unwrap_or_default()) == want
        })
        .collect();
    if matches.len() != 1 || inactive(matches[0]) {
        return Err(refused(
            "group/CAB must resolve to one active runtime ID or exact name",
        ));
    }
    Ok(matches[0])
}

/// Port of approval_recipients.resolve. The string is empty when nothing was
/// expanded. Member ids are the union, sorted.
pub fn resolve(
    c: &dyn Api,
    target: i64,
    groups: &[String],
    cabs: &[String],
) -> Result<(Vec<i64>, String)> {
    let mut context = Vec::new();
    for (kind, values) in [("groups", groups), ("cabs", cabs)] {
        if values.is_empty() {
            continue;
        }
        let path = if kind == "groups" {
            "bootstrap/agents_groups"
        } else {
            "cabs"
        };
        let choices = rows_of(&c.int_get(path, None)?, kind)?;
        let mut seen = HashSet::new();
        for value in values {
            let row = one_choice(&choices, value)?;
            let ident = positive_id(row.get("id").unwrap_or(&Value::Null))?;
            if !seen.insert(ident) {
                return Err(Error::value("duplicate recipient source"));
            }
            context.push(json!({
                "kind": kind,
                "id": ident,
                "members": members(c, target, kind, ident)?,
            }));
        }
    }
    let ids: Vec<i64> = context
        .iter()
        .filter_map(|row| row.get("members").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_i64)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let encoded = if context.is_empty() {
        String::new()
    } else {
        dumps(&Value::Array(context))
    };
    Ok((ids, encoded))
}

/// Port of approval_recipients.validate.
pub fn validate(c: &dyn Api, target: i64, context_json: &str) -> Result<()> {
    let text = if context_json.is_empty() {
        "[]"
    } else {
        context_json
    };
    let sources: Vec<Value> =
        serde_json::from_str(text).map_err(|e| Error::value(e.to_string()))?;
    for source in sources {
        let kind = source.get("kind").and_then(Value::as_str).unwrap_or("");
        let ident = positive_id(source.get("id").unwrap_or(&Value::Null))?;
        let got = members(c, target, kind, ident)?;
        let want: Vec<i64> = source
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|v| exact_int(v).unwrap_or(i64::MIN))
            .collect();
        if got != want {
            return Err(refused(
                "group/CAB membership changed since review; review again",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::approvals::build_plan_with;
    use crate::service::testing::*;

    struct World {
        members: Vec<i64>,
        paged: bool,
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        let mut api = FnApi::new(move |call: &Call| {
            let s = state.lock().unwrap();
            if call.path == "bootstrap/agents_groups" {
                return Ok(json!({
                    "agents": [],
                    "groups": [{"id": 71, "name": "Acme reviewers", "agent_ids": [999]}]
                }));
            }
            if call.path == "cabs" {
                return Ok(json!({"cabs": [{"id": 91, "name": "Release review"}]}));
            }
            let ids = if call.path.contains("/groups/") {
                s.members.clone()
            } else {
                vec![82, 83]
            };
            let chunk = if s.paged && call.path.contains("/groups/") {
                let page = call
                    .params
                    .as_ref()
                    .and_then(|p| p.get("page"))
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                let start = ((page - 1) * 30).max(0) as usize;
                ids.iter().skip(start).take(30).copied().collect::<Vec<_>>()
            } else {
                ids.clone()
            };
            let groups = call.path.contains("/groups/");
            let key = if groups {
                "approval_users"
            } else {
                "approval_agents"
            };
            Ok(json!({
                key: chunk.iter().map(|i| if groups {
                    json!({"id": i, "selected": false})
                } else {
                    json!({"id": i.to_string(), "name": "Acme reviewer", "email": "reviewer@example.com", "selected": false})
                }).collect::<Vec<_>>(),
                "meta": {"count": ids.len(), "page": 1, "per_page": 30}
            }))
        });
        api.domain = "acme.example.com".into();
        api
    }

    #[test]
    fn cases_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/approval_recipients_oracle.json")).unwrap();
        let cases = oracle["cases"].as_object().unwrap();
        assert_eq!(cases.len(), 6);
        for (name, case) in cases {
            let state = Arc::new(Mutex::new(World {
                members: vec![81, 82],
                paged: false,
            }));
            let c = api(state.clone());
            let mut context = String::new();
            for (i, (op, want)) in case["ops"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["outputs"].as_array().unwrap())
                .enumerate()
            {
                let ctx = format!("{name} op {i}");
                let got = match op["op"].as_str().unwrap() {
                    "set" => {
                        let mut s = state.lock().unwrap();
                        if let Some(v) = op.get("members").and_then(Value::as_array) {
                            s.members = v.iter().filter_map(Value::as_i64).collect();
                        }
                        if let Some(v) = op.get("paged").and_then(Value::as_bool) {
                            s.paged = v;
                        }
                        Ok(json!({}))
                    }
                    "resolve" => {
                        let groups = op["groups"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(py_str)
                            .collect::<Vec<_>>();
                        let cabs = op["cabs"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(py_str)
                            .collect::<Vec<_>>();
                        resolve(&c, 23, &groups, &cabs).map(|(ids, encoded)| {
                            context = encoded.clone();
                            let plan = build_plan_with(
                                "CHN-23",
                                "Acme review",
                                "Review",
                                &ids,
                                "everyone",
                                "acme.example.com",
                                &encoded,
                            )
                            .unwrap();
                            json!({
                                "ids": ids,
                                "context": serde_json::from_str::<Value>(if encoded.is_empty() { "[]" } else { &encoded }).unwrap(),
                                "hash": plan.hash(),
                                "notified_to": plan.payload()["approval_groups"][0]["notified_to"],
                                "sources": plan.review().get("recipient_sources").cloned().unwrap_or(json!([])),
                            })
                        })
                    }
                    "validate" => validate(&c, 23, &context).map(|_| json!({})),
                    "members" => {
                        let raw = op["raw"].clone();
                        let one = FnApi::new(move |_: &Call| Ok(raw.clone()));
                        members(&one, 23, "cabs", 91).map(|ids| json!({"ids": ids}))
                    }
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
