use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::mock::MockTransport;
use super::*;
use crate::ctx::{Ctx, Paths};

const ACME: &str = "acme.freshservice.com";
const CSRF_PAGE: &str = r#"<html><head><meta content="tok&amp;1" name='CSRF-Token'>
<script src="https://acme-org.myfreshworks.com/x.js"></script></head></html>"#;

type Writes = Arc<Mutex<Vec<(String, String, Option<u16>, bool)>>>;

struct Harness {
    client: Client,
    mock: Arc<MockTransport>,
    sleeps: Arc<Mutex<Vec<Duration>>>,
    journal: Writes,
}

fn ctx_at(root: &Path) -> Arc<Ctx> {
    Arc::new(Ctx::new(Paths::under(root)).with_tenant(ACME))
}

fn session_cookies() -> Cookies {
    [
        ("_x_m", "m"),
        ("_x_d", "d"),
        ("_x_w", "w"),
        ("fw-session-id", "fw1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

fn harness_at(root: &Path, mock: Arc<MockTransport>) -> Harness {
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let journal: Writes = Arc::new(Mutex::new(Vec::new()));
    let (s, j) = (sleeps.clone(), journal.clone());
    let client = Client::builder(ctx_at(root))
        .transport(mock.clone())
        .cookies(session_cookies())
        .sleeper(move |d| s.lock().unwrap().push(d))
        .journal(move |m, u, _body, o| {
            j.lock()
                .unwrap()
                .push((m.to_string(), u.to_string(), o.status, o.ok))
        })
        .build()
        .unwrap();
    Harness {
        client,
        mock,
        sleeps,
        journal,
    }
}

fn harness(mock: Arc<MockTransport>) -> Harness {
    harness_at(Path::new("/nonexistent/fsv-client-test"), mock)
}

fn ok_json(v: Value) -> Result<Response, TransportError> {
    Ok(Response::json(200, &v))
}

#[test]
fn client_is_send_sync() {
    fn assert<T: Send + Sync>() {}
    assert::<Client>();
    assert::<Arc<Client>>();
}

#[test]
fn ctx_paths_and_tenant() {
    let p = Paths::under(Path::new("/r"));
    assert_eq!(p.config_dir, Path::new("/r/config"));
    assert_eq!(p.session_file, Path::new("/r/config/session.json"));
    assert_eq!(p.schema_dir, Path::new("/r/cache/schema"));
    let t = Tenant::new(ACME);
    assert_eq!(t.api_v2, "https://acme.freshservice.com/api/v2");
    assert_eq!(t.api_int, "https://acme.freshservice.com/api/_");
    assert_eq!(t.login_url, "https://acme.freshservice.com/");
    let no_tenant = Arc::new(Ctx::new(p));
    let err = Client::builder(no_tenant)
        .cookies(session_cookies())
        .build()
        .err()
        .unwrap();
    assert!(err.to_string().contains("no Freshservice domain"));
}

#[test]
fn wire_url_matches_httpx() {
    // str(httpx.Client().build_request("GET", url, params=params).url), httpx 0.28.1
    let cases = [
        (
            "https://acme.freshservice.com/api/v2/changes",
            Some(
                json!({"query": "\"agent_id:5 AND created_at:>'2026-01-01'\"", "page": 2, "per_page": 100}),
            ),
            "https://acme.freshservice.com/api/v2/changes?query=%22agent_id%3A5+AND+created_at%3A%3E%272026-01-01%27%22&page=2&per_page=100",
        ),
        (
            "https://acme.freshservice.com/search/autocomplete/requesters",
            Some(json!({"q": "jane.doe+x@acme.com, bob smith", "qf": "details"})),
            "https://acme.freshservice.com/search/autocomplete/requesters?q=jane.doe%2Bx%40acme.com%2C+bob+smith&qf=details",
        ),
        (
            "https://acme.freshservice.com/search/changes",
            Some(json!({"term": "ünïcode 中 #1 & 50%", "page": 1, "search_sort": "created_at"})),
            "https://acme.freshservice.com/search/changes?term=%C3%BCn%C3%AFcode+%E4%B8%AD+%231+%26+50%25&page=1&search_sort=created_at",
        ),
        (
            "https://acme.freshservice.com/x?a=1&b=2",
            Some(
                json!({"b": "z", "t": true, "f": false, "fl": 1.5, "n": null, "l": ["x y", null, 3]}),
            ),
            "https://acme.freshservice.com/x?b=z&t=true&f=false&fl=1.5&n=&l=x+y&l=&l=3",
        ),
        (
            "https://acme.freshservice.com/x?a=1&b=2",
            Some(json!({})),
            "https://acme.freshservice.com/x",
        ),
        (
            "https://acme.freshservice.com/x?a=1&b=2#frag",
            None,
            "https://acme.freshservice.com/x?a=1&b=2",
        ),
        (
            "https://acme.freshservice.com/x",
            Some(json!({"s": "~-._*()!'/:;=?[]{}|\\^`<>\""})),
            "https://acme.freshservice.com/x?s=~-._%2A%28%29%21%27%2F%3A%3B%3D%3F%5B%5D%7B%7D%7C%5C%5E%60%3C%3E%22",
        ),
    ];
    for (url, p, want) in cases {
        let p = p.map(params);
        let got = wire_url(url, p.as_ref());
        assert_eq!(got, want);
        assert_eq!(
            Url::parse(&got).unwrap().as_str(),
            want,
            "url crate keeps the bytes"
        );
    }
}

#[test]
fn get_sends_params_headers_and_tenant_cookies() {
    let h = harness(MockTransport::queue(vec![ok_json(
        json!({"agent": {"id": 7}}),
    )]));
    let mut params = Map::new();
    params.insert("query".into(), json!("a b"));
    params.insert("skip".into(), Value::Null);
    params.insert("ids".into(), json!([1, 2]));
    let v = h.client.v2_get("/agents/me", Some(params)).unwrap();
    assert_eq!(v["agent"]["id"], 7);
    let req = &h.mock.requests()[0];
    assert_eq!(
        req.url,
        "https://acme.freshservice.com/api/v2/agents/me?query=a+b&skip=&ids=1&ids=2"
    );
    assert_eq!(req.header("user-agent"), Some(UA));
    assert_eq!(req.header("accept"), Some("application/json"));
    let cookie = req.header("cookie").unwrap();
    assert!(cookie.contains("_x_m=m") && cookie.contains("fw-session-id=fw1"));
    assert!(h.journal.lock().unwrap().is_empty(), "GET is not journaled");
}

#[test]
fn json_body_is_compact_and_journaled() {
    let h = harness(MockTransport::queue(vec![ok_json(
        json!({"change": {"id": 5}}),
    )]));
    h.client
        .v2_post("changes", &json!({"subject": "é", "n": 1}))
        .unwrap();
    let req = &h.mock.requests()[0];
    assert_eq!(
        req.body.as_deref(),
        Some("{\"subject\":\"é\",\"n\":1}".as_bytes())
    );
    assert_eq!(req.header("content-type"), Some("application/json"));
    assert_eq!(
        *h.journal.lock().unwrap(),
        vec![(
            "POST".to_string(),
            "https://acme.freshservice.com/api/v2/changes".to_string(),
            Some(200),
            true
        )]
    );
}

#[test]
fn get_retries_on_503_with_retry_after_then_succeeds() {
    let h = harness(MockTransport::queue(vec![
        Ok(Response::new(503).with_header("retry-after", "2")),
        Err(TransportError::new("ReadTimeout", "timed out")),
        Ok(Response::json(200, &json!({"ok": true}))
            .with_header("x-ratelimit-remaining", "99")
            .with_header("x-ratelimit-total", "100")),
    ]));
    assert_eq!(
        h.client.v2_get("changes", None).unwrap(),
        json!({"ok": true})
    );
    assert_eq!(h.mock.requests().len(), 3);
    assert_eq!(
        *h.sleeps.lock().unwrap(),
        vec![Duration::from_secs(2), Duration::from_secs_f64(1.5)]
    );
    assert_eq!(
        h.client.rate_limit_remaining().unwrap(),
        (Some(99), Some(100))
    );
    assert_eq!(
        h.mock.requests().len(),
        3,
        "cached rate limit needs no request"
    );
}

#[test]
fn retries_exhaust_to_api_error_and_writes_never_retry() {
    let h = harness(MockTransport::new(|_| {
        Ok(Response::json(500, &json!({"message": "boom"})))
    }));
    let err = h.client.v2_get("changes/1", None).unwrap_err();
    assert_eq!(err.status(), Some(500));
    assert_eq!(h.mock.requests().len(), 3);
    assert_eq!(
        err.as_api().unwrap().target.as_deref(),
        Some("/api/v2/changes/1")
    );

    let h = harness(MockTransport::new(|_| {
        Err(TransportError::new("ConnectError", "refused"))
    }));
    let err = h.client.v2_put("changes/1", &json!({})).unwrap_err();
    assert!(matches!(&err, Error::Http { kind, .. } if kind == "ConnectError"));
    assert_eq!(h.mock.requests().len(), 1);
    assert!(h.sleeps.lock().unwrap().is_empty());
    let j = h.journal.lock().unwrap();
    assert_eq!((j[0].2, j[0].3), (None, false));
}

#[test]
fn session_errors_for_redirect_401_and_html() {
    for resp in [
        Response::redirect(302, "/login"),
        Response::new(401),
        Response::html(200, "<html></html>"),
    ] {
        let h = harness(MockTransport::queue(vec![Ok(resp)]));
        let err = h.client.v2_get("changes", None).unwrap_err();
        assert!(
            matches!(err, Error::Session(ref m) if m == EXPIRED_MSG),
            "{err:?}"
        );
    }
}

#[test]
fn forbidden_probes_session() {
    // Valid session (probe page has the CSRF meta): the 403 stays an API error.
    let h = harness(MockTransport::new(|r| {
        Ok(if r.url.ends_with(PROBE_PATH) {
            Response::html(200, CSRF_PAGE)
        } else {
            Response::json(403, &json!({"message": "denied"}))
        })
    }));
    assert_eq!(
        h.client.v2_get("agents/me", None).unwrap_err().status(),
        Some(403)
    );
    assert_eq!(h.client.check_session().unwrap(), json!({}));
    let probes = h
        .mock
        .paths()
        .iter()
        .filter(|p| p.ends_with(PROBE_PATH))
        .count();
    assert_eq!(probes, 1, "positive probe is cached");

    // Expired session: probe redirects to login.
    let h = harness(MockTransport::new(|r| {
        Ok(if r.url.ends_with(PROBE_PATH) {
            Response::redirect(302, "https://login.example/")
        } else {
            Response::new(403)
        })
    }));
    assert!(matches!(
        h.client.v2_get("agents/me", None).unwrap_err(),
        Error::Session(_)
    ));
}

#[test]
fn redirects_follow_update_jar_and_keep_cookies_on_tenant() {
    let h = harness(MockTransport::new(|r| {
        Ok(match r.url.as_str() {
            "https://acme.freshservice.com/a/file" => {
                Response::redirect(302, "https://cdn.example.com/blob?sig=1").with_header(
                    "set-cookie",
                    "fw-session-id=fw2; Path=/; Domain=.freshservice.com; Secure",
                )
            }
            _ => Response::new(200).with_body(b"bytes".to_vec()),
        })
    }));
    let r = h
        .client
        .fetch_raw("https://acme.freshservice.com/a/file", true)
        .unwrap();
    assert_eq!(r.body, b"bytes");
    assert_eq!(r.url, "https://cdn.example.com/blob?sig=1");
    let reqs = h.mock.requests();
    assert!(reqs[0].header("cookie").is_some());
    assert_eq!(
        reqs[1].header("cookie"),
        None,
        "no tenant cookies to another host"
    );
    assert_eq!(h.client.cookie("fw-session-id").as_deref(), Some("fw2"));

    let h = harness(MockTransport::queue(vec![Ok(Response::redirect(
        302, "/next",
    ))]));
    assert_eq!(
        h.client
            .fetch_raw("https://acme.freshservice.com/a/file", false)
            .unwrap()
            .status,
        302
    );
    assert_eq!(h.mock.requests().len(), 1);
}

#[test]
fn redirect_limit_is_twenty() {
    let h = harness(MockTransport::new(|_| {
        Ok(Response::redirect(302, "/again"))
    }));
    let err = h
        .client
        .send(
            "GET",
            "https://acme.freshservice.com/a",
            &RequestOpts::default().follow(true),
        )
        .unwrap_err();
    assert!(matches!(&err, Error::Http { kind, .. } if kind == "TooManyRedirects"));
    assert_eq!(h.mock.requests().len(), MAX_REDIRECTS + 1, "not retried");
}

#[test]
fn post_redirect_becomes_get() {
    let h = harness(MockTransport::new(|r| {
        Ok(if r.method == "POST" {
            Response::redirect(302, "/done")
        } else {
            Response::new(200)
        })
    }));
    let opts = RequestOpts::default()
        .body(Body::Json(json!({"a": 1})))
        .follow(true);
    h.client
        .send("POST", "https://acme.freshservice.com/x", &opts)
        .unwrap();
    let reqs = h.mock.requests();
    assert_eq!(
        (reqs[1].method.as_str(), reqs[1].body.is_none()),
        ("GET", true)
    );
}

#[test]
fn int_post_fetches_csrf_once_and_fw_domain() {
    let h = harness(MockTransport::new(|r| {
        Ok(if r.url.ends_with("/a/changes") {
            Response::html(200, CSRF_PAGE)
        } else {
            Response::json(200, &json!({"id": 1}))
        })
    }));
    h.client.int_post("changes", &json!({"x": 1})).unwrap();
    h.client.int_delete("changes/1", None, None).unwrap();
    let writes = h.mock.writes();
    assert_eq!(writes.len(), 2);
    let reqs = h.mock.requests();
    let posts: Vec<_> = reqs.iter().filter(|r| r.method != "GET").collect();
    assert!(
        posts
            .iter()
            .all(|r| r.header("x-csrf-token") == Some("tok&1"))
    );
    assert_eq!(h.mock.gets().len(), 1, "CSRF cached");
    assert_eq!(
        h.client.fw_domain().as_deref(),
        Some("acme-org.myfreshworks.com")
    );
    assert_eq!(writes[1].2, None, "DELETE without body");
}

#[test]
fn csrf_failures() {
    let h = harness(MockTransport::new(|_| {
        Ok(Response::html(
            200,
            "<form><input type=\"password\"></form>",
        ))
    }));
    assert!(matches!(
        h.client.get_csrf().unwrap_err(),
        Error::Session(_)
    ));

    let h = harness(MockTransport::new(|_| Ok(Response::html(502, ""))));
    assert_eq!(h.client.get_csrf().unwrap_err().status(), Some(502));

    let h = harness(MockTransport::new(|_| {
        Ok(Response::html(200, "<html>nothing</html>"))
    }));
    assert_eq!(h.client.get_csrf().unwrap_err().status(), Some(0));

    let h = harness(MockTransport::new(|r| {
        Ok(if r.url.contains("freshservice") {
            Response::redirect(302, "https://acme.myfreshworks.com/login")
        } else {
            Response::html(200, CSRF_PAGE)
        })
    }));
    assert!(
        matches!(h.client.get_csrf().unwrap_err(), Error::Session(_)),
        "redirected off-tenant"
    );
}

#[test]
fn csrf_meta_parsing() {
    assert_eq!(
        csrf_from_html(r#"<META NAME="csrf-token" CONTENT="abc">"#).as_deref(),
        Some("abc")
    );
    assert_eq!(
        csrf_from_html("<meta content=x1 name=csrf-token />").as_deref(),
        Some("x1")
    );
    assert_eq!(
        csrf_from_html("<meta content=x1 name=csrf-token/>"),
        None,
        "unquoted value swallows the slash"
    );
    assert_eq!(
        csrf_from_html(r#"<meta name="csrf-token" content="">"#),
        None
    );
    assert!(looks_like_login_html("<div class='freshid'>"));
    assert!(!looks_like_login_html(CSRF_PAGE));
}

#[test]
fn fw_session_rotation_is_tracked() {
    let h = harness(MockTransport::queue(vec![Ok(Response::json(
        200,
        &json!({}),
    )
    .with_header("set-cookie", "fw-session-id=fw9; Path=/"))]));
    h.client.v2_get("x", None).unwrap();
    assert_eq!(h.client.state().fw_session_id.as_deref(), Some("fw9"));
}

#[test]
fn pagination_follows_link_header() {
    let h = harness(MockTransport::queue(vec![
        Ok(Response::json(200, &json!({"p": 1}))
            .with_header("link", "<https://x?page=2>; rel=\"next\"")),
        Ok(Response::json(200, &json!({"p": 2}))),
    ]));
    let mut params = Map::new();
    params.insert("per_page".into(), json!(100));
    let pages: Vec<Value> = h
        .client
        .v2_get_paginated("changes", Some(params), 20)
        .map(Result::unwrap)
        .collect();
    assert_eq!(pages, vec![json!({"p": 1}), json!({"p": 2})]);
    assert!(h.mock.gets()[1].ends_with("/api/v2/changes?per_page=100&page=2"));

    let h = harness(MockTransport::new(|_| {
        Ok(Response::json(200, &json!([])).with_header("link", "rel=\"next\""))
    }));
    assert_eq!(h.client.v2_get_paginated("changes", None, 2).count(), 2);
}

#[cfg(feature = "keepalive")]
#[test]
fn keepalive_posts_to_fw_domain() {
    let h = harness(MockTransport::new(|r| {
        Ok(if r.method == "GET" {
            Response::html(200, CSRF_PAGE)
        } else {
            Response::new(204)
        })
    }));
    assert!(h.client.keepalive());
    let w = h.mock.writes();
    assert_eq!(w[0].1, "https://acme-org.myfreshworks.com/api/v2/session");
    assert_eq!(
        w[0].2,
        Some(json!({"session_token": "fw1", "checkSessionOnlyOnce": false}))
    );
    assert_eq!(h.mock.requests().last().unwrap().header("cookie"), None);
    assert!(h.journal.lock().unwrap().is_empty());

    let h = harness(MockTransport::new(|_| {
        Ok(Response::html(200, "<html>no meta</html>"))
    }));
    assert!(!h.client.keepalive());
}

#[test]
fn lookup_choices_forces_tenant_and_caches() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness_at(
        dir.path(),
        MockTransport::queue(vec![ok_json(json!({"results": [{"id": 1}]}))]),
    );
    let link = "https://evil.example.com/api/_/lookup?field=x&q=old&blank=";
    assert_eq!(
        h.client.lookup_choices(link, "ab").unwrap(),
        vec![json!({"id": 1})]
    );
    assert_eq!(
        h.mock.gets(),
        vec!["https://acme.freshservice.com/api/_/lookup?field=x&q=ab".to_string()]
    );
    assert_eq!(
        h.client.lookup_choices(link, "ab").unwrap(),
        vec![json!({"id": 1})]
    );
    assert_eq!(h.mock.requests().len(), 1, "second call served from cache");
}

#[test]
fn log_line_redacts() {
    let body = Body::Json(json!({"password": "x", "a": 1}));
    assert_eq!(
        log::log_line(
            "post",
            "https://acme.freshservice.com/api/v2/x",
            None,
            &body,
            "200 5ms"
        ),
        "[api] POST /api/v2/x body={\"password\":\"[redacted]\",\"a\":1} -> 200 5ms"
    );
    let get = log::log_line(
        "GET",
        "https://acme.freshservice.com/a?b=1",
        None,
        &body,
        "200 1ms",
    );
    assert_eq!(get, "[api] GET /a?b=1 -> 200 1ms");
}

#[test]
fn cookie_jar_rules() {
    let tenant = url::Url::parse("https://acme.freshservice.com/a/b").unwrap();
    let other = url::Url::parse("https://sso.example.com/x/y").unwrap();
    let mut jar = CookieJar::new(ACME);
    jar.set_session("s", "1");
    jar.extract(
        &other,
        [
            "o=2; Domain=example.com; Path=/",
            "bad=3; Domain=evil.com",
            "p=4",
        ],
    );
    assert_eq!(jar.header_for(&tenant).as_deref(), Some("s=1"));
    assert_eq!(
        jar.header_for(&url::Url::parse("https://www.example.com/").unwrap())
            .as_deref(),
        Some("o=2")
    );
    assert_eq!(
        jar.header_for(&url::Url::parse("https://sso.example.com/x/z").unwrap())
            .as_deref(),
        Some("p=4; o=2")
    );
    assert_eq!(
        jar.header_for(&url::Url::parse("http://acme.freshservice.com/").unwrap()),
        None,
        "secure only"
    );
    jar.extract(&tenant, ["s=; Max-Age=0"]);
    assert_eq!(
        jar.get("s"),
        Some("1"),
        "default path /a does not match the session cookie's /"
    );
    jar.extract(&tenant, ["s=; Path=/; Max-Age=0"]);
    jar.extract(
        &tenant,
        ["t=1; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT"],
    );
    assert_eq!(jar.get("s"), None);
    assert_eq!(jar.get("t"), None);
}

const LOGIN: &str = r#"<html><form action="/login"><input type="password" name="p"></form></html>"#;

fn params(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}

#[test]
fn is_sensitive_key_is_substring_based() {
    for (k, want) in [
        ("x_csrf_token", true),
        ("client_secret", true),
        ("fw-session-id", true),
        ("passwd", true),
        ("Set-Cookie", true),
        ("subject", false),
        (" Authorization ", true),
        ("API-Key", true),
        ("name", false),
        ("Credentials", true),
    ] {
        assert_eq!(log::is_sensitive_key(k), want, "{k}");
    }
}

#[test]
fn redact_is_recursive() {
    assert_eq!(
        log::redact(
            &json!({"a": {"token": "t", "b": [{"password": "p", "x": 1}]}, "ok": 2, "l": [1, "s"]})
        ),
        json!({"a": {"token": "[redacted]", "b": [{"password": "[redacted]", "x": 1}]}, "ok": 2, "l": [1, "s"]})
    );
}

#[test]
fn request_target_merges_params() {
    assert_eq!(
        log::request_target("https://acme.freshservice.com", None),
        "/"
    );
    assert_eq!(
        log::request_target(
            "https://h/api/v2/x?a=1&e=",
            Some(&params(json!({"b": [1, null], "c": null, "d": "x y/z"})))
        ),
        "/api/v2/x?a=1&e=&b=1&b=&d=x%20y/z"
    );
    assert_eq!(
        log::request_target("https://h/a%20b", Some(&params(json!({"q": "é&"})))),
        "/a%20b?q=%C3%A9%26"
    );
}

#[test]
fn redact_data_previews() {
    for (data, want) in [
        (
            r#"{"password":"x","a":1}"#,
            r#"{"password":"[redacted]","a":1}"#,
        ),
        (
            "a=1&password=2&token=",
            "a=1&password=%5Bredacted%5D&token=%5Bredacted%5D",
        ),
        ("token", "[redacted]"),
        ("plain text", "plain text"),
        (r#"[{"secret":1}]"#, r#"[{"secret":"[redacted]"}]"#),
        ("a b=c d", "a+b=c+d"),
    ] {
        assert_eq!(log::redact_data(data.as_bytes()), want, "{data}");
    }
}

#[test]
fn body_preview_is_capped() {
    let long = log::body_preview(&Body::Json(json!({"k": "x".repeat(400)})));
    assert_eq!(long, format!(" body={{\"k\":\"{}...", "x".repeat(291)));
    assert_eq!(long.chars().count(), " body=".len() + 300);
    assert_eq!(
        log::body_preview(&Body::Data(b"a=1&pass=2".to_vec())),
        " body=a=1&pass=%5Bredacted%5D"
    );
    assert_eq!(log::body_preview(&Body::None), "");
    assert_eq!(log::body_preview(&Body::Raw(b"secret".to_vec())), "");
    assert_eq!(log::body_preview(&Body::Json(json!({}))), " body={}");
}

#[test]
fn retry_after_seconds_clamps() {
    for (raw, want) in [
        (Some("2"), Some(2.0)),
        (Some("30"), Some(10.0)),
        (Some("-5"), Some(0.0)),
        (Some("abc"), None),
        (Some(""), None),
        (None, None),
        (Some("  1.5 "), Some(1.5)),
        (Some("nan"), Some(0.0)),
        (Some("inf"), Some(10.0)),
        (Some("1_0"), Some(10.0)),
    ] {
        assert_eq!(log::retry_after_seconds(raw), want, "{raw:?}");
    }
}

#[test]
fn retry_delay_and_should_retry() {
    assert_eq!(retry_delay(0, None), 0.5);
    assert_eq!(retry_delay(1, None), 1.5);
    assert_eq!(retry_delay(5, None), 1.5);
    let r429 = Response::new(429).with_header("retry-after", "3");
    assert_eq!(retry_delay(0, Some(&r429)), 3.0);
    assert_eq!(retry_delay(1, Some(&Response::new(503))), 1.5);

    assert!(should_retry("get", Some(&Response::new(503)), false));
    assert!(!should_retry("GET", Some(&Response::new(404)), false));
    assert!(!should_retry("POST", Some(&Response::new(503)), false));
    assert!(should_retry("HEAD", None, true));
    assert!(!should_retry("options", None, false));
    assert!(!should_retry("DELETE", None, true));
}

#[test]
fn response_body_json_or_text_prefix() {
    assert_eq!(
        response_body(&Response::json(422, &json!({"errors": [1]}))),
        json!({"errors": [1]})
    );
    let long = Response::new(500).with_body("x".repeat(600).into_bytes());
    assert_eq!(response_body(&long), json!("x".repeat(500)));
    assert_eq!(
        response_body(&Response::html(500, "<html>oops</html>")),
        json!("<html>oops</html>")
    );
}

#[test]
fn make_cookie_is_secure_and_host_scoped() {
    let mut jar = CookieJar::new("ACME.freshservice.com");
    for (k, v) in session_cookies() {
        jar.set_session(&k, &v);
    }
    assert_eq!(jar.cookies().len(), 4);
    assert!(
        jar.cookies()
            .iter()
            .all(|c| c.secure && c.host_only && c.path == "/" && c.domain == ACME)
    );
    jar.set_session("_x_m", "m2");
    assert_eq!(jar.cookies().len(), 4, "same name replaces");
    assert_eq!(jar.get("_x_m"), Some("m2"));
}

#[test]
fn fw_cookie_prefers_tenant_host_over_dotted_domain() {
    let h = harness(MockTransport::new(|r| {
        Ok(if r.url.starts_with("https://sso.freshservice.com") {
            Response::json(200, &json!({})).with_header(
                "set-cookie",
                "fw-session-id=rotated; Domain=.freshservice.com; Path=/",
            )
        } else {
            Response::json(200, &json!({}))
        })
    }));
    h.client
        .send(
            "GET",
            "https://sso.freshservice.com/x",
            &RequestOpts::default(),
        )
        .unwrap();
    let dup = h
        .client
        .state()
        .jar
        .cookies()
        .iter()
        .filter(|c| c.name == "fw-session-id")
        .count();
    assert_eq!(dup, 2);
    assert_eq!(
        h.client.fw_cookie().as_deref(),
        Some("fw1"),
        "exact-host cookie wins"
    );
    assert_eq!(h.client.v2_get("x", None).unwrap(), json!({}));
}

#[test]
fn empty_rotated_fw_session_id_is_ignored() {
    let h = harness(MockTransport::queue(vec![Ok(Response::json(
        200,
        &json!({}),
    )
    .with_header("set-cookie", "fw-session-id=; Path=/"))]));
    h.client.v2_get("x", None).unwrap();
    assert_eq!(h.client.fw_cookie().as_deref(), Some(""));
    assert_eq!(h.client.state().fw_session_id.as_deref(), Some("fw1"));
}

#[test]
fn rotated_fw_session_id_is_persisted_for_stored_sessions() {
    let env = crate::testenv::isolated().with_domain(ACME);
    session::save_cookies(&session_cookies(), Some(session::Backend::File)).unwrap();
    let mock = MockTransport::queue(vec![Ok(
        Response::json(200, &json!({})).with_header("set-cookie", "fw-session-id=new; Path=/")
    )]);
    let client = Client::builder(Arc::new(Ctx::from_process()))
        .transport(mock.clone())
        .sleeper(|_| {})
        .journal(|_, _, _, _| {})
        .build()
        .unwrap();
    assert_eq!(client.ctx().paths.config_dir, env.config_dir());
    client.v2_get("x", None).unwrap();
    session::clear_cache();
    assert_eq!(session::load_cookies().unwrap()["fw-session-id"], "new");
}

#[test]
fn get_client_is_shared_until_reset() {
    let _env = crate::testenv::isolated();
    reset_client();
    assert!(get_client().is_err(), "no domain");
    let _env = _env.with_domain(ACME);
    assert!(get_client().is_err(), "no session");
    session::save_cookies(&session_cookies(), Some(session::Backend::File)).unwrap();
    let a = get_client().unwrap();
    let b = get_client().unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.tenant().domain, ACME);
    reset_client();
    let c = get_client().unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    reset_client();
}

#[test]
fn probe_session_outcomes() {
    for (resp, want) in [
        (Ok(Response::html(200, CSRF_PAGE)), Some(true)),
        (
            Ok(Response::redirect(302, "https://auth.example/login")),
            Some(false),
        ),
        (Ok(Response::new(401)), Some(false)),
        (Ok(Response::new(403)), Some(false)),
        (Ok(Response::html(200, LOGIN)), Some(false)),
        (Ok(Response::new(500)), None),
        (Err(TransportError::new("ConnectError", "down")), None),
    ] {
        let h = harness(MockTransport::new(move |_| resp.clone()));
        assert_eq!(h.client.probe_session(), want);
        let req = &h.mock.requests()[0];
        assert_eq!(req.url, "https://acme.freshservice.com/a/dashboard");
        assert_eq!(req.header("accept"), Some(HTML_ACCEPT));
    }
    let h = harness(MockTransport::queue(vec![Ok(Response::html(
        200, CSRF_PAGE,
    ))]));
    assert_eq!(h.client.probe_session(), Some(true));
    assert_eq!(h.client.probe_session(), Some(true), "cached for PROBE_TTL");
    assert_eq!(h.mock.requests().len(), 1);
}

#[test]
fn check_response_mapping() {
    let h = harness(MockTransport::queue(vec![Ok(Response::html(
        200, CSRF_PAGE,
    ))]));
    let check = |r: Response| {
        h.client.check_response(
            &r,
            "GET",
            "https://acme.freshservice.com/api/v2/x",
            Some(&params(json!({"p": 1}))),
        )
    };
    for status in [301, 302, 307] {
        assert!(
            matches!(check(Response::redirect(status, "")), Err(Error::Session(m)) if m == EXPIRED_MSG)
        );
    }
    assert!(check(Response::new(304)).is_ok());
    assert!(matches!(check(Response::new(401)), Err(Error::Session(_))));
    assert!(matches!(
        check(Response::html(200, "<p>")),
        Err(Error::Session(_))
    ));
    assert!(check(Response::json(200, &json!({}))).is_ok());
    let e = check(Response::json(422, &json!({"errors": "bad"}))).unwrap_err();
    let api = e.as_api().unwrap();
    assert_eq!(
        (api.status, api.method.as_deref(), api.target.as_deref()),
        (422, Some("GET"), Some("/api/v2/x?p=1"))
    );
    assert_eq!(api.body, json!({"errors": "bad"}));
    let e = check(Response::json(403, &json!({"message": "no access"}))).unwrap_err();
    assert_eq!(
        e.status(),
        Some(403),
        "valid session probe keeps the API error"
    );
}

#[test]
fn request_and_request_inner_journal_writes_only() {
    let h = harness(MockTransport::queue(vec![
        ok_json(json!({"a": 1})),
        Ok(Response::new(200)
            .with_header("content-type", "text/plain")
            .with_body(b"plain text".to_vec())),
        ok_json(json!({"change": {"id": 9}})),
        Ok(Response::json(422, &json!({"errors": "nope"}))),
    ]));
    let url = "https://acme.freshservice.com/api/v2/changes";
    let (status, r) = h.client.request_inner("GET", url, &RequestOpts::default());
    assert_eq!((status, r.unwrap()), (Some(200), json!({"a": 1})));
    assert_eq!(
        h.client
            .request("GET", url, RequestOpts::default())
            .unwrap(),
        json!("plain text")
    );
    let body = RequestOpts::default().body(Body::Json(json!({"s": 1})));
    assert_eq!(
        h.client.request("POST", url, body.clone()).unwrap(),
        json!({"change": {"id": 9}})
    );
    assert_eq!(
        h.client.request("put", url, body).unwrap_err().status(),
        Some(422)
    );
    assert_eq!(
        *h.journal.lock().unwrap(),
        vec![
            ("POST".into(), url.into(), Some(200), true),
            ("put".into(), url.into(), Some(422), false),
        ]
    );
}

#[test]
fn int_get_and_me() {
    let h = harness(MockTransport::queue(vec![
        ok_json(json!({"x": 1})),
        ok_json(json!({"agent": {"first_name": "Ada"}})),
        ok_json(json!({"other": 1})),
        ok_json(json!([1])),
    ]));
    assert_eq!(
        h.client
            .int_get("/changes/1", Some(params(json!({"q": "a"}))))
            .unwrap(),
        json!({"x": 1})
    );
    assert_eq!(
        h.mock.requests()[0].url,
        "https://acme.freshservice.com/api/_/changes/1?q=a"
    );
    assert_eq!(h.client.me().unwrap(), json!({"first_name": "Ada"}));
    assert_eq!(
        h.mock.requests()[1].url,
        "https://acme.freshservice.com/api/v2/agents/me"
    );
    assert_eq!(h.client.me().unwrap(), json!({}));
    assert_eq!(
        h.client.me().unwrap_err().to_string(),
        "'list' object has no attribute 'get'"
    );
}

#[test]
fn autocomplete_params_and_results() {
    let h = harness(MockTransport::queue(vec![
        ok_json(json!({"results": [{"id": 1}]})),
        ok_json(json!({})),
    ]));
    assert_eq!(
        h.client
            .autocomplete(
                "requesters",
                "ad a",
                Some(params(json!({"qf": "email", "x": 1})))
            )
            .unwrap(),
        vec![json!({"id": 1})]
    );
    assert_eq!(
        h.mock.requests()[0].url,
        "https://acme.freshservice.com/search/autocomplete/requesters?q=ad+a&qf=email&x=1"
    );
    assert!(
        h.client
            .autocomplete("agents", "a", None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        h.mock.requests()[1].url,
        "https://acme.freshservice.com/search/autocomplete/agents?q=a&qf=value"
    );
}

#[test]
fn int_request_sends_csrf_and_json() {
    let h = harness(MockTransport::new(|r| {
        Ok(if r.method == "GET" {
            Response::html(200, CSRF_PAGE)
        } else {
            Response::json(200, &json!({"ok": r.method}))
        })
    }));
    let out = h
        .client
        .int_request(
            "DELETE",
            "/changes/1/x",
            None,
            Some(params(json!({"force": "1"}))),
        )
        .unwrap();
    assert_eq!(out, json!({"ok": "DELETE"}));
    let del = h.mock.requests().last().cloned().unwrap();
    assert_eq!(
        del.url,
        "https://acme.freshservice.com/api/_/changes/1/x?force=1"
    );
    assert_eq!(del.header("x-csrf-token"), Some("tok&1"));
    assert_eq!(del.header("content-type"), Some("application/json"));
    assert_eq!(del.body, None);
    h.client
        .int_request("PATCH", "changes/2", Some(&json!({"a": 1})), None)
        .unwrap();
    assert_eq!(
        h.mock.requests().last().unwrap().body.as_deref(),
        Some(&b"{\"a\":1}"[..])
    );
    assert_eq!(h.mock.gets().len(), 1, "CSRF fetched once");
}

#[test]
fn fulltext_search_maps_sort_names() {
    let h = harness(MockTransport::new(|_| ok_json(json!({"changes": []}))));
    for sort in [
        Some("created"),
        Some("modified"),
        Some("relevance"),
        Some("other"),
        None,
    ] {
        h.client.fulltext_search("changes", "vpn", 2, sort).unwrap();
    }
    let urls: Vec<String> = h.mock.requests().iter().map(|r| r.url.clone()).collect();
    let base = "https://acme.freshservice.com/search/changes?term=vpn&page=2";
    assert_eq!(
        urls,
        [
            format!("{base}&search_sort=created_at"),
            format!("{base}&search_sort=updated_at"),
            format!("{base}&search_sort=relevance"),
            base.to_string(),
            base.to_string(),
        ]
    );
}

#[test]
fn int_put_and_int_patch_send_json_with_csrf() {
    let h = harness(MockTransport::new(|r| {
        Ok(if r.method == "GET" {
            Response::html(200, CSRF_PAGE)
        } else {
            Response::json(200, &json!({"m": r.method}))
        })
    }));
    assert_eq!(
        h.client.int_put("changes/1", &json!({"a": 1})).unwrap(),
        json!({"m": "PUT"})
    );
    assert_eq!(
        h.client.int_patch("/changes/2", &json!({"b": 2})).unwrap(),
        json!({"m": "PATCH"})
    );
    let reqs = h.mock.requests();
    let writes: Vec<_> = reqs.iter().filter(|r| r.method != "GET").collect();
    assert_eq!(
        writes[0].url,
        "https://acme.freshservice.com/api/_/changes/1"
    );
    assert_eq!(
        writes[1].url,
        "https://acme.freshservice.com/api/_/changes/2"
    );
    for (w, body) in writes.iter().zip([json!({"a": 1}), json!({"b": 2})]) {
        assert_eq!(w.header("x-csrf-token"), Some("tok&1"));
        assert_eq!(w.header("content-type"), Some("application/json"));
        assert_eq!(
            serde_json::from_slice::<Value>(w.body.as_deref().unwrap()).unwrap(),
            body
        );
    }
}

/// Expected lists come from running httpx (`Client(cookies=jar,
/// transport=MockTransport)`, `list(client.cookies.jar)`) through the same
/// sequence: session cookies via `jar.jar.set_cookie(_make_cookie(..))`,
/// then four responses with these `Set-Cookie` headers.
#[test]
fn cookies_follow_python_httpx_jar_order() {
    let steps: [(&str, &[&str]); 4] = [
        (
            "https://acme.freshservice.com/a",
            &[
                "newc=1; Path=/",
                "fw-session-id=rot; Path=/",
                "pa=1; Path=/a",
                "wsp_workspace=7; Path=/",
            ],
        ),
        (
            "https://sso.example.com/x/y",
            &["o1=1", "o2=2; Domain=.example.com; Path=/", "o3=3; Path=/x"],
        ),
        (
            "https://acme.freshservice.com/a/b",
            &[
                "late=1; Path=/",
                "_x_m=; Max-Age=0; Path=/",
                "pa2=2; Path=/a",
                "_x_m=again; Path=/",
            ],
        ),
        (
            "https://acme.freshservice.com/a",
            &["lastws_workspace=9; Domain=.acme.freshservice.com; Path=/"],
        ),
    ];
    let expected: [&[(&str, &str)]; 4] = [
        &[
            ("_x_m", "m"),
            ("_x_d", "d"),
            ("_x_w", "w"),
            ("fw-session-id", "rot"),
            ("user_workspace", "3"),
            ("newc", "1"),
            ("wsp_workspace", "7"),
            ("pa", "1"),
        ],
        &[
            ("_x_m", "m"),
            ("_x_d", "d"),
            ("_x_w", "w"),
            ("fw-session-id", "rot"),
            ("user_workspace", "3"),
            ("newc", "1"),
            ("wsp_workspace", "7"),
            ("pa", "1"),
            ("o1", "1"),
            ("o3", "3"),
            ("o2", "2"),
        ],
        &[
            ("_x_d", "d"),
            ("_x_w", "w"),
            ("fw-session-id", "rot"),
            ("user_workspace", "3"),
            ("newc", "1"),
            ("wsp_workspace", "7"),
            ("late", "1"),
            ("_x_m", "again"),
            ("pa", "1"),
            ("pa2", "2"),
            ("o1", "1"),
            ("o3", "3"),
            ("o2", "2"),
        ],
        &[
            ("_x_d", "d"),
            ("_x_w", "w"),
            ("fw-session-id", "rot"),
            ("user_workspace", "3"),
            ("newc", "1"),
            ("wsp_workspace", "7"),
            ("late", "1"),
            ("_x_m", "again"),
            ("pa", "1"),
            ("pa2", "2"),
            ("o1", "1"),
            ("o3", "3"),
            ("o2", "2"),
            ("lastws_workspace", "9"),
        ],
    ];
    let queue = Mutex::new(steps.iter().map(|(_, sc)| *sc).collect::<Vec<_>>());
    queue.lock().unwrap().reverse();
    let mock = MockTransport::new(move |_| {
        let sc = queue.lock().unwrap().pop().unwrap();
        let mut r = Response::json(200, &json!({}));
        for s in sc {
            r = r.with_header("set-cookie", s);
        }
        Ok(r)
    });
    let mut cookies = session_cookies();
    cookies.insert("user_workspace".into(), "3".into());
    let client = Client::builder(ctx_at(Path::new("/nonexistent/fsv-client-test")))
        .transport(mock.clone())
        .cookies(cookies)
        .build()
        .unwrap();
    let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
        v.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(
        client.cookies(),
        pairs(&[
            ("_x_m", "m"),
            ("_x_d", "d"),
            ("_x_w", "w"),
            ("fw-session-id", "fw1"),
            ("user_workspace", "3"),
        ]),
        "loaded session, stored order"
    );
    for ((url, _), want) in steps.iter().zip(expected) {
        client.send("GET", url, &RequestOpts::default()).unwrap();
        assert_eq!(client.cookies(), pairs(want), "after {url}");
    }
    assert_eq!(mock.gets().len(), 4);
}

/// `Client.me()` (`.get("agent", {})`) on each payload, from running Python.
#[test]
fn me_unwraps_the_agent() {
    for (payload, want) in [
        (json!({"agent": {"id": 5}}), Ok(json!({"id": 5}))),
        (json!({}), Ok(json!({}))),
        (json!({"agent": null}), Ok(Value::Null)),
        (json!([]), Err("'list' object has no attribute 'get'")),
    ] {
        let body = payload.clone();
        let h = harness(MockTransport::new(move |_| Ok(Response::json(200, &body))));
        let got = h.client.me().map_err(|e| e.to_string());
        assert_eq!(got, want.map_err(String::from), "{payload}");
        assert!(h.mock.paths()[0].ends_with("/api/v2/agents/me"));
    }
}

/// `client._query_string(url, params)` from running Python.
#[test]
fn log_query_string_matches_python() {
    let cases = [
        ("https://x/a?b=1&c=&d=%20e", Value::Null, "b=1&c=&d=%20e"),
        (
            "https://x/a",
            json!({"q": "a b/c", "n": null, "ids": [1, null, "x&y"], "t": true, "f": 1.5}),
            "q=a%20b/c&ids=1&ids=&ids=x%26y&t=True&f=1.5",
        ),
        ("https://x/a", json!({}), ""),
        (
            "https://x/a?x=1",
            json!({"é": "ü@,:"}),
            "x=1&%C3%A9=%C3%BC@,:",
        ),
    ];
    for (url, params, want) in cases {
        assert_eq!(
            super::log::query_string(url, params.as_object()),
            want,
            "{url} {params}"
        );
    }
}
