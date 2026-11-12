//! Captured change relationship writes, including the project proxy
//! (`change_relationships.py`).
//!
//! The plan's `context_hash` is `json.dumps(stable_snapshot(context),
//! sort_keys=True)`, the frozen parent and inventory, not a digest.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{CHANGES, parse_id};

use crate::attachments::owner_row;
use crate::recipients::EMAIL;
use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{self, Api, Journal, stable_snapshot};
use crate::watchers;

const PROXY_REFUSAL: &str = "project proxy did not return an accepted nested status/body";

static PROJECT_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z][A-Z0-9_]*$").expect("regex"));
static CATEGORY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[1-9][0-9]*$").expect("regex"));

/// `(list key, associate module_type)`.
fn kind_spec(kind: &str) -> Option<(&'static str, Option<&'static str>)> {
    Some(match kind {
        "caused-incident" => ("tickets", Some("ticket_cause")),
        "problem" => ("problems", Some("problem")),
        "release" => ("releases", None),
        "project" => ("projects", None),
        "project-task" => ("issues", None),
        _ => return None,
    })
}

fn dumps_sorted(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

fn is_int(v: &Value) -> bool {
    matches!(v, Value::Number(n) if n.is_i64() || n.is_u64())
}

fn filled(v: Option<&str>) -> bool {
    v.is_some_and(|s| !s.is_empty())
}

/// `proxy_body`.
pub fn proxy_body(raw: &Value) -> Result<Map<String, Value>> {
    let Some(obj) = raw.as_object() else {
        return Err(refused(PROXY_REFUSAL));
    };
    if !obj.get("status").is_some_and(is_int)
        || obj.get("status").and_then(Value::as_i64) != Some(200)
    {
        return Err(refused(PROXY_REFUSAL));
    }
    match obj.get("body") {
        Some(Value::Object(body)) => Ok(body.clone()),
        _ => Err(refused(PROXY_REFUSAL)),
    }
}

fn sorted_rows(rows: Vec<Value>) -> Result<Vec<Value>> {
    let mut identities = Vec::with_capacity(rows.len());
    for row in &rows {
        let Some(obj) = row.as_object() else {
            return Err(refused(
                "relationship inventory has invalid or duplicate IDs",
            ));
        };
        identities.push(watchers::positive_id(
            obj.get("id").unwrap_or(&Value::Null),
        )?);
    }
    if identities.len() != identities.iter().collect::<BTreeSet<_>>().len() {
        return Err(refused(
            "relationship inventory has invalid or duplicate IDs",
        ));
    }
    let mut rows = rows;
    rows.sort_by_key(|row| {
        watchers::positive_id(row.get("id").unwrap_or(&Value::Null)).unwrap_or(0)
    });
    Ok(rows)
}

/// `inventory`.
pub fn inventory(c: &dyn Api, target: i64, kind: &str) -> Result<Vec<Value>> {
    let Some((key, _)) = kind_spec(kind) else {
        return Err(Error::value("unsupported change relationship kind"));
    };
    let rows = if matches!(kind, "project" | "project-task") {
        let route = if kind == "project" {
            "fetch_linked_fr_projects"
        } else {
            "linked_issues"
        };
        let raw = c.int_get(
            &format!("freshrelease/settings/{route}"),
            Some(service::obj(json!({
                "placeholder[resourceId]": target.to_string(),
                "placeholder[resourceType]": "Change",
            }))),
        )?;
        let body = proxy_body(&raw)?;
        let rows = body.get(key).and_then(Value::as_array).cloned();
        let meta = body.get("meta").and_then(Value::as_object);
        let ok = match (rows.as_ref(), meta) {
            (Some(rows), Some(meta)) => {
                meta.get("page").and_then(Value::as_i64) == Some(1)
                    && matches!(meta.get("total_pages").and_then(Value::as_i64), Some(0 | 1))
                    && meta.get("total").is_some_and(is_int)
                    && meta.get("total").and_then(Value::as_i64) == Some(rows.len() as i64)
            }
            _ => false,
        };
        if !ok {
            return Err(refused(
                "complete single-page linked project inventory unavailable",
            ));
        }
        crate::reviewed_inventory::single_page_inventory(&Value::Object(body), key)?
    } else {
        let mut params = service::obj(json!({"per_page": 50, "page": 1}));
        if kind == "caused-incident" {
            params.insert("filter".into(), json!("associated_tickets_caused"));
        }
        crate::reviewed_inventory::complete_inventory(
            c,
            &format!("changes/{target}/{key}"),
            key,
            Some(&params),
            crate::reviewed_inventory::Family::Internal,
        )?
    };
    sorted_rows(rows)
}

/// Keywords of [`build_plan`].
pub struct RelArgs<'a> {
    pub kind: &'a str,
    pub action: &'a str,
    pub item: Option<&'a str>,
    pub project: Option<&'a Map<String, Value>>,
    pub project_key: Option<&'a str>,
    pub name: Option<&'a str>,
}

fn iso_date(text: &str) -> Result<(i32, u32, u32)> {
    let bad = || Error::value(format!("Invalid isoformat string: {text:?}"));
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(bad());
    }
    let year: i32 = text[0..4].parse().map_err(|_| bad())?;
    let month: u32 = text[5..7].parse().map_err(|_| bad())?;
    let day: u32 = text[8..10].parse().map_err(|_| bad())?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(bad()),
    };
    if day == 0 || day > max {
        return Err(bad());
    }
    Ok((year, month, day))
}

fn project_create(project: &Map<String, Value>, cid: i64) -> Result<Value> {
    const REQUIRED: [&str; 6] = [
        "description",
        "end_date",
        "name",
        "owner_email",
        "project_category",
        "start_date",
    ];
    let keys_ok = project.len() == REQUIRED.len()
        && REQUIRED.iter().all(|k| project.contains_key(*k))
        && project
            .values()
            .all(|v| v.as_str().is_some_and(|s| !strip(s).is_empty()));
    if !keys_ok {
        return Err(Error::value(
            "project requires name, description, start_date, end_date, owner_email and project_category text",
        ));
    }
    let text = |k: &str| project[k].as_str().unwrap_or("");
    if !EMAIL.is_match(text("owner_email")) || !CATEGORY.is_match(text("project_category")) {
        return Err(Error::value("invalid project owner/category"));
    }
    let start = iso_date(text("start_date"))?;
    let end = iso_date(text("end_date"))?;
    if end < start {
        return Err(Error::value("project end precedes start"));
    }
    let mut fields = project.clone();
    fields.insert("public_workspace".into(), json!("0"));
    fields.insert("user_emails".into(), json!([]));
    Ok(json!({
        "meta": {"subject": text("name")},
        "project": fields,
        "remote_resource_id": cid.to_string(),
        "remote_resource_type": "Change",
    }))
}

fn matches<'a>(rows: &'a [Value], item: &str) -> Vec<&'a Value> {
    rows.iter()
        .filter(|row| {
            row.get("id").is_some_and(|v| fsv_py::py_str(v) == item)
                || row.get("key").and_then(Value::as_str) == Some(item)
        })
        .collect()
}

/// `build_plan`.
pub fn build_plan(c: &dyn Api, target: &str, args: &RelArgs<'_>) -> Result<WritePlan> {
    let cid = watchers::positive_id(&json!(parse_id(target, &CHANGES)?))?;
    if kind_spec(args.kind).is_none() || !matches!(args.action, "add" | "remove") {
        return Err(Error::value(
            "choose a captured relationship kind and add/remove",
        ));
    }
    let parent = owner_row(c, "changes", cid)?;
    let rows = inventory(c, cid, args.kind)?;
    let (key, module) = kind_spec(args.kind).unwrap();
    let (method, path, body) = if matches!(args.kind, "project" | "project-task") {
        let found = args
            .item
            .map(|item| matches(&rows, item))
            .unwrap_or_default();
        if args.action == "remove" && found.len() != 1 {
            return Err(refused("exact linked project/task identity unavailable"));
        }
        if args.action == "add" && !found.is_empty() {
            return Err(refused("relationship already present"));
        }
        if args.kind == "project" && args.action == "add" {
            if filled(args.item) || args.project.is_none() {
                return Err(Error::value(
                    "project add is create-and-associate, not an existing-project link; pass project details",
                ));
            }
            let project = args.project.unwrap();
            (
                "POST",
                "freshrelease/settings/create_fr_project".to_string(),
                project_create(project, cid)?,
            )
        } else if args.kind == "project" {
            let row = found[0].as_object().cloned().unwrap_or_default();
            let id = watchers::positive_id(row.get("id").unwrap_or(&Value::Null))?;
            let key = row
                .get("key")
                .cloned()
                .ok_or_else(|| Error::other("KeyError: key"))?;
            let name = row
                .get("name")
                .cloned()
                .ok_or_else(|| Error::other("KeyError: name"))?;
            (
                "POST",
                "freshrelease/settings/unlink_fr_project".to_string(),
                json!({
                    "activity_data": {"key": key, "name": name},
                    "project_id": id.to_string(),
                    "project_key": key,
                    "remote_resource_id": cid.to_string(),
                    "remote_resource_type": "Change",
                }),
            )
        } else {
            let project_key = args.project_key.unwrap_or("");
            if !PROJECT_KEY.is_match(project_key) {
                return Err(Error::value(
                    "project-task requires an exact uppercase --project-key",
                ));
            }
            let item = args.item.unwrap_or("");
            let item_re = Regex::new(&format!("^{}-[1-9][0-9]*$", regex::escape(project_key)))
                .expect("regex");
            if !item_re.is_match(item) {
                return Err(Error::value(
                    "project-task item must be an exact project-key-number",
                ));
            }
            let projects = inventory(c, cid, "project")?;
            let selected: Vec<&Value> = projects
                .iter()
                .filter(|row| row.get("key").and_then(Value::as_str) == Some(project_key))
                .collect();
            if selected.len() != 1 {
                return Err(refused("project task requires an exact linked project"));
            }
            let name = if args.action == "remove" {
                found[0]
                    .get("name")
                    .or_else(|| found[0].get("title"))
                    .and_then(Value::as_str)
            } else {
                args.name
            };
            if name.is_none_or(|n| n.is_empty()) {
                return Err(Error::value("project task add needs its exact --name"));
            }
            let name = name.unwrap();
            if args.action == "remove" {
                let task_project =
                    fsv_py::py_str(found[0].get("project_id").unwrap_or(&Value::Null));
                let project_id = fsv_py::py_str(selected[0].get("id").unwrap_or(&Value::Null));
                if task_project != project_id {
                    return Err(refused("task and project identities conflict"));
                }
            }
            let mut body = json!({
                "activity_data": {"key": item, "name": name},
                "placeholder": {
                    "projectId": project_key,
                    "resourceId": cid.to_string(),
                    "resourceType": "Change",
                },
            });
            let route = if args.action == "add" {
                let subject = parent
                    .get("subject")
                    .cloned()
                    .ok_or_else(|| Error::other("KeyError: subject"))?;
                body["issue"] = json!({"id": item});
                body["meta"] = json!({"subject": subject});
                body["remote_resource_id"] = json!(cid.to_string());
                body["remote_resource_type"] = json!("Change");
                "create_issue"
            } else {
                body["issue_id"] = json!(item);
                "unlink_issue"
            };
            ("POST", format!("freshrelease/settings/{route}"), body)
        }
    } else {
        if args.project.is_some() || args.project_key.is_some() || args.name.is_some() {
            return Err(Error::value(
                "project options do not apply to this relationship",
            ));
        }
        let ident = watchers::positive_id(&json!(args.item.unwrap_or("")))?;
        let present = rows.iter().any(|row| {
            watchers::positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == Some(ident)
        });
        if present != (args.action == "remove") {
            return Err(refused(
                "relationship already present or removal target absent",
            ));
        }
        if args.kind == "caused-incident" && args.action == "add" {
            let record = owner_row(c, "tickets", ident)?;
            if casefold(
                &record
                    .get("ticket_type")
                    .map(fsv_py::py_str)
                    .unwrap_or_default(),
            ) != "incident"
            {
                return Err(refused("caused relationship requires a proven incident"));
            }
        } else if args.action == "add" {
            let raw = c.int_get(&format!("{key}/{ident}"), None)?;
            let row = match &raw {
                Value::Object(m) => m
                    .get(&key[..key.len().saturating_sub(1)])
                    .cloned()
                    .unwrap_or(raw.clone()),
                _ => Value::Null,
            };
            if watchers::positive_id(row.get("id").unwrap_or(&Value::Null)).ok() != Some(ident) {
                return Err(refused("exact relationship candidate unavailable"));
            }
        }
        if args.kind == "release" {
            (
                "PUT",
                format!("changes/{cid}/releases/{ident}"),
                json!({"type": if args.action == "add" { "link" } else { "unlink" }}),
            )
        } else if args.action == "add" {
            (
                "PUT",
                format!("changes/{cid}/{key}/associate"),
                json!({"item_ids": [ident], "module_type": module}),
            )
        } else {
            let mut path = format!("changes/{cid}/{key}/{ident}");
            if args.kind == "caused-incident" {
                path.push_str("?module_type=ticket_cause");
            }
            ("DELETE", path, json!({"type": "unlink"}))
        }
    };
    let intent = json!({
        "action": args.action,
        "item": args.item,
        "kind": args.kind,
        "name": if args.action == "add" { args.name } else { None },
        "project": args.project,
        "project_key": args.project_key,
        "target": cid,
    });
    let parent = Value::Object(parent);
    let context = dumps_sorted(&stable_snapshot(&json!({
        "intent": intent,
        "parent": parent,
        "rows": rows,
    }))?);
    Ok(WritePlan {
        action: format!("change_relationship_{}_{}", args.kind, args.action),
        target: cid,
        tenant: c.domain(),
        method: method.into(),
        path,
        payload_json: dumps_sorted(&body),
        context_hash: context,
    })
}

fn plan_from_context(c: &dyn Api, context: &Value) -> Result<WritePlan> {
    let intent = &context["intent"];
    let project = intent.get("project").and_then(Value::as_object);
    let item = match intent.get("item") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        Some(other) => Some(fsv_py::py_str(other)),
    };
    let name = match intent.get("name") {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    let project_key = match intent.get("project_key") {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    let kind = intent["kind"].as_str().unwrap_or("").to_string();
    let action = intent["action"].as_str().unwrap_or("").to_string();
    let target = fsv_py::py_str(&intent["target"]);
    build_plan(
        c,
        &target,
        &RelArgs {
            kind: &kind,
            action: &action,
            item: item.as_deref(),
            project,
            project_key: project_key.as_deref(),
            name: name.as_deref(),
        },
    )
}

fn id_set(rows: &[Value]) -> Result<BTreeSet<i64>> {
    rows.iter()
        .map(|row| watchers::positive_id(row.get("id").unwrap_or(&Value::Null)))
        .collect()
}

fn verify(plan: &WritePlan, c: &dyn Api, response: &Value) -> Result<Map<String, Value>> {
    let context: Value = serde_json::from_str(&plan.context_hash).unwrap_or(Value::Null);
    let intent = &context["intent"];
    let kind = intent["kind"].as_str().unwrap_or("");
    let action = intent["action"].as_str().unwrap_or("");
    let rows = inventory(c, plan.target, kind)?;
    let before = id_set(context["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]))?;
    let after = id_set(&rows)?;
    let ident = if matches!(kind, "project" | "project-task") {
        let body = proxy_body(response)?;
        if action == "remove" && body.get("success") != Some(&Value::Bool(true)) {
            return Err(refused("nested unlink success unavailable"));
        }
        if kind == "project" && action == "add" {
            let created = body.get("project").cloned().unwrap_or(Value::Null);
            let ident = watchers::positive_id(created.get("id").unwrap_or(&Value::Null))?;
            let exact: Vec<&Value> = rows
                .iter()
                .filter(|row| {
                    watchers::positive_id(row.get("id").unwrap_or(&Value::Null)).ok() == Some(ident)
                })
                .collect();
            let mut wanted = before.clone();
            wanted.insert(ident);
            let verified = after == wanted
                && !before.contains(&ident)
                && exact.len() == 1
                && exact[0].get("name") == intent["project"].get("name")
                && exact[0].get("key") == created.get("key");
            return Ok(service::obj(
                json!({"item_id": ident, "verified": verified}),
            ));
        }
        if action == "add" {
            let returned = body.get("issue").cloned().unwrap_or(Value::Null);
            let ident = watchers::positive_id(returned.get("id").unwrap_or(&Value::Null))?;
            let item = intent["item"].as_str().unwrap_or("");
            let mut wanted = before.clone();
            wanted.insert(ident);
            let verified = returned.get("key").and_then(Value::as_str) == Some(item)
                && !before.contains(&ident)
                && after == wanted
                && rows.iter().any(|row| {
                    row.get("key").and_then(Value::as_str) == Some(item)
                        && watchers::positive_id(row.get("id").unwrap_or(&Value::Null)).ok()
                            == Some(ident)
                });
            return Ok(service::obj(
                json!({"item_id": ident, "verified": verified}),
            ));
        }
        let item = intent.get("item").map(fsv_py::py_str).unwrap_or_default();
        let stored = context["rows"].as_array().cloned().unwrap_or_default();
        let exact = matches(&stored, &item);
        watchers::positive_id(
            exact
                .first()
                .and_then(|row| row.get("id"))
                .unwrap_or(&Value::Null),
        )?
    } else {
        watchers::positive_id(intent.get("item").unwrap_or(&Value::Null))?
    };
    let mut wanted = before;
    if action == "add" {
        wanted.insert(ident);
    } else {
        wanted.remove(&ident);
    }
    Ok(service::obj(
        json!({"item_id": ident, "verified": after == wanted}),
    ))
}

/// `send`.
pub fn send(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let rebuild = || {
        let context: Value = serde_json::from_str(&plan.context_hash).unwrap_or(Value::Null);
        plan_from_context(c, &context)
    };
    let check = |response: &Value| verify(plan, c, response);
    execute(plan, c, journal, reviewed_hash, &rebuild, &check)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, LazyLock, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::service::testing::{Call, FnApi, MemJournal};

    static ORACLE: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("testdata/change_relationships_oracle.json")).unwrap()
    });

    struct World {
        rows: BTreeMap<String, Vec<Value>>,
        partial: bool,
        fail: bool,
        nested_fail: bool,
        no_effect: bool,
    }

    fn empty_world() -> World {
        let mut rows = BTreeMap::new();
        for kind in [
            "caused-incident",
            "problem",
            "release",
            "project",
            "project-task",
        ] {
            rows.insert(kind.to_string(), Vec::new());
        }
        World {
            rows,
            partial: false,
            fail: false,
            nested_fail: false,
            no_effect: false,
        }
    }

    fn seed(kind: &str, action: &str) -> World {
        let mut world = empty_world();
        if kind == "project-task" {
            world.rows.insert(
                "project".into(),
                vec![json!({"id": 81, "key": "ACME", "name": "Acme project"})],
            );
        }
        if action == "remove" {
            let id = if kind == "project" {
                81
            } else if kind == "project-task" {
                91
            } else {
                71
            };
            let key = if kind == "project" { "ACME" } else { "ACME-1" };
            world.rows.insert(
                kind.into(),
                vec![json!({"id": id, "key": key, "name": "Acme task", "project_id": 81})],
            );
        }
        world
    }

    fn list_key(kind: &str) -> &'static str {
        kind_spec(kind).unwrap().0
    }

    fn api(state: Arc<Mutex<World>>) -> FnApi {
        FnApi::new(move |call: &Call| {
            let mut s = state.lock().unwrap();
            if call.method == "GET" && call.path == "changes/123" {
                return Ok(json!({"change": {"id": 123, "subject": "Acme change"}}));
            }
            if call.method == "GET" && call.path.starts_with("freshrelease") {
                let kind = if call.path.ends_with("fetch_linked_fr_projects") {
                    "project"
                } else {
                    "project-task"
                };
                let rows = s.rows.get(kind).cloned().unwrap_or_default();
                let pages = if s.partial { 2 } else { 1 };
                return Ok(json!({
                    "status": 200,
                    "body": {
                        list_key(kind): rows,
                        "meta": {"page": 1, "total": rows.len(), "total_pages": pages},
                    }
                }));
            }
            if call.method == "GET"
                && let Some(rest) = call.path.strip_prefix("changes/123/")
            {
                let key = rest.rsplit('/').next().unwrap_or(rest);
                let kind = [
                    "caused-incident",
                    "problem",
                    "release",
                    "project",
                    "project-task",
                ]
                .into_iter()
                .find(|k| list_key(k) == key)
                .unwrap_or(key);
                let rows = s.rows.get(kind).cloned().unwrap_or_default();
                return Ok(json!({key: rows, "meta": {"has_next": s.partial}}));
            }
            if call.method == "GET" {
                let mut parts = call.path.split('/');
                let module = parts.next().unwrap_or("");
                let ident = parts.next().unwrap_or("0");
                let singular = module.strip_suffix('s').unwrap_or(module);
                return Ok(json!({
                    singular: {"id": ident.parse::<i64>().unwrap_or(0), "ticket_type": "Incident"}
                }));
            }
            if s.fail {
                return Err(Error::session("expired"));
            }
            if call.path.starts_with("freshrelease") {
                if s.nested_fail {
                    return Ok(json!({"status": 500, "body": {"error": "failed"}}));
                }
                let kind = if call.path.ends_with("fr_project") {
                    "project"
                } else {
                    "project-task"
                };
                if call.path.contains("unlink") {
                    if !s.no_effect {
                        s.rows.insert(kind.into(), Vec::new());
                    }
                    return Ok(json!({"status": 200, "body": {"success": true}}));
                }
                let row = if kind == "project" {
                    json!({
                        "id": 81,
                        "key": "ACME",
                        "name": call.body.as_ref().unwrap()["project"]["name"],
                        "project_id": 81,
                    })
                } else {
                    json!({
                        "id": 91,
                        "key": "ACME-1",
                        "name": call.body.as_ref().unwrap()["activity_data"]["name"],
                        "project_id": 81,
                    })
                };
                if !s.no_effect {
                    s.rows.entry(kind.into()).or_default().push(row.clone());
                }
                let envelope = if kind == "project" {
                    json!({"status": 200, "body": {"project": row}})
                } else {
                    json!({"status": 200, "body": {"issue": row}})
                };
                return Ok(envelope);
            }
            let key = call.path.split('/').nth(2).unwrap_or("");
            let kind = [
                "caused-incident",
                "problem",
                "release",
                "project",
                "project-task",
            ]
            .into_iter()
            .find(|k| list_key(k) == key)
            .unwrap_or("");
            let adding = call.path.ends_with("associate")
                || call
                    .body
                    .as_ref()
                    .and_then(|b| b.get("type"))
                    .and_then(Value::as_str)
                    == Some("link");
            if !s.no_effect {
                let next = if adding {
                    vec![json!({"id": 71, "subject": "Acme record"})]
                } else {
                    Vec::new()
                };
                s.rows.insert(kind.into(), next.clone());
                if adding {
                    return Ok(json!({key: next}));
                }
            }
            Ok(Value::Null)
        })
    }

    fn project_body() -> Map<String, Value> {
        service::obj(json!({
            "name": "Acme project",
            "description": "Acme work",
            "start_date": "2026-01-01",
            "end_date": "2026-01-02",
            "owner_email": "person@example.com",
            "project_category": "1",
        }))
    }

    fn args<'a>(
        kind: &'a str,
        action: &'a str,
        item: Option<&'a str>,
        project: Option<&'a Map<String, Value>>,
        project_key: Option<&'a str>,
        name: Option<&'a str>,
    ) -> RelArgs<'a> {
        RelArgs {
            kind,
            action,
            item,
            project,
            project_key,
            name,
        }
    }

    fn posts(c: &FnApi) -> Vec<Call> {
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call.method.as_str(), "POST" | "PUT" | "DELETE"))
            .cloned()
            .collect()
    }

    #[test]
    fn plans_match_python() {
        let project = project_body();
        let cases = [
            ("caused-incident", "add", Some("71"), None, None, None),
            ("caused-incident", "remove", Some("71"), None, None, None),
            ("problem", "add", Some("71"), None, None, None),
            ("problem", "remove", Some("71"), None, None, None),
            ("release", "add", Some("71"), None, None, None),
            ("release", "remove", Some("71"), None, None, None),
            ("project", "add", None, Some(&project), None, None),
            ("project", "remove", Some("ACME"), None, None, None),
            (
                "project-task",
                "add",
                Some("ACME-1"),
                None,
                Some("ACME"),
                Some("Acme task"),
            ),
            (
                "project-task",
                "remove",
                Some("ACME-1"),
                None,
                Some("ACME"),
                None,
            ),
        ];
        for (kind, action, item, project, project_key, name) in cases {
            let state = Arc::new(Mutex::new(seed(kind, action)));
            let c = api(state);
            let plan = build_plan(
                &c,
                "CHN-123",
                &args(kind, action, item, project, project_key, name),
            )
            .unwrap();
            let want = &ORACLE[&format!("{kind}-{action}")];
            assert_eq!(
                plan.action,
                want["action"].as_str().unwrap(),
                "{kind} {action}"
            );
            assert_eq!(
                plan.method,
                want["method"].as_str().unwrap(),
                "{kind} {action}"
            );
            assert_eq!(plan.path, want["path"].as_str().unwrap(), "{kind} {action}");
            assert_eq!(
                plan.payload_json,
                want["payload"].as_str().unwrap(),
                "{kind} {action}"
            );
            assert_eq!(
                plan.tenant,
                want["tenant"].as_str().unwrap(),
                "{kind} {action}"
            );
            assert_eq!(
                plan.target,
                want["target"].as_i64().unwrap(),
                "{kind} {action}"
            );
            assert_eq!(
                plan.context_hash,
                want["context"].as_str().unwrap(),
                "{kind} {action}"
            );
            assert_eq!(
                plan.hash(),
                want["hash"].as_str().unwrap(),
                "{kind} {action}"
            );
            assert!(posts(&c).is_empty(), "{kind} {action}");
        }
    }

    #[test]
    fn release_send_verifies_and_does_not_replay() {
        let state = Arc::new(Mutex::new(seed("release", "add")));
        let c = api(Arc::clone(&state));
        let plan = build_plan(
            &c,
            "CHN-123",
            &args("release", "add", Some("71"), None, None, None),
        )
        .unwrap();
        let journal = MemJournal::default();
        let sent = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
        assert_eq!(sent["verified"], json!(true));
        assert_eq!(posts(&c).len(), 1);
        assert_eq!(posts(&c)[0].method, "PUT");
        assert_eq!(posts(&c)[0].body.as_ref().unwrap()["type"], json!("link"));
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        assert_eq!(posts(&c).len(), 1);
    }

    #[test]
    fn project_failures_send_once() {
        let state = Arc::new(Mutex::new(seed("project", "add")));
        let c = api(Arc::clone(&state));
        let project = project_body();
        let plan = build_plan(
            &c,
            "123",
            &args("project", "add", None, Some(&project), None, None),
        )
        .unwrap();
        state.lock().unwrap().fail = true;
        let journal = MemJournal::default();
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("unknown"), "{err}");
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"), "{err}");
        assert_eq!(posts(&c).len(), 1);

        let state = Arc::new(Mutex::new(seed("project", "add")));
        let c = api(Arc::clone(&state));
        let plan = build_plan(
            &c,
            "123",
            &args("project", "add", None, Some(&project), None, None),
        )
        .unwrap();
        state.lock().unwrap().nested_fail = true;
        let journal = MemJournal::default();
        let sent = send(&plan, &c, &journal, Some(&plan.hash())).unwrap();
        assert_eq!(sent["verified"], json!(false));
        let err = send(&plan, &c, &journal, Some(&plan.hash())).unwrap_err();
        assert!(err.to_string().contains("was sent"));

        let state = Arc::new(Mutex::new(seed("project", "add")));
        let c = api(Arc::clone(&state));
        let plan = build_plan(
            &c,
            "123",
            &args("project", "add", None, Some(&project), None, None),
        )
        .unwrap();
        state.lock().unwrap().no_effect = true;
        let sent = send(&plan, &c, &MemJournal::default(), Some(&plan.hash())).unwrap();
        assert_eq!(sent["verified"], json!(false));
    }

    #[test]
    fn refusals_send_nothing() {
        let state = Arc::new(Mutex::new(seed("project", "add")));
        let c = api(Arc::clone(&state));
        let err = build_plan(
            &c,
            "123",
            &args("project", "add", Some("81"), None, None, None),
        )
        .unwrap_err();
        assert!(err.to_string().contains("create-and-associate"), "{err}");
        state.lock().unwrap().partial = true;
        let project = project_body();
        let err = build_plan(
            &c,
            "123",
            &args("project", "add", None, Some(&project), None, None),
        )
        .unwrap_err();
        assert!(err.to_string().contains("complete"), "{err}");
        assert!(posts(&c).is_empty());
    }
}
