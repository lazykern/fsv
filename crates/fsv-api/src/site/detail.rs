//! Lifecycle bar, planning fields, and the schema-order property fallback.
//! HTTP stays with the caller; these functions only interpret JSON.

use serde_json::{Map, Value, json};

use fsv_core::pystr;
use fsv_forms::spec::is_planning_type;

use crate::QueryError;
use crate::site::format::{html_to_text, one_line};
use crate::site::list::{choice_name, obj_name};

/// One step on the change lifecycle bar.
#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleState {
    pub id: Value,
    pub name: String,
    pub mark: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lifecycle {
    pub name: String,
    pub states: Vec<LifecycleState>,
}

impl Lifecycle {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "states": self.states.iter().map(|s| json!({"id": s.id, "name": s.name, "mark": s.mark})).collect::<Vec<_>>(),
        })
    }
}

/// Planning field in form order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningEntry {
    pub name: String,
    pub label: String,
    pub help: String,
    pub text: String,
    pub files: Vec<String>,
}

impl PlanningEntry {
    pub fn empty(&self) -> bool {
        self.text.is_empty() && self.files.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "label": self.label,
            "help": self.help,
            "content": self.text,
            "files": self.files,
            "empty": self.empty(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prop {
    pub name: String,
    pub label: String,
    pub value: String,
    pub required: bool,
}

impl Prop {
    pub fn to_json(&self) -> Value {
        json!({"name": self.name, "label": self.label, "value": self.value, "required": self.required})
    }
}

const SKIP_PROPS: &[&str] = &["subject", "description"];

/// `load_lifecycle` after the flow JSON is in hand.
/// `None` when the record has no flow, the payload has no states, or nothing is current.
pub fn lifecycle_from(record: &Value, data: &Value) -> Option<Lifecycle> {
    if !record.get("state_flow_id").is_some_and(fsv_py::truthy) {
        return None;
    }
    let flow = flow_object(data)?;
    let states = flow
        .get("state_list")
        .filter(|v| fsv_py::truthy(v))?
        .as_array()?;
    let cur = record.get("status");
    let visited: Vec<String> = record
        .get("state_traversal")
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_array)
        .map(|items| items.iter().map(fsv_py::py_str).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for s in states.iter().filter_map(Value::as_object) {
        let sid = s.get("id").cloned().unwrap_or(Value::Null);
        let name = s
            .get("value")
            .filter(|v| fsv_py::truthy(v))
            .or_else(|| s.get("name").filter(|v| fsv_py::truthy(v)))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| fsv_py::py_str(&sid));
        let sid_s = fsv_py::py_str(&sid);
        let cur_s = cur
            .map(fsv_py::py_str)
            .unwrap_or_else(|| "None".to_string());
        let mark = if sid_s == cur_s {
            "current"
        } else if visited.iter().any(|v| v == &sid_s) {
            "done"
        } else {
            "todo"
        };
        out.push(LifecycleState {
            id: sid,
            name,
            mark: mark.to_string(),
        });
    }
    if !out.iter().any(|s| s.mark == "current") {
        return None;
    }
    let name = flow
        .get("name")
        .filter(|v| fsv_py::truthy(v))
        .map(fsv_py::py_str)
        .unwrap_or_default();
    Some(Lifecycle { name, states: out })
}

/// Bind one planning definition to a row. `Ok(None)` when nothing matches.
/// Errors use the same sentences as `create.match_planning_row`.
pub fn match_planning_row<'a>(
    defn: &Map<String, Value>,
    rows: &'a [Value],
) -> Result<Option<&'a Map<String, Value>>, QueryError> {
    let rows: Vec<&Map<String, Value>> = rows.iter().filter_map(Value::as_object).collect();
    let name = defn.get("name");
    let fid = defn.get("id");
    let label = norm_label(defn.get("label"));
    let mut hits: Vec<&Map<String, Value>> = if name.is_some_and(fsv_py::truthy) {
        let want = fsv_py::py_str(name.unwrap());
        rows.iter()
            .copied()
            .filter(|r| {
                r.get("name")
                    .filter(|v| fsv_py::truthy(v))
                    .map(fsv_py::py_str)
                    .unwrap_or_default()
                    == want
            })
            .collect()
    } else {
        Vec::new()
    };
    if hits.is_empty() && fid.is_some_and(|v| !v.is_null()) {
        let want = fsv_py::py_str(fid.unwrap());
        hits = rows
            .iter()
            .copied()
            .filter(|r| {
                ["field_id", "planning_field_id", "id"].iter().any(|k| {
                    r.get(*k)
                        .is_some_and(|v| !v.is_null() && fsv_py::py_str(v) == want)
                })
            })
            .collect();
    }
    if hits.is_empty() && !label.is_empty() {
        hits = rows
            .iter()
            .copied()
            .filter(|r| {
                norm_label(r.get("label")) == label
                    && !fid.is_some_and(|id| {
                        !id.is_null()
                            && ["field_id", "planning_field_id"].iter().any(|k| {
                                r.get(*k).is_some_and(|v| {
                                    !v.is_null() && fsv_py::py_str(v) != fsv_py::py_str(id)
                                })
                            })
                    })
            })
            .collect();
    }
    let who = planning_identity(defn);
    if hits.len() > 1 {
        return Err(QueryError(format!("ambiguous planning rows for {who}")));
    }
    if let Some(hit) = hits.first()
        && fid.is_some_and(|v| !v.is_null())
        && ["field_id", "planning_field_id"].iter().any(|k| {
            hit.get(*k)
                .is_some_and(|v| !v.is_null() && fsv_py::py_str(v) != fsv_py::py_str(fid.unwrap()))
        })
    {
        return Err(QueryError(format!(
            "conflicting planning row identity for {who}"
        )));
    }
    Ok(hits.first().copied())
}

/// Why planning content could not be shown. `NoDefinitions` is `(None, None)` in Python.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningOutcome {
    Entries(Vec<PlanningEntry>),
    NoDefinitions,
    /// The rows are not a list of objects (`Missing("planning rows unavailable")`).
    Unavailable,
    Unidentified,
    Ambiguous(String),
}

/// The change form's planning-type fields, in form order.
pub fn change_planning_defs(schema: Option<&Value>) -> Vec<Value> {
    schema
        .and_then(|s| s.get("fields"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|f| {
                    f.is_object()
                        && is_planning_type(
                            &f.get("field_type").map(fsv_py::py_str).unwrap_or_default(),
                        )
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// [`planning_rows`] for changes, with the definitions from the change form.
pub fn planning_from(schema: Option<&Value>, data: &Value) -> PlanningOutcome {
    planning_rows(&change_planning_defs(schema), data, "changes")
}

/// The body of fsv.views.load_planning after the fetch: `defs` against the
/// `{module}/planning-fields` response.
pub fn planning_rows(defs: &[Value], data: &Value, module: &str) -> PlanningOutcome {
    let defs: Vec<&Map<String, Value>> = defs.iter().filter_map(Value::as_object).collect();
    if defs.is_empty() {
        return PlanningOutcome::NoDefinitions;
    }
    let key = if module == "problems" {
        "problem_planning_fields"
    } else {
        "change_planning_fields"
    };
    let rows_val = match data {
        Value::Object(map) => map.get(key).unwrap_or(&Value::Null),
        other => other,
    };
    let Some(rows) = rows_val
        .as_array()
        .filter(|items| items.iter().all(Value::is_object))
    else {
        return PlanningOutcome::Unavailable;
    };
    let mut out = Vec::new();
    for d in defs {
        let name = d.get("name").map(fsv_py::py_str).unwrap_or_default();
        let matched = match match_planning_row(d, rows) {
            Ok(row) => row,
            Err(err) => return PlanningOutcome::Ambiguous(err.0),
        };
        if matched.is_none() && rows.iter().any(unidentified_row) {
            return PlanningOutcome::Unidentified;
        }
        let v = matched.cloned().unwrap_or_else(Map::new);
        let text = html_to_text(&json!(
            crate::planning_text::section_content(Some(&Value::Object(v.clone())))
                .unwrap_or_default()
        ));
        let label = d
            .get("label")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| name.clone());
        let help_src = d
            .get("helptext")
            .filter(|v| fsv_py::truthy(v))
            .or_else(|| d.get("description").filter(|v| fsv_py::truthy(v)))
            .cloned()
            .unwrap_or(Value::String(String::new()));
        out.push(PlanningEntry {
            name,
            label: pystr::html_unescape(&label).into_owned(),
            help: one_line(&help_src),
            text,
            files: file_names(v.get("attachments").or_else(|| v.get("files"))),
        });
    }
    PlanningOutcome::Entries(out)
}

/// Schema order, labels, stored values. Used when the form engine cannot run.
pub fn flat_properties(record: &Value, schema: Option<&Value>) -> Vec<Prop> {
    let mut out = Vec::new();
    let cf = record.get("custom_fields").filter(|v| fsv_py::truthy(v));
    let Some(fields) = schema
        .and_then(|s| s.get("fields"))
        .and_then(Value::as_array)
    else {
        return out;
    };
    let record_map = record.as_object();
    for f in fields.iter().filter_map(Value::as_object) {
        let name = f.get("name").map(fsv_py::py_str).unwrap_or_default();
        if name.is_empty()
            || SKIP_PROPS.contains(&name.as_str())
            || is_planning_type(&f.get("field_type").map(fsv_py::py_str).unwrap_or_default())
        {
            continue;
        }
        let raw = if f.get("default_field").is_some_and(fsv_py::truthy) {
            record_map.and_then(|m| {
                let id_key = format!("{name}_id");
                if m.contains_key(&id_key) {
                    m.get(&id_key)
                } else {
                    m.get(&name)
                }
            })
        } else {
            cf.and_then(Value::as_object).and_then(|m| m.get(&name))
        };
        let mut val = if !matches!(raw, Some(Value::Array(_) | Value::Object(_))) {
            choice_name(schema, &name, raw)
        } else {
            String::new()
        };
        if val.is_empty() {
            val = match raw {
                Some(Value::Object(_)) => obj_name(raw),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|x| {
                        let n = obj_name(Some(x));
                        if n.is_empty() { fsv_py::py_str(x) } else { n }
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                Some(Value::Null) | None => String::new(),
                Some(other) => fsv_py::py_str(other),
            };
        }
        let label = f
            .get("label")
            .filter(|v| fsv_py::truthy(v))
            .map(fsv_py::py_str)
            .unwrap_or_else(|| name.clone());
        out.push(Prop {
            name,
            label: pystr::html_unescape(&label).into_owned(),
            value: val,
            required: f.get("required").is_some_and(fsv_py::truthy),
        });
    }
    out
}

fn flow_object(data: &Value) -> Option<&Map<String, Value>> {
    let obj = data.as_object()?;
    match obj.get("state_flow") {
        Some(Value::Object(flow)) => Some(flow),
        Some(_) => None,
        None => Some(obj),
    }
}

fn norm_label(value: Option<&Value>) -> String {
    let text = match value.filter(|v| !v.is_null()) {
        Some(v) => fsv_py::py_str(v),
        None => String::new(),
    };
    pystr::casefold(&pystr::collapse_ws(&pystr::html_unescape(&text)))
}

fn planning_identity(defn: &Map<String, Value>) -> String {
    let label = norm_label(defn.get("label"));
    if !label.is_empty() {
        return fsv_py::py_repr_str(&label);
    }
    if let Some(name) = defn.get("name").filter(|v| fsv_py::truthy(v)) {
        return fsv_py::py_repr(name);
    }
    defn.get("id")
        .map(fsv_py::py_repr)
        .unwrap_or_else(|| "None".to_string())
}

fn unidentified_row(row: &Value) -> bool {
    let Some(map) = row.as_object() else {
        return false;
    };
    !["name", "label", "field_id", "planning_field_id"]
        .iter()
        .any(|k| map.get(*k).is_some_and(fsv_py::truthy))
}

fn file_names(rows: Option<&Value>) -> Vec<String> {
    rows.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .map(|a| {
                    ["name", "content_file_name", "file_name"]
                        .iter()
                        .find_map(|k| a.get(*k).filter(|v| fsv_py::truthy(v)))
                        .or_else(|| a.get("id"))
                        .map_or_else(|| "None".to_string(), fsv_py::py_str)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lifecycle_and_planning_to_json_match_python() {
        let o: Value =
            serde_json::from_str(include_str!("../testdata/coverage_oracle.json")).unwrap();
        let life = Lifecycle {
            name: "Acme".into(),
            states: vec![
                LifecycleState {
                    id: json!(1),
                    name: "Open".into(),
                    mark: "done".into(),
                },
                LifecycleState {
                    id: json!("2"),
                    name: "Rev".into(),
                    mark: "current".into(),
                },
            ],
        };
        assert_eq!(life.to_json(), o["views"]["lifecycle"]);
        let entry =
            |name: &str, label: &str, help: &str, text: &str, files: &[&str]| PlanningEntry {
                name: name.into(),
                label: label.into(),
                help: help.into(),
                text: text.into(),
                files: files.iter().map(|f| f.to_string()).collect(),
            };
        let got: Vec<Value> = [
            entry("reason", "Reason", "Why", "text", &[]),
            entry("plan", "Plan", "", "", &["a.pdf"]),
            entry("x", "X", "", "", &[]),
        ]
        .iter()
        .map(PlanningEntry::to_json)
        .collect();
        assert_eq!(json!(got), o["views"]["planning"]);
    }

    #[test]
    fn lifecycle_marks_current_and_visited() {
        let record = json!({"state_flow_id": 9, "status": 2, "state_traversal": [1]});
        let data = json!({"state_flow": {"name": "Acme", "state_list": [
            {"id": 1, "value": "Open"},
            {"id": 2, "value": "Review"},
            {"id": 3, "value": "Closed"}
        ]}});
        let life = lifecycle_from(&record, &data).unwrap();
        assert_eq!(life.name, "Acme");
        assert_eq!(
            life.states
                .iter()
                .map(|s| s.mark.as_str())
                .collect::<Vec<_>>(),
            ["done", "current", "todo"]
        );
        assert!(lifecycle_from(&json!({"state_flow_id": 9, "status": 9}), &data).is_none());
    }

    #[test]
    fn planning_matches_by_name_and_keeps_files() {
        let schema = json!({"fields": [
            {"name": "reason", "label": "Reason", "field_type": "default_change_reason", "helptext": "why"},
            {"name": "subject", "label": "Subject", "field_type": "default_subject"}
        ]});
        let data = json!({"change_planning_fields": [{
            "name": "reason",
            "description_html": "<p>Because</p>",
            "attachments": [{"name": "plan.txt"}]
        }]});
        match planning_from(Some(&schema), &data) {
            PlanningOutcome::Entries(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].name, "reason");
                assert_eq!(rows[0].text, "Because");
                assert_eq!(rows[0].files, ["plan.txt"]);
                assert!(!rows[0].empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn flat_properties_skip_subject_and_use_choice() {
        let schema = json!({"fields": [
            {"name": "subject", "label": "Subject", "default_field": true},
            {"name": "priority", "label": "Priority", "default_field": true, "required": true, "choices": [
                {"id": 2, "value": "Medium"}
            ]},
            {"name": "cf_region", "label": "Region", "default_field": false}
        ]});
        let record =
            json!({"subject": "Hi", "priority": 2, "custom_fields": {"cf_region": "APAC"}});
        let props = flat_properties(&record, Some(&schema));
        assert_eq!(props.len(), 2);
        assert_eq!(props[0].value, "Medium");
        assert!(props[0].required);
        assert_eq!(props[1].value, "APAC");
    }
}
