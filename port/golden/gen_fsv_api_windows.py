"""Golden `windows` for fsv-api `change_windows`: plan hashes, legacy hashes,
payloads and reviews of `create_plan` / `delete_plan` over the `wapi` fake of
`tests/test_templates_windows.py`, with the cases of the Rust test
`change_windows_tests::hashes_and_reviews_match_python`.

    cd /tmp/fsv-py-<sha> && FSV_PY_SRC=$PWD FSV_PY_COMMIT=<sha> python \
        port/golden/gen_fsv_api_windows.py
"""
from __future__ import annotations

import datetime as dt
import json
import os
import sys
import tempfile
from zoneinfo import ZoneInfo

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.environ["XDG_STATE_HOME"] = os.path.join(BOOT, "state")
os.environ.pop("FSV_TZ", None)

import httpx  # noqa: E402

from fsv import change_windows as W, config, schema  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.freshservice.com"
config.API_INT = "https://acme.freshservice.com/api/_"
config.API_V2 = "https://acme.freshservice.com/api/v2"
SCH = {"fields": [{"name": "change_type", "label": "Change Type", "workspace_id": 3,
                   "choices": [{"id": 1, "value": "Normal"}, {"id": 4, "value": "Emergency"}]}]}
schema.load = lambda res, client=None, force=False: SCH
import fsv.formmeta  # noqa: E402

fsv.formmeta.configured_workspace = lambda: None

NOW_WINDOW = {"id": 61, "name": "Globex freeze", "start_time": "2030-11-20T09:00:00Z", "end_time": "2030-11-20T10:00:00Z"}
NY_A = {"id": 71, "name": "Acme A", "start_time": "2030-11-14T02:00:00-05:00", "end_time": "2030-11-14T04:00:00-05:00"}
NY_C = {"id": 72, "name": "Acme C", "start_time": "2030-11-14T04:00:00-05:00", "end_time": "2030-11-14T06:00:00-05:00"}
s = {"windows": [NOW_WINDOW], "overlaps": [], "agent": {"id": 7, "time_zone": "Eastern Time (US & Canada)"}}


def handler(req):
    path = req.url.path.removeprefix("/api/_/")
    if req.method == "GET" and path.endswith("agents/me"):
        return httpx.Response(200, json={"agent": s["agent"]})
    if req.method == "GET":
        assert path == "maintenance_windows"
        return httpx.Response(200, json={"maintenance_windows": s["windows"]})
    if path == W.OVERLAP_PATH:
        return httpx.Response(200, json={"overlapping_windows": s["overlaps"]})
    raise AssertionError(path)


c = Client(cookies={})
c._client.close()
c._client = httpx.Client(transport=httpx.MockTransport(handler))
c._csrf = "synthetic"


def plan(name="Acme release", description="Quarterly", start="2030-11-21 10:00", end="2030-11-21 13:30", **kw):
    return W.create_plan(c, name, description, start, end, ["change_type=Normal"], **kw)


def row(p):
    return {"hash": p.hash, "legacy": p.legacy_hash, "payload_json": p.write.payload_json,
            "overlap_json": p.overlap_json, "start": p.start, "end": p.end, "review": p.review()}


out = {"default": row(plan()), "allow": row(plan(allow_overlap=True))}
s["windows"] = [NY_A, NY_C]
zone = {"name": "Acme zone", "description": "", "start": "2030-11-14 15:00", "end": "2030-11-14 17:00"}
out["bangkok"] = row(plan(**zone, timezone="Asia/Bangkok"))
os.environ["FSV_TZ"] = "Asia/Bangkok"
out["env"] = row(plan(**zone))
out["offset"] = row(plan(name="Acme zone", description="", start="2030-11-14 15:00+07:00",
                         end="2030-11-14T16:00+07:00", timezone="Europe/London"))
os.environ.pop("FSV_TZ")
s["windows"] = [{"id": 76, "name": "Acme odd", "start_time": "soon", "end_time": "later"}]
out["unknown_allow"] = row(plan(name="Acme zone", description="", start="2030-11-14 08:00", end="2030-11-14 09:00",
                                allow_overlap=True))
s["windows"] = [NOW_WINDOW]
d = W.delete_plan(c, 61)
out["delete"] = {"hash": d.hash, "review": d.review()}
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
