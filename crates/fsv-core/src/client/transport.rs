//! One HTTP exchange. No redirects, cookies or retries: [`super::Client`]
//! does those so every transport (reqwest, test mocks) behaves the same.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

/// httpx `timeout=30`, applied per phase (connect, read), never as a total deadline.
pub const TIMEOUT: Duration = Duration::from_secs(30);

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

    /// Body decoded as text (invalid UTF-8 replaced).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Body parsed as JSON regardless of content type (`r.json()`).
    pub fn json_value(&self) -> Result<Value> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// `has_redirect_location`: a redirect status with a `Location` header.
    pub fn is_redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308) && self.header("location").is_some()
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
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(None)
            .connect_timeout(TIMEOUT)
            .build()
            .map_err(|e| Error::other(e.to_string()))?;
        Ok(ReqwestTransport { client })
    }
}

impl Transport for ReqwestTransport {
    fn send(&self, req: &Request) -> std::result::Result<Response, TransportError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| TransportError::new("UnsupportedProtocol", e.to_string()))?;
        let mut rb = self.client.request(method, &req.url);
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
        match req.body_limit.filter(|_| status < 400) {
            Some(limit) if declared.is_some_and(|n| n > limit) => {}
            Some(limit) => {
                let chunked = resp
                    .headers()
                    .get("transfer-encoding")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
                read_chunks_to_limit(&mut resp, limit, chunked, &mut body).map_err(map_read)?;
            }
            None => {
                resp.read_to_end(&mut body).map_err(map_read)?;
            }
        }
        Ok(Response {
            status,
            url: req.url.clone(),
            headers,
            body,
        })
    }
}

/// Python reads `Response.iter_bytes()` chunk by chunk (httpcore reads the
/// socket 64 KiB at a time; a chunked body arrives one HTTP chunk at a time)
/// and stops after the chunk that takes the total past `limit`, so the size it
/// reports is the bytes read up to the end of that chunk. A close-delimited
/// body is read here in the same 64 KiB steps; a chunked one a read at a time.
fn read_chunks_to_limit(
    resp: &mut impl Read,
    limit: u64,
    chunked: bool,
    body: &mut Vec<u8>,
) -> std::io::Result<()> {
    const CHUNK: usize = 64 * 1024;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let mut got = 0;
        while got < CHUNK {
            let n = resp.read(&mut buf[got..])?;
            if n == 0 {
                break;
            }
            got += n;
            if chunked {
                break;
            }
        }
        if got == 0 {
            return Ok(());
        }
        body.extend_from_slice(&buf[..got]);
        if body.len() as u64 > limit {
            return Ok(());
        }
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
    let kind = if e.is_connect() {
        if e.is_timeout() {
            "ConnectTimeout"
        } else {
            "ConnectError"
        }
    } else if e.is_timeout() {
        "ReadTimeout"
    } else if e.is_body() || e.is_decode() {
        "ReadError"
    } else {
        "NetworkError"
    };
    TransportError::new(kind, error_chain(&e.without_url()))
}

fn map_read(e: std::io::Error) -> TransportError {
    let timed_out = e.kind() == std::io::ErrorKind::TimedOut
        || e.get_ref()
            .and_then(|i| i.downcast_ref::<reqwest::Error>())
            .is_some_and(reqwest::Error::is_timeout);
    TransportError::new(
        if timed_out {
            "ReadTimeout"
        } else {
            "ReadError"
        },
        error_chain(&e),
    )
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
    fn an_unknown_length_reports_the_chunk_that_crossed_the_limit() {
        let url = serve("200 OK", "", vec![b'x'; 5000]);
        assert_eq!(get(&url, Some(100)).body.len(), 5000);
        let url = serve("200 OK", "", vec![b'x'; 200_000]);
        assert_eq!(get(&url, Some(100)).body.len(), 65_536);
        let url = serve("200 OK", "", vec![b'x'; 200_000]);
        assert_eq!(get(&url, Some(70_000)).body.len(), 131_072);
        let url = serve("200 OK", "", vec![b'x'; 5000]);
        assert_eq!(get(&url, Some(5000)).body.len(), 5000);
    }

    #[test]
    fn a_chunked_body_stops_after_the_http_chunk_that_crossed_the_limit() {
        let mut body = Vec::new();
        for _ in 0..4 {
            body.extend_from_slice(b"2000\r\n");
            body.extend_from_slice(&[b'x'; 8192]);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"0\r\n\r\n");
        let url = serve("200 OK", "Transfer-Encoding: chunked\r\n", body);
        let n = get(&url, Some(100)).body.len();
        assert!((101..=8192).contains(&n), "{n}");
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
