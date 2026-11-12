//! Form schema of a resource (`schema.py`): the fetch with its v2 choice
//! merge, the on-disk cache, cache documents and choice labels.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use fsv_core::config::{self, Paths};
use fsv_core::error::ApiError;
use fsv_core::resource::Resource;
use fsv_core::{Client, Error, Result, cache};

use crate::service::Api;

/// Field definitions rarely change. `schema.CACHE_TTL`.
pub const CACHE_TTL_SECS: u64 = 7 * 86_400;

pub const PRIORITY: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High"), (4, "Urgent")];
pub const IMPACT: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High")];
pub const RISK: &[(i64, &str)] = &[(1, "Low"), (2, "Medium"), (3, "High"), (4, "Very High")];

/// Non-empty field list stored under `data` or `fields`. Empty or malformed
/// documents are a cache miss.
pub fn doc_fields(doc: &Value) -> Option<&Vec<Value>> {
    let obj = doc.as_object()?;
    for key in ["data", "fields"] {
        if let Some(items) = obj.get(key).and_then(Value::as_array)
            && !items.is_empty()
        {
            return Some(items);
        }
    }
    None
}

/// `{saved_at, fields}` from an envelope or an old cache document.
/// Key presence wins over truthiness, so a stored empty list stays.
pub fn unwrap_schema(doc: &Map<String, Value>) -> Value {
    let fields = doc_fields(&Value::Object(doc.clone()))
        .cloned()
        .unwrap_or_default();
    let saved_at = doc.get("saved_at").cloned().unwrap_or(Value::from(0));
    serde_json::json!({"saved_at": saved_at, "fields": fields})
}

/// The field named `name`, or `None`.
pub fn field<'a>(name: &str, schema: &'a Value) -> Option<&'a Map<String, Value>> {
    schema.get("fields")?.as_array()?.iter().find_map(|f| {
        let obj = f.as_object()?;
        (obj.get("name").and_then(Value::as_str) == Some(name)).then_some(obj)
    })
}

/// Label of a dropdown value. Unlike `site::choice_name`, this compares with
/// Python `==` (so `1 == 1.0` and `True == 1`) and falls back to `str(val)`.
pub fn choice_label(field_name: &str, val: &Value, schema: &Value) -> String {
    let Some(f) = field(field_name, schema) else {
        return if val.is_null() {
            String::new()
        } else {
            fsv_py::py_str(val)
        };
    };
    if val.is_null() {
        return String::new();
    }
    if val.as_str() == Some("") {
        return String::new();
    }
    let choices = f.get("choices").and_then(Value::as_array);
    if let Some(choices) = choices {
        for c in choices.iter().filter_map(Value::as_object) {
            let id_hit = c.get("id").is_some_and(|id| py_eq(id, val));
            let value_hit = c.get("value").is_some_and(|v| py_eq(v, val));
            if id_hit || value_hit {
                return c
                    .get("value")
                    .filter(|v| fsv_py::truthy(v))
                    .or_else(|| c.get("name").filter(|v| fsv_py::truthy(v)))
                    .map(fsv_py::py_str)
                    .unwrap_or_else(|| fsv_py::py_str(val));
            }
        }
    }
    fsv_py::py_str(val)
}

/// Copy v2 `choices` onto an internal field when v2 lists more of them.
/// Internal fields stay in their original order. A v2 field with no `name`
/// key is ignored, matching `{"name" in f}`.
pub fn merge_v2_choices(int_fields: &mut [Value], v2_fields: &[Value]) {
    for field in int_fields.iter_mut() {
        let Some(obj) = field.as_object_mut() else {
            continue;
        };
        let Some(name) = obj.get("name").filter(|v| !v.is_null()) else {
            continue;
        };
        let Some(v2) = v2_fields.iter().find_map(|f| {
            let obj = f.as_object()?;
            if !obj.contains_key("name") {
                return None;
            }
            obj.get("name")
                .is_some_and(|n| py_eq(n, name))
                .then_some(obj)
        }) else {
            continue;
        };
        if choice_len(v2.get("choices")) > choice_len(obj.get("choices")) {
            let choices = v2.get("choices").cloned().unwrap_or(Value::Array(vec![]));
            obj.insert("choices".to_string(), choices);
        }
    }
}

fn choice_len(v: Option<&Value>) -> usize {
    v.and_then(Value::as_array).map(|a| a.len()).unwrap_or(0)
}

/// Python `==` for JSON values the schema compares (ids and choice values).
pub(crate) fn py_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Bool(b), n) | (n, Value::Bool(b)) if n.is_number() => {
            num(n) == Some(if *b { 1.0 } else { 0.0 })
        }
        (Value::Number(_), Value::Number(_)) => num(a) == num(b),
        _ => false,
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_u64().map(|u| u as f64))
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

fn py_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// `schema_cache_path(name)` under `paths` for the tenant `domain_key`
/// (`config.cache_domain_key`).
pub fn cache_path(paths: &Paths, domain_key: &str, name: &str) -> PathBuf {
    paths.schema_dir.join(format!("{domain_key}--{name}.json"))
}

/// `_fetch_schema`: internal API first (it has choices for reference
/// fields), v2 as the fallback, v2 choices merged in where v2 has more.
///
/// Never empty: when neither API yields fields the session error wins, then
/// any API error, then the first error, else an `APIError(200)`.
pub fn fetch(c: &dyn Api, res: &Resource) -> Result<Vec<Value>> {
    let mut errors: Vec<Error> = Vec::new();
    let mut int_fields: Vec<Value> = Vec::new();
    match c.int_get(res.form_fields_path, None) {
        Ok(Value::Object(raw)) => {
            if let Some(f) = raw.get(res.form_fields_key).filter(|v| fsv_py::truthy(v)) {
                int_fields = f.as_array().cloned().unwrap_or_default();
            }
        }
        Ok(other) => errors.push(Error::other(format!(
            "'{}' object has no attribute 'get'",
            py_type(&other)
        ))),
        Err(e) => errors.push(e),
    }
    let v2_fields = match c.v2_get(res.form_fields_path, None) {
        Ok(Value::Object(raw)) => raw
            .get(res.form_fields_key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Ok(other) => {
            errors.push(Error::other(format!(
                "'{}' object has no attribute 'get'",
                py_type(&other)
            )));
            Vec::new()
        }
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    if int_fields.is_empty() {
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
        let msg = format!("no form fields returned for {}", res.name);
        return Err(ApiError::with_request(200, json!(msg), "GET", res.form_fields_path).into());
    }
    if !v2_fields.is_empty() {
        merge_v2_choices(&mut int_fields, &v2_fields);
    }
    Ok(int_fields)
}

/// `_cached_path`: the first candidate holding fields, and whether it is
/// older than [`CACHE_TTL_SECS`] (a bad or future `saved_at` is stale).
fn cached_doc(candidates: &[PathBuf]) -> Option<(PathBuf, Map<String, Value>, bool)> {
    for path in candidates {
        let (Some(doc), _) = cache::load(path) else {
            continue;
        };
        if doc_fields(&Value::Object(doc.clone())).is_none() {
            continue;
        }
        let saved = match doc.get("saved_at") {
            Some(Value::Number(n)) => n.as_f64(),
            Some(_) => None,
            None => std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64()),
        };
        let stale = match saved.filter(|s| s.is_finite()) {
            Some(s) => {
                let age = now() - s;
                !(0.0..=CACHE_TTL_SECS as f64).contains(&age)
            }
            None => true,
        };
        return Some((path.clone(), doc, stale));
    }
    None
}

fn with_provenance(doc: &Map<String, Value>, stale: bool) -> Value {
    let mut out = match unwrap_schema(doc) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    out.insert("stale".into(), Value::Bool(stale));
    out.insert("source".into(), json!("cache"));
    Value::Object(out)
}

/// `config.ensure_dirs` for `paths`.
fn ensure_dirs(paths: &Paths) -> Result<()> {
    std::fs::create_dir_all(&paths.config_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&paths.config_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::create_dir_all(&paths.schema_dir)?;
    std::fs::create_dir_all(&paths.filters_dir)?;
    Ok(())
}

/// Port of fsv.schema.load over the process config (`config::paths()`
/// and the current tenant's cache key).
///
/// Returns `{saved_at, fields, stale, source}`. A fresh cache is served as
/// is; a stale one is served with a warning on stderr while a background
/// thread refreshes it; a miss or `force` fetches and saves. Empty schemas
/// are never cached or returned; a failed fetch is an error.
pub fn load(res: &'static Resource, client: &Arc<Client>, force: bool) -> Result<Value> {
    load_in(
        res,
        client,
        force,
        &config::paths(),
        &config::cache_scope_key(None),
    )
}

/// [`load`] with the cache under `paths` for tenant `domain_key`.
pub fn load_in(
    res: &'static Resource,
    client: &Arc<Client>,
    force: bool,
    paths: &Paths,
    domain_key: &str,
) -> Result<Value> {
    let (doc, warning) = load_in_quiet(res, client, force, paths, domain_key)?;
    if let Some(w) = warning {
        eprintln!("{w}");
    }
    Ok(doc)
}

/// [`load_in`] that returns the stale-cache warning instead of printing it
/// (for callers that own the terminal, like the TUI).
pub fn load_in_quiet(
    res: &'static Resource,
    client: &Arc<Client>,
    force: bool,
    paths: &Paths,
    domain_key: &str,
) -> Result<(Value, Option<String>)> {
    let primary = cache_path(paths, domain_key, res.name);
    load_at(
        res,
        client,
        force,
        &primary,
        std::slice::from_ref(&primary),
        paths,
    )
}

/// The stderr line [`load`] prints when it serves a stale cache.
pub fn stale_warning(res: &Resource) -> String {
    format!(
        "Warning: stale {0} schema; refreshing in background. Run `fsv {0} fields --refresh` for current definitions.",
        res.name
    )
}

fn load_at(
    res: &'static Resource,
    client: &Arc<Client>,
    force: bool,
    primary: &Path,
    candidates: &[PathBuf],
    paths: &Paths,
) -> Result<(Value, Option<String>)> {
    if let Some((path, doc, stale)) = cached_doc(candidates)
        && !force
    {
        if !stale {
            let _ = cache::update_meta(&path, true, true, None);
            return Ok((with_provenance(&doc, false), None));
        }
        let c = client.clone();
        let _ = cache::refresh_async(
            primary,
            "schema",
            move || fetch(c.as_ref(), res).map(Value::Array),
            None,
        );
        return Ok((with_provenance(&doc, true), Some(stale_warning(res))));
    }
    fetch_and_save(res, client.as_ref(), primary, paths).map(|doc| (doc, None))
}

/// The miss/`force` half of [`load_at`]: fetch, refuse an empty schema, save.
fn fetch_and_save(
    res: &Resource,
    client: &dyn Api,
    primary: &Path,
    paths: &Paths,
) -> Result<Value> {
    let t0 = Instant::now();
    let fields = fetch(client, res)?;
    let latency = t0.elapsed().as_millis() as u64;
    if fields.is_empty() {
        let msg = format!("empty schema for {}", res.name);
        return Err(ApiError::with_request(200, json!(msg), "GET", res.form_fields_path).into());
    }
    ensure_dirs(paths)?;
    cache::save(
        primary,
        "schema",
        &Value::Array(fields.clone()),
        "",
        latency,
    )?;
    Ok(json!({"saved_at": now(), "fields": fields, "stale": false, "source": "server"}))
}

/// [`load`] with `force=True` over any [`Api`]: always fetches and saves
/// to the process cache. Writes that bind to current
/// definitions use this.
pub fn load_forced(res: &Resource, client: &dyn Api) -> Result<Value> {
    load_forced_in(
        res,
        client,
        &config::paths(),
        &config::cache_scope_key(None),
    )
}

/// [`load_forced`] with the cache under `paths` for tenant `domain_key`.
pub fn load_forced_in(
    res: &Resource,
    client: &dyn Api,
    paths: &Paths,
    domain_key: &str,
) -> Result<Value> {
    fetch_and_save(res, client, &cache_path(paths, domain_key, res.name), paths)
}

fn choice_labels(choices: &Value, out: &mut Vec<String>) {
    match choices {
        Value::Object(map) => {
            for (key, value) in map {
                if !out.contains(key) {
                    out.push(key.clone());
                }
                if value.is_object() || value.is_array() {
                    choice_labels(value, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::Object(map) => {
                        for key in ["value", "label", "name"] {
                            if let Some(text) = map
                                .get(key)
                                .filter(|v| !v.is_null() && v.as_str() != Some(""))
                            {
                                let text = fsv_py::py_str(text);
                                if !out.contains(&text) {
                                    out.push(text);
                                }
                            }
                        }
                        for key in ["nested_options", "choices"] {
                            if map.get(key).is_some_and(|v| v.is_object() || v.is_array()) {
                                choice_labels(&map[key], out);
                            }
                        }
                    }
                    Value::Array(pair) => {
                        for part in pair {
                            if part.is_string() || part.is_i64() || part.is_u64() {
                                let text = fsv_py::py_str(part);
                                if !out.contains(&text) {
                                    out.push(text);
                                }
                            }
                        }
                    }
                    Value::Null | Value::Bool(_) => {}
                    other => {
                        let text = fsv_py::py_str(other);
                        if !out.contains(&text) {
                            out.push(text);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// One form-fields GET against a cached schema. `None` when the cache is
/// current, missing, or the check itself fails. Never blocks a write.
pub fn freshness_warning(res: &Resource, client: &dyn Api, schema: &Value) -> Option<String> {
    if schema.get("source").and_then(Value::as_str) != Some("cache") {
        return None;
    }
    let raw = client.int_get(res.form_fields_path, None).ok()?;
    let fresh = raw.get(res.form_fields_key)?.as_array()?;
    let cached = schema.get("fields")?.as_array()?;
    let mut changed = Vec::new();
    for field in fresh {
        let Some(name) = field.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(old) = cached
            .iter()
            .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
        else {
            continue;
        };
        let mut have = Vec::new();
        choice_labels(old.get("choices").unwrap_or(&Value::Null), &mut have);
        let mut live = Vec::new();
        choice_labels(field.get("choices").unwrap_or(&Value::Null), &mut live);
        if live.iter().any(|label| !have.contains(label)) {
            changed.push(name.to_string());
        }
    }
    if changed.is_empty() {
        return None;
    }
    let extra = if changed.len() > 5 { " ..." } else { "" };
    let shown = changed
        .iter()
        .take(5)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "cached {} schema is out of date: the server has new choices for {shown}{extra}; run `fsv {} fields --refresh` (or `fsv cache clear schema`) and review again",
        res.name, res.name
    ))
}

#[cfg(test)]
mod load_tests {
    //! `tests/test_schema_cache.py`, over a real client and a temp cache.

    use std::sync::Arc;
    use std::time::Duration;

    use fsv_core::client::Response;
    use fsv_core::client::mock::MockTransport;
    use fsv_core::resource::CHANGES;
    use fsv_core::{Ctx, Error};

    use super::*;

    const KEY: &str = "acme.freshservice.com";

    fn fields() -> Value {
        json!([{"name": "status", "label": "Status", "choices": [{"id": 1, "value": "Open"}]}])
    }

    /// A reply for one form-fields endpoint: a status alone, or 200 + JSON.
    #[derive(Clone)]
    enum R {
        Status(u16),
        Ok(Value),
    }

    struct World {
        client: Arc<Client>,
        mock: Arc<MockTransport>,
        dir: tempfile::TempDir,
    }

    impl World {
        fn new(int: R, v2: R) -> World {
            let dir = tempfile::tempdir().unwrap();
            let ctx = Arc::new(Ctx::new(Paths::under(dir.path())).with_tenant(KEY));
            let mock = MockTransport::new(move |r| {
                let rest = r.url.splitn(4, '/').nth(3).unwrap_or("");
                let path = format!("/{}", rest.split('?').next().unwrap_or(""));
                let reply = match path.as_str() {
                    "/api/_/change_form_fields" => int.clone(),
                    "/api/v2/change_form_fields" => v2.clone(),
                    p if p.starts_with("/a/") => {
                        return Ok(Response::html(
                            200,
                            r#"<html><meta name="csrf-token" content="tok"></html>"#,
                        ));
                    }
                    _ => R::Status(404),
                };
                Ok(match reply {
                    R::Status(s) => Response::json(s, &json!({"description": "nope"})),
                    R::Ok(v) => Response::json(200, &v),
                })
            });
            let cookies = [("_x_w", "w"), ("fw-session-id", "fw1")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let client = Client::builder(ctx)
                .transport(mock.clone())
                .cookies(cookies)
                .sleeper(|_| {})
                .build()
                .unwrap();
            World {
                client: Arc::new(client),
                mock,
                dir,
            }
        }

        fn paths(&self) -> Paths {
            Paths::under(self.dir.path())
        }

        fn file(&self) -> PathBuf {
            cache_path(&self.paths(), KEY, "changes")
        }

        fn load(&self, force: bool) -> Result<Value> {
            load_in(&CHANGES, &self.client, force, &self.paths(), KEY)
        }

        /// Form-field fetches made (`c.calls` in the Python fake).
        fn calls(&self) -> usize {
            self.mock
                .paths()
                .iter()
                .filter(|p| p.ends_with("change_form_fields"))
                .count()
        }

        fn data(&self) -> Value {
            let text = std::fs::read_to_string(self.file()).unwrap();
            serde_json::from_str::<Value>(&text).unwrap()["data"].clone()
        }

        fn write(&self, doc: &Value) {
            std::fs::create_dir_all(self.file().parent().unwrap()).unwrap();
            std::fs::write(self.file(), doc.to_string()).unwrap();
        }

        fn save(&self, data: &Value) {
            cache::save(&self.file(), "schema", data, "", 0).unwrap();
        }

        fn set_saved_at(&self, saved: Value) {
            let mut doc: Value =
                serde_json::from_str(&std::fs::read_to_string(self.file()).unwrap()).unwrap();
            doc["saved_at"] = saved;
            self.write(&doc);
        }
    }

    fn form(v: Value) -> R {
        R::Ok(json!({"change_fields": v}))
    }

    #[test]
    fn fetch_matches_python_fetch_schema() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/coverage_oracle.json")).unwrap();
        let reply = |v: &Value| match v.as_u64() {
            Some(s) => R::Status(s as u16),
            None => R::Ok(v.clone()),
        };
        for (name, case) in oracle["schema_fetch"].as_object().unwrap() {
            let w = World::new(reply(&case["int"]), reply(&case["v2"]));
            match fetch(w.client.as_ref(), &CHANGES) {
                Ok(fields) => assert_eq!(json!(fields), case["ok"], "{name}"),
                Err(Error::Api(e)) => assert_eq!(json!(e.status), case["status"], "{name}"),
                Err(e) => panic!("{name}: {e}"),
            }
        }
    }

    #[test]
    fn both_fetches_fail_raises_session_error_and_caches_nothing() {
        let w = World::new(R::Status(401), R::Status(403));
        assert!(matches!(w.load(false), Err(Error::Session(_))));
        assert!(!w.file().exists());
    }

    #[test]
    fn api_errors_propagate_when_no_session_error() {
        let w = World::new(R::Status(500), R::Status(404));
        let err = w.load(false).unwrap_err();
        assert_eq!(err.status(), Some(500), "{err}");
        assert!(!w.file().exists());
    }

    #[test]
    fn both_empty_raises_and_is_not_cached() {
        let w = World::new(form(json!([])), form(json!([])));
        let err = w.load(false).unwrap_err();
        assert_eq!(err.status(), Some(200));
        assert!(
            err.to_string()
                .contains("no form fields returned for changes"),
            "{err}"
        );
        assert!(!w.file().exists());
    }

    #[test]
    fn v2_fallback_used_when_internal_fails() {
        let w = World::new(R::Status(404), form(fields()));
        let got = w.load(false).unwrap();
        assert_eq!(got["fields"], fields());
        assert_eq!(
            (got["stale"].clone(), got["source"].clone()),
            (json!(false), json!("server"))
        );
        assert_eq!(w.data(), fields());
    }

    #[test]
    fn v2_choices_are_merged_when_v2_has_more() {
        let v2 = json!([{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Closed"}]}]);
        let w = World::new(form(fields()), form(v2.clone()));
        let got = w.load(false).unwrap();
        assert_eq!(got["fields"][0]["choices"], v2[0]["choices"]);
        assert_eq!(got["fields"][0]["label"], json!("Status"));
    }

    #[test]
    fn poisoned_cache_entry_is_a_miss() {
        for payload in [json!([]), Value::Null, json!({"oops": 1})] {
            let w = World::new(form(fields()), form(json!([])));
            w.save(&payload);
            assert_eq!(w.load(false).unwrap()["fields"], fields());
            assert_eq!(w.calls(), 2, "{payload}");
            assert_eq!(w.data(), fields());
        }
    }

    #[test]
    fn valid_cache_is_served_without_fetch() {
        let w = World::new(R::Status(401), R::Status(401));
        w.save(&fields());
        let got = w.load(false).unwrap();
        assert_eq!(got["fields"], fields());
        assert_eq!(
            (got["stale"].clone(), got["source"].clone()),
            (json!(false), json!("cache"))
        );
        assert_eq!(w.calls(), 0);
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(w.file()).unwrap()).unwrap();
        assert_eq!(doc["fetch_count"], json!(2));
    }

    #[test]
    fn stale_cache_is_marked_and_refreshed_in_the_background() {
        let fresh = json!([{"name": "status", "label": "Status (new)"}]);
        for saved in [
            json!(1),
            json!(true),
            json!("unknown"),
            json!(999_999_999_999_i64),
            Value::Null,
        ] {
            let w = World::new(form(fresh.clone()), form(json!([])));
            w.save(&fields());
            w.set_saved_at(saved.clone());
            let got = w.load(false).unwrap();
            assert_eq!(got["fields"], fields(), "{saved}");
            assert_eq!(
                (got["stale"].clone(), got["source"].clone()),
                (json!(true), json!("cache")),
                "{saved}"
            );
            let mut refreshed = false;
            for _ in 0..100 {
                if w.data() == fresh {
                    refreshed = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(refreshed, "{saved}");
        }
    }

    #[test]
    fn force_refresh_bypasses_even_fresh_cache_and_returns_provenance() {
        let w = World::new(form(fields()), form(json!([])));
        w.save(&json!([{"name": "old"}]));
        let got = w.load(true).unwrap();
        assert_eq!(got["fields"], fields());
        assert_eq!(
            (got["stale"].clone(), got["source"].clone()),
            (json!(false), json!("server"))
        );
        assert!(got["saved_at"].as_f64().unwrap() > 0.0);
        assert_eq!(w.calls(), 2);
        assert_eq!(w.data(), fields());
    }

    #[test]
    fn failed_force_refresh_does_not_fall_back_to_stale_cache() {
        let w = World::new(R::Status(401), R::Status(401));
        w.save(&fields());
        assert!(matches!(w.load(true), Err(Error::Session(_))));
        assert_eq!(w.data(), fields());
    }

    #[test]
    fn legacy_cache_uses_schema_ttl_not_generic_ttl() {
        let w = World::new(R::Status(401), R::Status(401));
        w.write(&json!({"fields": fields(), "saved_at": now() - 7200.0}));
        let got = w.load(false).unwrap();
        assert_eq!(
            (got["stale"].clone(), got["source"].clone()),
            (json!(false), json!("cache"))
        );
        assert_eq!(got["fields"], fields());
        assert_eq!(w.calls(), 0);
    }

    #[test]
    fn cache_older_than_the_ttl_is_stale() {
        let w = World::new(R::Status(401), R::Status(401));
        w.write(&json!({"_kind": "schema", "data": fields(), "saved_at": now() - CACHE_TTL_SECS as f64 - 60.0}));
        assert_eq!(w.load(false).unwrap()["stale"], json!(true));
    }

    /// `schema.py`: `print(f"Warning: stale {res.name} schema; ...", file=sys.stderr)`.
    #[test]
    fn quiet_load_returns_the_stale_warning_instead_of_printing() {
        let w = World::new(R::Status(401), R::Status(401));
        w.write(&json!({"_kind": "schema", "data": fields(), "saved_at": now() - CACHE_TTL_SECS as f64 - 60.0}));
        let (got, warning) = load_in_quiet(&CHANGES, &w.client, false, &w.paths(), KEY).unwrap();
        assert_eq!(got["stale"], json!(true));
        assert_eq!(
            warning.as_deref(),
            Some(
                "Warning: stale changes schema; refreshing in background. Run `fsv changes fields --refresh` for current definitions."
            )
        );
        w.save(&fields());
        let (got, warning) = load_in_quiet(&CHANGES, &w.client, false, &w.paths(), KEY).unwrap();
        assert_eq!((got["stale"].clone(), warning), (json!(false), None));
    }

    #[test]
    fn cache_path_matches_the_process_config() {
        let p = Paths::under(Path::new("/x"));
        assert_eq!(
            cache_path(&p, KEY, "changes"),
            Path::new("/x/cache/schema/acme.freshservice.com--changes.json")
        );
        let process = cache_path(&config::paths(), &config::cache_scope_key(None), "tickets");
        assert_eq!(process, config::schema_cache_path("tickets", None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `saved_at` strings `NOW<offset>` in the oracle stand for `time.time() + offset`.
    #[test]
    fn cached_doc_matches_python_cached_path() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/coverage_oracle.json")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        for (name, case) in oracle["schema_cache"].as_object().unwrap() {
            let mut paths = Vec::new();
            for (i, doc) in case["docs"].as_array().unwrap().iter().enumerate() {
                let mut doc = doc.clone();
                if let Some(off) = doc["saved_at"].as_str().and_then(|s| s.strip_prefix("NOW")) {
                    doc["saved_at"] = json!(now() + off.parse::<f64>().unwrap());
                }
                let p = dir.path().join(format!("{name}{i}.json"));
                std::fs::write(&p, doc.to_string()).unwrap();
                paths.push(p);
            }
            let got = cached_doc(&paths);
            assert_eq!(got.is_some(), case["found"] == json!(true), "{name}");
            if let Some((path, _, stale)) = got {
                let index = paths.iter().position(|p| *p == path);
                assert_eq!(json!(index), case["index"], "{name}");
                assert_eq!(json!(stale), case["stale"], "{name}");
            }
        }
    }

    #[test]
    fn doc_fields_rejects_empty_and_reads_either_key() {
        assert!(doc_fields(&json!({"fields": []})).is_none());
        assert!(doc_fields(&json!([])).is_none());
        assert_eq!(
            doc_fields(&json!({"data": [{"name": "status"}]}))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn unwrap_keeps_an_explicit_empty_field_list_out_of_the_miss_path() {
        let doc = json!({"saved_at": 3, "fields": []})
            .as_object()
            .unwrap()
            .clone();
        let got = unwrap_schema(&doc);
        assert_eq!(
            got.get("fields").and_then(Value::as_array).unwrap().len(),
            0
        );
        assert_eq!(got.get("saved_at"), Some(&json!(3)));
    }

    #[test]
    fn choice_label_uses_python_equality_and_string_fallback() {
        let schema = json!({"fields": [{"name": "priority", "choices": [
            {"id": 1, "value": "Low"},
            {"id": 2, "value": "Medium"}
        ]}]});
        assert_eq!(choice_label("priority", &json!(1), &schema), "Low");
        assert_eq!(choice_label("priority", &json!(1.0), &schema), "Low");
        assert_eq!(choice_label("priority", &json!(true), &schema), "Low");
        assert_eq!(choice_label("priority", &json!(9), &schema), "9");
        assert_eq!(choice_label("priority", &Value::Null, &schema), "");
        assert_eq!(choice_label("missing", &json!(1), &schema), "1");
    }

    #[test]
    fn merge_keeps_internal_choices_unless_v2_has_more() {
        let mut int_fields = vec![
            json!({"name": "status", "choices": [{"id": 1}]}),
            json!({"name": "source", "choices": [{"id": 1}]}),
        ];
        let v2 = vec![
            json!({"name": "status", "choices": [{"id": 1}, {"id": 2}]}),
            json!({"name": "source", "choices": []}),
            json!({"id": 9, "choices": [{"id": 1}, {"id": 2}, {"id": 3}]}),
        ];
        merge_v2_choices(&mut int_fields, &v2);
        assert_eq!(int_fields[0]["choices"].as_array().unwrap().len(), 2);
        assert_eq!(int_fields[1]["choices"].as_array().unwrap().len(), 1);
    }
}
