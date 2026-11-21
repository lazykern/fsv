import json

from netdiff import *  # noqa
from netsrv import Reset

STATUS = ["auth", "status", "-v"]
REPLY = ["tickets", "reply", "1", "hello", "--yes", "-v"]
HTML_HDR = [("Content-Type", "text/html; charset=utf-8")]


def table(routes, default=None):
    """routes: path -> action | callable(req, n) | list of actions (last repeats)."""
    def handler(req, n):
        r = routes.get(req.path, default)
        if r is None:
            return json_resp({"message": "no route"}, 404)
        if callable(r):
            return r(req, n)
        if isinstance(r, list):
            return r[min(n, len(r)) - 1]
        return r
    return handler


ME_OK = json_resp(ME)
CHANGES_OK = json_resp({"changes": []})
APP_OK = html_resp(APP_HTML)
STATUS_ROUTES = {"/api/v2/agents/me": ME_OK, "/api/v2/changes": CHANGES_OK}
REPLY_OK = json_resp({"conversation": {"id": 77}}, 201)


def reply_routes(**over):
    r = {"/a/changes": APP_OK, "/api/_/tickets/1/reply": REPLY_OK}
    r.update(over)
    return r


def session_after(home):
    d = json.loads((home / "config" / "session.json").read_text())
    return d["cookies"]


def status(name, routes, **kw):
    scenario(name, STATUS, table(routes), **kw)


def reply(name, routes, **kw):
    scenario(name, REPLY, table(routes), **kw)


# ---------- baseline and bodies ----------
status("status-ok", STATUS_ROUTES)


def body(mode, payload=None, ctype="application/json", headers=None):
    p = json.dumps(ME) if payload is None else payload
    return table({"/api/v2/agents/me": Resp(200, [("Content-Type", ctype)] + (headers or []), p, mode=mode),
                  "/api/v2/changes": CHANGES_OK})


for mode in ("gzip", "chunked", "close", "truncate", "badchunk"):
    scenario(f"body-{mode}", STATUS, body(mode))
scenario("body-empty-json", STATUS, body("cl", ""))
scenario("body-whitespace-json", STATUS, body("cl", " \n"))
scenario("body-nonjson-200", STATUS, body("cl", "not json"))
scenario("body-large-1mb", STATUS, body("chunked", json.dumps({"agent": {"first_name": "x" * 50000}})))
scenario("body-html-200", STATUS, body("cl", "<html>login</html>", ctype="text/html"))
scenario("body-html-upper-ct", STATUS, body("cl", "<html>login</html>", ctype="TEXT/HTML"))
scenario("body-json-upper-ct", STATUS, body("cl", json.dumps(ME), ctype="Application/JSON"))
scenario("body-json-bom", STATUS, body("cl", b"\xef\xbb\xbf" + json.dumps(ME).encode()))
scenario("body-json-nan", STATUS, body("cl", '{"agent": {"first_name": NaN}}'))
scenario("body-json-utf16", STATUS, body("cl", json.dumps(ME).encode("utf-16")))
scenario("body-json-latin1-charset", STATUS, body("cl", json.dumps({"agent": {"first_name": "café"}}, ensure_ascii=False).encode("latin-1"), ctype="application/json; charset=latin-1"))
scenario("body-json-bad-utf8", STATUS, body("cl", b'{"agent": {"first_name": "\xff\xfe"}}'))
scenario("body-gzip-bad", STATUS, table({"/api/v2/agents/me": Resp(200, [("Content-Type", "application/json"), ("Content-Encoding", "gzip")], b"not gzip at all")}))
scenario("body-deflate", STATUS, table({"/api/v2/agents/me": Resp(200, [("Content-Type", "application/json"), ("Content-Encoding", "deflate")], __import__("zlib").compress(json.dumps(ME).encode())), "/api/v2/changes": CHANGES_OK}))

# ---------- error bodies ----------
for code in (400, 404, 409, 422, 500):
    status(f"err-{code}-json", {"/api/v2/agents/me": json_resp({"description": "bad", "errors": [{"field": "x", "message": "m"}]}, code)}, )
status("err-422-message", {"/api/v2/agents/me": json_resp({"message": "nope"}, 422)})
status("err-400-nonjson", {"/api/v2/agents/me": Resp(400, [("Content-Type", "text/plain")], "plain text error")})
status("err-400-json-as-text", {"/api/v2/agents/me": Resp(400, [("Content-Type", "text/plain")], '{"message": "inner"}')})
status("err-400-long-body", {"/api/v2/agents/me": Resp(400, [("Content-Type", "text/plain")], "é" * 900)})
status("err-400-empty", {"/api/v2/agents/me": Resp(400, [], "")})
status("err-400-html", {"/api/v2/agents/me": html_resp("<html><body>Bad <b>request</b></body></html>", 400)})
status("err-400-bad-utf8", {"/api/v2/agents/me": Resp(400, [("Content-Type", "text/plain")], b"abc\xff\xfedef")})
status("err-400-latin1", {"/api/v2/agents/me": Resp(400, [("Content-Type", "text/plain; charset=latin-1")], "café ok".encode("latin-1"))})
status("err-418", {"/api/v2/agents/me": json_resp({"message": "teapot"}, 418)})
status("err-499-no-reason", {"/api/v2/agents/me": Resp(499, [("Content-Type", "application/json")], '{"message":"x"}', reason="")})

# ---------- retry / backoff ----------
status("retry-429-then-ok", {"/api/v2/agents/me": [json_resp({}, 429, [("Retry-After", "1")]), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-503-always", {"/api/v2/agents/me": json_resp({"message": "down"}, 503)})
status("retry-502-then-ok", {"/api/v2/agents/me": [json_resp({}, 502), json_resp({}, 500), ME_OK], "/api/v2/changes": CHANGES_OK})
for ra in ("0", "abc", "-3", "nan", "1e0", "0.3", " 2 ", "", "1.5"):
    status(f"retry-after-{ra or 'empty'}", {"/api/v2/agents/me": [json_resp({}, 503, [("Retry-After", ra)]), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-after-httpdate", {"/api/v2/agents/me": [json_resp({}, 503, [("Retry-After", "Wed, 21 Oct 2037 07:28:00 GMT")]), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-after-big", {"/api/v2/agents/me": [json_resp({}, 429, [("Retry-After", "12")]), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-504-twice-then-ok", {"/api/v2/agents/me": [json_resp({}, 504), json_resp({}, 504, [("Retry-After", "0")]), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-drop-then-ok", {"/api/v2/agents/me": [Drop(), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-drop-always", {"/api/v2/agents/me": Drop()})
status("retry-garbage-response", {"/api/v2/agents/me": Raw(b"GARBAGE\r\n\r\n")})
status("retry-truncate-then-ok", {"/api/v2/agents/me": [Resp(200, [("Content-Type", "application/json")], json.dumps(ME), mode="truncate"), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-408", {"/api/v2/agents/me": json_resp({}, 408)})
status("retry-501", {"/api/v2/agents/me": json_resp({}, 501)})
status("ratelimit-headers", {"/api/v2/agents/me": Resp(200, [("Content-Type", "application/json"), ("X-RateLimit-Remaining", "5"), ("X-RateLimit-Total", "100")], json.dumps(ME)), "/api/v2/changes": Resp(200, [("Content-Type", "application/json"), ("X-RateLimit-Remaining", "4"), ("X-RateLimit-Total", "100")], "{}")})
status("ratelimit-bad", {"/api/v2/agents/me": Resp(200, [("Content-Type", "application/json"), ("X-RateLimit-Remaining", "x"), ("X-RateLimit-Total", "100")], json.dumps(ME)), "/api/v2/changes": CHANGES_OK})

status("retry-reset-then-ok", {"/api/v2/agents/me": [Reset(), ME_OK], "/api/v2/changes": CHANGES_OK})
status("retry-reset-always", {"/api/v2/agents/me": Reset()})
status("retry-early-eof-headers", {"/api/v2/agents/me": Raw(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Len")})
status("retry-http10-response", {"/api/v2/agents/me": Raw(b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n" + json.dumps(ME).encode()), "/api/v2/changes": CHANGES_OK})
status("resp-duplicate-content-length", {"/api/v2/agents/me": Raw(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n{}....")})
status("resp-header-nonascii", {"/api/v2/agents/me": Resp(200, [("Content-Type", "application/json"), ("X-Note", "caf\u00e9")], json.dumps(ME)), "/api/v2/changes": CHANGES_OK})

# ---------- no replay of writes ----------
reply("write-ok", reply_routes())
reply("write-503", reply_routes(**{"/api/_/tickets/1/reply": json_resp({"message": "down"}, 503)}))
reply("write-429-ra", reply_routes(**{"/api/_/tickets/1/reply": json_resp({}, 429, [("Retry-After", "1")])}))
reply("write-reset", reply_routes(**{"/api/_/tickets/1/reply": Reset()}))
reply("write-drop", reply_routes(**{"/api/_/tickets/1/reply": Drop()}))
reply("write-truncate", reply_routes(**{"/api/_/tickets/1/reply": Resp(200, [("Content-Type", "application/json")], "{}", mode="truncate")}))
reply("write-403-session-live", reply_routes(**{"/api/_/tickets/1/reply": json_resp({"message": "no"}, 403), "/a/dashboard": APP_OK}))
reply("write-403-session-dead", reply_routes(**{"/api/_/tickets/1/reply": json_resp({"message": "no"}, 403), "/a/dashboard": redirect(302, "https://idp.myfreshworks.com/login")}))
reply("write-401", reply_routes(**{"/api/_/tickets/1/reply": json_resp({}, 401)}))
reply("write-302", reply_routes(**{"/api/_/tickets/1/reply": redirect(302, "/login")}))
reply("write-empty-200", reply_routes(**{"/api/_/tickets/1/reply": Resp(200, [("Content-Type", "application/json")], "")}))
reply("write-text-200", reply_routes(**{"/api/_/tickets/1/reply": Resp(200, [("Content-Type", "text/plain")], "ok")}))
scenario("write-large-body", ["tickets", "reply", "1", "x" * 400000, "--yes", "-v"], table(reply_routes()))
scenario("write-unicode-body", ["tickets", "reply", "1", "héllo ☃ \U0001F600 \"q\" <b>", "--yes", "-v"], table(reply_routes()))
scenario("write-secret-redaction", ["tickets", "reply", "1", "token=abc password=hunter2", "--yes", "-v"], table(reply_routes()))

# ---------- CSRF fetch ----------
reply("csrf-second-page", reply_routes(**{"/a/changes": html_resp("<html></html>", 403), "/a/tickets": APP_OK}))
reply("csrf-third-page", reply_routes(**{"/a/changes": html_resp("x", 404), "/a/tickets": html_resp("x", 404), "/a/dashboard": APP_OK}))
reply("csrf-no-meta", reply_routes(**{"/a/changes": html_resp("<html>hi</html>"), "/a/tickets": html_resp("<p>x</p>"), "/a/dashboard": html_resp("<p>y</p>")}))
reply("csrf-login-html", reply_routes(**{"/a/changes": html_resp('<form><input type="password"></form>')}))
reply("csrf-401", reply_routes(**{"/a/changes": html_resp("x", 401)}))
reply("csrf-all-500", reply_routes(**{"/a/changes": html_resp("x", 500), "/a/tickets": html_resp("x", 500), "/a/dashboard": html_resp("x", 500)}))
reply("csrf-all-403", reply_routes(**{"/a/changes": html_resp("x", 403), "/a/tickets": html_resp("x", 403), "/a/dashboard": html_resp("x", 403)}))
reply("csrf-fw-domain", reply_routes(**{"/a/changes": html_resp('<meta content="tok&amp;1" name="csrf-token"><a href="https://acme.myfreshworks.com/x">')}))
reply("csrf-single-quote-attrs", reply_routes(**{"/a/changes": html_resp("<META NAME='csrf-token' CONTENT='abc'>")}))
reply("csrf-redirect-same-host", reply_routes(**{"/a/changes": redirect(302, "/a/changes2"), "/a/changes2": APP_OK}))
reply("csrf-redirect-relative-dotdot", reply_routes(**{"/a/changes": redirect(301, "../x/./y?z=1#frag"), "/x/y": APP_OK}))
reply("csrf-redirect-cross-host", reply_routes(**{"/a/changes": redirect(302, "https://other.example.com/a/changes")}), )
reply("csrf-redirect-idp", reply_routes(**{"/a/changes": redirect(302, "https://idp.myfreshworks.com/login")}))
reply("csrf-redirect-to-http", reply_routes(**{"/a/changes": redirect(302, "http://acme.freshservice.com/a/changes3"), "/a/changes3": APP_OK}))
reply("csrf-redirect-scheme-relative", reply_routes(**{"/a/changes": redirect(307, "//other.example.com/page")}))
reply("csrf-redirect-subdomain", reply_routes(**{"/a/changes": redirect(302, "https://sub.acme.freshservice.com/a/changes4"), "/a/changes4": APP_OK}))
reply("csrf-redirect-loop", reply_routes(**{"/a/changes": redirect(302, "/a/changes")}))
reply("csrf-redirect-no-location", reply_routes(**{"/a/changes": Resp(302, [], "", reason="Found")}))
reply("csrf-redirect-empty-location", reply_routes(**{"/a/changes": redirect(302, "")}))
reply("csrf-redirect-ftp", reply_routes(**{"/a/changes": redirect(302, "ftp://acme.freshservice.com/x")}))
reply("csrf-redirect-303-308", reply_routes(**{"/a/changes": redirect(303, "/a/changes5"), "/a/changes5": redirect(308, "/a/changes6"), "/a/changes6": APP_OK}))
reply("csrf-redirect-300-304", reply_routes(**{"/a/changes": Resp(300, [("Location", "/a/changes7")], ""), "/a/tickets": Resp(304, [], ""), "/a/dashboard": APP_OK}))
reply("csrf-redirect-cookies",
      reply_routes(**{"/a/changes": redirect(302, "/a/changes8", [("Set-Cookie", "hop=1; Path=/"), ("Set-Cookie", "fw-session-id=hopfw; Path=/")]),
                      "/a/changes8": APP_OK}))
reply("csrf-redirect-cross-cookie",
      reply_routes(**{"/a/changes": redirect(302, "https://other.example.com/a/changes", [("Set-Cookie", "hop=1; Path=/")])}))
reply("csrf-https-to-http-cross", reply_routes(**{"/a/changes": redirect(302, "http://other.example.com/a/x")}))
reply("csrf-redirect-port", reply_routes(**{"/a/changes": redirect(302, "https://acme.freshservice.com:8443/a/x")}))

# ---------- API-level redirects and sessions ----------
for code in (301, 302, 303, 307, 308):
    status(f"api-redirect-{code}", {"/api/v2/agents/me": redirect(code, "https://other.example.com/login")})
status("api-redirect-http", {"/api/v2/agents/me": redirect(302, "http://acme.freshservice.com/login")})
status("api-304", {"/api/v2/agents/me": Resp(304, [], "")})
status("api-401", {"/api/v2/agents/me": json_resp({}, 401)})
status("api-403-live", {"/api/v2/agents/me": json_resp({"message": "forbidden"}, 403), "/a/dashboard": APP_OK, "/api/v2/changes": CHANGES_OK})
status("api-403-dead-redirect", {"/api/v2/agents/me": json_resp({}, 403), "/a/dashboard": redirect(302, "/")})
status("api-403-dead-401", {"/api/v2/agents/me": json_resp({}, 403), "/a/dashboard": html_resp("x", 401)})
status("api-403-dead-403", {"/api/v2/agents/me": json_resp({}, 403), "/a/dashboard": html_resp("x", 403)})
status("api-403-dead-login-html", {"/api/v2/agents/me": json_resp({}, 403), "/a/dashboard": html_resp('<input type="password">')})
status("api-403-probe-500", {"/api/v2/agents/me": json_resp({"message": "f"}, 403), "/a/dashboard": html_resp("x", 500)})
status("api-403-probe-drop", {"/api/v2/agents/me": json_resp({"message": "f"}, 403), "/a/dashboard": Drop()})
status("api-403-probe-html-nometa", {"/api/v2/agents/me": json_resp({"message": "f"}, 403), "/a/dashboard": html_resp("<p>plain page</p>")})
status("api-403-probe-404", {"/api/v2/agents/me": json_resp({"message": "f"}, 403), "/a/dashboard": html_resp("x", 404)})
status("api-403-probe-ttl", {"/api/v2/agents/me": json_resp({"message": "f"}, 403), "/a/dashboard": APP_OK, "/api/v2/changes": json_resp({"m": "f"}, 403)})

# ---------- cookies ----------
def cookie_status(name, set_cookies, **kw):
    status(name, {"/api/v2/agents/me": Resp(200, [("Content-Type", "application/json")] + [("Set-Cookie", c) for c in set_cookies], json.dumps(ME)),
                  "/api/v2/changes": CHANGES_OK}, after=session_after, **kw)


cookie_status("cookie-fw-rotate", ["fw-session-id=newfw; Path=/; HttpOnly; Secure"])
cookie_status("cookie-fw-rotate-quoted", ['fw-session-id="quoted val"; Path=/'])
cookie_status("cookie-new-and-multiple", ["a=1; Path=/", "b=2; Path=/api", "c=3; Path=/other", "d=4; Domain=acme.freshservice.com", "e=5; Domain=.freshservice.com", "f=6; Domain=other.com", "g=7; Secure; HttpOnly; SameSite=Lax"])
cookie_status("cookie-delete-max-age", ["_x_m=gone; Max-Age=0; Path=/", "z=1; Max-Age=60; Path=/"])
cookie_status("cookie-delete-expires", ["_x_d=gone; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Path=/"])
cookie_status("cookie-expires-future", ["k=v; Expires=Wed, 21 Oct 2037 07:28:00 GMT; Path=/"])
cookie_status("cookie-path-narrow-replace", ["_x_w=narrow; Path=/api"])
cookie_status("cookie-replace-same", ["_x_m=m2; Path=/"])
cookie_status("cookie-empty-value", ["emp=; Path=/", "=novalue", "bare", "sp = x ; Path=/"])
cookie_status("cookie-domain-parent-replace", ["_x_m=parentval; Domain=freshservice.com; Path=/", "fw-session-id=dotted; Domain=.acme.freshservice.com; Path=/"])
cookie_status("cookie-case-names", ["A=1; path=/; secure", "a=2; PATH=/"])
cookie_status("cookie-comma-value", ["x=1,2; Path=/", "y=a b; Path=/"])
cookie_status("cookie-quotes-version", ['q="v1"; Version=1; Path=/', 'w=\\"esc\\"; Path=/'])
cookie_status("cookie-nonascii", ["u=café; Path=/"])
cookie_status("cookie-secure-http-redirect-n/a", ["s=1; Secure; Path=/"])
status("cookie-on-error-response", {"/api/v2/agents/me": Resp(500, [("Content-Type", "application/json"), ("Set-Cookie", "e=1; Path=/"), ("Set-Cookie", "fw-session-id=errfw; Path=/")], "{}")}, after=session_after)
status("cookie-on-403", {"/api/v2/agents/me": Resp(403, [("Content-Type", "application/json"), ("Set-Cookie", "fw-session-id=f403; Path=/")], "{}"), "/a/dashboard": APP_OK}, after=session_after)

# ---------- verbose / misc ----------
status("verbose-ok-ratelimit", {"/api/v2/agents/me": Resp(200, [("Content-Type", "application/json"), ("X-RateLimit-Remaining", "9"), ("X-RateLimit-Total", "50")], json.dumps(ME)), "/api/v2/changes": CHANGES_OK})
scenario("no-session", ["auth", "status"], table({}), session=False)
scenario("session-missing-cookie", ["auth", "status"], table(STATUS_ROUTES), cookies={"_x_m": "m1"})
scenario("list-pagination-link", ["changes", "ls", "--all"], table({"/api/v2/changes": [Resp(200, [("Content-Type", "application/json"), ("Link", '<https://acme.freshservice.com/api/v2/changes?page=2>; rel="next"')], json.dumps({"changes": []})), json_resp({"changes": []})]}))

# ---------- transport failures (slow: 30 s per phase) ----------
scenario("connect-proxy-refuse", STATUS, table(STATUS_ROUTES), connect_mode="refuse")
scenario("connect-proxy-close", STATUS, table(STATUS_ROUTES), connect_mode="close")
scenario("connect-no-proxy-tls-untrusted", STATUS, table(STATUS_ROUTES), env_extra={"SSL_CERT_FILE": "/nonexistent-ca.pem"})
scenario("timeout-get-slow", STATUS, table({"/api/v2/agents/me": Hang(60)}), slow=True)
scenario("timeout-write-slow", REPLY, table(reply_routes(**{"/api/_/tickets/1/reply": Hang(60)})), slow=True)
scenario("timeout-connect-slow", STATUS, table(STATUS_ROUTES), connect_mode="hang", slow=True)

MASKED = {"body-nonjson-200", "body-json-nan", "body-json-latin1-charset", "body-json-bad-utf8", "body-gzip-bad",
          "retry-garbage-response", "resp-duplicate-content-length", "connect-proxy-refuse"}
for _sc in SCENARIOS:
    if _sc["name"] in MASKED:
        _sc["mask"] = True
