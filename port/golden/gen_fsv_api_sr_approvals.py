"""Golden SR approvals (Python 9070409) for fsv-api `sr_approvals`.

    cd /tmp/fsv-py-9070409 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=9070409... uv run --offline python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_sr_approvals.py \
        > crates/fsv-api/src/testdata/sr_approvals_oracle.json
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

from fsv import config, sr_approvals as flow  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.example.com"


class Client:
    def __init__(self):
        self.groups = []
        self.persist = True

    def int_get(self, path, params=None):
        return copy.deepcopy({"approval_chains": [{"id": 71, "approval_status": 0, "approval_groups": self.groups}],
                              "meta": {"triggerable_level": 1}})

    def _request(self, method, path, **kw):
        if not self.persist:
            return {}
        if method == "POST":
            body = json.loads(kw["content"])["approval_groups"][0]
            self.groups.append({"id": 91, "name": body["name"], "approval_type": body["approval_type"],
                                "approval_status": 0,
                                "approvals": [{"id": 101, "member_id": 81, "user_id": 81,
                                               "status": {"id": 0, "name": "requested"}, "remark": [],
                                               "last_reminded_at": None}]})
        else:
            group = self.groups[0]
            row = group["approvals"][0]
            if path.endswith("/resend"):
                row["last_reminded_at"] = "2026-12-01T10:00:00Z"
            elif path.endswith("/cancel"):
                group["approval_status"] = 3
                row["status"] = {"id": 3, "name": "cancelled"}
            else:
                body = json.loads(kw["content"])
                ident = body["approval_status"]
                row["status"] = {"id": ident, "name": "approved" if ident == 1 else "rejected"}
                row["remark"].append({"status": ident, "data": body["remark"]})
        return {"approval_chain": {"id": 999}}

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
                c.persist = op.get("persist", c.persist)
                if "approval_status" in op and c.groups:
                    c.groups[0]["approval_status"] = op["approval_status"]
                out = {}
            elif kind == "build":
                plan = flow.build_plan(c, op.get("target", 23), op["operation"], ident=op.get("ident"),
                                        name=op.get("name"), message=op.get("message"),
                                        approvers=op.get("approvers"), rule=op.get("rule", "everyone"),
                                        remark=op.get("remark", ""))
                out = {"plan": plan_out(plan)}
            elif kind == "send":
                reviewed = plan.hash if op.get("hash", "full") == "full" else plan.hash[:12]
                out = {"result": flow.send(plan, c, reviewed_hash=reviewed)}
            else:
                raise AssertionError(kind)
        except Exception as exc:  # noqa: BLE001
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        outputs.append(out)
    return outputs


def request():
    return {"op": "build", "target": "SR-23", "operation": "request", "name": "Acme review",
            "message": "Please review", "approvers": ["81"]}


CASES = {
    "approve": [request(), {"op": "send"},
                {"op": "build", "operation": "approve", "ident": 101}, {"op": "send"}],
    "reject": [request(), {"op": "send"},
               {"op": "build", "operation": "reject", "ident": 101, "remark": "Needs revision"}, {"op": "send"}],
    "cancel": [request(), {"op": "send"},
               {"op": "build", "operation": "cancel", "ident": 91}, {"op": "send"}],
    "remind": [request(), {"op": "send"},
               {"op": "build", "operation": "remind", "ident": 101}, {"op": "send"}],
    "echo": [request(), {"op": "set", "persist": False}, {"op": "send"}, {"op": "send"}],
    "short_hash": [request(), {"op": "send"},
                   {"op": "build", "operation": "approve", "ident": 101},
                   {"op": "send", "hash": "prefix"}, {"op": "send"}],
    "reject_remark": [{"op": "build", "operation": "reject", "ident": 101}],
    "immutable": [request(), {"op": "send"},
                  {"op": "build", "operation": "approve", "ident": 101}, {"op": "send"},
                  {"op": "set", "approval_status": 1},
                  {"op": "build", "operation": "cancel", "ident": 91}],
}
json.dump(stamped({"cases": {name: {"ops": ops, "outputs": run(ops)} for name, ops in CASES.items()}}),
          sys.stdout, indent=1, ensure_ascii=False)
print()
