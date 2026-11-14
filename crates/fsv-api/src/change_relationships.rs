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
use fsv_core::pystr::strip;
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
                let eq = |k: &str, n: i64| {
                    crate::schema::py_eq(meta.get(k).unwrap_or(&Value::Null), &json!(n))
                };
                eq("page", 1)
                    && (eq("total_pages", 0) || eq("total_pages", 1))
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
    /// The decoded `--project-json` (any JSON but `null`).
    pub project: Option<&'a Value>,
    pub project_key: Option<&'a str>,
    pub name: Option<&'a str>,
}

/// `date.fromisoformat(text)`, with CPython's `ValueError` text.
fn iso_date(text: &str) -> Result<jiff::civil::Date> {
    fsv_forms::pydt::date_fromisoformat_checked(text).map_err(Error::value)
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

/// `_refusal`: one-line refusal for a failed read while building a
/// relationship plan. Only API errors are mapped; anything else is returned.
pub fn refusal(e: Error, what: &str) -> Error {
    let Some(api) = e.as_api() else {
        return e;
    };
    if api.status == 404 {
        return refused(format!("{what} not found (HTTP 404); nothing was sent"));
    }
    let status = if api.status != 0 {
        format!("HTTP {}", api.status)
    } else {
        "local error".to_string()
    };
    refused(format!(
        "could not read {what} ({status}); nothing was sent"
    ))
}

/// `build_plan`.
pub fn build_plan(c: &dyn Api, target: &str, args: &RelArgs<'_>) -> Result<WritePlan> {
    let cid = watchers::positive_id(&json!(parse_id(target, &CHANGES)?))?;
    if kind_spec(args.kind).is_none() || !matches!(args.action, "add" | "remove") {
        return Err(Error::value(
            "choose a captured relationship kind and add/remove",
        ));
    }
    // Every read here (parent change, relationship inventory, candidate
    // record) is a precondition: a server error is a clean refusal.
    build_plan_inner(c, cid, args).map_err(|e| {
        refusal(
            e,
            &format!("change {cid} or its {} relationships", args.kind),
        )
    })
}

/// `_build_plan`.
fn build_plan_inner(c: &dyn Api, cid: i64, args: &RelArgs<'_>) -> Result<WritePlan> {
    let parent = owner_row(c, "changes", cid).map_err(|e| refusal(e, &format!("change {cid}")))?;
    let rows = inventory(c, cid, args.kind).map_err(|e| {
        refusal(
            e,
            &format!("the {} relationship inventory of change {cid}", args.kind),
        )
    })?;
    let (key, module) = kind_spec(args.kind).unwrap();
    let (method, path, body) = if matches!(args.kind, "project" | "project-task") {
        let found = args
            .item
            .filter(|item| !item.is_empty())
            .map(|item| matches(&rows, item))
            .unwrap_or_default();
        if args.action == "remove" && found.len() != 1 {
            return Err(refused("exact linked project/task identity unavailable"));
        }
        if args.action == "add" && !found.is_empty() {
            return Err(refused("relationship already present"));
        }
        if args.kind == "project" && args.action == "add" {
            let Some(project) = args
                .project
                .and_then(Value::as_object)
                .filter(|_| !filled(args.item))
            else {
                return Err(Error::value(
                    "project add is create-and-associate, not an existing-project link; pass project details",
                ));
            };
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
            let projects = inventory(c, cid, "project")
                .map_err(|e| refusal(e, &format!("the linked projects of change {cid}")))?;
            let selected: Vec<&Value> = projects
                .iter()
                .filter(|row| row.get("key").and_then(Value::as_str) == Some(project_key))
                .collect();
            if selected.len() != 1 {
                return Err(refused("project task requires an exact linked project"));
            }
            let name = if args.action == "remove" {
                // Identity wins over a missing name, matching Python: the
                // project_id check runs before `not name`.
                let task_project =
                    fsv_py::py_str(found[0].get("project_id").unwrap_or(&Value::Null));
                let project_id = fsv_py::py_str(selected[0].get("id").unwrap_or(&Value::Null));
                if task_project != project_id {
                    return Err(refused("task and project identities conflict"));
                }
                found[0]
                    .get("name")
                    .filter(|v| fsv_py::truthy(v))
                    .or_else(|| found[0].get("title"))
                    .and_then(Value::as_str)
            } else {
                args.name
            };
            if name.is_none_or(|n| n.is_empty()) {
                return Err(Error::value("project task add needs its exact --name"));
            }
            let name = name.unwrap();
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
        let mut present = false;
        for row in &rows {
            if watchers::positive_id(row.get("id").unwrap_or(&Value::Null))? == ident {
                present = true;
                break;
            }
        }
        if present != (args.action == "remove") {
            return Err(refused(
                "relationship already present or removal target absent",
            ));
        }
        if args.kind == "caused-incident" && args.action == "add" {
            let record = owner_row(c, "tickets", ident)
                .map_err(|e| refusal(e, &format!("incident {ident}")))?;
            crate::ticket_kind::require_kind(
                &Value::Object(record),
                crate::ticket_kind::INCIDENT,
                "caused relationship",
            )?;
        } else if args.action == "add" {
            let raw = c
                .int_get(&format!("{key}/{ident}"), None)
                .map_err(|e| refusal(e, &format!("{} {ident}", &key[..key.len() - 1])))?;
            let row = match &raw {
                Value::Object(m) => m
                    .get(&key[..key.len().saturating_sub(1)])
                    .cloned()
                    .unwrap_or(raw.clone()),
                _ => Value::Null,
            };
            if watchers::positive_id(row.get("id").unwrap_or(&Value::Null))? != ident {
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
    let project = intent.get("project").filter(|v| !v.is_null());
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
        kind_fields: Value,
        missing: BTreeSet<String>,
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
            kind_fields: json!({"type": "Incident"}),
            missing: BTreeSet::new(),
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
            if call.method == "GET" && s.missing.contains(&call.path) {
                return Err(
                    fsv_core::ApiError::new(404, json!({"errors": "Record not found"})).into(),
                );
            }
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
                let mut row = json!({"id": ident.parse::<i64>().unwrap_or(0)});
                for (k, v) in s.kind_fields.as_object().into_iter().flatten() {
                    row[k] = v.clone();
                }
                return Ok(json!({ singular: row }));
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

    fn project_body() -> Value {
        Value::Object(service::obj(json!({
            "name": "Acme project",
            "description": "Acme work",
            "start_date": "2026-01-01",
            "end_date": "2026-01-02",
            "owner_email": "person@example.com",
            "project_category": "1",
        })))
    }

    fn args<'a>(
        kind: &'a str,
        action: &'a str,
        item: Option<&'a str>,
        project: Option<&'a Value>,
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

    /// `date.fromisoformat` on the project dates: CPython's accepted forms
    /// and `ValueError` texts (from `build_plan` runs).
    #[test]
    fn project_dates_are_python_fromisoformat() {
        for (start, want) in [
            ("2024-13-01", Some("month must be in 1..12")),
            ("20240105", None),
            ("2024-W01-1", None),
            ("+024-01-05", Some("Invalid isoformat string: '+024-01-05'")),
            ("2024-02-30", Some("day is out of range for month")),
            ("0000-01-01", Some("year 0 is out of range")),
        ] {
            let mut project = project_body();
            project["start_date"] = json!(start);
            project["end_date"] = json!("2024-12-01");
            let c = api(Arc::new(Mutex::new(seed("project", "add"))));
            let got = build_plan(
                &c,
                "CHN-123",
                &args("project", "add", None, Some(&project), None, None),
            );
            match want {
                None => assert!(got.is_ok(), "{start}: {:?}", got.err()),
                Some(msg) => assert_eq!(got.unwrap_err().to_string(), msg, "{start}"),
            }
        }
    }

    /// Python's `positive_id(row.get("id")) != ident` raises on a candidate
    /// with no usable id (from a `build_plan` run with `{"problem": {"id": None}}`).
    #[test]
    fn candidate_without_id_is_the_positive_id_error() {
        let mut world = seed("problem", "add");
        world.kind_fields = json!({"id": null});
        let c = api(Arc::new(Mutex::new(world)));
        let err = build_plan(
            &c,
            "CHN-123",
            &args("problem", "add", Some("72"), None, None, None),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "expected a positive numeric identity");
    }

    /// `matches[0].get("name") or matches[0].get("title")`: a null, empty or
    /// zero task name falls back to the title (payloads and hashes from
    /// `build_plan` runs on the same rows).
    #[test]
    fn project_task_remove_name_falls_back_to_title() {
        for (name, hash) in [
            (
                json!(null),
                "5a017c2c9825205d223f45b8862580facc517e1712431499ee2ba5e06b64c387",
            ),
            (
                json!(""),
                "7d25e6a2b02162e970accf55ea87c6937d3d0c5244bef2964e1c74b955136872",
            ),
            (
                json!(0),
                "ea07749d1484172c6b0abf31316c9fd2aafbd5670cf9c371c29a10477658455b",
            ),
        ] {
            let mut world = seed("project-task", "remove");
            world.rows.insert(
                "project-task".into(),
                vec![json!({"id": 91, "key": "ACME-1", "name": name, "title": "Acme title", "project_id": 81})],
            );
            let c = api(Arc::new(Mutex::new(world)));
            let plan = build_plan(
                &c,
                "CHN-123",
                &args(
                    "project-task",
                    "remove",
                    Some("ACME-1"),
                    None,
                    Some("ACME"),
                    None,
                ),
            )
            .unwrap();
            assert_eq!(
                plan.payload_json,
                r#"{"activity_data": {"key": "ACME-1", "name": "Acme title"}, "issue_id": "ACME-1", "placeholder": {"projectId": "ACME", "resourceId": "123", "resourceType": "Change"}}"#
            );
            assert_eq!(plan.hash(), hash, "{name}");
        }
    }

    /// A decoded `--project-json` that is not an object stays in the intent:
    /// project remove does not read it, but its plan hash does (hashes from
    /// `build_plan(..., project=[1])` and `{"a": 1}` runs).
    #[test]
    fn project_value_of_any_shape_is_hashed() {
        for (project, hash) in [
            (
                json!([1]),
                "7b0e130627ffcf9c5a570ea616b2c2c2bde6ddaf6ad53c111d722e92f6186dc5",
            ),
            (
                json!({"a": 1}),
                "606e2935d6444aae21b6eee819759d5634b241724f420c109d2e977bf0678e58",
            ),
        ] {
            let c = api(Arc::new(Mutex::new(seed("project", "remove"))));
            let plan = build_plan(
                &c,
                "CHN-123",
                &args(
                    "project",
                    "remove",
                    Some("ACME"),
                    Some(&project),
                    None,
                    None,
                ),
            )
            .unwrap();
            assert_eq!(plan.hash(), hash, "{project}");
        }
        let c = api(Arc::new(Mutex::new(seed("release", "add"))));
        let err = build_plan(
            &c,
            "CHN-123",
            &args("release", "add", Some("71"), Some(&json!([1])), None, None),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "project options do not apply to this relationship"
        );
    }

    /// The single-page check is Python `==` (`1.0`, `True` pass it); the
    /// shared inventory reader then refuses them (`inventory` runs).
    #[test]
    fn project_page_meta_compares_as_python() {
        let single = "complete single-page linked project inventory unavailable";
        let mutation = "complete, consistent mutation inventory unavailable";
        for (meta, want) in [
            (
                json!({"page": 1.0, "total": 1, "total_pages": 1.0}),
                mutation,
            ),
            (
                json!({"page": true, "total": 1, "total_pages": true}),
                mutation,
            ),
            (
                json!({"page": 1, "total": 1, "total_pages": false}),
                mutation,
            ),
            (json!({"page": 1, "total": 1.0, "total_pages": 1}), single),
            (json!({"page": 1, "total": true, "total_pages": 1}), single),
        ] {
            let body = json!({"status": 200, "body": {"projects": [{"id": 81, "key": "ACME"}], "meta": meta}});
            let c = FnApi::new(move |_: &Call| Ok(body.clone()));
            let err = inventory(&c, 123, "project").unwrap_err();
            assert_eq!(err.to_string(), want, "{meta}");
        }
    }

    /// `matches = [...] if item else []`: an empty `--item` matches no row,
    /// even one whose key is empty.
    #[test]
    fn empty_item_matches_no_project() {
        let mut world = seed("project", "remove");
        world.rows.insert(
            "project".into(),
            vec![json!({"id": 81, "key": "", "name": "Acme project", "project_id": 81})],
        );
        let c = api(Arc::new(Mutex::new(world)));
        let err = build_plan(
            &c,
            "CHN-123",
            &args("project", "remove", Some(""), None, None, None),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "exact linked project/task identity unavailable"
        );
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

    /// `test_caused_incident_type_proof_uses_native_exact_read`.
    #[test]
    fn caused_incident_type_proof_uses_native_exact_read() {
        for (fields, allowed) in [
            (json!({"type": "Incident"}), true),
            (json!({"ticket_type": "Incident"}), true),
            (json!({"type": "Incident", "ticket_type": "incident"}), true),
            (json!({"type": "Service Request"}), false),
            (
                json!({"type": "Incident", "ticket_type": "Service Request"}),
                false,
            ),
            (json!({}), false),
            (json!({"type": ""}), false),
            (json!({"type": 1}), false),
        ] {
            let mut world = empty_world();
            world.kind_fields = fields.clone();
            let c = api(Arc::new(Mutex::new(world)));
            let got = build_plan(
                &c,
                "CHN-123",
                &args("caused-incident", "add", Some("71"), None, None, None),
            );
            if allowed {
                let plan = got.unwrap();
                assert_eq!(
                    plan.payload(),
                    json!({"item_ids": [71], "module_type": "ticket_cause"}),
                    "{fields}"
                );
            } else {
                let err = got.unwrap_err().to_string();
                assert!(
                    err.contains("incident") || err.contains("ticket type"),
                    "{fields}: {err}"
                );
            }
            assert!(posts(&c).is_empty());
        }
    }

    /// Python checks project_id before `not name`, so a conflicting task
    /// with no name/title reports the identity error.
    #[test]
    fn project_task_identity_conflict_precedes_missing_name() {
        let mut conflict = empty_world();
        conflict.rows.insert(
            "project".into(),
            vec![json!({"id": 81, "key": "ACME", "name": "Acme project"})],
        );
        conflict.rows.insert(
            "project-task".into(),
            vec![json!({"id": 91, "key": "ACME-1", "project_id": 99})],
        );
        let c = api(Arc::new(Mutex::new(conflict)));
        let err = build_plan(
            &c,
            "CHN-123",
            &args(
                "project-task",
                "remove",
                Some("ACME-1"),
                None,
                Some("ACME"),
                None,
            ),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "task and project identities conflict");

        let mut unnamed = empty_world();
        unnamed.rows.insert(
            "project".into(),
            vec![json!({"id": 81, "key": "ACME", "name": "Acme project"})],
        );
        unnamed.rows.insert(
            "project-task".into(),
            vec![json!({"id": 91, "key": "ACME-1", "project_id": 81})],
        );
        let c = api(Arc::new(Mutex::new(unnamed)));
        let err = build_plan(
            &c,
            "CHN-123",
            &args(
                "project-task",
                "remove",
                Some("ACME-1"),
                None,
                Some("ACME"),
                None,
            ),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "project task add needs its exact --name");
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

    /// `test_missing_ids_are_one_line_refusals` (library half).
    #[test]
    fn missing_ids_are_one_line_refusals() {
        for (kind, missing, named) in [
            ("release", "releases/62", "release 62 not found"),
            ("problem", "problems/62", "problem 62 not found"),
            ("caused-incident", "tickets/62", "incident 62 not found"),
            ("release", "changes/123", "change 123 not found"),
            (
                "problem",
                "changes/123/problems",
                "mutation inventory unavailable",
            ),
        ] {
            let mut world = empty_world();
            world.missing.insert(missing.into());
            let c = api(Arc::new(Mutex::new(world)));
            let err = build_plan(
                &c,
                "CHN-123",
                &args(kind, "add", Some("62"), None, None, None),
            )
            .unwrap_err();
            assert_eq!(err.exit_code(), 2, "{kind} {missing}");
            assert!(err.to_string().contains(named), "{kind} {missing}: {err}");
            assert!(posts(&c).is_empty());
        }
    }

    #[test]
    fn refusal_words_other_statuses() {
        let e = refusal(fsv_core::ApiError::new(500, json!({})).into(), "change 5");
        assert_eq!(
            e.to_string(),
            "could not read change 5 (HTTP 500); nothing was sent"
        );
        let e = refusal(fsv_core::ApiError::new(0, json!({})).into(), "change 5");
        assert_eq!(
            e.to_string(),
            "could not read change 5 (local error); nothing was sent"
        );
        let e = refusal(Error::session("expired"), "change 5");
        assert_eq!(e.exit_code(), 3);
    }
}
