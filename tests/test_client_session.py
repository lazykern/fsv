"""Session-expiry detection, CSRF, cookies, pagination, redaction (offline)."""
from __future__ import annotations

import http.cookiejar

import httpx
import pytest
from typer.testing import CliRunner

import fsv.client as client
import fsv.config as config
from fsv.errors import APIError, SessionError

COOKIES = {"_x_m": "a", "_x_d": "b", "_x_w": "c", "fw-session-id": "tok"}
APP_OK = '<html><head><meta name="csrf-token" content="TOK123"></head></html>'
LOGIN = '<html><form action="/login"><input type="password" name="p"></form></html>'
HTML = {"content-type": "text/html; charset=utf-8"}
JSON = {"content-type": "application/json"}


def make_client(handler, persist=False):
    c = client.Client(cookies=COOKIES)
    jar = c._client.cookies
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler), cookies=jar, follow_redirects=False)
    c._persist = persist
    return c


def router(routes):
    """routes: {path: (status, headers, body)}; records requests."""
    seen = []

    def handler(req: httpx.Request) -> httpx.Response:
        seen.append(req)
        status, headers, body = routes[req.url.path]
        return httpx.Response(status, headers=headers, content=body)

    handler.seen = seen
    return handler


API = f"https://acme.freshservice.com/api/v2/agents/me"


def test_any_3xx_is_session_expired():
    for status, loc in ((301, "https://acme.freshservice.com/login"), (302, "/somewhere"), (307, "")):
        c = make_client(router({"/api/v2/x": (status, {"location": loc}, "")}))
        with pytest.raises(SessionError, match="session expired or invalid; run `fsv auth login`"):
            c.v2_get("x")


def test_401_is_session_expired():
    c = make_client(router({"/api/v2/x": (401, JSON, "{}")}))
    with pytest.raises(SessionError):
        c.v2_get("x")


def test_login_html_on_non_api_url_is_session_expired():
    c = make_client(router({"/search/changes": (200, HTML, LOGIN)}))
    with pytest.raises(SessionError):
        c.fulltext_search("changes", "x")


def test_403_with_dead_session_probe_redirect_is_session_expired():
    h = router({"/api/v2/agents/me": (403, JSON, '{"message":"forbidden"}'),
                "/a/dashboard": (302, {"location": "https://auth.example/login"}, "")})
    with pytest.raises(SessionError):
        make_client(h).me()


def test_403_with_login_page_probe_is_session_expired():
    h = router({"/api/v2/agents/me": (403, JSON, "{}"), "/a/dashboard": (200, HTML, LOGIN)})
    with pytest.raises(SessionError):
        make_client(h).me()


def test_real_permission_403_is_not_misreported():
    h = router({"/api/v2/agents/me": (403, JSON, '{"message":"no access"}'), "/a/dashboard": (200, HTML, APP_OK)})
    with pytest.raises(APIError) as e:
        make_client(h).me()
    assert e.value.status == 403


def test_403_probe_network_failure_keeps_original_403(monkeypatch):
    monkeypatch.setattr(client.time, "sleep", lambda _s: None)
    def handler(req):
        if req.url.path == "/a/dashboard":
            raise httpx.ConnectError("down")
        return httpx.Response(403, headers=JSON, content="{}")

    with pytest.raises(APIError):
        make_client(handler).me()


def test_check_session_returns_empty_identity_for_restricted_agent():
    h = router({"/api/v2/agents/me": (403, JSON, "{}"), "/a/dashboard": (200, HTML, APP_OK)})
    assert make_client(h).check_session() == {}


def test_check_session_expired_raises():
    h = router({"/api/v2/agents/me": (302, {"location": "/login"}, "")})
    with pytest.raises(SessionError):
        make_client(h).check_session()


def test_check_session_returns_agent():
    h = router({"/api/v2/agents/me": (200, JSON, '{"agent": {"first_name": "Ada"}}')})
    assert make_client(h).check_session() == {"first_name": "Ada"}


# ---- CSRF -------------------------------------------------------------

@pytest.mark.parametrize("html", [
    '<meta name="csrf-token" content="TOK">',
    '<meta content="TOK" name="csrf-token">',
    "<meta name='csrf-token' content='TOK'>",
    '<META   CONTENT=TOK   NAME=csrf-token />',
    '<meta charset="utf-8" name="csrf-token" data-x="1" content="TOK">',
])
def test_csrf_meta_parsing_is_attribute_order_and_quote_agnostic(html):
    assert client._csrf_from_html(f"<head>{html}</head>") == "TOK"


def test_csrf_html_entities_unescaped():
    assert client._csrf_from_html('<meta name="csrf-token" content="a&#43;b&amp;c">') == "a+b&c"


def test_csrf_login_page_raises_session_error():
    h = router({p: (200, HTML, LOGIN) for p in ("/a/changes", "/a/tickets", "/a/dashboard")})
    with pytest.raises(SessionError):
        make_client(h)._get_csrf()


def test_csrf_redirect_to_other_host_raises_session_error():
    def handler(req):
        if req.url.host == "acme.freshservice.com":
            return httpx.Response(302, headers={"location": "https://auth.example/login"})
        return httpx.Response(200, headers=HTML, content=LOGIN)

    c = make_client(handler)
    with pytest.raises(SessionError):
        c._get_csrf()


def test_csrf_401_raises_session_error():
    h = router({p: (401, HTML, "") for p in ("/a/changes", "/a/tickets", "/a/dashboard")})
    with pytest.raises(SessionError):
        make_client(h)._get_csrf()


def test_csrf_server_error_is_api_error():
    h = router({p: (503, HTML, "") for p in ("/a/changes", "/a/tickets", "/a/dashboard")})
    c = make_client(h)
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(h), follow_redirects=False)
    import fsv.client as cl
    orig = cl._MAX_RETRIES
    cl._MAX_RETRIES = 0
    try:
        with pytest.raises(APIError):
            c._get_csrf()
    finally:
        cl._MAX_RETRIES = orig


def test_csrf_found_on_later_page():
    h = router({"/a/changes": (404, HTML, ""), "/a/tickets": (200, HTML, APP_OK)})
    assert make_client(h)._get_csrf() == "TOK123"


def test_csrf_missing_without_login_markers_is_api_error():
    h = router({p: (200, HTML, "<html>hello</html>") for p in ("/a/changes", "/a/tickets", "/a/dashboard")})
    with pytest.raises(APIError):
        make_client(h)._get_csrf()


# ---- cookies ----------------------------------------------------------

def test_cookies_are_secure_and_host_scoped():
    c = client.Client(cookies=COOKIES)
    cookies = list(c._client.cookies.jar)
    assert len(cookies) == 4
    assert all(ck.secure and ck.domain == "acme.freshservice.com" for ck in cookies)


def _dup(c):
    jar = c._client.cookies.jar
    jar.set_cookie(http.cookiejar.Cookie(
        0, "fw-session-id", "rotated", None, False, ".freshservice.com", True, True, "/", True, True,
        None, True, None, None, {}, False))


def test_duplicate_fw_session_cookie_does_not_raise_cookie_conflict():
    h = router({"/api/v2/x": (200, JSON, "{}")})
    c = make_client(h)
    _dup(c)
    with pytest.raises(httpx.CookieConflict):
        c._client.cookies.get("fw-session-id")  # the bug being avoided
    assert c._fw_cookie() == "tok"  # exact-host cookie wins
    assert c.v2_get("x") == {}


def test_rotated_fw_session_id_is_persisted_when_enabled(monkeypatch):
    saved = []
    monkeypatch.setattr(client, "update_cookie", lambda k, v: saved.append((k, v)))
    c = make_client(router({"/api/v2/x": (200, JSON, "{}")}), persist=True)
    c._client.cookies.jar.clear()
    c._client.cookies.jar.set_cookie(client._make_cookie("fw-session-id", "new", "acme.freshservice.com"))
    c.v2_get("x")
    assert saved == [("fw-session-id", "new")]


# ---- pagination -------------------------------------------------------

def _pages(n_pages, always_next=True):
    def handler(req):
        page = int(req.url.params["page"])
        link = {"link": '<https://x/?page=%d>; rel="next"' % (page + 1)} if (always_next or page < n_pages) else {}
        return httpx.Response(200, headers={**JSON, **link}, content="[]")
    return handler


def test_pagination_warns_when_max_pages_hit_with_more_remaining(capsys):
    pages = list(make_client(_pages(99)).v2_get_paginated("changes", max_pages=3))
    assert len(pages) == 3
    assert "stopped after 3 pages" in capsys.readouterr().err


def test_pagination_no_warning_when_exhausted(capsys):
    pages = list(make_client(_pages(2, always_next=False)).v2_get_paginated("changes", max_pages=5))
    assert len(pages) == 2
    assert "stopped after" not in capsys.readouterr().err


def test_pagination_shares_expiry_checks():
    for resp in ((302, {"location": "/login"}, ""), (200, HTML, LOGIN), (401, JSON, "{}")):
        c = make_client(router({"/api/v2/changes": resp}))
        with pytest.raises(SessionError):
            list(c.v2_get_paginated("changes"))


# ---- misc -------------------------------------------------------------

def test_lookup_choices_link_is_forced_relative():
    seen = []

    def handler(req):
        seen.append(str(req.url))
        return httpx.Response(200, headers=JSON, content='{"results": [{"id": 1}]}')

    c = make_client(handler)
    assert c.lookup_choices("https://evil.example/search/x?a=1", "q1") == [{"id": 1}]
    assert seen[0].startswith("https://acme.freshservice.com/search/x?")
    c.lookup_choices("search/y", "q2")
    assert seen[1].startswith("https://acme.freshservice.com/search/y")


def test_keepalive_false_on_error_status():
    c = make_client(router({"/api/v2/session": (500, JSON, "{}")}))
    c._fw_domain = "x.myfreshworks.com"
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(lambda r: httpx.Response(500)))
    assert c.keepalive() is False


def test_redact_is_substring_based():
    out = client._redact({"x_csrf_token": "1", "client_secret": "2", "passwd": "3", "Set-Cookie": "4",
                          "fw-session-id": "5", "subject": "ok", "nested": [{"api_key_token": "6"}]})
    assert out == {"x_csrf_token": "[redacted]", "client_secret": "[redacted]", "passwd": "[redacted]",
                   "Set-Cookie": "[redacted]", "fw-session-id": "[redacted]", "subject": "ok",
                   "nested": [{"api_key_token": "[redacted]"}]}


def test_data_previews_are_redacted():
    assert "hunter2" not in client._redact_data({"password": "hunter2", "a": 1})
    assert "hunter2" not in client._redact_data('{"password": "hunter2"}')
    form = client._redact_data("user=bob&password=hunter2")
    assert "hunter2" not in form and "user=bob" in form
    assert "hunter2" not in client._body_preview({"data": b"auth_token=hunter2"})
    assert client._redact_data("plain text") == "plain text"


def test_retry_after_garbage_is_ignored():
    assert client._retry_after_seconds(httpx.Headers({"retry-after": "inf"})) == 10.0
    assert client._retry_after_seconds(httpx.Headers({"retry-after": "soon"})) is None


# ---- auth login / status CLI -----------------------------------------

def _login(monkeypatch, check, extra=()):
    import fsv.cli as cli
    monkeypatch.setattr(client.Client, "check_session", check)
    monkeypatch.setattr(client, "reset_client", lambda: None)
    monkeypatch.setattr(client, "get_client", lambda: client.Client(cookies=COOKIES))
    monkeypatch.setattr(cli, "_refresh_cache", lambda **_: None)
    return CliRunner().invoke(cli.app, ["auth", "login", "--domain", "acme.freshservice.com", "--store", "file",
                                        "--header", "_x_m=a; _x_d=b; _x_w=c; fw-session-id=tok", *extra])


def _expired(self):
    raise SessionError("expired")


def test_login_refuses_invalid_cookies_without_force(monkeypatch):
    result = _login(monkeypatch, _expired)
    assert result.exit_code == 3
    flat = " ".join(result.output.split())
    assert "nothing saved" in flat and "--force" in flat
    assert not config.SESSION_FILE.exists()


def test_login_force_saves_invalid_cookies(monkeypatch):
    result = _login(monkeypatch, _expired, ["--force"])
    assert result.exit_code == 0, result.output
    assert config.SESSION_FILE.exists()


def test_login_unverifiable_is_refused_without_force(monkeypatch):
    def down(self):
        raise httpx.ConnectError("down")
    result = _login(monkeypatch, down)
    assert result.exit_code == 1 and not config.SESSION_FILE.exists()


def test_login_message_uses_actual_session_path(monkeypatch, tmp_path):
    result = _login(monkeypatch, lambda self: {"first_name": "Ada", "email": "ada@example.com"})
    assert result.exit_code == 0, result.output
    assert "~/.config/fsv/session.json" not in result.output
    assert "session.json" in "".join(result.output.split())


def test_auth_status_with_restricted_agent(monkeypatch):
    import fsv.cli as cli
    import fsv.session as session
    session.save_cookies(COOKIES, "file")

    class C:
        def check_session(self):
            return {}

        def rate_limit_remaining(self):
            return 10, 100

    monkeypatch.setattr(cli, "_client", lambda: C())
    result = CliRunner().invoke(cli.app, ["auth", "status"])
    assert result.exit_code == 0, result.output
    assert "identity unavailable (403)" in result.output and "session:     valid" in result.output


def test_auth_status_expired_session_shows_clear_error(monkeypatch):
    import fsv.cli as cli
    import fsv.session as session
    session.save_cookies(COOKIES, "file")

    class C:
        def check_session(self):
            raise SessionError("session expired or invalid; run `fsv auth login`")

    monkeypatch.setattr(cli, "_client", lambda: C())
    result = CliRunner().invoke(cli.app, ["auth", "status"])
    assert result.exit_code == 3
    assert "session expired or invalid" in result.output
