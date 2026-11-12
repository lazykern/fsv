//! Stable read-side baselines, never normalization of submitted write
//! payloads (`reviewed_snapshot.py`).

use serde_json::{Map, Value};

use crate::pystr;

/// Attachment delivery locations and server bookkeeping are not file identity.
const ATTACHMENT_FIELDS: [&str; 12] = [
    "id",
    "name",
    "size",
    "content_type",
    "attachable_type",
    "parent_id",
    "has_access",
    "uploaded_by",
    "inline",
    "is_inline",
    "deleted",
    "is_deleted",
];
const VOLATILE_FIELDS: [&str; 6] = [
    "created_at",
    "updated_at",
    "last_viewed_at",
    "fetched_at",
    "expires_at",
    "url_expires_at",
];
const SIGNATURE_KEYS: [&str; 4] = ["signature", "policy", "key-pair-id", "awsaccesskeyid"];

fn signing(key: &str) -> bool {
    let key = pystr::casefold(key);
    SIGNATURE_KEYS.contains(&key.as_str()) || key.starts_with("x-amz-")
}

fn signed_query(key: &str) -> bool {
    signing(key) || pystr::casefold(key) == "expires"
}

/// Port of reviewed_snapshot._stable_url: a signed link keeps its path and
/// non-signing query; any other link is returned unchanged.
fn stable_url(original: &str) -> Result<String, String> {
    let mut parsed = pystr::urlsplit(&pystr::html_unescape(original))?;
    let pairs = pystr::parse_qsl(&parsed.query, true);
    if !pairs.iter().any(|(k, _)| signing(k)) {
        return Ok(original.to_string());
    }
    let kept: Vec<&(String, String)> = pairs.iter().filter(|(k, _)| !signed_query(k)).collect();
    parsed.query = pystr::urlencode(&kept.iter().map(|(k, v)| (k, v)).collect::<Vec<_>>());
    let result = pystr::urlunsplit(&parsed);
    Ok(if original.contains("&amp;") {
        result.replace('&', "&amp;")
    } else {
        result
    })
}

/// End of the `https?://[^\s<>"']+` match starting at `start`, if any.
fn url_end(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    let body = rest
        .strip_prefix("https://")
        .or_else(|| rest.strip_prefix("http://"))?;
    let len = body
        .find(|c: char| pystr::is_space(c) || matches!(c, '<' | '>' | '"' | '\''))
        .unwrap_or(body.len());
    (len > 0).then(|| start + (rest.len() - body.len()) + len)
}

/// `_URL.sub(_stable_url, text)`.
fn stable_text(text: &str) -> Result<String, String> {
    if !text.contains("http") {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while let Some(found) = text[at..].find("http") {
        let start = at + found;
        match url_end(text, start) {
            Some(end) => {
                out.push_str(&text[copied..start]);
                out.push_str(&stable_url(&text[start..end])?);
                copied = end;
                at = end;
            }
            None => at = start + 1,
        }
    }
    out.push_str(&text[copied..]);
    Ok(out)
}

fn project(value: &Value, attachment: bool) -> Result<Value, String> {
    Ok(match value {
        Value::Object(m) => {
            let mut out = Map::new();
            for (key, item) in m {
                let keep = if attachment {
                    ATTACHMENT_FIELDS.contains(&key.as_str())
                } else {
                    !VOLATILE_FIELDS.contains(&key.as_str())
                };
                if !keep {
                    continue;
                }
                let item = if key == "custom_fields" {
                    item.clone()
                } else {
                    project(item, key == "attachments" || key == "files")?
                };
                out.insert(key.clone(), item);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|item| project(item, attachment))
                .collect::<Result<_, _>>()?,
        ),
        Value::String(s) => Value::String(stable_text(s)?),
        other => other.clone(),
    })
}

/// Port of reviewed_snapshot.stable_snapshot: server reads projected onto
/// content and identity, keeping unknown business fields.
///
/// File inventories (`attachments`, `files`) keep only the stable file
/// fields; other records drop server timestamps; `custom_fields` are kept
/// verbatim. Signed links in text lose their signing query. `Err` carries
/// the `ValueError` text of a link `urllib.parse.urlsplit` rejects.
pub fn stable_snapshot(value: &Value) -> Result<Value, String> {
    project(value, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{plan_hash, snapshot_hash};
    use serde_json::json;

    fn oracle() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/reviewed_snapshot.json")).unwrap()
    }

    #[test]
    fn snapshots_match_python() {
        let doc = oracle();
        let rows = doc["snapshots"].as_array().unwrap();
        assert!(rows.len() > 50);
        for row in rows {
            let value = &row["value"];
            let body = (!value.is_null()).then_some(value);
            assert_eq!(json!(plan_hash(body)), row["plan_hash"], "{value}");
            match row.get("error") {
                Some(err) => {
                    assert_eq!(stable_snapshot(value).unwrap_err(), err.as_str().unwrap());
                    assert!(snapshot_hash(body).is_err());
                }
                None => {
                    assert_eq!(stable_snapshot(value).unwrap(), row["snapshot"], "{value}");
                    assert_eq!(
                        json!(snapshot_hash(body).unwrap()),
                        row["snapshot_hash"],
                        "{value}"
                    );
                }
            }
        }
    }

    #[test]
    fn urlsplit_and_urlunsplit_match_python() {
        let doc = oracle();
        for row in doc["urlsplit"].as_array().unwrap() {
            let url = row["url"].as_str().unwrap();
            match row.get("error") {
                Some(err) => assert_eq!(pystr::urlsplit(url).unwrap_err(), err.as_str().unwrap()),
                None => {
                    let s = pystr::urlsplit(url).unwrap_or_else(|e| panic!("{url:?}: {e}"));
                    let got = json!([s.scheme, s.netloc, s.path, s.query, s.fragment]);
                    assert_eq!(got, row["split"], "{url:?}");
                    assert_eq!(
                        pystr::urlunsplit(&s),
                        row["unsplit"].as_str().unwrap(),
                        "{url:?}"
                    );
                }
            }
        }
        for row in doc["urlunsplit"].as_array().unwrap() {
            let p: Vec<String> = serde_json::from_value(row[0].clone()).unwrap();
            let s = pystr::SplitUrl {
                scheme: p[0].clone(),
                netloc: p[1].clone(),
                path: p[2].clone(),
                query: p[3].clone(),
                fragment: p[4].clone(),
            };
            assert_eq!(pystr::urlunsplit(&s), row[1].as_str().unwrap(), "{p:?}");
        }
    }

    #[test]
    fn python_reviewed_snapshot_properties() {
        let attachment = |sig: &str| {
            json!({"id": 701, "name": "acme.txt", "size": 12, "content_type": "text/plain",
                "attachment_url": format!("https://files.example.com/acme.txt?Expires=100&Signature={sig}&Key-Pair-Id=key"),
                "created_at": sig, "updated_at": sig, "has_access": true})
        };
        let before = json!({"parent": {"id": 123, "attachments": [attachment("first")]}});
        let fresh = json!({"parent": {"id": 123, "attachments": [attachment("second")]}});
        assert_eq!(snapshot_hash(Some(&before)), snapshot_hash(Some(&fresh)));
        assert_ne!(plan_hash(Some(&before)), plan_hash(Some(&fresh)));
        assert_eq!(snapshot_hash(None), Ok(None));
    }
}
