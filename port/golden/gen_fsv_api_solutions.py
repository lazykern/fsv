"""Golden solutions (Python ad40a51) for fsv-api `solutions`.

    cd /tmp/fsv-py-0719e33 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=0719e33... uv run --offline python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_solutions.py \
        > crates/fsv-api/src/testdata/solutions_oracle.json
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import config, solutions as flow  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.example.com"


class Client:
    def __init__(self):
        self.articles = {}
        self.linked = []
        self.persist = True
        self.writes = []

    def int_get(self, path, params=None):
        if path == "solutions/categories":
            return {"categories": [{"id": 71, "name": "Acme", "default_category": True, "workspace_id": 1}],
                    "meta": {"count": 1, "page": None, "per_page": "300"}}
        if path == "solutions/folders":
            assert params["category_id"] == 71
            return {"folders": [{"id": 81, "name": "Drafts", "category_id": 71, "default_folder": True, "workspace_id": 1}],
                    "meta": {"count": 1, "page": None, "per_page": "300"}}
        if path.startswith("solutions/articles/"):
            return {"article": copy.deepcopy(self.articles[int(path.split("/")[-1])])}
        if path.endswith("/solutions"):
            return {"articles": copy.deepcopy(self.linked), "has_more": False}
        return {"problem": {"id": 23, "subject": "Acme problem", "workspace_id": 2}}

    def _request(self, method, path, **kw):
        body = json.loads(kw["content"])
        self.writes.append((method, path, body))
        if method == "POST":
            row = {"id": 91, **body}
            if self.persist:
                self.articles[91] = {**row, "description": "<div style='font:inherit'>" + body["description"] + "</div>"}
            return {"article": row}
        if self.persist:
            if body["type"] == "link":
                self.linked.append(copy.deepcopy(self.articles[91]))
            else:
                self.linked.clear()
        return {}

    def _get_csrf(self):
        return "synthetic"


def plan_out(plan):
    return {"action": plan.action, "target": plan.target, "tenant": plan.tenant, "method": plan.method,
            "path": plan.path, "payload": plan.payload, "context_hash": plan.context_hash,
            "hash": plan.hash, "review": plan.review(), "inputs": json.loads(plan.inputs_json)}


def run(ops):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    c = Client()
    plan = None
    outputs = []
    for op in ops:
        try:
            kind = op["op"]
            if kind == "set":
                if "persist" in op:
                    c.persist = op["persist"]
                out = {}
            elif kind == "create":
                plan = flow.create_plan(c, op["title"], op["description"], op["kind"],
                                         category=op.get("category"), selected_folder=op.get("selected_folder"))
                out = {"plan": plan_out(plan)}
            elif kind == "send_create":
                out = {"result": flow.send_create(plan, c, reviewed_hash=plan.hash)}
            elif kind == "link":
                plan = flow.link_plan(c, op.get("target", 23), op.get("ident", 91), unlink=op.get("unlink", False))
                out = {"plan": plan_out(plan)}
            elif kind == "send_link":
                out = {"result": flow.send_link(plan, c, reviewed_hash=plan.hash)}
            else:
                raise AssertionError(kind)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        out["writes"] = len(c.writes)
        outputs.append(out)
    return outputs


def cycle(kind):
    return [{"op": "create", "title": "Acme article", "description": "Synthetic guidance", "kind": kind},
            {"op": "send_create"},
            {"op": "link", "target": "PRB-23", "ident": 91}, {"op": "send_link"},
            {"op": "link", "target": 23, "ident": 91, "unlink": True}, {"op": "send_link"}]


CASES = {
    "workaround": cycle("workaround"),
    "permanent": cycle("permanent"),
    "echo": [{"op": "create", "title": "Acme article", "description": "Guidance", "kind": "workaround"},
             {"op": "set", "persist": False}, {"op": "send_create"}, {"op": "send_create"}],
    "invalid": [{"op": "create", "title": "Acme", "description": "Guidance", "kind": "permanent",
                 "selected_folder": "unknown"},
                {"op": "create", "title": "Acme", "description": "Guidance", "kind": "publish"}],
}

json.dump(stamped({"cases": {name: {"ops": ops, "outputs": run(ops)} for name, ops in CASES.items()}}),
          sys.stdout, indent=1, ensure_ascii=False)
print()
