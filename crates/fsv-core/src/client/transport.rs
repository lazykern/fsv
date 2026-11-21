//! One HTTP exchange. No redirects, cookies or retries: [`super::Client`]
//! does those so every transport (reqwest, test mocks) behaves the same.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

/// httpx `timeout=30`, applied per phase (connect, read), never as a total deadline.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// httpx `keepalive_expiry`: an idle connection is not reused after this.
const KEEPALIVE_EXPIRY: Duration = Duration::from_secs(5);

/// Request bodies above this size get no deadline on the send-until-headers
/// wait (blocking reqwest only offers a deadline there, and uploads exceed it).
const DEADLINE_BODY_LIMIT: usize = 1 << 20;

/// A single request as it goes on the wire.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// Read at most this many body bytes of a successful (< 400) response,
    /// and none at all when `Content-Length` already exceeds it: a download
    /// guard, like Python's streamed read that stops at the limit.
    pub body_limit: Option<u64>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }

    /// The body parsed as JSON, when it is JSON.
    pub fn json(&self) -> Option<Value> {
        self.body
            .as_deref()
            .and_then(|b| serde_json::from_slice(b).ok())
    }
}

/// A single response. `url` is the URL that produced it (the last hop).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    pub status: u16,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Response {
            status,
            ..Default::default()
        }
    }

    /// `application/json` response with `value` as the body.
    pub fn json(status: u16, value: &Value) -> Self {
        Response::new(status)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(serde_json::to_vec(value).unwrap_or_default())
    }

    /// `text/html` response.
    pub fn html(status: u16, text: &str) -> Self {
        Response::new(status)
            .with_header("content-type", "text/html; charset=utf-8")
            .with_body(text.as_bytes().to_vec())
    }

    /// Redirect to `location`.
    pub fn redirect(status: u16, location: &str) -> Self {
        Response::new(status).with_header("location", location)
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .push((name.to_ascii_lowercase(), value.to_string()));
        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    /// First value of a header (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        find_header(&self.headers, name)
    }

    /// Every value of a header, in order (e.g. `set-cookie`).
    pub fn header_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers
            .iter()
            .filter(move |(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn content_type(&self) -> &str {
        self.header("content-type").unwrap_or("")
    }

    /// Body decoded as text with the `charset` of the content type (httpx
    /// `Response.text`): UTF-8 unless it names latin-1 or ASCII; invalid bytes
    /// become U+FFFD.
    pub fn text(&self) -> String {
        let charset = self
            .content_type()
            .split(';')
            .skip(1)
            .find_map(|p| {
                let (k, v) = p.split_once('=')?;
                k.trim()
                    .eq_ignore_ascii_case("charset")
                    .then(|| v.trim().trim_matches(['"', '\'']).to_ascii_lowercase())
            })
            .unwrap_or_default();
        match charset.replace('_', "-").as_str() {
            "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" | "l1" | "8859" | "cp819" => {
                self.body.iter().map(|&b| char::from(b)).collect()
            }
            "ascii" | "us-ascii" | "646" => self
                .body
                .iter()
                .map(|&b| if b < 0x80 { char::from(b) } else { '\u{fffd}' })
                .collect(),
            _ => String::from_utf8_lossy(&self.body).into_owned(),
        }
    }

    /// Body parsed as JSON regardless of content type (`r.json()`): Python's
    /// `json.loads(bytes)`, which detects UTF-8 (with or without a BOM),
    /// UTF-16 and UTF-32.
    pub fn json_value(&self) -> Result<Value> {
        match decode_json_bytes(&self.body) {
            Some(text) => Ok(serde_json::from_str(&text)?),
            None => Ok(serde_json::from_slice(&self.body)?),
        }
    }

    /// `has_redirect_location`: a redirect status with a `Location` header.
    pub fn is_redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308) && self.header("location").is_some()
    }
}

/// `json.detect_encoding` plus a strict decode; `None` when the bytes are not
/// valid in the detected encoding (the caller reports the parse error).
fn decode_json_bytes(b: &[u8]) -> Option<String> {
    #[derive(Clone, Copy)]
    enum Enc {
        Utf8,
        Utf16(bool),
        Utf32(bool),
    }
    let (enc, skip) = if b.starts_with(&[0xff, 0xfe, 0, 0]) {
        (Enc::Utf32(false), 4)
    } else if b.starts_with(&[0, 0, 0xfe, 0xff]) {
        (Enc::Utf32(true), 4)
    } else if b.starts_with(&[0xff, 0xfe]) {
        (Enc::Utf16(false), 2)
    } else if b.starts_with(&[0xfe, 0xff]) {
        (Enc::Utf16(true), 2)
    } else if b.starts_with(&[0xef, 0xbb, 0xbf]) {
        (Enc::Utf8, 3)
    } else if b.len() >= 4 && b[0] == 0 {
        (if b[1] != 0 { Enc::Utf16(true) } else { Enc::Utf32(true) }, 0)
    } else if b.len() >= 4 && b[1] == 0 {
        (if b[2] != 0 || b[3] != 0 { Enc::Utf16(false) } else { Enc::Utf32(false) }, 0)
    } else if b.len() == 2 && b[0] == 0 {
        (Enc::Utf16(true), 0)
    } else if b.len() == 2 && b[1] == 0 {
        (Enc::Utf16(false), 0)
    } else {
        (Enc::Utf8, 0)
    };
    let body = &b[skip..];
    match enc {
        Enc::Utf8 => std::str::from_utf8(body).ok().map(str::to_string),
        Enc::Utf16(be) => {
            if body.len() % 2 != 0 {
                return None;
            }
            let units = body.chunks_exact(2).map(|c| {
                if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            });
            char::decode_utf16(units).collect::<std::result::Result<String, _>>().ok()
        }
        Enc::Utf32(be) => {
            if body.len() % 4 != 0 {
                return None;
            }
            body.chunks_exact(4)
                .map(|c| {
                    let n = if be {
                        u32::from_be_bytes([c[0], c[1], c[2], c[3]])
                    } else {
                        u32::from_le_bytes([c[0], c[1], c[2], c[3]])
                    };
                    char::from_u32(n)
                })
                .collect()
        }
    }
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// `httpx.TransportError`; `kind` is the httpx class name (`ConnectError`, `ReadTimeout`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportError {
    pub kind: String,
    pub message: String,
}

impl TransportError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        TransportError {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

impl From<TransportError> for Error {
    fn from(e: TransportError) -> Self {
        Error::Http {
            kind: e.kind,
            message: e.message,
        }
    }
}

/// Sends one request and returns the response without following redirects.
pub trait Transport: Send + Sync {
    fn send(&self, req: &Request) -> std::result::Result<Response, TransportError>;
}

/// Blocking reqwest + rustls.
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self> {
        let mut builder = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(None)
            .connect_timeout(TIMEOUT)
            .pool_idle_timeout(KEEPALIVE_EXPIRY);
        if let Some(roots) = env_roots(
            std::env::var("SSL_CERT_FILE").ok().as_deref(),
            std::env::var("SSL_CERT_DIR").ok().as_deref(),
        )? {
            builder = builder.tls_built_in_root_certs(false);
            for cert in roots {
                builder = builder.add_root_certificate(cert);
            }
        }
        let client = builder
            .build()
            .map_err(|e| Error::other(e.to_string()))?;
        Ok(ReqwestTransport { client })
    }
}

/// httpx `trust_env`: `SSL_CERT_FILE`, else `SSL_CERT_DIR`, replaces the
/// built-in roots. Empty values count as unset.
fn env_roots(file: Option<&str>, dir: Option<&str>) -> Result<Option<Vec<reqwest::Certificate>>> {
    let parse = |path: &std::path::Path| -> Result<Vec<reqwest::Certificate>> {
        let pem = std::fs::read(path).map_err(|e| {
            let text = e.to_string();
            let text = text.split(" (os error").next().unwrap_or_default();
            Error::other(format!("[Errno {}] {text}", e.raw_os_error().unwrap_or(0)))
        })?;
        reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| Error::other(e.to_string()))
    };
    if let Some(file) = file.filter(|f| !f.is_empty()) {
        return parse(std::path::Path::new(file)).map(Some);
    }
    if let Some(dir) = dir.filter(|d| !d.is_empty()) {
        let mut certs = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| Error::other(format!("{e}: {dir}")))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file())
            .collect();
        entries.sort();
        for path in entries {
            if let Ok(found) = parse(&path) {
                certs.extend(found);
            }
        }
        return Ok(Some(certs));
    }
    Ok(None)
}

impl Transport for ReqwestTransport {
    fn send(&self, req: &Request) -> std::result::Result<Response, TransportError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| TransportError::new("UnsupportedProtocol", e.to_string()))?;
        if let Some((scheme, _)) = req.url.split_once("://")
            && !scheme.eq_ignore_ascii_case("http")
            && !scheme.eq_ignore_ascii_case("https")
        {
            return Err(TransportError::new(
                "UnsupportedProtocol",
                format!(
                    "Request URL has an unsupported protocol '{}://'.",
                    scheme.to_ascii_lowercase()
                ),
            ));
        }
        let mut rb = self
            .client
            .request(method, &req.url)
            .header("Connection", "keep-alive")
            .header("Accept-Encoding", "gzip, deflate");
        for (k, v) in &req.headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        let body_len = req.body.as_ref().map_or(0, Vec::len);
        if let Some(body) = &req.body {
            rb = rb.body(body.clone());
        }
        // The blocking response applies this timeout to each body read.
        if body_len <= DEADLINE_BODY_LIMIT {
            rb = rb.timeout(TIMEOUT);
        }
        let mut resp = rb.send().map_err(map_reqwest)?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut body = Vec::new();
        let declared = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let chunked = resp
            .headers()
            .get("transfer-encoding")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
        let read = |resp: &mut reqwest::blocking::Response, body: &mut Vec<u8>, limit: Option<u64>| {
            let mut buf = [0u8; 16384];
            loop {
                let want = limit.map_or(buf.len(), |l| {
                    usize::try_from(l.saturating_sub(body.len() as u64)).map_or(buf.len(), |n| n.min(buf.len()))
                });
                if want == 0 {
                    return Ok(());
                }
                match resp.read(&mut buf[..want]) {
                    Ok(0) => return Ok(()),
                    Ok(n) => body.extend_from_slice(&buf[..n]),
                    Err(e) => return Err(e),
                }
            }
        };
        let outcome = match req.body_limit.filter(|_| status < 400) {
            Some(limit) if declared.is_some_and(|n| n > limit) => Ok(()),
            Some(limit) => read(&mut resp, &mut body, Some(limit.saturating_add(1))),
            None => read(&mut resp, &mut body, None),
        };
        if let Err(e) = outcome {
            return Err(map_read(&e, chunked, declared, body.len()));
        }
        Ok(Response {
            status,
            url: req.url.clone(),
            headers,
            body,
        })
    }
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut parts = vec![e.to_string()];
    let mut src = e.source();
    while let Some(s) = src {
        let text = s.to_string();
        if !parts.contains(&text) {
            parts.push(text);
        }
        src = s.source();
    }
    parts.join(": ")
}

fn map_reqwest(e: reqwest::Error) -> TransportError {
    let (connect, timeout, body) = (e.is_connect(), e.is_timeout(), e.is_body() || e.is_decode());
    let text = error_chain(&e.without_url());
    let lower = text.to_ascii_lowercase();
    if lower.contains("gzip") || lower.contains("deflate") {
        return TransportError::new("DecodingError", text);
    }
    if connect {
        if timeout {
            return TransportError::new("ConnectTimeout", text);
        }
        if lower.contains("tunnel error: unexpected end of file") {
            return TransportError::new(
                "RemoteProtocolError",
                "Server disconnected without sending a response.",
            );
        }
        if lower.contains("tunnel error") || lower.contains("proxy") {
            return TransportError::new("ProxyError", text);
        }
        return TransportError::new("ConnectError", text);
    }
    if timeout {
        return TransportError::new("ReadTimeout", text);
    }
    if lower.contains("connection closed before message completed")
        || lower.contains("close_notify")
    {
        return TransportError::new(
            "RemoteProtocolError",
            "Server disconnected without sending a response.",
        );
    }
    if lower.contains("invalid http") || lower.contains("invalid status") || lower.contains("parsed") {
        return TransportError::new("RemoteProtocolError", text);
    }
    if lower.contains("connection reset") || lower.contains("broken pipe") {
        return TransportError::new("ReadError", text);
    }
    TransportError::new(
        if body {
            "ReadError"
        } else {
            "NetworkError"
        },
        text,
    )
}

fn map_read(
    e: &std::io::Error,
    chunked: bool,
    declared: Option<u64>,
    got: usize,
) -> TransportError {
    let timed_out = e.kind() == std::io::ErrorKind::TimedOut
        || e.get_ref()
            .and_then(|i| i.downcast_ref::<reqwest::Error>())
            .is_some_and(reqwest::Error::is_timeout);
    let text = error_chain(e);
    if timed_out {
        return TransportError::new("ReadTimeout", text);
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("gzip") || lower.contains("deflate") {
        return TransportError::new("DecodingError", text);
    }
    if lower.contains("end of file before message length reached") {
        let message = match declared {
            Some(n) => format!(
                "peer closed connection without sending complete message body (received {got} bytes, expected {n})"
            ),
            None => "peer closed connection without sending complete message body".to_string(),
        };
        return TransportError::new("RemoteProtocolError", message);
    }
    if lower.contains("chunk") || (chunked && lower.contains("end of file")) {
        return TransportError::new(
            "RemoteProtocolError",
            "peer closed connection without sending complete message body (incomplete chunked read)",
        );
    }
    TransportError::new("ReadError", text)
}

#[cfg(test)]
mod body_limit_tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    fn serve(status_line: &str, headers: &str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let head = format!("HTTP/1.1 {status_line}\r\n{headers}Connection: close\r\n\r\n");
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(head.as_bytes());
                let _ = sock.write_all(&body);
            }
        });
        url
    }

    fn get(url: &str, limit: Option<u64>) -> Response {
        let req = Request {
            method: "GET".into(),
            url: url.into(),
            headers: vec![],
            body: None,
            body_limit: limit,
        };
        ReqwestTransport::new().unwrap().send(&req).unwrap()
    }

    #[test]
    fn a_declared_length_over_the_limit_reads_no_body() {
        let url = serve("200 OK", "Content-Length: 5000\r\n", vec![b'x'; 5000]);
        assert!(get(&url, Some(100)).body.is_empty());
    }

    #[test]
    fn an_unknown_length_stops_one_byte_past_the_limit() {
        let url = serve("200 OK", "", vec![b'x'; 5000]);
        assert_eq!(get(&url, Some(100)).body.len(), 101);
    }

    #[test]
    fn a_body_within_the_limit_is_whole_and_errors_are_never_cut() {
        let url = serve("200 OK", "Content-Length: 50\r\n", vec![b'x'; 50]);
        assert_eq!(get(&url, Some(100)).body.len(), 50);
        let url = serve("500 Internal Server Error", "", vec![b'e'; 400]);
        assert_eq!(get(&url, Some(10)).body.len(), 400);
        let url = serve("200 OK", "", vec![b'x'; 5000]);
        assert_eq!(get(&url, None).body.len(), 5000);
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    fn json(bytes: &[u8]) -> Result<Value> {
        Response::new(200).with_body(bytes.to_vec()).json_value()
    }

    #[test]
    fn json_detects_bom_and_wide_encodings_like_json_loads() {
        let want = serde_json::json!({"a": "é"});
        let text = r#"{"a": "é"}"#;
        assert_eq!(json(text.as_bytes()).unwrap(), want);
        assert_eq!(json(&[b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat()).unwrap(), want);
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(json(&[b"\xff\xfe".as_slice(), &utf16].concat()).unwrap(), want);
        assert_eq!(json(&utf16).unwrap(), want);
        let utf16be: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(json(&utf16be).unwrap(), want);
        let utf32: Vec<u8> = text.chars().flat_map(|c| (c as u32).to_le_bytes()).collect();
        assert_eq!(json(&utf32).unwrap(), want);
        assert!(json(b"{\"a\": \"\xff\"}").is_err());
    }

    #[test]
    fn text_follows_the_content_type_charset() {
        let r = |ct: &str, body: &[u8]| {
            Response::new(200)
                .with_header("content-type", ct)
                .with_body(body.to_vec())
                .text()
        };
        assert_eq!(r("text/plain; charset=latin-1", b"caf\xe9"), "café");
        assert_eq!(r("text/plain; charset=\"ISO-8859-1\"", b"caf\xe9"), "café");
        assert_eq!(r("text/plain; charset=ascii", b"a\xe9"), "a\u{fffd}");
        assert_eq!(r("text/plain", b"caf\xc3\xa9"), "café");
        assert_eq!(r("text/plain; charset=nonsense", b"a\xffb"), "a\u{fffd}b");
    }

    #[test]
    fn ssl_cert_env_empty_values_count_as_unset() {
        assert!(env_roots(None, None).unwrap().is_none());
        assert!(env_roots(Some(""), Some("")).unwrap().is_none());
        let err = env_roots(Some("/nonexistent-ca.pem"), None).unwrap_err().to_string();
        assert_eq!(err, "[Errno 2] No such file or directory");
    }

    #[test]
    fn scheme_other_than_http_is_unsupported_protocol() {
        let req = Request {
            method: "GET".into(),
            url: "ftp://acme.freshservice.com/x".into(),
            ..Default::default()
        };
        let e = ReqwestTransport::new().unwrap().send(&req).unwrap_err();
        assert_eq!(e.kind, "UnsupportedProtocol");
        assert_eq!(e.message, "Request URL has an unsupported protocol 'ftp://'.");
    }
}
