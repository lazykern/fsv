"""Golden outputs for form-picked lookups (`crates/fsv-app/src/form/tests.rs`,
`crates/fsv-flows/src/drafts.rs`).

A `change_session` picks a group and an agent as `LookupValue`s (the TUI
picker's value). Records the plan's review rows, payload and hash and the
saved draft text, plus `drafts._kv` over lookup answers (the lines kept for
hidden and unknown answers).

    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_app_form_lookup.py
"""
from __future__ import annotations

import json
import os
import sys
import tempfile

ROOT = tempfile.mkdtemp(prefix="fsv-golden-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(ROOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(ROOT, "cache")
os.environ["XDG_STATE_HOME"] = os.path.join(ROOT, "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"
os.environ["COLUMNS"] = "80"
os.environ.pop("FSV_TZ", None)
os.makedirs(os.environ["FSV_CONFIG_DIR"])
with open(os.path.join(os.environ["FSV_CONFIG_DIR"], "config.json"), "w") as f:
    f.write('{"domain": "acme.freshservice.com"}')

import httpx  # noqa: E402

from fsv import client as client_mod  # noqa: E402
from fsv import config  # noqa: E402
from fsv.drafts import _kv  # noqa: E402
from fsv.formui.session import change_session  # noqa: E402
from fsv.forms import LookupValue  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()


def field(i, name, label, ft, required, default):
    return {"id": i, "name": name, "label": label, "field_type": ft, "required": required, "position": i,
            "default_field": default, "choices": [], "nested_fields": [], "workspace_id": 2}


FIELDS = [field(1, "subject", "Subject", "default_subject", True, True),
          field(2, "requester", "Requester", "default_requester", True, True),
          field(3, "group_id", "Group", "default_group", False, True),
          field(4, "agent_id", "Agent", "default_agent", False, True)]


def route(req: httpx.Request) -> httpx.Response:
    p = req.url.path.removeprefix("/api/_/")
    if req.method == "GET":
        if p == "change_form_fields":
            return httpx.Response(200, json={"change_fields": FIELDS})
        if p.endswith("business_rules"):
            return httpx.Response(200, json={"business_rules": [], "user": {"time_zone": "Tokyo"}})
        if p == "bootstrap/agents_groups":
            return httpx.Response(200, json={"groups": [{"id": 4, "name": "Ops"}]})
        return httpx.Response(200, json={})
    return httpx.Response(404, json={"description": "not found"})


c = client_mod.Client(cookies={"_x_w": "w", "fw-session-id": "fw1"})
c._client.close()
c._client = httpx.Client(transport=httpx.MockTransport(route), follow_redirects=False)
c._csrf = "tok"

s = change_session(c, requester=LookupValue(9, "Jane Doe", "jane@example.com"))
s.set("subject", "Deploy")
s.set("group_id", LookupValue(4, "Ops"))
s.set("agent_id", LookupValue(7, "Ann Agent", "ann@example.com"))
plan = s.build_plan()
draft = os.path.join(ROOT, "change-draft.yaml")
s.save_draft(draft, plan)
with open(draft, encoding="utf-8") as f:
    text = f.read()

KV = {
    "both": LookupValue(7, "Ann Agent", "ann@example.com"),
    "display": LookupValue(4, "Ops"),
    "email": LookupValue(None, "", "x@y.io"),
    "id": LookupValue(12),
    "colon": LookupValue(5, "Team: A"),
    "list": [LookupValue(4, "Ops"), LookupValue(7, "Ann Agent", "ann@example.com"), 3, "a, b"],
}

out = {
    "errors": plan.errors,
    "rows": [list(r) for r in plan.review.rows],
    "payload": plan.payload,
    "hash": plan.hash,
    "draft": text,
    "kv": {k: _kv(k, v) for k, v in KV.items()},
}
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False, default=str)
