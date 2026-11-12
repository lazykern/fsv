//! Attachment downloads (`cli_sr._download`) and the download inventory rows
//! (`cli_sr._attachment_rows`).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use fsv_py::{py_str, truthy};

use crate::catalog::{domain_url, is_file_value, safe_target};
use crate::create::attachment_url;
use crate::service::Api;

fn objects(v: Option<&Value>) -> impl Iterator<Item = &Map<String, Value>> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
}

fn with_source(row: &Map<String, Value>, source: &str) -> Value {
    let mut row = row.clone();
    row.insert("source".into(), json!(source));
    Value::Object(row)
}

/// `_attachment_rows(record, items, [{} ...])`: record attachments, then
/// each requested item's general attachments (`requested item`) and the
/// files in its file fields tagged with the field name (deduplicated by id).
pub fn attachment_rows(record: &Value, items: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for a in objects(record.get("attachments")) {
        out.push(with_source(a, "request"));
        if let Some(id) = a.get("id").filter(|v| !v.is_null()) {
            by_id.insert(py_str(id), out.len() - 1);
        }
    }
    for it in items {
        for a in objects(it.get("attachments")) {
            let key = a.get("id").filter(|v| !v.is_null()).map(py_str);
            if key.as_ref().is_some_and(|k| by_id.contains_key(k)) {
                continue;
            }
            out.push(with_source(a, "requested item"));
            if let Some(key) = key {
                by_id.insert(key, out.len() - 1);
            }
        }
        let Some(fields) = it.get("custom_fields").and_then(Value::as_object) else {
            continue;
        };
        for (name, v) in fields {
            if !is_file_value(v) {
                continue;
            }
            for x in v.as_array().into_iter().flatten() {
                let key = match x {
                    Value::Object(m) => py_str(m.get("id").unwrap_or(&Value::Null)),
                    other => py_str(other),
                };
                if let Some(&i) = by_id.get(&key) {
                    out[i]["source"] = json!(name);
                    continue;
                }
                let row = match x {
                    Value::Object(m) => with_source(m, name),
                    other => {
                        json!({"id": other, "name": format!("attachment-{}", py_str(other)), "source": name})
                    }
                };
                out.push(row);
                by_id.insert(key, out.len() - 1);
            }
        }
    }
    out
}

/// Rows of a `{"attachments": [...]}` list tagged with `source`.
pub fn sourced(list: Option<&Value>, source: &str) -> Vec<Value> {
    objects(list).map(|a| with_source(a, source)).collect()
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

/// `cli_sr._download(c, att, directory, force=False)`: never overwrites;
/// the result row's `status` is `downloaded`, `exists`, `rejected` or `error`.
pub fn download(c: &dyn Api, att: &Value, directory: &str) -> fsv_core::Result<Value> {
    let name = match att.get("name").filter(|v| truthy(v)) {
        Some(n) => py_str(n),
        None => format!(
            "attachment-{}",
            att.get("id").map(py_str).unwrap_or_default()
        ),
    };
    let mut row = Map::new();
    row.insert("id".into(), att.get("id").cloned().unwrap_or(Value::Null));
    row.insert("name".into(), json!(name));
    row.insert(
        "source".into(),
        att.get("source").cloned().unwrap_or(Value::Null),
    );
    let finish = |mut row: Map<String, Value>, extra: Value| {
        if let Value::Object(m) = extra {
            row.extend(m);
        }
        Ok(Value::Object(row))
    };
    let target = match safe_target(directory, &name) {
        Ok(t) => t,
        Err(e) => return finish(row, json!({"status": "rejected", "error": e.to_string()})),
    };
    let domain = c.domain();
    let mut url = attachment_url(att, &domain);
    if url.is_empty()
        && let Some(id) = att.get("id").filter(|v| !v.is_null())
    {
        url = domain_url(
            &format!("/helpdesk/attachments/{}?download=true", py_str(id)),
            &domain,
        );
    }
    if url.is_empty() {
        return finish(row, json!({"status": "error", "error": "no download URL"}));
    }
    let shown = target.to_string_lossy().into_owned();
    if target.exists() {
        return finish(row, json!({"path": shown, "status": "exists"}));
    }
    let r = c.fetch(&url)?;
    if r.status >= 400 {
        return finish(
            row,
            json!({"status": "error", "error": format!("HTTP {}", r.status)}),
        );
    }
    std::fs::create_dir_all(expanduser(directory))?;
    match write_new(&target, &r.body) {
        Ok(()) => finish(
            row,
            json!({"path": shown, "size": r.body.len(), "status": "downloaded"}),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            finish(row, json!({"path": shown, "status": "exists"}))
        }
        Err(e) => Err(e.into()),
    }
}

fn expanduser(p: &str) -> PathBuf {
    match (p.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(p),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fsv_core::client::Response;
    use fsv_core::client::mock::MockTransport;
    use fsv_core::config::Paths;
    use fsv_core::{Client, Ctx};

    use super::*;

    fn compact(v: &Value) -> String {
        serde_json::to_string(v).unwrap()
    }

    #[test]
    fn requested_item_files_take_the_field_name_and_dedupe_by_id() {
        let record = json!({"attachments": [{"id": 1, "name": "a"}, {"name": "noid"}]});
        let items = [
            json!({"custom_fields": {"laptop_photo": [{"id": 1, "name": "a"}, {"id": 8, "name": "b", "attachment_url": "u"}], "proof": [9], "note": "x"}}),
        ];
        assert_eq!(
            compact(&json!(attachment_rows(&record, &items))),
            r#"[{"id":1,"name":"a","source":"laptop_photo"},{"name":"noid","source":"request"},{"id":8,"name":"b","attachment_url":"u","source":"laptop_photo"},{"id":9,"name":"attachment-9","source":"proof"}]"#
        );
    }

    /// Requested-item general attachments (`gen_fsv_api_sr_attachments.py`).
    #[test]
    fn requested_item_general_attachments_match_python() {
        let oracle: Value =
            serde_json::from_str(include_str!("testdata/sr_attachments_oracle.json")).unwrap();
        for (name, case) in oracle.as_object().unwrap() {
            if name == "_python" {
                continue;
            }
            let items = case["items"].as_array().unwrap();
            assert_eq!(
                json!(attachment_rows(&case["record"], items)),
                case["rows"],
                "{name}"
            );
        }
    }

    #[test]
    fn downloads_write_new_files_only_like_cli_sr_download() {
        let home = tempfile::tempdir().unwrap();
        let ctx =
            Arc::new(Ctx::new(Paths::under(home.path())).with_tenant("acme.freshservice.com"));
        let mock = MockTransport::new(|r| {
            let path = r.url.splitn(4, '/').nth(3).unwrap_or("");
            Ok(match path.split('?').next().unwrap_or("") {
                "helpdesk/attachments/1" => Response::new(200).with_body(b"hello".to_vec()),
                "helpdesk/attachments/6" => Response::new(200).with_body(b"six".to_vec()),
                "helpdesk/attachments/7" => Response::new(500),
                _ => Response::new(404),
            })
        });
        let client = Client::builder(ctx)
            .transport(mock.clone())
            .sleeper(|_| {})
            .build()
            .unwrap();
        let out = tempfile::tempdir().unwrap();
        let dir = out.path().to_string_lossy().into_owned();
        let real = std::fs::canonicalize(out.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let get =
            |att: Value| compact(&download(&client, &att, &dir).unwrap()).replace(&real, "<DIR>");
        let att = json!({"id": 1, "name": "a.txt", "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/1", "source": "request"});
        assert_eq!(
            get(att.clone()),
            r#"{"id":1,"name":"a.txt","source":"request","path":"<DIR>/a.txt","size":5,"status":"downloaded"}"#
        );
        assert_eq!(
            get(att),
            r#"{"id":1,"name":"a.txt","source":"request","path":"<DIR>/a.txt","status":"exists"}"#
        );
        assert_eq!(
            get(json!({"id": 2, "name": "../x"})),
            r#"{"id":2,"name":"../x","source":null,"status":"rejected","error":"unsafe file name '../x': contains '..'"}"#
        );
        assert_eq!(
            get(json!({"id": 6, "name": "six.bin", "source": "cloud file"})),
            r#"{"id":6,"name":"six.bin","source":"cloud file","path":"<DIR>/six.bin","size":3,"status":"downloaded"}"#
        );
        assert_eq!(
            get(json!({"name": "z"})),
            r#"{"id":null,"name":"z","source":null,"status":"error","error":"no download URL"}"#
        );
        assert_eq!(
            get(
                json!({"id": 7, "name": "e", "attachment_url": "https://acme.freshservice.com/helpdesk/attachments/7"})
            ),
            r#"{"id":7,"name":"e","source":null,"status":"error","error":"HTTP 500"}"#
        );
        assert_eq!(
            get(
                json!({"id": 6, "canonical_url": "https://acme.freshservice.com/helpdesk/attachments/6"})
            ),
            r#"{"id":6,"name":"attachment-6","source":null,"path":"<DIR>/attachment-6","size":3,"status":"downloaded"}"#
        );
        let fetched: Vec<String> = mock
            .requests()
            .iter()
            .filter(|r| r.url.contains("/helpdesk/"))
            .map(|r| r.url.clone())
            .collect();
        assert_eq!(
            fetched,
            [
                "https://acme.freshservice.com/helpdesk/attachments/1?download=true",
                "https://acme.freshservice.com/helpdesk/attachments/6?download=true",
                "https://acme.freshservice.com/helpdesk/attachments/7?download=true",
                "https://acme.freshservice.com/helpdesk/attachments/6?download=true",
            ]
        );
        let mut names: Vec<String> = std::fs::read_dir(out.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "attachment-6", "six.bin"]);
        assert_eq!(std::fs::read(out.path().join("a.txt")).unwrap(), b"hello");
    }
}
