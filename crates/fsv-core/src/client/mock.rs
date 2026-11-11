//! Recording [`Transport`] for tests in this and downstream crates.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::transport::{Request, Response, Transport, TransportError};

type Handler = dyn Fn(&Request) -> Result<Response, TransportError> + Send + Sync;

/// Answers requests from a handler and records every request it sees.
pub struct MockTransport {
    handler: Box<Handler>,
    requests: Mutex<Vec<Request>>,
}

impl MockTransport {
    /// Answer each request with `handler`.
    pub fn new(
        handler: impl Fn(&Request) -> Result<Response, TransportError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(MockTransport {
            handler: Box::new(handler),
            requests: Mutex::new(Vec::new()),
        })
    }

    /// Answer requests in order from `responses`; a `ConnectError` once exhausted.
    pub fn queue(responses: Vec<Result<Response, TransportError>>) -> Arc<Self> {
        let queue = Mutex::new(VecDeque::from(responses));
        Self::new(move |_| {
            let mut q = queue.lock().unwrap_or_else(|p| p.into_inner());
            q.pop_front().unwrap_or_else(|| {
                Err(TransportError::new(
                    "ConnectError",
                    "mock: no response queued",
                ))
            })
        })
    }

    /// Every request, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// URLs of GET requests.
    pub fn gets(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == "GET")
            .map(|r| r.url)
            .collect()
    }

    /// `(method, url, JSON body)` of non-GET/HEAD requests.
    pub fn writes(&self) -> Vec<(String, String, Option<Value>)> {
        self.requests()
            .into_iter()
            .filter(|r| r.method != "GET" && r.method != "HEAD")
            .map(|r| {
                let body = r.json();
                (r.method, r.url, body)
            })
            .collect()
    }

    /// `host + path` of every request (no query), in order.
    pub fn paths(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|r| match url::Url::parse(&r.url) {
                Ok(u) => format!("{}{}", u.host_str().unwrap_or(""), u.path()),
                Err(_) => r.url.clone(),
            })
            .collect()
    }
}

impl Transport for MockTransport {
    fn send(&self, req: &Request) -> Result<Response, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(req.clone());
        (self.handler)(req)
    }
}
