from __future__ import annotations

import atexit
import html as _html
import http.cookiejar
import json
import re
import time
from typing import Any, Iterator
from urllib.parse import parse_qsl, quote, urlencode, urlsplit

import httpx

from fsv import config
from fsv.errors import APIError
from fsv.render import err
from fsv.session import SessionError, load_cookies, update_cookie

UA = "fsv/0.1 (Freshservice CLI)"


_instance: "Client | None" = None
_VERBOSE = False
# A key is redacted when it *contains* any of these (case-insensitive,
# "_" and "-" ignored), so "x_csrf_token", "client_secret", "fw-session-id",
# "passwd" and "Set-Cookie" are all covered.
_REDACT_SUBSTRINGS = ("token", "secret", "pass", "csrf", "cookie", "session",
                      "authorization", "apikey", "credential")
_RETRYABLE_STATUS = {429, 500, 502, 503, 504}
_IDEMPOTENT_METHODS = {"GET", "HEAD", "OPTIONS"}
_MAX_RETRIES = 2
_RETRY_DELAYS = (0.5, 1.5)


def set_verbose(value: bool) -> None:
    global _VERBOSE
    _VERBOSE = bool(value)


def is_verbose() -> bool:
    return _VERBOSE


def _is_sensitive_key(key: Any) -> bool:
    k = str(key).strip().lower().replace("_", "").replace("-", "")
    return any(sub in k for sub in _REDACT_SUBSTRINGS)


def _redact(value: Any) -> Any:
    if isinstance(value, dict):
        out: dict[Any, Any] = {}
        for k, v in value.items():
            out[k] = "[redacted]" if _is_sensitive_key(k) else _redact(v)
        return out
    if isinstance(value, list):
        return [_redact(v) for v in value]
    if isinstance(value, tuple):
        return tuple(_redact(v) for v in value)
    return value


def _query_string(url: str, params: dict[str, Any] | None) -> str:
    parts = list(parse_qsl(urlsplit(url).query, keep_blank_values=True))
    for key, value in (params or {}).items():
        if value is None:
            continue
        if isinstance(value, (list, tuple)):
            for item in value:
                parts.append((str(key), "" if item is None else str(item)))
        else:
            parts.append((str(key), str(value)))
    if not parts:
        return ""
    return "&".join(
        f"{quote(key, safe='-._~')}={quote(value, safe='-._~,/:@')}"
        for key, value in parts
    )


def _request_target(url: str, params: dict[str, Any] | None) -> str:
    parsed = urlsplit(url)
    path = parsed.path or "/"
    query = _query_string(url, params)
    return f"{path}?{query}" if query else path


def _redact_data(data: Any) -> str:
    """Redacted preview of a ``data=`` payload (dict, JSON text or form-encoded)."""
    if isinstance(data, dict):
        return json.dumps(_redact(data), ensure_ascii=False, separators=(",", ":"), default=str)
    text = data.decode("utf-8", errors="replace") if isinstance(data, (bytes, bytearray)) else str(data)
    try:
        parsed = json.loads(text)
    except ValueError:
        parsed = None
    if isinstance(parsed, (dict, list)):
        return json.dumps(_redact(parsed), ensure_ascii=False, separators=(",", ":"), default=str)
    pairs = parse_qsl(text, keep_blank_values=True)
    if pairs and "=" in text:
        return urlencode([(k, "[redacted]" if _is_sensitive_key(k) else v) for k, v in pairs])
    if _is_sensitive_key(text):
        return "[redacted]"
    return text


def _body_preview(kw: dict[str, Any]) -> str:
    raw = ""
    if kw.get("json") is not None:
        raw = json.dumps(_redact(kw["json"]), ensure_ascii=False, separators=(",", ":"), default=str)
    elif kw.get("data") is not None:
        raw = _redact_data(kw["data"])
    if not raw:
        return ""
    if len(raw) > 300:
        raw = raw[:297] + "..."
    return f" body={raw}"


def _log_line(method: str, url: str, kw: dict[str, Any], outcome: str) -> None:
    if not _VERBOSE:
        return
    body = _body_preview(kw) if method.upper() not in {"GET", "HEAD"} else ""
    err.print(f"[api] {method.upper()} {_request_target(url, kw.get('params'))}{body} -> {outcome}", markup=False, highlight=False)


def _retry_after_seconds(headers: httpx.Headers) -> float | None:
    raw = headers.get("retry-after")
    if not raw:
        return None
    try:
        return max(0.0, min(float(raw), 10.0))
    except (ValueError, OverflowError):
        return None


def _retry_delay(attempt: int, response: httpx.Response | None = None) -> float:
    if response is not None:
        retry_after = _retry_after_seconds(response.headers)
        if retry_after is not None:
            return retry_after
    return _RETRY_DELAYS[min(attempt, len(_RETRY_DELAYS) - 1)]


def _should_retry(method: str, response: httpx.Response | None = None, exc: BaseException | None = None) -> bool:
    if method.upper() not in _IDEMPOTENT_METHODS:
        return False
    if exc is not None:
        return isinstance(exc, httpx.TransportError)
    return response is not None and response.status_code in _RETRYABLE_STATUS


def _response_body(r: httpx.Response) -> object:
    try:
        return r.json()
    except Exception:
        return r.text[:500]


_EXPIRED_MSG = "session expired or invalid; run `fsv auth login`"
_PROBE_PATH = "/a/dashboard"
_PROBE_TTL = 60.0
_META_RE = re.compile(r"<meta\b[^>]*>", re.IGNORECASE)
_ATTR_RE = re.compile(r"""([^\s=/>"']+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))""")


def _csrf_from_html(html: str) -> str | None:
    """Extract <meta name="csrf-token" content="..."> regardless of attribute
    order, quote style or case."""
    for tag in _META_RE.findall(html):
        attrs = {m.group(1).lower(): _html.unescape(next(g for g in m.groups()[1:] if g is not None))
                 for m in _ATTR_RE.finditer(tag)}
        if attrs.get("name", "").lower() == "csrf-token" and attrs.get("content"):
            return attrs["content"]
    return None


def _looks_like_login_html(html: str) -> bool:
    """HTML with no CSRF meta that shows a password field or Freshid markers."""
    if _csrf_from_html(html):
        return False
    low = html.lower()
    return 'type="password"' in low or "type='password'" in low or "freshid" in low or "/login" in low


def _make_cookie(name: str, value: str, domain: str) -> "http.cookiejar.Cookie":
    """Host-scoped, Secure cookie (httpx's Cookies.set has no ``secure`` arg)."""
    return http.cookiejar.Cookie(
        version=0, name=name, value=value, port=None, port_specified=False,
        domain=domain, domain_specified=True, domain_initial_dot=False,
        path="/", path_specified=True, secure=True, expires=None, discard=True,
        comment=None, comment_url=None, rest={}, rfc2109=False,
    )


def get_client() -> "Client":
    global _instance
    if _instance is None:
        _instance = Client()
        atexit.register(_instance.close)
    return _instance


def reset_client() -> None:
    """Close and discard the singleton (e.g. after re-login)."""
    global _instance
    if _instance is not None:
        _instance.close()
        _instance = None


class Client:
    def __init__(self, cookies: dict[str, str] | None = None) -> None:
        """*cookies* overrides the stored session (used to validate fresh
        cookies before they are saved)."""
        config.require_domain()
        self._csrf: str | None = None
        self._fw_domain: str | None = None
        self._rl_rem: int | None = None
        self._rl_tot: int | None = None
        self._probe_ok_until = 0.0
        self._persist = cookies is None
        if cookies is None:
            cookies = load_cookies()
        self._fw_session_id: str | None = cookies.get("fw-session-id")
        jar = httpx.Cookies()
        for k, v in cookies.items():
            jar.jar.set_cookie(_make_cookie(k, v, config.DOMAIN))
        self._client = httpx.Client(
            cookies=jar,
            timeout=30.0,
            follow_redirects=False,
            headers={"User-Agent": UA, "Accept": "application/json"},
        )

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def _send(self, method: str, url: str, **kw: Any) -> httpx.Response:
        last_exc: BaseException | None = None
        for attempt in range(_MAX_RETRIES + 1):
            t0 = time.time()
            try:
                r = self._client.request(method, url, **kw)
            except Exception as e:
                last_exc = e
                if attempt < _MAX_RETRIES and _should_retry(method, exc=e):
                    delay = _retry_delay(attempt)
                    _log_line(method, url, kw, f"{type(e).__name__}: {e}; retry in {delay:g}s")
                    time.sleep(delay)
                    continue
                _log_line(method, url, kw, f"{type(e).__name__}: {e}")
                raise

            rem = r.headers.get("x-ratelimit-remaining")
            tot = r.headers.get("x-ratelimit-total")
            try:
                if rem:
                    self._rl_rem = int(rem)
                if tot:
                    self._rl_tot = int(tot)
            except ValueError:
                pass
            rl = f" rl={rem}/{tot}" if rem and tot else ""
            elapsed = f"{(time.time() - t0) * 1000:.0f}ms"
            if attempt < _MAX_RETRIES and _should_retry(method, response=r):
                delay = _retry_delay(attempt, r)
                _log_line(method, url, kw, f"{r.status_code} {elapsed}{rl}; retry in {delay:g}s")
                time.sleep(delay)
                continue
            _log_line(method, url, kw, f"{r.status_code} {elapsed}{rl}")
            return r
        raise last_exc or RuntimeError("request failed without response")

    def _fw_cookie(self) -> str | None:
        """Current fw-session-id, tolerant of duplicate cookies (host vs dotted
        domain) which make ``cookies.get`` raise CookieConflict."""
        jar = getattr(self._client.cookies, "jar", None)
        if jar is None:  # plain mapping (tests / alternative transports)
            return self._client.cookies.get("fw-session-id")
        found = [c for c in jar if c.name == "fw-session-id"]
        if not found:
            return None
        exact = [c for c in found if (c.domain or "").lstrip(".") == config.DOMAIN]
        return (exact or found)[-1].value

    def _probe_session(self) -> bool | None:
        """Decide whether the session is still valid after a 403.

        Heuristic: a genuine permission 403 (e.g. /agents/me for a restricted
        agent) leaves the session usable, while an expired session gets 403
        from API endpoints too.  So fetch the cheap HTML app shell
        (``/a/dashboard``, reachable by any logged-in agent) without following
        redirects: 200 with the CSRF meta tag => valid; 3xx/401/403 or login
        markup => expired.  Returns None when the probe itself fails (network
        error / 5xx) so callers keep the original 403.  A positive result is
        remembered briefly to avoid probing on every 403.
        """
        if time.time() < self._probe_ok_until:
            return True
        try:
            r = self._send("GET", f"https://{config.DOMAIN}{_PROBE_PATH}",
                           headers={"Accept": "text/html,application/xhtml+xml"},
                           follow_redirects=False)
        except Exception:
            return None
        if 300 <= r.status_code < 400 or r.status_code in (401, 403):
            return False
        if r.status_code != 200:
            return None
        if _looks_like_login_html(r.text):
            return False
        self._probe_ok_until = time.time() + _PROBE_TTL
        return True

    def _check_response(self, r: httpx.Response, method: str, url: str, params: dict[str, Any] | None,
                        expect_json: bool = True) -> None:
        """Shared status handling for _request and pagination.

        Raises SessionError for: any 3xx (except 304), 401, HTML from the
        tenant, and a 403 when the session probe says the session is dead.
        Raises APIError for other >= 400.
        """
        if 300 <= r.status_code < 400 and r.status_code != 304:
            raise SessionError(_EXPIRED_MSG)
        if r.status_code == 401:
            raise SessionError(_EXPIRED_MSG)
        ct = r.headers.get("content-type", "")
        if r.status_code < 400 and "text/html" in ct and expect_json:
            raise SessionError(_EXPIRED_MSG)
        if r.status_code == 403 and self._probe_session() is False:
            raise SessionError(_EXPIRED_MSG)
        if r.status_code >= 400:
            raise APIError(r.status_code, _response_body(r), method, _request_target(url, params))

    def _request(self, method: str, url: str, **kw: Any) -> Any:
        if method.upper() in ("GET", "HEAD"):
            return self._request_inner(method, url, **kw)
        from fsv import journal
        self._last_status = None
        try:
            result = self._request_inner(method, url, **kw)
        except BaseException as e:
            journal.record(method, url, kw.get("json"), ok=False,
                           status=getattr(e, "status", None) or self._last_status,
                           error=type(e).__name__)
            raise
        journal.record(method, url, kw.get("json"), ok=True, status=self._last_status, response=result)
        return result

    _last_status: int | None = None

    def _request_inner(self, method: str, url: str, **kw: Any) -> Any:
        r = self._send(method, url, **kw)
        self._last_status = r.status_code
        self._check_response(r, method, url, kw.get("params"))
        fw = self._fw_cookie()
        if fw and fw != self._fw_session_id:
            self._fw_session_id = fw
            if self._persist:
                update_cookie("fw-session-id", fw)
        ct = r.headers.get("content-type", "")
        # Successful unlink/update endpoints may label a bodyless 200/204 as
        # JSON. Leave independent readback to establish the remote outcome.
        if not r.content.strip():
            return None
        return r.json() if ct.startswith("application/json") else r.text

    def v2_get(self, path: str, params: dict[str, Any] | None = None) -> Any:
        return self._request("GET", f"{config.API_V2}/{path.lstrip('/')}", params=params)

    def v2_post(self, path: str, body: dict[str, Any]) -> Any:
        return self._request("POST", f"{config.API_V2}/{path.lstrip('/')}", json=body,
                             headers={"Content-Type": "application/json"})

    def v2_put(self, path: str, body: dict[str, Any]) -> Any:
        return self._request("PUT", f"{config.API_V2}/{path.lstrip('/')}", json=body,
                             headers={"Content-Type": "application/json"})

    def int_get(self, path: str, params: dict[str, Any] | None = None) -> Any:
        return self._request("GET", f"{config.API_INT}/{path.lstrip('/')}", params=params)

    def fulltext_search(
        self,
        entity: str,
        term: str,
        page: int = 1,
        sort: str | None = None,
    ) -> dict[str, Any]:
        params: dict[str, Any] = {"term": term, "page": page}
        if sort == "created":
            params["search_sort"] = "created_at"
        elif sort == "modified":
            params["search_sort"] = "updated_at"
        elif sort == "relevance":
            params["search_sort"] = "relevance"
        return self._request("GET", f"https://{config.DOMAIN}/search/{entity}", params=params)

    def autocomplete(self, kind: str, query: str, params: dict[str, Any] | None = None) -> list[dict]:
        p = {"q": query, "qf": "value", **(params or {})}
        return self._request("GET", f"https://{config.DOMAIN}/search/autocomplete/{kind}", params=p).get("results", [])

    def lookup_choices(self, link: str, query: str) -> list[dict]:
        from hashlib import md5
        from fsv.cache import load as _cache_load, save as _cache_save
        cache_dir = config.CACHE_DIR / "lookup"
        cache_dir.mkdir(parents=True, exist_ok=True)
        key = md5(f"{config.DOMAIN}|{link}|{query}".encode()).hexdigest()[:16]
        cache_path = cache_dir / f"{key}.json"

        doc, stale = _cache_load(cache_path)
        if doc and not stale:
            return doc.get("data", [])

        from urllib.parse import parse_qs, urlparse
        parsed_link = urlparse(link)
        # Server-supplied links are forced relative to the tenant: never
        # follow an absolute URL to another host with our cookies.
        rel_path = "/" + parsed_link.path.lstrip("/")
        url = f"https://{config.DOMAIN}{rel_path}"
        qs = parse_qs(parsed_link.query)
        params = {k: v[0] for k, v in qs.items()}
        params["q"] = query
        results = self._request("GET", url, params=params).get("results", [])
        _cache_save(cache_path, "lookup", results)
        return results

    def _int_request(self, method: str, path: str,
                     body: dict[str, Any] | None = None,
                     params: dict[str, Any] | None = None) -> Any:
        url = f"{config.API_INT}/{path.lstrip('/')}"

        # Resolve CSRF before the mutation; a 403 after sending does not prove
        # that the server performed no side effect, so never replay the write.
        token = self._get_csrf()
        return self._request(method, url, json=body, params=params,
                             headers={"Content-Type": "application/json", "X-CSRF-Token": token})

    def int_post(self, path: str, body: dict[str, Any]) -> Any:
        return self._int_request("POST", path, body=body)

    def int_put(self, path: str, body: dict[str, Any]) -> Any:
        return self._int_request("PUT", path, body=body)

    def int_patch(self, path: str, body: dict[str, Any]) -> Any:
        return self._int_request("PATCH", path, body=body)

    def int_delete(self, path: str, body: dict[str, Any] | None = None,
                   params: dict[str, Any] | None = None) -> Any:
        return self._int_request("DELETE", path, body=body, params=params)

    def _get_csrf(self) -> str:
        if self._csrf:
            return self._csrf
        _html_headers = {"Accept": "text/html,application/xhtml+xml"}
        last_status = 0
        saw_login = False
        for path in ("/a/changes", "/a/tickets", "/a/dashboard"):
            r = self._send(
                "GET",
                f"https://{config.DOMAIN}{path}",
                headers=_html_headers,
                follow_redirects=True,
            )
            last_status = r.status_code
            if r.status_code == 401 or 300 <= r.status_code < 400:
                raise SessionError(_EXPIRED_MSG)
            if r.status_code != 200:
                saw_login = saw_login or r.status_code == 403
                continue
            if r.url.host and r.url.host != config.DOMAIN:
                raise SessionError(_EXPIRED_MSG)  # redirected to the identity provider
            html = r.text
            token = _csrf_from_html(html)
            if token:
                self._csrf = token
                if not self._fw_domain:
                    fw_m = re.search(r'https://([A-Za-z0-9-]+\.myfreshworks\.com)', html)
                    if fw_m:
                        self._fw_domain = fw_m.group(1)
                return self._csrf
            if _looks_like_login_html(html):
                raise SessionError(_EXPIRED_MSG)
        if saw_login:
            raise SessionError(_EXPIRED_MSG)
        if last_status >= 500:
            raise APIError(last_status, "could not load the app page to obtain a CSRF token")
        raise APIError(0, "csrf-token meta tag not found on app pages (Freshservice page layout may have changed)")

    def keepalive(self) -> bool:
        """POST session heartbeat to Freshworks. Best-effort, returns False on any failure."""
        fw_session_id = self._fw_session_id
        if not fw_session_id:
            return False
        if not self._fw_domain:
            try:
                self._get_csrf()
            except Exception:
                pass
        if not self._fw_domain:
            return False
        try:
            r = self._send(
                "POST",
                f"https://{self._fw_domain}/api/v2/session",
                json={"session_token": fw_session_id, "checkSessionOnlyOnce": False},
                headers={"Content-Type": "application/json"},
            )
            return r.status_code < 400
        except Exception:
            return False

    def me(self) -> dict:
        return self._request("GET", f"{config.API_V2}/agents/me").get("agent", {})

    def check_session(self) -> dict:
        """Validate the session and return the agent identity ({} if hidden).

        Raises SessionError if the session is expired/invalid.  A 403 on
        /agents/me for a *valid* session (restricted agent) is not an error:
        returns {} so callers can show "identity unavailable".  Other API
        errors propagate.
        """
        try:
            return self.me()
        except APIError as e:
            if e.status == 403:
                return {}
            raise

    def v2_get_paginated(self, path: str, params: dict[str, Any] | None = None,
                         max_pages: int = 20) -> Iterator[Any]:
        params = dict(params or {})
        for page in range(1, max_pages + 1):
            params["page"] = page
            url = f"{config.API_V2}/{path.lstrip('/')}"
            r = self._send("GET", url, params=params)
            self._check_response(r, "GET", url, params)
            data = r.json()
            yield data
            link = r.headers.get("link", "")
            if 'rel="next"' not in link:
                break
            if page == max_pages:
                err.print(
                    f"[yellow]warning[/yellow]: stopped after {max_pages} pages; more results exist "
                    f"(narrow the query to see them)",
                    highlight=False,
                )

    def rate_limit_remaining(self) -> tuple[int | None, int | None]:
        if self._rl_rem is not None:
            return self._rl_rem, self._rl_tot
        self._send("GET", f"{config.API_V2}/changes?per_page=1")
        return self._rl_rem, self._rl_tot
