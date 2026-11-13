//! Reviewed ticket task lifecycle using fresh ticket-task metadata
//! (`ticket_tasks.py`).

use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_core::resource::{TICKETS, parse_id};
use fsv_py::{py_repr, py_str, truthy};

use crate::readback::submitted_value_matches;
use crate::reviewed_write::{WritePlan, execute_as, refused};
use crate::service::{Api, Journal, MAX_DETAIL_PAGES, collect_pages, obj, snapshot_hash};
use crate::task_status;
use crate::watchers::positive_id;

/// Port of ticket_tasks.TaskPlan: the write plus the JSON inputs `send`
/// rebuilds from. The hash is the write's (the inputs are not bound).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPlan {
    pub write: WritePlan,
    pub inputs_json: String,
}

impl TaskPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    pub fn review(&self) -> Map<String, Value> {
        self.write.review()
    }

    pub fn inputs(&self) -> Value {
        serde_json::from_str(&self.inputs_json).unwrap_or(Value::Null)
    }
}

/// The `build_plan` keyword arguments. `task` and `values` keep their JSON
/// shape because they are bound verbatim into `inputs_json`.
#[derive(Debug, Clone)]
pub struct TaskArgs {
    pub action: String,
    pub task: Value,
    pub title: Option<String>,
    pub status: String,
    pub values: Option<Map<String, Value>>,
}

impl Default for TaskArgs {
    fn default() -> Self {
        TaskArgs {
            action: String::new(),
            task: Value::Null,
            title: None,
            status: "Open".into(),
            values: None,
        }
    }
}

fn dumps(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

fn value_error(e: crate::QueryError) -> Error {
    Error::value(e.0)
}

/// Port of ticket_tasks.inventory: the complete task list of a ticket,
/// unique by id and sorted by it.
pub fn inventory(c: &dyn Api, target: i64) -> Result<Vec<Value>> {
    let params = obj(json!({"page": 1, "per_page": 30}));
    let pages = collect_pages(
        |p, q| c.int_get(p, q),
        &format!("tickets/{target}/tasks"),
        "tasks",
        Some(&params),
        "ticket tasks",
        MAX_DETAIL_PAGES,
    )?;
    if !pages.meta.get("complete").is_some_and(truthy) {
        return Err(refused("complete ticket task inventory unavailable"));
    }
    let mut keyed = pages
        .rows
        .into_iter()
        .map(|r| Ok((positive_id(r.get("id").unwrap_or(&Value::Null))?, r)))
        .collect::<Result<Vec<_>>>()?;
    let unique: std::collections::HashSet<i64> = keyed.iter().map(|(i, _)| *i).collect();
    if unique.len() != keyed.len() {
        return Err(refused("ambiguous task identities"));
    }
    keyed.sort_by_key(|(i, _)| *i);
    Ok(keyed.into_iter().map(|(_, r)| r).collect())
}

/// Port of ticket_tasks.fields_from: the `ticket_task_fields` field list.
pub fn fields_from(c: &dyn Api) -> Result<Vec<Value>> {
    let raw = c.int_get("ticket_task_fields", None)?;
    for key in ["ticket_task_fields", "task_fields", "fields"] {
        if let Some(list) = raw.get(key).and_then(Value::as_array)
            && list.iter().all(Value::is_object)
        {
            return Ok(list.clone());
        }
    }
    Err(refused("ticket task metadata unavailable"))
}

/// `str(field.get(key, ""))`.
fn text_of(field: &Value, key: &str) -> String {
    field.get(key).map_or_else(String::new, py_str)
}

/// A dict key as `json.dumps` writes it.
fn json_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// Port of ticket_tasks.custom_values: `--set` values for proven, active
/// custom task fields, resolved by unique runtime name or label.
pub fn custom_values(fields: &[Value], values: &Map<String, Value>) -> Result<Map<String, Value>> {
    let mut result = Map::new();
    for (key, value) in values {
        let want = casefold(key);
        let matches: Vec<&Value> = fields
            .iter()
            .filter(|f| {
                want == casefold(&text_of(f, "name")) || want == casefold(&text_of(f, "label"))
            })
            .collect();
        let [field] = matches[..] else {
            return Err(Error::value(format!(
                "custom field must match unique runtime name or label: {key}"
            )));
        };
        let name = field.get("name").cloned().unwrap_or(Value::Null);
        let field_type = text_of(field, "field_type");
        let native = field.get("default") == Some(&json!(false));
        if !(field.get("custom") == Some(&json!(true))
            || field.get("is_custom") == Some(&json!(true))
            || native
            || field_type.starts_with("custom_"))
        {
            return Err(Error::value("--set accepts only proven custom task fields"));
        }
        if ["deleted", "is_deleted", "deleted_at"]
            .iter()
            .any(|k| field.get(*k).is_some_and(truthy))
        {
            return Err(Error::value("inactive task field"));
        }
        let value = if field.get("choices").is_some_and(truthy) {
            let mut status = field.clone();
            status["name"] = json!("status");
            match task_status::resolve(&py_str(value), std::slice::from_ref(&status)) {
                Ok((id, _)) => json!(id),
                Err(_) => {
                    let offered: Vec<String> = task_status::active_choices(&[status])
                        .map_err(value_error)?
                        .into_iter()
                        .map(|(_, label)| label)
                        .collect();
                    let label = field.get("label").filter(|v| truthy(v)).unwrap_or(&name);
                    return Err(Error::value(format!(
                        "{}: {} is not an active choice; offered: {}",
                        py_str(label),
                        py_repr(value),
                        offered.join(", ")
                    )));
                }
            }
        } else if matches!(field_type.as_str(), "custom_text" | "custom_paragraph")
            || matches!(
                field.get("dom_type").and_then(Value::as_str),
                Some("text" | "textarea")
            )
        {
            if !value.is_string() {
                return Err(Error::value("task text field requires text"));
            }
            value.clone()
        } else {
            return Err(Error::value(format!(
                "unproven task custom field type: {}",
                py_str(&name)
            )));
        };
        let key = json_key(&name);
        let key = match key.strip_prefix("cf_") {
            Some(rest) if native && name.is_string() => rest.to_string(),
            _ => key,
        };
        if result.contains_key(&key) {
            return Err(Error::value("duplicate custom field assignment"));
        }
        result.insert(key, value);
    }
    Ok(result)
}

/// Required on the task form but supplied by the parent ticket: the
/// accepted web create body omits it, so it is neither asked for nor sent.
pub const CONTEXT_FIELDS: [&str; 1] = ["workspace_id"];

/// Port of ticket_tasks.required_for_create: a top-level or
/// `field_options` flag that is `true` or `"true"`.
pub fn required_for_create(field: &Value) -> bool {
    let options = field.get("field_options").filter(|v| truthy(v));
    [
        "required",
        "required_for_create",
        "required_on_create",
        "required_for_agents",
    ]
    .iter()
    .map(|k| field.get(*k))
    .chain([options.and_then(|o| o.get("required_for_create"))])
    .any(|v| v == Some(&json!(true)) || v == Some(&json!("true")))
}

const DELETED: [&str; 3] = ["deleted", "is_deleted", "deleted_at"];

/// Port of ticket_tasks.reminder: the id of the unique active `Never`
/// choice of the unique active `notify_before` field.
pub fn reminder(fields: &[Value]) -> Result<i64> {
    let matches: Vec<&Value> = fields
        .iter()
        .filter(|f| f.get("name") == Some(&json!("notify_before")))
        .collect();
    let [field] = matches[..] else {
        return Err(refused(
            "unique active runtime notify_before metadata unavailable",
        ));
    };
    if DELETED.iter().any(|k| field.get(*k).is_some_and(truthy)) {
        return Err(refused(
            "unique active runtime notify_before metadata unavailable",
        ));
    }
    let choices: Vec<&Value> = field
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| {
            let label = [c.get("value"), c.get("label")]
                .into_iter()
                .flatten()
                .find(|v| truthy(v))
                .map_or_else(String::new, py_str);
            c.is_object()
                && !DELETED
                    .iter()
                    .chain(&["deactivated"])
                    .any(|k| c.get(*k).is_some_and(truthy))
                && casefold(&label) == "never"
        })
        .collect();
    let ident = match choices[..] {
        [c] => c.get("id").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let text = py_str(&ident);
    match ident {
        Value::Bool(_) => None,
        _ if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => text.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| refused("active runtime notify_before choice is unavailable or ambiguous"))
}

/// Port of ticket_tasks.build_plan: `add` (POST), `status` (PUT of the
/// task) or `delete` (PUT `update`), bound to the inventory and metadata.
/// `status` is sent except on `delete`, which binds no status. Owner and
/// group stay unset.
pub fn build_plan(c: &dyn Api, target: &str, args: &TaskArgs) -> Result<TaskPlan> {
    let status = (args.action != "delete").then_some(args.status.as_str());
    build_plan_assigned(c, target, args, None, None, status)
}

/// [`build_plan`] plus optional owner, group, and a status that can be
/// omitted. `status == None` leaves status out of the payload (`update`,
/// `delete`); `add` and `status` default it to `Open`.
pub fn build_plan_assigned(
    c: &dyn Api,
    target: &str,
    args: &TaskArgs,
    owner: Option<&str>,
    group: Option<&str>,
    status: Option<&str>,
) -> Result<TaskPlan> {
    let target = positive_id(&json!(parse_id(target, &TICKETS)?))?;
    let action = args.action.as_str();
    if !matches!(action, "add" | "update" | "status" | "delete") {
        return Err(Error::value("unknown task action"));
    }
    let status = status.or(matches!(action, "add" | "status").then_some("Open"));
    let before = inventory(c, target)?;
    let fields = fields_from(c)?;
    let tid = if action == "add" {
        None
    } else {
        Some(positive_id(&args.task)?)
    };
    if let Some(tid) = tid {
        let found = before
            .iter()
            .filter(|r| positive_id(&r["id"]).ok() == Some(tid))
            .count();
        if found != 1 {
            return Err(refused("exact task unavailable"));
        }
    }
    let mut path = format!("tickets/{target}/tasks");
    let payload = if let (true, Some(tid)) = (action == "delete", tid) {
        path.push_str("/update");
        json!({"id": tid.to_string(), "action": "delete"})
    } else {
        let mut payload = Map::new();
        if let Some(status) = status {
            let id = task_status::resolve(status, &fields)
                .map_err(value_error)?
                .0;
            payload.insert("status".into(), json!(id));
        }
        if matches!(action, "status" | "update")
            && let Some(tid) = tid
        {
            path.push_str(&format!("/{tid}"));
        }
        if matches!(action, "add" | "update") {
            if action == "add" || args.title.is_some() {
                let title = args.title.as_deref().unwrap_or_default();
                if strip(title).is_empty() || title.chars().any(|ch| (ch as u32) < 32) {
                    return Err(Error::value(
                        "nonempty task title without controls required",
                    ));
                }
                payload.insert("title".into(), json!(title));
            }
            assign_party(&mut payload, c, &fields, "owner_id", owner)?;
            assign_party(&mut payload, c, &fields, "group_id", group)?;
            if action == "add"
                && fields
                    .iter()
                    .any(|f| f.get("name") == Some(&json!("notify_before")))
            {
                payload.insert("notify_before".into(), json!(reminder(&fields)?));
            }
            let empty = Map::new();
            let values = args.values.as_ref().unwrap_or(&empty);
            let custom = custom_values(&fields, values)?;
            let answered = |field: &Value| {
                let name = field.get("name").map_or_else(|| "None".to_string(), py_str);
                if payload.contains_key(&name) || CONTEXT_FIELDS.contains(&name.as_str()) {
                    return true;
                }
                let key = match name.strip_prefix("cf_") {
                    Some(rest) if field.get("default") == Some(&json!(false)) => rest,
                    _ => &name,
                };
                custom.get(key).is_some_and(|v| !v.is_null() && v != "")
            };
            let missing: Vec<String> = fields
                .iter()
                .filter(|f| required_for_create(f) && !answered(f))
                .map(|f| {
                    f.get("label")
                        .filter(|v| truthy(v))
                        .or(f.get("name"))
                        .map_or_else(|| "None".to_string(), py_str)
                })
                .collect();
            if action == "add" && !missing.is_empty() {
                return Err(refused(format!(
                    "required task fields missing: {}",
                    missing.join(", ")
                )));
            }
            if action == "add" || !values.is_empty() {
                payload.insert("custom_fields".into(), Value::Object(custom));
            }
            if payload.is_empty() {
                return Err(Error::value("provide at least one task update field"));
            }
        }
        Value::Object(payload)
    };
    let mut inputs = json!({
        "action": action, "task": args.task, "title": args.title,
        "status": status, "values": args.values,
    });
    if let Some(owner) = owner {
        inputs["owner"] = json!(owner);
    }
    if let Some(group) = group {
        inputs["group"] = json!(group);
    }
    Ok(TaskPlan {
        write: WritePlan {
            action: format!("ticket_task_{action}"),
            target,
            tenant: c.domain(),
            method: if action == "add" { "POST" } else { "PUT" }.into(),
            path,
            payload_json: dumps(&payload),
            context_hash: snapshot_hash(&json!({"before": before, "fields": fields}))?,
        },
        inputs_json: dumps(&inputs),
    })
}

fn assign_party(
    payload: &mut Map<String, Value>,
    c: &dyn Api,
    fields: &[Value],
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let matches: Vec<&Value> = fields
        .iter()
        .filter(|f| {
            f.get("name") == Some(&json!(key))
                && !DELETED.iter().any(|k| f.get(*k).is_some_and(truthy))
        })
        .collect();
    if matches.len() != 1 {
        return Err(refused(format!(
            "unique active task field unavailable: {key}"
        )));
    }
    let id = if key == "owner_id" {
        if crate::agent_lookup::is_email(value) {
            let rows = c
                .autocomplete("agents", value, None)
                .map_err(|_| Error::value("could not resolve agent; lookup unavailable"))?;
            crate::agent_lookup::exact_agent_id(&rows, value, false)?
        } else {
            positive_id(&json!(value))?
        }
    } else {
        match crate::tasks::resolve_group(c, Some(value))? {
            Some(id) => id,
            None => {
                return Err(Error::value("group must be a positive ID or exact name"));
            }
        }
    };
    payload.insert(key.into(), json!(id));
    Ok(())
}

fn args_from(inputs: &Value) -> TaskArgs {
    TaskArgs {
        action: inputs["action"].as_str().unwrap_or_default().into(),
        task: inputs["task"].clone(),
        title: inputs["title"].as_str().map(str::to_string),
        status: inputs["status"].as_str().unwrap_or("Open").into(),
        values: inputs["values"].as_object().cloned(),
    }
}

/// Port of ticket_tasks.send: one write, then an independent inventory
/// read-back of the task's submitted fields.
pub fn send(
    plan: &TaskPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let inputs = plan.inputs();
    let args = args_from(&inputs);
    let status = match inputs.get("status") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        Some(other) => Some(py_str(other)),
    };
    let owner = inputs
        .get("owner")
        .and_then(Value::as_str)
        .map(str::to_string);
    let group = inputs
        .get("group")
        .and_then(Value::as_str)
        .map(str::to_string);
    let before_ids: Vec<i64> = inventory(c, plan.write.target)?
        .iter()
        .filter_map(|r| positive_id(&r["id"]).ok())
        .collect();
    let target = plan.write.target.to_string();
    let rebuild = || {
        build_plan_assigned(
            c,
            &target,
            &args,
            owner.as_deref(),
            group.as_deref(),
            status.as_deref(),
        )
        .map(|p| p.hash())
    };
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let rows = inventory(c, plan.write.target)?;
        let has = |tid: i64| {
            rows.iter()
                .filter(|r| positive_id(&r["id"]).ok() == Some(tid))
                .collect::<Vec<_>>()
        };
        let tid = match args.action.as_str() {
            "delete" => {
                let tid = positive_id(&args.task)?;
                return Ok(obj(
                    json!({"task_id": tid, "verified": has(tid).is_empty()}),
                ));
            }
            "add" => {
                let raw = match response {
                    Value::Object(m) => m.get("task").unwrap_or(response),
                    _ => &Value::Object(Map::new()),
                };
                let Value::Object(raw) = raw else {
                    return Err(Error::other("task response is not an object"));
                };
                let tid = positive_id(raw.get("id").unwrap_or(&Value::Null))?;
                if before_ids.contains(&tid) {
                    return Ok(obj(json!({"task_id": tid, "verified": false})));
                }
                tid
            }
            _ => positive_id(&args.task)?,
        };
        let found = has(tid);
        let empty = Map::new();
        let row = match found[..] {
            [Value::Object(m)] => m,
            _ => &empty,
        };
        let mut fields = Map::new();
        for (k, v) in plan.write.payload().as_object().into_iter().flatten() {
            let status = match row.get(k) {
                None => "unavailable",
                Some(actual) if submitted_value_matches(actual, v) => "matched",
                Some(_) => "mismatched",
            };
            fields.insert(k.clone(), json!(status));
        }
        let exempt = crate::readback::exempt_unexposed_reminder(&mut fields, !row.is_empty());
        let verified = !row.is_empty()
            && fields
                .iter()
                .all(|(k, v)| (exempt && k == "notify_before") || v == "matched");
        let mut out = obj(json!({"task_id": tid, "fields": fields, "verified": verified}));
        if exempt && verified {
            out.insert(
                "readback_note".into(),
                json!(crate::readback::REMINDER_NOTE),
            );
        }
        Ok(out)
    };
    execute_as(
        &plan.write,
        &plan.hash(),
        c,
        journal,
        reviewed_hash,
        &rebuild,
        &verify,
    )
}

#[cfg(test)]
mod tests;
