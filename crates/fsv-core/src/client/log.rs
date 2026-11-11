//! `--verbose` request log formatting with secret redaction.

use serde_json::{Map, Value};

use super::Body;
use crate::pystr;

/// A key is redacted when it *contains* any of these (case-insensitive,
/// `_` and `-` ignored), so `x_csrf_token`, `client_secret`, `fw-session-id`,
/// `passwd` and `Set-Cookie` are all covered.
pub const REDACT_SUBSTRINGS: &[&str] = &[
    "token",
    "secret",
    "pass",
    "csrf",
    "cookie",
    "session",
    "authorization",
    "apikey",
    "credential",
];

pub fn is_sensitive_key(key: &str) -> bool {
    let k = pystr::strip(key).to_lowercase().replace(['_', '-'], "");
    REDACT_SUBSTRINGS.iter().any(|s| k.contains(s))
}

/// Replace values of sensitive keys with `[redacted]`, recursively.
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if is_sensitive_key(k) {
                            Value::String("[redacted]".into())
                        } else {
                            redact(v)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(redact).collect()),
        other => other.clone(),
    }
}

/// Query string shown in logs: the URL's own query plus `params` (None skipped).
pub fn query_string(url: &str, params: Option<&Map<String, Value>>) -> String {
    let query = pystr::urlsplit(url).map(|u| u.query).unwrap_or_default();
    let mut parts = pystr::parse_qsl(&query, true);
    for (key, value) in params.into_iter().flatten() {
        match value {
            Value::Null => continue,
            Value::Array(items) => {
                for item in items {
                    let v = if item.is_null() {
                        String::new()
                    } else {
                        fsv_py::py_str(item)
                    };
                    parts.push((key.clone(), v));
                }
            }
            other => parts.push((key.clone(), fsv_py::py_str(other))),
        }
    }
    parts
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                pystr::quote(k, "-._~"),
                pystr::quote(v, "-._~,/:@")
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// `path?query` for logs and `APIError.target`.
pub fn request_target(url: &str, params: Option<&Map<String, Value>>) -> String {
    let path = pystr::urlsplit(url).map(|u| u.path).unwrap_or_default();
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        path
    };
    let query = query_string(url, params);
    if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    }
}

fn compact(v: &Value) -> String {
    fsv_py::compact(&redact(v))
}

/// Redacted preview of a `data=` payload (JSON text or form-encoded).
pub fn redact_data(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data).into_owned();
    if let Ok(parsed @ (Value::Object(_) | Value::Array(_))) = serde_json::from_str::<Value>(&text)
    {
        return compact(&parsed);
    }
    let pairs = pystr::parse_qsl(&text, true);
    if !pairs.is_empty() && text.contains('=') {
        let redacted: Vec<(String, String)> = pairs
            .into_iter()
            .map(|(k, v)| {
                let v = if is_sensitive_key(&k) {
                    "[redacted]".to_string()
                } else {
                    v
                };
                (k, v)
            })
            .collect();
        return pystr::urlencode(&redacted);
    }
    if is_sensitive_key(&text) {
        return "[redacted]".into();
    }
    text
}

/// ` body=...` suffix (empty when there is nothing to show), capped at 300 chars.
pub fn body_preview(body: &Body) -> String {
    let raw = match body {
        Body::Json(v) => compact(v),
        Body::Data(d) => redact_data(d),
        Body::None | Body::Raw(_) => String::new(),
    };
    if raw.is_empty() {
        return String::new();
    }
    let raw = if raw.chars().count() > 300 {
        format!("{}...", raw.chars().take(297).collect::<String>())
    } else {
        raw
    };
    format!(" body={raw}")
}

/// `[api] METHOD path?query[ body=...] -> outcome`; bodies are omitted for GET/HEAD.
pub fn log_line(
    method: &str,
    url: &str,
    params: Option<&Map<String, Value>>,
    body: &Body,
    outcome: &str,
) -> String {
    let upper = method.to_uppercase();
    let body = if matches!(upper.as_str(), "GET" | "HEAD") {
        String::new()
    } else {
        body_preview(body)
    };
    format!(
        "[api] {upper} {}{body} -> {outcome}",
        request_target(url, params)
    )
}

/// `Retry-After` in seconds, clamped to `[0, 10]`; `None` when not a number.
pub fn retry_after_seconds(raw: Option<&str>) -> Option<f64> {
    let raw = raw.filter(|r| !r.is_empty())?;
    let cleaned = pystr::strip(raw).replace('_', "");
    let f: f64 = cleaned.parse().ok()?;
    if f.is_nan() {
        return Some(0.0);
    }
    Some(f.clamp(0.0, 10.0))
}
