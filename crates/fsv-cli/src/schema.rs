//! `schema.load`: the cached form schema of a resource. No lower crate owns
//! the fetch and the on-disk cache yet.

use std::time::Instant;

use serde_json::{Map, Value, json};

use fsv_api::{doc_fields, merge_v2_choices, unwrap_schema};
use fsv_core::error::ApiError;
use fsv_core::resource::Resource;
use fsv_core::{Client, Error, Result, cache, config};

/// `_fetch_schema`: internal API first, v2 choices merged in; never empty.
pub fn fetch(c: &Client, res: &Resource) -> Result<Vec<Value>> {
    let mut errors: Vec<Error> = Vec::new();
    let mut int_fields: Option<Vec<Value>> = None;
    match c.int_get(res.form_fields_path, None) {
        Ok(raw) => {
            if let Some(f) = raw.get(res.form_fields_key).filter(|v| fsv_py::truthy(v)) {
                int_fields = Some(f.as_array().cloned().unwrap_or_default());
            }
        }
        Err(e) => errors.push(e),
    }
    let v2_fields = match c.v2_get(res.form_fields_path, None) {
        Ok(raw) => raw
            .get(res.form_fields_key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let Some(mut int_fields) = int_fields.filter(|f| !f.is_empty()) else {
        if !v2_fields.is_empty() {
            return Ok(v2_fields);
        }
        if let Some(i) = errors.iter().position(|e| matches!(e, Error::Session(_))) {
            return Err(errors.swap_remove(i));
        }
        if let Some(i) = errors.iter().position(|e| matches!(e, Error::Api(_))) {
            return Err(errors.swap_remove(i));
        }
        if !errors.is_empty() {
            return Err(errors.swap_remove(0));
        }
        return Err(ApiError::with_request(
            200,
            json!(format!("no form fields returned for {}", res.name)),
            "GET",
            res.form_fields_path,
        )
        .into());
    };
    if !v2_fields.is_empty() {
        merge_v2_choices(&mut int_fields, &v2_fields);
    }
    Ok(int_fields)
}

/// `schema.load(res, client, force)`: fresh cache, stale cache with a
/// background refresh, or a blocking fetch.
pub fn load(res: &'static Resource, c: &std::sync::Arc<Client>, force: bool) -> Result<Value> {
    let primary = config::schema_cache_path(res.name, None);
    let mut found: Option<(std::path::PathBuf, Map<String, Value>, bool)> = None;
    for path in config::schema_cache_candidates(res.name, None) {
        if let (Some(doc), stale) = cache::load(&path)
            && doc_fields(&Value::Object(doc.clone())).is_some()
        {
            found = Some((path, doc, stale));
            break;
        }
    }
    if let Some((path, doc, stale)) = &found
        && !force
    {
        if !*stale {
            let _ = cache::update_meta(path, true, true, None);
        } else {
            let c = c.clone();
            let _ = cache::refresh_async(
                &primary,
                "schema",
                move || fetch(&c, res).map(Value::Array),
                None,
            );
        }
        return Ok(unwrap_schema(doc));
    }
    let t0 = Instant::now();
    let fields = fetch(c, res)?;
    let latency = t0.elapsed().as_millis() as u64;
    if fields.is_empty() {
        return Err(ApiError::with_request(
            200,
            json!(format!("empty schema for {}", res.name)),
            "GET",
            res.form_fields_path,
        )
        .into());
    }
    config::ensure_dirs()?;
    cache::save(
        &primary,
        "schema",
        &Value::Array(fields.clone()),
        "",
        latency,
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    Ok(json!({"saved_at": now, "fields": fields}))
}

/// The field list of a loaded schema.
pub fn fields(schema: &Value) -> Vec<Value> {
    schema
        .get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
