"""Golden approval recipients (Python e173437) for fsv-api `approval_recipients`.

    cd /tmp/fsv-py-d870fa7 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=d870fa7... uv run --offline python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_approval_recipients.py \
        > crates/fsv-api/src/testdata/approval_recipients_oracle.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile

BOOT = tempfile.mkdtemp(prefix="fsv-gen-")
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import approval_recipients as flow  # noqa: E402
from fsv import change_approvals as approvals  # noqa: E402
from fsv import config  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.example.com"


class Client:
    def __init__(self):
        self.members = [81, 82]
        self.paged = False

    def int_get(self, path, params=None):
        if path == "bootstrap/agents_groups":
            return {"agents": [], "groups": [{"id": 71, "name": "Acme reviewers", "agent_ids": [999]}]}
        if path == "cabs":
            return {"cabs": [{"id": 91, "name": "Release review"}]}
        ids = self.members if "/groups/" in path else [82, 83]
        chunk = ids
        if self.paged and "/groups/" in path:
            page = (params or {}).get("page", 1)
            start = max(page - 1, 0) * 30
            chunk = ids[start:start + 30]
        key = "approval_users" if "/groups/" in path else "approval_agents"
        rows = [{"id": i, "selected": False} for i in chunk] if key == "approval_users" else [
            {"id": str(i), "name": "Acme reviewer", "email": "reviewer@example.com", "selected": False}
            for i in chunk
        ]
        return {key: rows, "meta": {"count": len(ids), "page": 1, "per_page": 30}}


def plan_out(ids, context):
    plan = approvals.build_plan(
        "CHN-23", "Acme review", "Review", approver_ids=ids, recipient_context_json=context
    )
    return {
        "ids": ids,
        "context": json.loads(context) if context else [],
        "hash": plan.hash,
        "notified_to": plan.payload["approval_groups"][0]["notified_to"],
        "sources": plan.review()["recipient_sources"],
    }


def run(ops):
    c = Client()
    context = ""
    outputs = []
    for op in ops:
        try:
            kind = op["op"]
            if kind == "set":
                if "members" in op:
                    c.members = list(op["members"])
                if "paged" in op:
                    c.paged = bool(op["paged"])
                out = {}
            elif kind == "resolve":
                ids, context = flow.resolve(c, 23, op["groups"], op["cabs"])
                out = plan_out(ids, context)
            elif kind == "validate":
                flow.validate(c, 23, context)
                out = {}
            elif kind == "members":
                raw = op["raw"]
                one = type("C", (), {"int_get": lambda self, path, params=None: raw})()
                out = {"ids": flow.members(one, 23, "cabs", 91)}
            else:
                raise AssertionError(kind)
        except Exception as exc:  # noqa: BLE001 — oracle records the Python failure
            out = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
        outputs.append(out)
    return outputs


CASES = {
    "flatten": [
        {"op": "resolve", "groups": ["Acme reviewers"], "cabs": ["91"]},
        {"op": "validate"},
        {"op": "set", "members": [81]},
        {"op": "validate"},
    ],
    "empty": [
        {"op": "set", "members": []},
        {"op": "resolve", "groups": ["71"], "cabs": []},
        {"op": "resolve", "groups": [], "cabs": ["missing"]},
    ],
    "duplicate": [
        {"op": "resolve", "groups": ["71", "Acme reviewers"], "cabs": []},
    ],
    "paged": [
        {"op": "set", "paged": True, "members": list(range(1, 41))},
        {"op": "resolve", "groups": ["71"], "cabs": []},
    ],
    "cab_ok": [
        {"op": "members", "raw": {"approval_agents": [{"id": "81", "selected": False}]}},
        {"op": "members", "raw": {"approval_users": [{"id": "81", "selected": False}]}},
        {"op": "members", "raw": {"approval_agents": [{"id": 81}], "approval_users": [{"id": "81"}]}},
    ],
    "cab_bad": [
        {"op": "members", "raw": {"approval_agents": []}},
        {"op": "members", "raw": {"approval_agents": [{"id": 81}, {"id": "81"}]}},
        {"op": "members", "raw": {"approval_agents": [{"id": 81, "deactivated": True}]}},
        {"op": "members", "raw": {"approval_agents": [{"id": 81}], "meta": {"count": 2}}},
        {"op": "members", "raw": {"approval_agents": [{"id": 81}], "meta": {"partial": True}}},
        {"op": "members", "raw": {"approval_agents": [{"id": 81}], "approval_users": [{"id": 82}]}},
        {"op": "members", "raw": {"approval_agents": None, "approval_users": [{"id": 81}]}},
    ],
}


def main():
    doc = {"cases": {name: {"ops": ops, "outputs": run(ops)} for name, ops in CASES.items()}}
    json.dump(stamped(doc), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
