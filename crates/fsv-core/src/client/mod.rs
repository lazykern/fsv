//! Freshservice HTTP client (`client.py`).
//!
//! `Client` is `Send + Sync` and shared as `Arc<Client>`; the cookie jar,
//! CSRF token and rate-limit counters sit behind one `Mutex` that is never
//! held across network I/O.

mod cookies;
mod log;
pub mod mock;
mod transport;

use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};
use regex::Regex;
use serde_json::{Map, Value, json};
use url::Url;

pub use cookies::{Cookie, CookieJar};
pub use transport::{Request, ReqwestTransport, Response, TIMEOUT, Transport, TransportError};

use crate::ctx::{Ctx, Tenant};
use crate::error::{ApiError, Error, Result};
use crate::journal::{self, WriteOutcome};
use crate::session::{self, Cookies};
use crate::{cache, pystr, term};

pub const UA: &str = "fsv/0.1 (Freshservice CLI)";
pub const MAX_REDIRECTS: usize = 20;
pub const EXPIRED_MSG: &str = "session expired or invalid; run `fsv auth login`";
const RETRYABLE_STATUS: [u16; 5] = [429, 500, 502, 503, 504];
const IDEMPOTENT_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];
const MAX_RETRIES: usize = 2;
const RETRY_DELAYS: [f64; 2] = [0.5, 1.5];
const PROBE_PATH: &str = "/a/dashboard";
const PROBE_TTL: Duration = Duration::from_secs(60);
const HTML_ACCEPT: &str = "text/html,application/xhtml+xml";

/// Request payload: httpx `json=`, `data=` and `content=`.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Body {
    #[default]
    None,
    Json(Value),
    /// Form-encoded bytes (`data=`).
    Data(Vec<u8>),
    /// Exact wire bytes (`content=`); not shown in the verbose log.
    Raw(Vec<u8>),
}

/// Per-request options (httpx keyword arguments).
#[derive(Clone, Debug, Default)]
pub struct RequestOpts {
    pub params: Option<Map<String, Value>>,
    pub body: Body,
    pub headers: Vec<(String, String)>,
    /// Follow redirects (`follow_redirects=`); off by default like the Python client.
    pub follow: bool,
}

impl RequestOpts {
    pub fn params(mut self, params: Map<String, Value>) -> Self {
        self.params = Some(params);
        self
    }

    pub fn body(mut self, body: Body) -> Self {
        self.body = body;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn follow(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }

    fn html() -> Self {
        RequestOpts::default().header("Accept", HTML_ACCEPT)
    }
}

/// Sleeps between retries.
pub type Sleeper = dyn Fn(Duration) + Send + Sync;
/// Records a write request (`journal.record`).
pub type JournalHook = dyn Fn(&str, &str, Option<&Value>, WriteOutcome<'_>) + Send + Sync;

#[derive(Default)]
struct State {
    jar: CookieJar,
    csrf: Option<String>,
    fw_domain: Option<String>,
    fw_session_id: Option<String>,
    rl_rem: Option<i64>,
    rl_tot: Option<i64>,
    probe_ok_until: Option<Instant>,
}

enum SendErr {
    /// `httpx.TransportError`: retryable for idempotent methods.
    Transport(TransportError),
    /// Anything else (too many redirects, bad URL): never retried.
    Fatal(Error),
}

pub struct Client {
    ctx: Arc<Ctx>,
    tenant: Tenant,
    transport: Arc<dyn Transport>,
    state: Mutex<State>,
    persist: bool,
    sleep: Arc<Sleeper>,
    journal: Arc<JournalHook>,
}

/// Builds a [`Client`]; the defaults are the real transport, stored session
/// cookies, `thread::sleep` and the on-disk journal.
pub struct ClientBuilder {
    ctx: Arc<Ctx>,
    transport: Option<Arc<dyn Transport>>,
    cookies: Option<Cookies>,
    sleep: Option<Arc<Sleeper>>,
    journal: Option<Arc<JournalHook>>,
}

impl ClientBuilder {
    pub fn transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Use these cookies instead of the stored session; rotated cookies are then not persisted.
    pub fn cookies(mut self, cookies: Cookies) -> Self {
        self.cookies = Some(cookies);
        self
    }

    pub fn sleeper(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Some(Arc::new(sleep));
        self
    }

    pub fn journal(
        mut self,
        hook: impl Fn(&str, &str, Option<&Value>, WriteOutcome<'_>) + Send + Sync + 'static,
    ) -> Self {
        self.journal = Some(Arc::new(hook));
        self
    }

    pub fn build(self) -> Result<Client> {
        let tenant = self.ctx.require_tenant()?.clone();
        let persist = self.cookies.is_none();
        let cookies = match self.cookies {
            Some(c) => c,
            None => session::load_cookies()?,
        };
        let transport: Arc<dyn Transport> = match self.transport {
            Some(t) => t,
            None => Arc::new(ReqwestTransport::new()?),
        };
        let mut jar = CookieJar::new(&tenant.domain);
        for (k, v) in &cookies {
            jar.set_session(k, v);
        }
        let state = State {
            jar,
            fw_session_id: cookies.get("fw-session-id").cloned(),
            ..Default::default()
        };
        Ok(Client {
            ctx: self.ctx,
            tenant,
            transport,
            state: Mutex::new(state),
            persist,
            sleep: self.sleep.unwrap_or_else(|| Arc::new(std::thread::sleep)),
            journal: self.journal.unwrap_or_else(|| Arc::new(journal::record)),
        })
    }
}

impl Client {
    /// Client for `ctx.tenant` using the stored session.
    pub fn new(ctx: Arc<Ctx>) -> Result<Client> {
        Client::builder(ctx).build()
    }

    /// Client using `cookies` (validate fresh cookies before they are saved).
    pub fn with_cookies(ctx: Arc<Ctx>, cookies: Cookies) -> Result<Client> {
        Client::builder(ctx).cookies(cookies).build()
    }

    pub fn builder(ctx: Arc<Ctx>) -> ClientBuilder {
        ClientBuilder {
            ctx,
            transport: None,
            cookies: None,
            sleep: None,
            journal: None,
        }
    }

    pub fn ctx(&self) -> &Arc<Ctx> {
        &self.ctx
    }

    pub fn tenant(&self) -> &Tenant {
        &self.tenant
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Current value of a cookie in the jar.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.state().jar.get(name).map(str::to_string)
    }

    /// Freshworks identity host seen on the app page (`*.myfreshworks.com`).
    pub fn fw_domain(&self) -> Option<String> {
        self.state().fw_domain.clone()
    }

    fn tenant_url(&self, path: &str) -> String {
        format!("https://{}{path}", self.tenant.domain)
    }

    // ---------- transport ----------

    fn log_line(&self, method: &str, url: &str, opts: &RequestOpts, outcome: &str) {
        if self.ctx.flags.verbose {
            term::err().print_plain(&log::log_line(
                method,
                url,
                opts.params.as_ref(),
                &opts.body,
                outcome,
            ));
        }
    }

    /// One httpx request: redirect loop with cookie extraction on every hop.
    fn exchange(
        &self,
        method: &str,
        url: &str,
        opts: &RequestOpts,
    ) -> std::result::Result<Response, SendErr> {
        let invalid = |e: url::ParseError| {
            SendErr::Fatal(Error::Http {
                kind: "InvalidURL".into(),
                message: e.to_string(),
            })
        };
        let mut method = method.to_uppercase();
        let mut current = Url::parse(&wire_url(url, opts.params.as_ref())).map_err(invalid)?;
        let (mut body, default_ct) = encode_body(&opts.body);
        let mut hops = 0;
        loop {
            if hops > MAX_REDIRECTS {
                return Err(SendErr::Fatal(Error::Http {
                    kind: "TooManyRedirects".into(),
                    message: "Exceeded maximum allowed redirects.".into(),
                }));
            }
            let mut headers = vec![
                ("User-Agent".to_string(), UA.to_string()),
                ("Accept".to_string(), "application/json".to_string()),
            ];
            if let (Some(ct), Some(_)) = (default_ct, &body) {
                set_header(&mut headers, "Content-Type", ct);
            }
            for (k, v) in &opts.headers {
                set_header(&mut headers, k, v);
            }
            if let Some(cookie) = self.state().jar.header_for(&current) {
                set_header(&mut headers, "Cookie", &cookie);
            }
            let req = Request {
                method: method.clone(),
                url: current.to_string(),
                headers,
                body: body.clone(),
            };
            let mut resp = self.transport.send(&req).map_err(SendErr::Transport)?;
            resp.url = req.url;
            self.state()
                .jar
                .extract(&current, resp.header_all("set-cookie"));
            if !(opts.follow && resp.is_redirect()) {
                return Ok(resp);
            }
            let location = resp.header("location").unwrap_or_default();
            current = current.join(location).map_err(invalid)?;
            if (resp.status == 303 && method != "HEAD")
                || (matches!(resp.status, 301 | 302) && method == "POST")
            {
                method = "GET".into();
                body = None;
            }
            hops += 1;
        }
    }

    /// `_send`: retries idempotent requests on transport errors and 429/5xx,
    /// records rate-limit headers and logs. No status mapping.
    pub fn send(&self, method: &str, url: &str, opts: &RequestOpts) -> Result<Response> {
        for attempt in 0..=MAX_RETRIES {
            let t0 = Instant::now();
            let r = match self.exchange(method, url, opts) {
                Ok(r) => r,
                Err(SendErr::Transport(e)) => {
                    let outcome = format!("{}: {}", e.kind, e.message);
                    if attempt < MAX_RETRIES && is_idempotent(method) {
                        let delay = retry_delay(attempt, None);
                        self.log_line(
                            method,
                            url,
                            opts,
                            &format!("{outcome}; retry in {}s", pystr::format_g(delay)),
                        );
                        (self.sleep)(Duration::from_secs_f64(delay));
                        continue;
                    }
                    self.log_line(method, url, opts, &outcome);
                    return Err(e.into());
                }
                Err(SendErr::Fatal(e)) => {
                    let outcome = match &e {
                        Error::Http { kind, message } => format!("{kind}: {message}"),
                        other => other.to_string(),
                    };
                    self.log_line(method, url, opts, &outcome);
                    return Err(e);
                }
            };
            let rem = r.header("x-ratelimit-remaining").unwrap_or("").to_string();
            let tot = r.header("x-ratelimit-total").unwrap_or("").to_string();
            self.record_rate_limit(&rem, &tot);
            let rl = if !rem.is_empty() && !tot.is_empty() {
                format!(" rl={rem}/{tot}")
            } else {
                String::new()
            };
            let elapsed = format!("{:.0}ms", t0.elapsed().as_secs_f64() * 1000.0);
            if attempt < MAX_RETRIES
                && is_idempotent(method)
                && RETRYABLE_STATUS.contains(&r.status)
            {
                let delay = retry_delay(attempt, Some(&r));
                self.log_line(
                    method,
                    url,
                    opts,
                    &format!(
                        "{} {elapsed}{rl}; retry in {}s",
                        r.status,
                        pystr::format_g(delay)
                    ),
                );
                (self.sleep)(Duration::from_secs_f64(delay));
                continue;
            }
            self.log_line(method, url, opts, &format!("{} {elapsed}{rl}", r.status));
            return Ok(r);
        }
        Err(Error::other("request failed without response"))
    }

    /// A Python `int()` failure on either header skips the rest, as in `_send`.
    fn record_rate_limit(&self, rem: &str, tot: &str) {
        let mut st = self.state();
        if !rem.is_empty() {
            match pystr::strip(rem).parse() {
                Ok(n) => st.rl_rem = Some(n),
                Err(_) => return,
            }
        }
        if !tot.is_empty() {
            if let Ok(n) = pystr::strip(tot).parse() {
                st.rl_tot = Some(n);
            }
        }
    }

    /// GET for downloads: no retry, no journal, no status mapping, no log.
    pub fn fetch_raw(&self, url: &str, follow: bool) -> Result<Response> {
        match self.exchange("GET", url, &RequestOpts::default().follow(follow)) {
            Ok(r) => Ok(r),
            Err(SendErr::Transport(e)) => Err(e.into()),
            Err(SendErr::Fatal(e)) => Err(e),
        }
    }

    // ---------- status mapping ----------

    /// `_probe_session`: is the session still valid after a 403?
    /// `None` when the probe itself fails, so callers keep the original 403.
    fn probe_session(&self) -> Option<bool> {
        if self
            .state()
            .probe_ok_until
            .is_some_and(|t| Instant::now() < t)
        {
            return Some(true);
        }
        let r = self
            .send("GET", &self.tenant_url(PROBE_PATH), &RequestOpts::html())
            .ok()?;
        if (300..400).contains(&r.status) || matches!(r.status, 401 | 403) {
            return Some(false);
        }
        if r.status != 200 {
            return None;
        }
        if looks_like_login_html(&r.text()) {
            return Some(false);
        }
        self.state().probe_ok_until = Some(Instant::now() + PROBE_TTL);
        Some(true)
    }

    /// `_check_response`: 3xx (not 304), 401, HTML and a dead-session 403
    /// are session errors; other >= 400 are API errors.
    fn check_response(
        &self,
        r: &Response,
        method: &str,
        url: &str,
        params: Option<&Map<String, Value>>,
    ) -> Result<()> {
        if (300..400).contains(&r.status) && r.status != 304 {
            return Err(Error::session(EXPIRED_MSG));
        }
        if r.status == 401 {
            return Err(Error::session(EXPIRED_MSG));
        }
        if r.status < 400 && r.content_type().contains("text/html") {
            return Err(Error::session(EXPIRED_MSG));
        }
        if r.status == 403 && self.probe_session() == Some(false) {
            return Err(Error::session(EXPIRED_MSG));
        }
        if r.status >= 400 {
            let target = log::request_target(url, params);
            return Err(ApiError::with_request(r.status, response_body(r), method, target).into());
        }
        Ok(())
    }

    /// `_request`: send, map status, journal writes, persist a rotated
    /// `fw-session-id`. Returns JSON for `application/json`, else the text.
    pub fn request(&self, method: &str, url: &str, opts: RequestOpts) -> Result<Value> {
        let upper = method.to_uppercase();
        let (status, result) = self.request_inner(method, url, &opts);
        if upper == "GET" || upper == "HEAD" {
            return result;
        }
        let json = match &opts.body {
            Body::Json(v) => Some(v),
            _ => None,
        };
        let outcome = match &result {
            Ok(v) => WriteOutcome {
                status,
                ok: true,
                response: Some(v),
                error: false,
            },
            Err(e) => WriteOutcome {
                status: e.status().filter(|s| *s != 0).or(status),
                ok: false,
                response: None,
                error: true,
            },
        };
        (self.journal)(method, url, json, outcome);
        result
    }

    fn request_inner(
        &self,
        method: &str,
        url: &str,
        opts: &RequestOpts,
    ) -> (Option<u16>, Result<Value>) {
        let r = match self.send(method, url, opts) {
            Ok(r) => r,
            Err(e) => return (None, Err(e)),
        };
        let status = Some(r.status);
        if let Err(e) = self.check_response(&r, method, url, opts.params.as_ref()) {
            return (status, Err(e));
        }
        self.track_fw_session();
        let result = if r.content_type().starts_with("application/json") {
            r.json_value()
        } else {
            Ok(Value::String(r.text()))
        };
        (status, result)
    }

    fn track_fw_session(&self) {
        let rotated = {
            let mut st = self.state();
            match st.jar.get("fw-session-id").map(str::to_string) {
                Some(fw) if st.fw_session_id.as_deref() != Some(fw.as_str()) => {
                    st.fw_session_id = Some(fw.clone());
                    Some(fw)
                }
                _ => None,
            }
        };
        if let (Some(fw), true) = (rotated, self.persist) {
            session::update_cookie("fw-session-id", &fw);
        }
    }

    // ---------- endpoints ----------

    fn v2_url(&self, path: &str) -> String {
        format!("{}/{}", self.tenant.api_v2, path.trim_start_matches('/'))
    }

    fn int_url(&self, path: &str) -> String {
        format!("{}/{}", self.tenant.api_int, path.trim_start_matches('/'))
    }

    fn json_opts(body: &Value) -> RequestOpts {
        RequestOpts::default()
            .body(Body::Json(body.clone()))
            .header("Content-Type", "application/json")
    }

    pub fn v2_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        self.request(
            "GET",
            &self.v2_url(path),
            RequestOpts {
                params,
                ..Default::default()
            },
        )
    }

    pub fn v2_post(&self, path: &str, body: &Value) -> Result<Value> {
        self.request("POST", &self.v2_url(path), Self::json_opts(body))
    }

    pub fn v2_put(&self, path: &str, body: &Value) -> Result<Value> {
        self.request("PUT", &self.v2_url(path), Self::json_opts(body))
    }

    pub fn int_get(&self, path: &str, params: Option<Map<String, Value>>) -> Result<Value> {
        self.request(
            "GET",
            &self.int_url(path),
            RequestOpts {
                params,
                ..Default::default()
            },
        )
    }

    pub fn fulltext_search(
        &self,
        entity: &str,
        term: &str,
        page: i64,
        sort: Option<&str>,
    ) -> Result<Value> {
        let mut params = Map::new();
        params.insert("term".into(), json!(term));
        params.insert("page".into(), json!(page));
        let search_sort = match sort {
            Some("created") => Some("created_at"),
            Some("modified") => Some("updated_at"),
            Some("relevance") => Some("relevance"),
            _ => None,
        };
        if let Some(s) = search_sort {
            params.insert("search_sort".into(), json!(s));
        }
        self.request(
            "GET",
            &self.tenant_url(&format!("/search/{entity}")),
            RequestOpts::default().params(params),
        )
    }

    pub fn autocomplete(
        &self,
        kind: &str,
        query: &str,
        params: Option<Map<String, Value>>,
    ) -> Result<Vec<Value>> {
        let mut p = Map::new();
        p.insert("q".into(), json!(query));
        p.insert("qf".into(), json!("value"));
        p.extend(params.unwrap_or_default());
        let r = self.request(
            "GET",
            &self.tenant_url(&format!("/search/autocomplete/{kind}")),
            RequestOpts::default().params(p),
        )?;
        results_of(&r)
    }

    /// Choices for a lookup field, cached under `<cache>/lookup`. The
    /// server-supplied `link` is forced relative to the tenant.
    pub fn lookup_choices(&self, link: &str, query: &str) -> Result<Vec<Value>> {
        let cache_dir = self.ctx.paths.cache_dir.join("lookup");
        std::fs::create_dir_all(&cache_dir)?;
        let digest = hex::encode(Md5::digest(
            format!("{}|{link}|{query}", self.tenant.domain).as_bytes(),
        ));
        let cache_path = cache_dir.join(format!("{}.json", &digest[..16]));

        if let (Some(doc), false) = cache::load(&cache_path) {
            return Ok(match doc.get("data") {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            });
        }

        let parsed = pystr::urlsplit(link).unwrap_or_default();
        let rel_path = format!(
            "/{}",
            pystr::urlparse_path(&parsed.path).trim_start_matches('/')
        );
        let mut params: Map<String, Value> = pystr::parse_qs_first(&parsed.query)
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        params.insert("q".into(), json!(query));
        let r = self.request(
            "GET",
            &self.tenant_url(&rel_path),
            RequestOpts::default().params(params),
        )?;
        let results = results_of(&r)?;
        cache::save(&cache_path, "lookup", &Value::Array(results.clone()), "", 0)?;
        Ok(results)
    }

    /// Mutation on the internal API with a CSRF token fetched first. A 403
    /// after sending does not prove no side effect, so writes are never replayed.
    fn int_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        params: Option<Map<String, Value>>,
    ) -> Result<Value> {
        let url = self.int_url(path);
        let token = self.get_csrf()?;
        let opts = RequestOpts {
            params,
            body: body.map_or(Body::None, |b| Body::Json(b.clone())),
            ..Default::default()
        }
        .header("Content-Type", "application/json")
        .header("X-CSRF-Token", &token);
        self.request(method, &url, opts)
    }

    pub fn int_post(&self, path: &str, body: &Value) -> Result<Value> {
        self.int_request("POST", path, Some(body), None)
    }

    pub fn int_put(&self, path: &str, body: &Value) -> Result<Value> {
        self.int_request("PUT", path, Some(body), None)
    }

    pub fn int_patch(&self, path: &str, body: &Value) -> Result<Value> {
        self.int_request("PATCH", path, Some(body), None)
    }

    pub fn int_delete(
        &self,
        path: &str,
        body: Option<&Value>,
        params: Option<Map<String, Value>>,
    ) -> Result<Value> {
        self.int_request("DELETE", path, body, params)
    }

    /// `_get_csrf`: the `csrf-token` meta tag from the first app page that has it.
    pub fn get_csrf(&self) -> Result<String> {
        if let Some(t) = self.state().csrf.clone() {
            return Ok(t);
        }
        static FW: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"https://([A-Za-z0-9-]+\.myfreshworks\.com)").expect("regex")
        });
        let mut last_status = 0;
        let mut saw_login = false;
        for path in ["/a/changes", "/a/tickets", "/a/dashboard"] {
            let r = self.send(
                "GET",
                &self.tenant_url(path),
                &RequestOpts::html().follow(true),
            )?;
            last_status = r.status;
            if r.status == 401 || (300..400).contains(&r.status) {
                return Err(Error::session(EXPIRED_MSG));
            }
            if r.status != 200 {
                saw_login = saw_login || r.status == 403;
                continue;
            }
            let host = Url::parse(&r.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string));
            if host.is_some_and(|h| !h.is_empty() && h != self.tenant.domain) {
                return Err(Error::session(EXPIRED_MSG));
            }
            let html = r.text();
            if let Some(token) = csrf_from_html(&html) {
                let mut st = self.state();
                st.csrf = Some(token.clone());
                if st.fw_domain.is_none() {
                    st.fw_domain = FW.captures(&html).map(|c| c[1].to_string());
                }
                return Ok(token);
            }
            if looks_like_login_html(&html) {
                return Err(Error::session(EXPIRED_MSG));
            }
        }
        if saw_login {
            return Err(Error::session(EXPIRED_MSG));
        }
        if last_status >= 500 {
            return Err(ApiError::new(
                last_status,
                "could not load the app page to obtain a CSRF token",
            )
            .into());
        }
        Err(ApiError::new(0, "csrf-token meta tag not found on app pages (Freshservice page layout may have changed)").into())
    }

    /// POST the Freshworks session heartbeat. Best-effort; `false` on any failure.
    pub fn keepalive(&self) -> bool {
        let Some(fw_session_id) = self.state().fw_session_id.clone() else {
            return false;
        };
        if self.fw_domain().is_none() {
            let _ = self.get_csrf();
        }
        let Some(fw_domain) = self.fw_domain() else {
            return false;
        };
        let body = json!({"session_token": fw_session_id, "checkSessionOnlyOnce": false});
        match self.send(
            "POST",
            &format!("https://{fw_domain}/api/v2/session"),
            &Self::json_opts(&body),
        ) {
            Ok(r) => r.status < 400,
            Err(_) => false,
        }
    }

    /// `/agents/me` → the `agent` object.
    pub fn me(&self) -> Result<Value> {
        match self.request("GET", &self.v2_url("agents/me"), RequestOpts::default())? {
            Value::Object(mut m) => Ok(m.remove("agent").unwrap_or_else(|| json!({}))),
            other => Err(Error::other(format!(
                "'{}' object has no attribute 'get'",
                py_type(&other)
            ))),
        }
    }

    /// Validate the session and return the agent identity (`{}` when a
    /// restricted agent gets 403 on `/agents/me`).
    pub fn check_session(&self) -> Result<Value> {
        match self.me() {
            Err(Error::Api(e)) if e.status == 403 => Ok(json!({})),
            other => other,
        }
    }

    /// Pages of a v2 list, following `Link: rel="next"` up to `max_pages`.
    pub fn v2_get_paginated(
        &self,
        path: &str,
        params: Option<Map<String, Value>>,
        max_pages: usize,
    ) -> Pages<'_> {
        Pages {
            client: self,
            url: self.v2_url(path),
            params: params.unwrap_or_default(),
            page: 1,
            max_pages,
            pending: None,
            done: false,
        }
    }

    /// `(remaining, total)` from the last response, else from a 1-item GET.
    pub fn rate_limit_remaining(&self) -> Result<(Option<i64>, Option<i64>)> {
        {
            let st = self.state();
            if st.rl_rem.is_some() {
                return Ok((st.rl_rem, st.rl_tot));
            }
        }
        self.send(
            "GET",
            &format!("{}/changes?per_page=1", self.tenant.api_v2),
            &RequestOpts::default(),
        )?;
        let st = self.state();
        Ok((st.rl_rem, st.rl_tot))
    }
}

/// Iterator returned by [`Client::v2_get_paginated`]; stops after the first error.
pub struct Pages<'a> {
    client: &'a Client,
    url: String,
    params: Map<String, Value>,
    page: usize,
    max_pages: usize,
    /// `(link header, page)` of the page last yielded, checked on the next call
    /// so the max-pages warning prints only when the caller asks for more.
    pending: Option<(String, usize)>,
    done: bool,
}

impl Iterator for Pages<'_> {
    type Item = Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((link, page)) = self.pending.take() {
            if !link.contains("rel=\"next\"") {
                self.done = true;
            } else if page == self.max_pages {
                term::err().print(&format!(
                    "[yellow]warning[/yellow]: stopped after {} pages; more results exist (narrow the query to see them)",
                    self.max_pages
                ));
                self.done = true;
            }
        }
        if self.done || self.page > self.max_pages {
            return None;
        }
        let page = self.page;
        self.page += 1;
        self.params.insert("page".into(), json!(page));
        let opts = RequestOpts::default().params(self.params.clone());
        let result = self
            .client
            .send("GET", &self.url, &opts)
            .and_then(|r| {
                self.client
                    .check_response(&r, "GET", &self.url, Some(&self.params))
                    .map(|()| r)
            })
            .and_then(|r| {
                r.json_value()
                    .map(|data| (data, r.header("link").unwrap_or("").to_string()))
            });
        match result {
            Ok((data, link)) => {
                self.pending = Some((link, page));
                Some(Ok(data))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

// ---------- process-wide helper (binary edge only) ----------

static INSTANCE: Mutex<Option<Arc<Client>>> = Mutex::new(None);

/// Shared client built from the process config (`get_client`). Library code
/// takes a `Client` or `Arc<Ctx>` instead.
pub fn get_client() -> Result<Arc<Client>> {
    let mut g = INSTANCE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(c) = g.as_ref() {
        return Ok(c.clone());
    }
    let c = Arc::new(Client::new(Arc::new(Ctx::from_process()))?);
    *g = Some(c.clone());
    Ok(c)
}

/// Discard the shared client (e.g. after re-login).
pub fn reset_client() {
    INSTANCE.lock().unwrap_or_else(|p| p.into_inner()).take();
}

// ---------- helpers ----------

fn is_idempotent(method: &str) -> bool {
    IDEMPOTENT_METHODS.contains(&method.to_uppercase().as_str())
}

fn retry_delay(attempt: usize, response: Option<&Response>) -> f64 {
    if let Some(d) = response.and_then(|r| log::retry_after_seconds(r.header("retry-after"))) {
        return d;
    }
    RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)]
}

/// The URL with `params` merged into its query, encoded like the log target.
fn wire_url(url: &str, params: Option<&Map<String, Value>>) -> String {
    let base = url
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    let query = log::query_string(url, params);
    if query.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query}")
    }
}

/// Wire bytes and default `Content-Type` (httpx 0.28 `json=` is compact, non-ASCII kept).
fn encode_body(body: &Body) -> (Option<Vec<u8>>, Option<&'static str>) {
    match body {
        Body::None => (None, None),
        Body::Json(v) => {
            let text = fsv_py::Dumps::default()
                .ensure_ascii(false)
                .separators(",", ":")
                .dump(v);
            (Some(text.into_bytes()), Some("application/json"))
        }
        Body::Data(d) => (Some(d.clone()), Some("application/x-www-form-urlencoded")),
        Body::Raw(d) => (Some(d.clone()), None),
    }
}

fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    headers.push((name.to_string(), value.to_string()));
}

/// `r.json()`, else the first 500 characters of the text.
fn response_body(r: &Response) -> Value {
    r.json_value()
        .unwrap_or_else(|_| Value::String(r.text().chars().take(500).collect()))
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

/// `.get("results", [])` on a response object.
fn results_of(r: &Value) -> Result<Vec<Value>> {
    match r {
        Value::Object(m) => Ok(match m.get("results") {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        }),
        other => Err(Error::other(format!(
            "'{}' object has no attribute 'get'",
            py_type(other)
        ))),
    }
}

/// `<meta name="csrf-token" content="...">` regardless of attribute order, quotes or case.
pub fn csrf_from_html(html: &str) -> Option<String> {
    static META: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)<meta\b[^>]*>").expect("regex"));
    static ATTR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"([^\s=/>"']+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#).expect("regex")
    });
    for tag in META.find_iter(html) {
        let mut attrs: Vec<(String, String)> = Vec::new();
        for c in ATTR.captures_iter(tag.as_str()) {
            let raw = c
                .get(2)
                .or_else(|| c.get(3))
                .or_else(|| c.get(4))
                .map_or("", |m| m.as_str());
            let key = c[1].to_lowercase();
            attrs.retain(|(k, _)| *k != key);
            attrs.push((key, pystr::html_unescape(raw).into_owned()));
        }
        let get = |k: &str| {
            attrs
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        if get("name").is_some_and(|n| n.to_lowercase() == "csrf-token") {
            if let Some(content) = get("content").filter(|c| !c.is_empty()) {
                return Some(content.to_string());
            }
        }
    }
    None
}

/// HTML with no CSRF meta that shows a password field or Freshid markers.
pub fn looks_like_login_html(html: &str) -> bool {
    if csrf_from_html(html).is_some() {
        return false;
    }
    let low = html.to_lowercase();
    low.contains("type=\"password\"")
        || low.contains("type='password'")
        || low.contains("freshid")
        || low.contains("/login")
}

#[cfg(test)]
mod tests;
