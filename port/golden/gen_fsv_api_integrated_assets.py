"""Golden integrated inventory (Python ad40a51) for fsv-api `integrated_assets`.

The Rust test replays each op against a client with the same row rules.

    cd /tmp/fsv-py-36408b2 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=36408b2... uv run --offline python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_integrated_assets.py \
        > crates/fsv-api/src/testdata/integrated_assets_oracle.json
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

from fsv import config, integrated_assets as flow  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.freshservice.com"


class Client:
    def __init__(self):
        self.rows = []
        self.unlink = False
        self.search = {"page": 1, "error": None, "total_rows": 1,
                       "data": [{"device_pk": 81, "name": {"id": 81, "label": "Acme"}}]}
        self.inventory = None

    def int_get(self, path, params=None):
        if self.inventory is not None:
            return copy.deepcopy(self.inventory)
        n = len(self.rows)
        return {"data": copy.deepcopy(self.rows),
                "meta": {"total_count": n, "total_pages": 1 if n else 0}}

    def _request(self, method, url, params=None, content=None, **_kw):
        if method == "GET":
            return copy.deepcopy(self.search)
        payload = json.loads(content)
        if "itam_cmdb_request_id" in payload:
            if self.unlink:
                self.rows[:] = [r for r in self.rows if int(r["item_id"]) != 81]
            else:
                self.rows.clear()
        else:
            key = "businessapp_ids" if "businessapp_ids" in payload else "device_ids"
            cat = "Business service" if key == "businessapp_ids" else "Device"
            self.rows.extend({"item_id": i, "itam_cmdb_request_id": int(i) + 900, "category": cat}
                             for i in payload[key])
        return {}

    def _get_csrf(self):
        return "synthetic"


def plan_out(plan):
    return {"action": plan.action, "target": plan.target, "tenant": plan.tenant, "method": plan.method,
            "path": plan.path, "payload": plan.payload, "context_hash": plan.context_hash,
            "hash": plan.hash, "review": plan.review()}


def run(ops):
    os.environ["XDG_STATE_HOME"] = tempfile.mkdtemp(prefix="fsv-state-")
    c = Client()
    plan = None
    outputs = []
    for op in ops:
        try:
            kind = op["op"]
            if kind == "set":
                for k, v in op.items():
                    if k != "op":
                        setattr(c, k, copy.deepcopy(v))
                out = {}
            elif kind == "search":
                out = {"result": flow.search(c, op.get("q", ""), kind=op.get("kind", "devices"), page=op.get("page", 1))}
            elif kind == "build":
                plan = flow.build_plan(c, op["module"], op.get("target", 23), op["ids"],
                                        kind=op.get("kind", "devices"), remove=op.get("remove", False))
                out = {"plan": plan_out(plan)}
            elif kind == "send":
                reviewed = {"full": plan.hash, "none": None, "wrong": "wrong"}[op.get("hash", "full")]
                out = {"result": flow.send(plan, c, op["module"], op["ids"], kind=op.get("kind", "devices"),
                                            remove=op.get("remove", False), reviewed_hash=reviewed)}
            elif kind == "verify":
                out = {"result": flow.verify_devices(c, op["module"], op.get("target", 23), op["devices"])}
            elif kind == "rows":
                out = {"rows": copy.deepcopy(c.rows)}
            else:
                raise AssertionError(kind)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        outputs.append(out)
    return outputs


DEVICE = {"device_pk": 81, "name": {"id": 81, "label": "Acme"}, "type": "Computer"}
CASES = {}
for module in ("tickets", "sr", "problems", "changes"):
    for kind in ("devices", "services"):
        CASES[f"link_{module}_{kind}"] = [
            {"op": "build", "module": module, "ids": [81, 82], "kind": kind},
            {"op": "send", "module": module, "ids": [81, 82], "kind": kind},
            {"op": "build", "module": module, "ids": [81], "kind": kind, "remove": True},
            {"op": "set", "unlink": True},
            {"op": "send", "module": module, "ids": [81], "kind": kind, "remove": True},
            {"op": "rows"},
        ]
CASES["search"] = [{"op": "search", "q": "Acme"}]
CASES["search_page"] = [{"op": "search", "page": 2}]
CASES["search_kind"] = [{"op": "search", "kind": "printers"}]
CASES["search_page0"] = [{"op": "search", "page": 0}]
CASES["search_error"] = [{"op": "set", "search": {"page": 1, "error": "down", "total_rows": 0, "data": []}},
                         {"op": "search"}]
CASES["search_conflict"] = [{"op": "set", "search": {"page": 1, "error": None, "total_rows": 1,
                            "data": [{"device_pk": 82, "name": {"id": 81, "label": "Acme"}}]}},
                            {"op": "search"}]
CASES["search_duplicate"] = [{"op": "set", "search": {"page": 1, "error": None, "total_rows": 2, "data": [
    {"device_pk": 81, "name": {"id": 81}}, {"device_pk": 81, "name": {"id": 81}}]}}, {"op": "search"}]
CASES["search_short_total"] = [{"op": "set", "search": {"page": 1, "error": None, "total_rows": 0, "data": [
    {"name": {"id": 81}}]}}, {"op": "search"}]
CASES["bad_module"] = [{"op": "build", "module": "assets", "ids": [81]}]
CASES["empty_ids"] = [{"op": "build", "module": "tickets", "ids": []}]
CASES["assets_kind"] = [{"op": "build", "module": "tickets", "ids": [81], "kind": "assets"}]
CASES["remove_missing"] = [{"op": "build", "module": "tickets", "ids": [81], "remove": True}]
CASES["already"] = [
    {"op": "build", "module": "tickets", "ids": [81]},
    {"op": "send", "module": "tickets", "ids": [81]},
    {"op": "build", "module": "tickets", "ids": [81]},
]
CASES["wrong_hash"] = [
    {"op": "build", "module": "tickets", "ids": [81]},
    {"op": "send", "module": "tickets", "ids": [81], "hash": "wrong"},
    {"op": "rows"},
]
CASES["incomplete"] = [
    {"op": "set", "inventory": {"data": [], "meta": {"total_count": 1, "total_pages": 1}}},
    {"op": "build", "module": "tickets", "ids": [81]},
]
CASES["verify_empty"] = [{"op": "verify", "module": "changes", "devices": ["81"]}]
CASES["verify_linked"] = [
    {"op": "build", "module": "changes", "ids": [81]},
    {"op": "send", "module": "changes", "ids": [81]},
    {"op": "verify", "module": "changes", "devices": ["81"]},
]
CASES["mapped_device"] = [{"op": "set", "search": {"page": 1, "error": None, "total_rows": 1, "data": [DEVICE]}},
                          {"op": "search", "q": "Globex", "page": 1}]
json.dump(stamped({"cases": {name: {"ops": ops, "outputs": run(ops)} for name, ops in CASES.items()}}),
          sys.stdout, indent=1, ensure_ascii=False)
print()
