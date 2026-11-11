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
        "https://acme.freshservice.com/api/v2/agents/me?query=a%20b&ids=1&ids=2"
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
