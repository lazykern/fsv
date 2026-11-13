//! `render._state_label` / `render._change_flow`: the visited-states trail of a change.

use serde_json::{Value, json};

use fsv_api::choice_label;
use fsv_core::term::escape;

/// Python `==` (`1 == 1.0 == True`).
fn py_eq(a: &Value, b: &Value) -> bool {
    fsv_api::readback::py_equal(a, b)
}

/// `_state_label`: a state's own name, else its status choice label, else `str(value)`.
pub fn state_label(value: &Value, schema: Option<&Value>) -> String {
    let empty = json!({});
    let schema = schema.unwrap_or(&empty);
    let mut value = value;
    if let Value::Object(m) = value {
        for key in ["name", "display_name", "value", "status_name"] {
            if let Some(v) = m.get(key).filter(|v| fsv_py::truthy(v)) {
                return fsv_py::py_str(v);
            }
        }
        value = m
            .get("id")
            .filter(|v| fsv_py::truthy(v))
            .or_else(|| m.get("status"))
            .unwrap_or(&Value::Null);
    }
    let label = choice_label("status", value, schema);
    if label.is_empty() {
        fsv_py::py_str(value)
    } else {
        label
    }
}

/// `_change_flow`: `Open → [bold]Planning[/]` over `state_traversal`; `""` without one.
pub fn change_flow(item: &Value, schema: Option<&Value>) -> String {
    let Some(states) = item
        .get("state_traversal")
        .filter(|v| fsv_py::truthy(v))
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    let current = item.get("status").unwrap_or(&Value::Null);
    let status_name = item.get("status_name");
    let mut parts = Vec::new();
    for state in states {
        let sid = match state {
            Value::Object(m) => m.get("id").unwrap_or(&Value::Null),
            other => other,
        };
        let label = state_label(state, schema);
        let mut text = escape(&label);
        if py_eq(sid, current) || status_name.and_then(Value::as_str) == Some(label.as_str()) {
            text = format!("[bold]{text}[/]");
        }
        parts.push(text);
    }
    parts.join(" → ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_state_matches_bool_one() {
        let item = json!({
            "status": true,
            "status_name": "Elsewhere",
            "state_traversal": [{"id": 1, "name": "Open"}, {"id": 2, "name": "Planning"}],
        });
        assert_eq!(change_flow(&item, None), "[bold]Open[/] → Planning");
    }
}
