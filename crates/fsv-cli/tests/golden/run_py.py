"""Run the Python fsv against the same canned routes the Rust tests use.

Usage (from the Python repo): uv run python run_py.py ROUTES.json LOG.jsonl -- ARGS...
Environment (HOME, FSV_CONFIG_DIR, ...) is the caller's. Prints nothing of
its own; stdout/stderr/exit code are fsv's.
"""

import json
import os
import sys
from urllib.parse import parse_qsl, urlsplit

import httpx

if os.environ.get("FSV_TEST_TTY"):
    sys.stdin.isatty = lambda: True

routes_path, log_path = sys.argv[1], sys.argv[2]
args = sys.argv[sys.argv.index("--") + 1:]
routes = json.load(open(routes_path))


def matches(route, req):
    if route.get("method", "GET") != req.method:
        return False
    if route.get("path") != req.url.path:
        return False
    pairs = parse_qsl(urlsplit(str(req.url)).query, keep_blank_values=True)
    for k, v in (route.get("query") or {}).items():
        want = v if isinstance(v, str) else json.dumps(v)
        if (k, want) not in pairs:
            return False
    return True


def handler(req):
    body = req.content
    try:
        parsed = json.loads(body) if body else None
    except Exception:
        parsed = body.decode("utf-8", "replace")
    with open(log_path, "a") as f:
        f.write(json.dumps({"method": req.method, "url": str(req.url), "body": parsed}) + "\n")
    for r in routes:
        if matches(r, req):
            status = r.get("status", 200)
            headers = dict(r.get("headers") or {})
            if "text" in r:
                headers.setdefault("content-type", "text/html; charset=utf-8")
                return httpx.Response(status, text=r["text"], headers=headers)
            return httpx.Response(status, json=r.get("json", {}), headers=headers)
    return httpx.Response(404, json={"errors": "no route"})


_orig = httpx.Client.__init__


def _init(self, *a, **kw):
    kw["transport"] = httpx.MockTransport(handler)
    _orig(self, *a, **kw)


httpx.Client.__init__ = _init

from fsv.cli import app  # noqa: E402

sys.argv = ["fsv", *args]
app(prog_name="fsv")
