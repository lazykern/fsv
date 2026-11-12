"""Shared oracle harness: hermetic env + a fake client/journal that mirror
fsv-api's Rust `FakeApi` / `MemJournal` (routes "METHOD path", last reply
repeats, unknown route -> HTTP 404 {"description": "nope"})."""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

_BOOT = tempfile.mkdtemp(prefix="fsv-oracle-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(_BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(_BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.environ["XDG_STATE_HOME"] = os.path.join(_BOOT, "state")
os.environ["TZ"] = "UTC"

from fsv import config, journal  # noqa: E402
from fsv.errors import APIError, SessionError  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.freshservice.com"
config.API_INT = "https://acme.freshservice.com/api/_"


class Resp:
    def __init__(self, status, content=b"", headers=None):
        self.status_code = status
        self.content = content
        self.headers = headers or {}


class Fake:
    def __init__(self, routes=None, cookies=()):
        self.routes = {k: list(v) if isinstance(v, list) else [v] for k, v in (routes or {}).items()}
        self.calls = []
        self._csrf = "tok"
        self._cookies = list(cookies)

    def _reply(self, method, path):
        q = self.routes.get(f"{method} {path}")
        if not q:
            return ("status", 404)
        return q.pop(0) if len(q) > 1 else q[0]

    def _do(self, method, path, params=None, body=None):
        self.calls.append({"method": method, "path": path, "params": params, "body": body})
        r = self._reply(method, path)
        if not isinstance(r, tuple):
            return r
        if r[0] == "status":
            raise APIError(r[1], {"description": "nope"})
        if r[0] == "fail":
            raise APIError(r[1], r[2])
        if r[0] == "session":
            raise SessionError("expired")
        if r[0] == "raw":
            return r[2].decode("utf-8", "replace")
        return r[1]

    def int_get(self, path, params=None):
        return self._do("GET", path.lstrip("/"), params)

    def int_post(self, path, body=None):
        return self._do("POST", path, None, body)

    def int_put(self, path, body=None):
        return self._do("PUT", path, None, body)

    def int_patch(self, path, body=None):
        return self._do("PATCH", path, None, body)

    def int_delete(self, path, body=None, params=None):
        return self._do("DELETE", path, params, body)

    def autocomplete(self, kind, query, params=None):
        v = self._do("GET", f"autocomplete/{kind}", {"q": query})
        return v if isinstance(v, list) else []

    def lookup_choices(self, link, query):
        v = self._do("GET", link, {"q": query})
        return v if isinstance(v, list) else []

    def fulltext_search(self, entity, term, page=1, sort=None, **kw):
        return self._do("GET", f"search/{entity}", {"term": term})

    def _get_csrf(self):
        return "tok"

    def _request(self, method, url, content=None, json=None, headers=None, params=None, **kw):
        path = url.removeprefix(config.API_INT + "/")
        body = json
        if content is not None:
            import json as _j
            try:
                body = _j.loads(content)
            except Exception:
                body = None
        return self._do(method, path, params, body)

    @property
    def _client(self):
        fake = self

        class _C:
            class cookies:
                jar = [type("Ck", (), {"name": "fw-workspace", "value": v})() for v in fake._cookies]

            def get(self, url, follow_redirects=True):
                fake.calls.append({"method": "FETCH", "path": url})
                r = fake._reply("FETCH", url)
                if isinstance(r, tuple) and r[0] == "raw":
                    return Resp(r[1], r[2])
                if isinstance(r, tuple) and r[0] == "status":
                    return Resp(r[1])
                return Resp(404)

        return _C()

    def writes(self):
        return [c for c in self.calls if c["method"] != "GET"]

    def gets(self):
        return [c["path"] for c in self.calls if c["method"] == "GET"]


class MemJournal:
    def __init__(self):
        self.rows = []

    def read(self, *a, **k):
        return list(self.rows)

    def record_plan(self, event, plan, action="", target=None, created_id=None, steps=None, error=None, **kw):
        self.rows.append({"kind": f"plan:{action}", "event": event, "plan": plan, "target": target, "created_id": created_id})

    def recent_plan(self, plan, action, hours=24):
        rows = [r for r in self.rows if r["kind"] == f"plan:{action}" and r["plan"] == plan]
        done = [r for r in rows if r["event"] == "done"]
        if done:
            return done[-1]
        return rows[-1] if rows and rows[-1]["event"] == "started" else None

    def events(self):
        return [r["event"] for r in self.rows]


def install_journal():
    j = MemJournal()
    journal.read = j.read
    journal.record_plan = j.record_plan
    journal.recent_plan = j.recent_plan
    return j


OUT = {}


def rec(name, fn):
    """Record fn()'s result, or the exception as {"error": type, "msg": str}."""
    try:
        OUT[name] = fn()
    except BaseException as e:  # noqa: BLE001
        OUT[name] = {"__error__": type(e).__name__, "msg": str(e)}


def dump():
    def default(o):
        if isinstance(o, (set, frozenset, tuple)):
            return list(o)
        if hasattr(o, "__dict__"):
            return {k: v for k, v in vars(o).items() if not k.startswith("_")}
        return repr(o)

    print(json.dumps(stamped(OUT), indent=1, ensure_ascii=False, default=default, sort_keys=False))
