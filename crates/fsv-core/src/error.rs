//! Error type shared by every fsv crate (`errors.py`).
//!
//! Variants mirror the Python exception classes so [`Error::exit_code`]
//! reproduces `errors.exit_code_for`.

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::pystr;

/// Success.
pub const EXIT_OK: i32 = 0;
/// Anything not covered below.
pub const EXIT_OTHER: i32 = 1;
/// Validation / usage error.
pub const EXIT_USAGE: i32 = 2;
/// Session expired or missing.
pub const EXIT_SESSION: i32 = 3;
/// Conflict / duplicate.
pub const EXIT_CONFLICT: i32 = 4;
/// Server rejected the request (4xx/5xx).
pub const EXIT_REJECTED: i32 = 5;

/// Crate-wide result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// fsv error (Python exception hierarchy flattened).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `APIError`: the server (or engine) rejected a request. Boxed so
    /// `Result<T>` stays small.
    #[error("{0}")]
    Api(Box<ApiError>),
    /// `SessionError`: no session, expired session, wrong tenant.
    #[error("{0}")]
    Session(String),
    /// `ValueError` and subclasses (usage/validation) → exit 2.
    #[error("{0}")]
    Value(String),
    /// Flow exits (`FlowExit`, `typer.Exit`, `SystemExit`). An empty message
    /// means the reason was already printed.
    #[error("{message}")]
    Exit { code: i32, message: String },
    /// `EditorAbort`: the user left the editor unchanged.
    #[error("{0}")]
    EditorAbort(String),
    /// Transport failure (`httpx.TransportError`); `kind` is the Python class name.
    #[error("{message}")]
    Http { kind: String, message: String },
    /// Any other exception (`RuntimeError`, ...) → exit 1.
    #[error("{0}")]
    Other(String),
    /// I/O failure.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Ctrl-C or a refused `confirm(..., abort=True)` (click `Abort`).
    #[error("Aborted!")]
    Aborted,
    /// `raise Outer(message) from source`: keeps the cause's exit-code class.
    #[error("{message}")]
    Context {
        message: String,
        #[source]
        source: Box<Error>,
    },
}

impl Error {
    /// `SessionError(msg)`.
    pub fn session(msg: impl Into<String>) -> Self {
        Error::Session(msg.into())
    }
    /// `ValueError(msg)`.
    pub fn value(msg: impl Into<String>) -> Self {
        Error::Value(msg.into())
    }
    /// `RuntimeError(msg)` / generic exception.
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
    /// Flow exit with an explicit code.
    pub fn exit(code: i32, msg: impl Into<String>) -> Self {
        Error::Exit {
            code,
            message: msg.into(),
        }
    }
    /// Wrap as `raise Other(message) from self`.
    pub fn context(self, message: impl Into<String>) -> Self {
        Error::Context {
            message: message.into(),
            source: Box::new(self),
        }
    }

    /// The API error, if this is one (not looking through `Context`).
    pub fn as_api(&self) -> Option<&ApiError> {
        match self {
            Error::Api(e) => Some(e),
            _ => None,
        }
    }

    /// HTTP status of an API error (Python `getattr(e, "status", None)`).
    pub fn status(&self) -> Option<u16> {
        self.as_api().map(|e| e.status)
    }

    /// `exit_code_for`: the documented process exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Session(_) => EXIT_SESSION,
            Error::Api(e) => e.exit_code(),
            Error::Value(_) => EXIT_USAGE,
            Error::Exit { code, .. } => *code,
            Error::Aborted => EXIT_OTHER,
            Error::Context { source, .. } => match source.as_ref() {
                inner @ (Error::Api(_) | Error::Session(_)) => inner.exit_code(),
                _ => EXIT_OTHER,
            },
            Error::EditorAbort(_) | Error::Http { .. } | Error::Other(_) | Error::Io(_) => {
                EXIT_OTHER
            }
        }
    }
}

impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Error::Api(Box::new(e))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        // json.JSONDecodeError is a ValueError.
        Error::Value(e.to_string())
    }
}

/// `APIError(status, body, method, target)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiError {
    pub status: u16,
    /// Parsed JSON body, or the text (first 500 chars) as a string.
    pub body: Value,
    pub method: Option<String>,
    pub target: Option<String>,
}

impl ApiError {
    /// Port of fsv.errors.APIError.__init__ without method and target.
    pub fn new(status: u16, body: impl Into<Value>) -> Self {
        Self {
            status,
            body: body.into(),
            method: None,
            target: None,
        }
    }

    /// Port of fsv.errors.APIError.__init__.
    pub fn with_request(
        status: u16,
        body: Value,
        method: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            status,
            body,
            method: Some(method.into()),
            target: Some(target.into()),
        }
    }

    /// Clean single-line message (Python `str(APIError)`).
    pub fn message(&self) -> String {
        let what = describe_request(self.method.as_deref(), self.target.as_deref());
        let msg = server_message(&self.body, 240);
        // Status 0 is a local guard failure, never a real HTTP response.
        let mut head = if self.status == 0 {
            "Error".to_string()
        } else {
            format!("HTTP {}", self.status)
        };
        if !what.is_empty() {
            head = format!("{head} {what}");
        }
        if msg.is_empty() {
            head
        } else {
            format!("{head}: {msg}")
        }
    }

    /// Request line plus the raw body (`APIError.detail()`).
    pub fn detail(&self) -> String {
        let where_ = format!(
            "{} {}",
            self.method.as_deref().unwrap_or("").to_uppercase(),
            self.target_str()
        );
        let where_ = pystr::strip(&where_);
        let body = match &self.body {
            Value::String(s) => s.clone(),
            other => fsv_py::py_repr(other),
        };
        if where_.is_empty() {
            body
        } else {
            format!("{where_}\n{body}")
        }
    }

    fn target_str(&self) -> String {
        self.target.clone().unwrap_or_else(|| "None".into())
    }

    fn exit_code(&self) -> i32 {
        static DUPLICATE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)already (exists|been taken|associated)|duplicate").expect("regex")
        });
        if self.status == 409
            || (matches!(self.status, 400 | 422) && DUPLICATE.is_match(&fsv_py::py_str(&self.body)))
        {
            EXIT_CONFLICT
        } else {
            EXIT_REJECTED
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for ApiError {}

/// Remove `<script>`/`<style>` blocks and tags, unescape entities, collapse whitespace.
pub fn strip_tags(text: &str) -> String {
    static BLOCKS: LazyLock<fancy_regex::Regex> =
        LazyLock::new(|| fancy_regex::Regex::new(r"(?is)<(script|style).*?</\1>").expect("regex"));
    static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("regex"));
    let no_blocks = BLOCKS.replace_all(text, " ");
    let no_tags = TAGS.replace_all(&no_blocks, " ");
    pystr::collapse_ws(&pystr::html_unescape(&no_tags))
}

use crate::value::truthy_opt as truthy;

fn server_one(x: &Value) -> String {
    match x {
        Value::Object(map) => {
            let msg = ["message", "description", "error", "detail", "msg"]
                .iter()
                .find(|k| truthy(map.get(**k)))
                .and_then(|k| map.get(*k));
            let field = map.get("field");
            if msg.is_none() && truthy(map.get("errors")) {
                return server_one(&map["errors"]);
            }
            let text = msg.map(server_one).unwrap_or_default();
            if truthy(field) && !text.is_empty() {
                format!("{}: {text}", fsv_py::py_str(field.unwrap_or(&Value::Null)))
            } else {
                text
            }
        }
        Value::Array(items) => items
            .iter()
            .take(5)
            .map(server_one)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        Value::Null => String::new(),
        other => strip_tags(&fsv_py::py_str(other)),
    }
}

/// One-line human message extracted from an error response body.
pub fn server_message(body: &Value, limit: usize) -> String {
    let mut text = server_one(body);
    if let Value::Object(map) = body
        && truthy(map.get("errors"))
        && truthy(map.get("description"))
    {
        let extra = server_one(&map["errors"]);
        if !extra.is_empty() && !text.contains(&extra) {
            text = format!("{text} ({extra})");
        }
    }
    if text.chars().count() > limit {
        let cut: String = text.chars().take(limit.saturating_sub(1)).collect();
        text = format!("{}…", pystr::rstrip(&cut));
    }
    text
}

/// Short phrase for the request (`"creating change"`, `"listing tickets"`).
pub fn describe_request(method: Option<&str>, target: Option<&str>) -> String {
    let (Some(method), Some(target)) = (method, target) else {
        return String::new();
    };
    if method.is_empty() || target.is_empty() {
        return String::new();
    }
    let path = target.split_once('?').map_or(target, |(p, _)| p);
    let segs: Vec<&str> = path
        .split('/')
        .filter(|s| !s.is_empty() && !matches!(*s, "api" | "_" | "v2" | "a"))
        .collect();
    let named: Vec<&str> = segs
        .iter()
        .copied()
        .filter(|s| !pystr::is_digit(s))
        .collect();
    let noun = named
        .last()
        .copied()
        .unwrap_or("resource")
        .replace('_', " ");
    let is_item = segs.last().is_some_and(|s| pystr::is_digit(s));
    let m = method.to_uppercase();
    let singular = noun.trim_end_matches('s');
    if m == "GET" {
        return if is_item {
            format!("fetching {singular}")
        } else {
            format!("listing {noun}")
        };
    }
    let verb = match m.as_str() {
        "POST" => Some("creating"),
        "PUT" | "PATCH" => Some("updating"),
        "DELETE" => Some("deleting"),
        _ => None,
    };
    match verb {
        Some(v) => format!(
            "{v} {}",
            if is_item || m != "POST" {
                singular
            } else {
                noun.as_str()
            }
        ),
        None => format!("on {m} {path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exit_code_mapping() {
        assert_eq!(Error::session("x").exit_code(), 3);
        assert_eq!(Error::from(ApiError::new(409, "conflict")).exit_code(), 4);
        assert_eq!(
            Error::from(ApiError::new(
                422,
                json!({"errors": "Name has already been taken"})
            ))
            .exit_code(),
            4
        );
        assert_eq!(Error::from(ApiError::new(400, "bad")).exit_code(), 5);
        assert_eq!(Error::from(ApiError::new(503, "down")).exit_code(), 5);
        assert_eq!(Error::value("bad").exit_code(), 2);
        assert_eq!(Error::other("x").exit_code(), 1);
    }

    #[test]
    fn wrapped_api_error_keeps_cause_code() {
        let e = Error::from(ApiError::new(500, "x")).context("update failed");
        assert_eq!(e.exit_code(), 5);
        assert_eq!(e.to_string(), "update failed");
        assert_eq!(Error::value("v").context("x").exit_code(), 1);
    }

    #[test]
    fn exit_code_for_matches_python() {
        let codes = [
            Error::session("x").exit_code(),
            Error::from(ApiError::new(409, "c")).exit_code(),
            Error::from(ApiError::new(400, json!({"errors": "Duplicate record"}))).exit_code(),
            Error::from(ApiError::new(422, "Already associated")).exit_code(),
            Error::from(ApiError::new(404, "x")).exit_code(),
            Error::value("v").exit_code(),
            Error::other("r").exit_code(),
            Error::session("s").context("outer").exit_code(),
            Error::from(ApiError::new(409, "c"))
                .context("outer")
                .exit_code(),
            Error::value("v").context("outer").exit_code(),
        ];
        assert_eq!(codes, [3, 4, 4, 4, 5, 2, 1, 3, 4, 1]);
        assert_eq!(Error::exit(7, "").exit_code(), 7);
        assert_eq!(Error::Aborted.exit_code(), 1);
    }

    #[test]
    fn strip_tags_removes_blocks_and_tags() {
        assert_eq!(
            strip_tags("<p>Hi&amp;<b>x</b></p><script>bad()</script>  <STYLE>a{}</style>end"),
            "Hi& x end"
        );
    }

    #[test]
    fn server_message_shapes() {
        assert_eq!(
            server_message(
                &json!({"description": "Validation failed", "errors": [
                    {"field": "subject", "message": "is required"}, {"field": "x", "message": "bad"}
                ]}),
                240
            ),
            "Validation failed (subject: is required; x: bad)"
        );
        assert_eq!(
            server_message(&json!({"message": "x".repeat(300)}), 240),
            format!("{}…", "x".repeat(239))
        );
        assert_eq!(server_message(&Value::Null, 240), "");
        assert_eq!(
            server_message(&json!([{"msg": "a"}, {"detail": "b"}, null]), 240),
            "a; b"
        );
        assert_eq!(
            server_message(&json!({"errors": {"error": "nope"}}), 240),
            "nope"
        );
        assert_eq!(
            server_message(&json!("<h1>Not Found</h1>"), 240),
            "Not Found"
        );
        assert_eq!(server_message(&json!(42), 240), "42");
    }

    #[test]
    fn describe_request_phrases() {
        for (m, t, want) in [
            (Some("GET"), Some("/api/v2/changes?x=1"), "listing changes"),
            (Some("GET"), Some("/api/v2/changes/5"), "fetching change"),
            (Some("POST"), Some("/api/v2/changes"), "creating changes"),
            (
                Some("POST"),
                Some("/api/_/changes/5/notes"),
                "creating notes",
            ),
            (Some("PUT"), Some("/api/v2/changes/5"), "updating change"),
            (
                Some("DELETE"),
                Some("/api/_/ticket_filters/9"),
                "deleting ticket filter",
            ),
            (Some("PATCH"), Some("/api/_/changes"), "updating change"),
            (Some("OPTIONS"), Some("/api/v2/x"), "on OPTIONS /api/v2/x"),
            (None, Some("/x"), ""),
            (Some("GET"), None, ""),
            (Some("get"), Some("/"), "listing resource"),
            (
                Some("GET"),
                Some("/a/service_items"),
                "listing service items",
            ),
        ] {
            assert_eq!(describe_request(m, t), want, "{m:?} {t:?}");
        }
    }

    #[test]
    fn api_error_init_message_and_detail() {
        let e = ApiError::with_request(
            422,
            json!({"description": "Validation failed", "errors": [{"field": "subject", "message": "is required"}]}),
            "POST",
            "/api/v2/changes",
        );
        assert_eq!(
            e.to_string(),
            "HTTP 422 creating changes: Validation failed (subject: is required)"
        );
        assert_eq!(
            e.detail(),
            "POST /api/v2/changes\n{'description': 'Validation failed', 'errors': [{'field': 'subject', 'message': 'is required'}]}"
        );
        let e = ApiError::new(500, "");
        assert_eq!(
            (e.to_string(), e.detail()),
            ("HTTP 500".to_string(), "None\n".to_string())
        );
        let e = ApiError::with_request(404, json!("<h1>gone</h1>"), "get", "/api/v2/tickets/3");
        assert_eq!(e.message(), "HTTP 404 fetching ticket: gone");
        assert_eq!(e.detail(), "GET /api/v2/tickets/3\n<h1>gone</h1>");
        assert_eq!(Error::from(e).status(), Some(404));
    }

    /// `str(APIError(...))` at Python `92002d7` (`820af12`: status 0 heads with `Error`).
    #[test]
    fn api_error_status_zero_heads_with_error() {
        for (status, body, req, want) in [
            (0, json!(""), None, "Error"),
            (0, json!("boom"), None, "Error: boom"),
            (0, Value::Null, None, "Error"),
            (500, json!(""), None, "HTTP 500"),
            (500, json!("x"), None, "HTTP 500: x"),
            (
                0,
                json!("boom"),
                Some(("post", "/api/v2/changes")),
                "Error creating changes: boom",
            ),
            (0, json!(""), Some(("get", "x")), "Error listing x"),
            (
                500,
                json!("x"),
                Some(("put", "/api/v2/tickets/1?a=b")),
                "HTTP 500 updating ticket: x",
            ),
            (
                500,
                json!(""),
                Some(("delete", "/a")),
                "HTTP 500 deleting resource",
            ),
        ] {
            let e = match req {
                Some((m, t)) => ApiError::with_request(status, body, m, t),
                None => ApiError::new(status, body),
            };
            assert_eq!(e.to_string(), want);
        }
        let e = ApiError::with_request(0, json!("boom"), "post", "/api/v2/changes");
        assert_eq!(e.detail(), "POST /api/v2/changes\nboom");
    }
}
