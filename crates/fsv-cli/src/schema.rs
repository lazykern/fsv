//! `schema.load` for the CLI: [`fsv_api::schema::load`] over the process
//! config.

use serde_json::Value;

pub use fsv_api::schema::load;

/// The field list of a loaded schema.
pub fn fields(schema: &Value) -> Vec<Value> {
    schema
        .get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
