from __future__ import annotations

import atexit
import json
import re
import time
from typing import Any, Iterator
from urllib.parse import parse_qsl, quote, urlsplit

import httpx

from fsv import config
from fsv.errors import APIError
from fsv.render import err
from fsv.session import SessionError, load_cookies, update_cookie

UA = "fsv/0.1 (Freshservice CLI)"


_instance: "Client | None" = None
_VERBOSE = False
_REDACT_KEYS = {
    "access-token",
    "api-key",
    "apikey",
    "authorization",
    "cookie",
    "csrf",
    "password",
    "refresh-token",
    "secret",
    "session-token",
    "token",
    "x-csrf-token",
}
_RETRYABLE_STATUS = {429, 500, 502, 503, 504}
_IDEMPOTENT_METHODS = {"GET", "HEAD", "OPTIONS"}
_MAX_RETRIES = 2
_RETRY_DELAYS = (0.5, 1.5)


def set_verbose(value: bool) -> None:
    global _VERBOSE
    _VERBOSE = bool(value)


def is_verbose() -> bool:
    return _VERBOSE


def _redact(value: Any) -> Any:
    if isinstance(value, dict):
        out: dict[Any, Any] = {}
        for k, v in value.items():
            key = str(k).strip().lower().replace("_", "-")
            out[k] = "[redacted]" if key in _REDACT_KEYS else _redact(v)
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


def _body_preview(kw: dict[str, Any]) -> str:
    raw = ""
    if kw.get("json") is not None:
        raw = json.dumps(_redact(kw["json"]), ensure_ascii=False, separators=(",", ":"), default=str)
    elif kw.get("data") is not None:
        data = kw["data"]
        raw = data.decode("utf-8", errors="replace") if isinstance(data, bytes) else str(data)
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
    except ValueError:
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
    def __init__(self) -> None:
        config.require_domain()
        self._csrf: str | None = None
        self._fw_domain: str | None = None
        self._rl_rem: int | None = None
        self._rl_tot: int | None = None
        cookies = load_cookies()
        self._fw_session_id: str | None = cookies.get("fw-session-id")
        self._client = httpx.Client(
            cookies=cookies,
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
            if rem:
                self._rl_rem = int(rem)
            if tot:
                self._rl_tot = int(tot)
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

    def _request(self, method: str, url: str, **kw: Any) -> Any:
        r = self._send(method, url, **kw)
        location = (r.headers.get("location") or "").lower()
        if r.status_code in (301, 302) and "freshid" in location:
            raise SessionError("session expired; run `fsv auth login`")
        if r.status_code == 401:
            raise SessionError("session expired; run `fsv auth login`")
        ct = r.headers.get("content-type", "")
        if r.status_code == 200 and "text/html" in ct and "/api/" in url:
            raise SessionError("session expired; run `fsv auth login`")
        if r.status_code >= 400:
            raise APIError(r.status_code, _response_body(r), method, _request_target(url, kw.get("params")))
        fw = self._client.cookies.get("fw-session-id")
        if fw and fw != self._fw_session_id:
            self._fw_session_id = fw
            update_cookie("fw-session-id", fw)
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
        from fsv.config import CACHE_DIR

        cache_dir = CACHE_DIR / "lookup"
        cache_dir.mkdir(parents=True, exist_ok=True)
        key = md5(f"{config.DOMAIN}|{link}|{query}".encode()).hexdigest()[:16]
        cache_path = cache_dir / f"{key}.json"

        doc, stale = _cache_load(cache_path)
        if doc and not stale:
            return doc.get("data", [])

        url = f"https://{config.DOMAIN}{link.split('?')[0]}"
        from urllib.parse import parse_qs, urlparse
        qs = parse_qs(urlparse(link).query)
        params = {k: v[0] for k, v in qs.items()}
        params["q"] = query
        results = self._request("GET", url, params=params).get("results", [])
        _cache_save(cache_path, "lookup", results)
        return results

    def _int_request(self, method: str, path: str,
                     body: dict[str, Any] | None = None,
                     params: dict[str, Any] | None = None) -> Any:
        url = f"{config.API_INT}/{path.lstrip('/')}"

        def _do() -> Any:
            return self._request(method, url, json=body, params=params,
                                 headers={"Content-Type": "application/json",
                                          "X-CSRF-Token": self._get_csrf()})

        try:
            return _do()
        except APIError as e:
            if e.status == 403:
                self._csrf = None
                return _do()
            raise

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
        for path in ("/a/changes", "/a/tickets", "/a/dashboard"):
            html = self._send(
                "GET",
                f"https://{config.DOMAIN}{path}",
                headers=_html_headers,
                follow_redirects=True,
            ).text
            m = (re.search(r'name="csrf-token"\s+content="([^"]+)"', html)
                 or re.search(r'<meta\s+content="([^"]+)"\s+name="csrf-token"', html))
            if m:
                self._csrf = m.group(1)
                if not self._fw_domain:
                    fw_m = re.search(r'https://([A-Za-z0-9-]+\.myfreshworks\.com)', html)
                    if fw_m:
                        self._fw_domain = fw_m.group(1)
                return self._csrf
        raise APIError(0, "csrf-token meta tag not found; session may be expired — run `fsv auth login`")

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
            self._send(
                "POST",
                f"https://{self._fw_domain}/api/v2/session",
                json={"session_token": fw_session_id, "checkSessionOnlyOnce": False},
                headers={"Content-Type": "application/json"},
            )
            return True
        except Exception:
            return False

    def me(self) -> dict:
        return self._request("GET", f"{config.API_V2}/agents/me").get("agent", {})

    def v2_get_paginated(self, path: str, params: dict[str, Any] | None = None,
                         max_pages: int = 20) -> Iterator[Any]:
        params = dict(params or {})
        for page in range(1, max_pages + 1):
            params["page"] = page
            r = self._send("GET", f"{config.API_V2}/{path.lstrip('/')}", params=params)
            location = (r.headers.get("location") or "").lower()
            if r.status_code in (301, 302) and "freshid" in location:
                raise SessionError("session expired; run `fsv auth login`")
            if r.status_code == 401:
                raise SessionError("session expired; run `fsv auth login`")
            ct = r.headers.get("content-type", "")
            if r.status_code == 200 and "text/html" in ct:
                raise SessionError("session expired; run `fsv auth login`")
            if r.status_code >= 400:
                raise APIError(r.status_code, _response_body(r), "GET", _request_target(f"{config.API_V2}/{path.lstrip('/')}", params))
            data = r.json()
            yield data
            link = r.headers.get("link", "")
            if 'rel="next"' not in link:
                break

    def rate_limit_remaining(self) -> tuple[int | None, int | None]:
        if self._rl_rem is not None:
            return self._rl_rem, self._rl_tot
        self._send("GET", f"{config.API_V2}/changes?per_page=1")
        return self._rl_rem, self._rl_tot
