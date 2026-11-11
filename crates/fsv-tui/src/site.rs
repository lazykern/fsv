//! The `views.py` helpers the TUI calls on the UI thread, bound to
//! `fsv_api::site` when the `site` feature is on.

use jiff::tz::TimeZone;
use serde_json::{Map, Value};

use crate::backend::{Res, View};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub key: String,
    pub label: String,
}

pub fn default_view(views: &[View]) -> Option<&View> {
    views.iter().find(|v| v.marked_default).or(views.first())
}

#[cfg(feature = "site")]
pub fn tab_label(name: &str) -> String {
    fsv_api::site::tab_label(name)
}

#[cfg(not(feature = "site"))]
pub fn tab_label(name: &str) -> String {
    let s = name.replace('_', " ");
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase())
        .unwrap_or_default()
}

/// `change_approvals.RULES` as `(key, label)` in definition order.
#[cfg(feature = "site")]
pub fn approval_rules() -> Vec<(&'static str, &'static str)> {
    fsv_api::approvals::RULES.iter().map(|(k, _, l)| (*k, *l)).collect()
}

#[cfg(not(feature = "site"))]
pub fn approval_rules() -> Vec<(&'static str, &'static str)> {
    vec![("everyone", "Everyone"), ("anyone", "Anyone"), ("majority", "Majority"), ("first-responder", "First responder")]
}

#[cfg(feature = "site")]
mod imp {
    use super::*;
    use fsv_api::site;

    pub fn list_columns(
        res: Res,
        schema: Option<&Value>,
        headers: Option<&[Value]>,
    ) -> Vec<Column> {
        site::list_columns(res, schema, headers)
            .into_iter()
            .map(|c| Column {
                key: c.key,
                label: c.label,
            })
            .collect()
    }

    pub fn list_row(
        item: &Value,
        res: Res,
        columns: &[Column],
        schema: Option<&Value>,
        me_id: Option<&Value>,
        tz: Option<&TimeZone>,
    ) -> Map<String, Value> {
        let cols: Vec<site::Column> = columns
            .iter()
            .map(|c| site::Column {
                key: c.key.clone(),
                label: c.label.clone(),
            })
            .collect();
        site::list_row(item, res, &cols, schema, me_id, tz)
    }

    pub fn sort_label(order_by: &str, schema: Option<&Value>) -> String {
        site::sort_label(Some(order_by), schema)
    }

    pub fn list_meta(data: &Value, count: usize, page: i64, per_page: i64) -> (Option<i64>, bool) {
        site::list_meta(data, count, page, per_page)
    }

    pub fn choice_label(field: &str, value: &Value, schema: &Value) -> String {
        fsv_api::choice_label(field, value, schema)
    }

    pub fn resolve_task_status(text: &str, fields: &[Value]) -> Result<Value, String> {
        fsv_api::resolve(text, fields)
            .map(|(id, _)| Value::from(id))
            .map_err(|e| e.to_string())
    }

    pub fn task_status_display(value: &Value) -> String {
        fsv_api::display(value, None)
    }
}

#[cfg(not(feature = "site"))]
mod imp {
    use super::*;

    pub fn list_columns(
        _res: Res,
        _schema: Option<&Value>,
        _headers: Option<&[Value]>,
    ) -> Vec<Column> {
        Vec::new()
    }

    pub fn list_row(
        _item: &Value,
        _res: Res,
        _columns: &[Column],
        _schema: Option<&Value>,
        _me_id: Option<&Value>,
        _tz: Option<&TimeZone>,
    ) -> Map<String, Value> {
        Map::new()
    }

    pub fn sort_label(order_by: &str, _schema: Option<&Value>) -> String {
        match order_by {
            "" | "created_at" => "Created Date".into(),
            "updated_at" => "Last Modified".into(),
            other => other.replace('_', " "),
        }
    }

    pub fn list_meta(
        _data: &Value,
        count: usize,
        _page: i64,
        per_page: i64,
    ) -> (Option<i64>, bool) {
        (None, count as i64 == per_page)
    }

    pub fn choice_label(field: &str, value: &Value, schema: &Value) -> String {
        let fields = schema
            .get("fields")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        fields
            .iter()
            .find(|f| f.get("name").and_then(Value::as_str) == Some(field))
            .and_then(|f| f.get("choices").and_then(Value::as_array))
            .and_then(|cs| cs.iter().find(|c| c.get("id") == Some(value)))
            .and_then(|c| c.get("value").or_else(|| c.get("name")))
            .map(fsv_py::py_str)
            .unwrap_or_default()
    }

    pub fn resolve_task_status(_text: &str, _fields: &[Value]) -> Result<Value, String> {
        Err("task status lookup is not available in this build".into())
    }

    pub fn task_status_display(value: &Value) -> String {
        if value.is_object() {
            let picked = ["name", "label", "value", "id"]
                .iter()
                .find_map(|k| value.get(*k).filter(|v| fsv_py::truthy(v)));
            return picked.map_or_else(|| "None".into(), fsv_py::py_str);
        }
        format!("{} (unknown)", fsv_py::py_str(value))
    }
}

pub use imp::*;
