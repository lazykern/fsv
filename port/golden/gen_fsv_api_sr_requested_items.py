"""Golden service-request requested-item plans, refusals and readbacks.

    mkdir -p /tmp/fsv-py-6c6b767
    git -C /Users/phusit.somb/lab/work/fsv archive 6c6b767 | tar -x -C /tmp/fsv-py-6c6b767
    cd /tmp/fsv-py-6c6b767 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-6c6b767 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 6c6b767) \\
        uv run --offline --project /tmp/fsv-py-6c6b767 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_sr_requested_items.py
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-sri-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import config, journal  # noqa: E402
from fsv import sr_requested_items as flow  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.reviewed_write import ReviewedWriteError  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

FIELDS = [
    {"id": i, "name": name, "label": name.title(), "field_type": typ, "type": 1001 if name == "title" else 1008,
     "required": name == "title", "field_options": {}, "position": i}
    for i, name, typ in [(1, "title", "custom_text"), (2, "details", "custom_paragraph")]
]
ITEM = {"id": 801, "display_id": 71, "name": "Acme service", "workspace_id": 2, "custom_fields": FIELDS}


def world():
    return {
        "rows": [{"id": 901, "ticket_id": 123, "item_id": 801, "item": ITEM, "is_parent": True,
                  "stage": {"id": 1, "name": "Requested"}}],
        "answers": {901: {"title": "Original", "details": "Original details"}},
        "writes": [], "blocked": False, "missing": False, "rules_fail": False,
    }


def client(s):
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "tickets/123":
                return httpx.Response(200, json={"ticket": {"id": 123, "ticket_type": "Service Request"}})
            if "business_rules" in path:
                if s["rules_fail"]:
                    return httpx.Response(403, json={"error": "denied"})
                return httpx.Response(200, json={"business_rules": []})
            if path == "catalog/service-items/71":
                return httpx.Response(200, json={"service_item": ITEM})
            if path == "tickets/123/requested_items":
                return httpx.Response(200, json={"requested_items": copy.deepcopy(s["rows"]),
                                                 "meta": {"e_signature_limit_reached": False}})
            ident = int(path.rsplit("/", 1)[1])
            answers = copy.deepcopy(s["answers"][ident])
            if s["missing"] and s["writes"]:
                answers.pop("details", None)
            return httpx.Response(200, json={
                "requested_item": {"id": ident, "item_id": 71, "custom_fields": answers,
                                   "item": {"custom_fields": FIELDS, "description": "Acme item"}},
                "business_rules": {
                    "hidden_req_item_fields": {str(ident): ["details"] if s["blocked"] else []},
                    "non_editable_item_fields": {str(ident): []},
                },
            })
        body = json.loads(req.content) if req.content else None
        s["writes"].append([req.method, path, body])
        if req.method == "POST":
            s["rows"].append({"id": 902, "ticket_id": 123, "item_id": 801, "item": ITEM, "is_parent": False,
                              "stage": {"id": 1, "name": "Requested"}})
            s["answers"][902] = body["custom_fields"]
            return httpx.Response(200, json={"requested_item": {"id": 902, "item_id": 71}})
        ident = int(path.split("/")[3])
        if req.method == "DELETE":
            s["rows"] = [r for r in s["rows"] if r["id"] != ident]
            return httpx.Response(204, headers={"content-type": "application/json"})
        if req.method == "PATCH":
            next(r for r in s["rows"] if r["id"] == ident)["stage"] = {"id": 2, "name": "Delivered"}
        else:
            s["answers"][ident].update(body["custom_fields"])
        return httpx.Response(200, json={"requested_item": {"id": ident, "item_id": 801}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    return c


def opts(action):
    if action == "add":
        return {"catalog_item": "71", "answers": {"title": "Acme request", "details": "Additional item"}}
    if action == "edit":
        return {"item": 901, "answers": {"title": "Acme edited", "details": "Updated details"}}
    return {"item": 902 if action == "remove" else 901, **({"stage": 2} if action == "stage" else {})}


def snap(plan):
    return {
        "hash": plan.hash,
        "action": plan.action,
        "method": plan.method,
        "path": plan.path,
        "payload": plan.payload_json,
        "tenant": plan.tenant,
        "target": plan.target,
        "context": plan.context_hash,
    }


def wire(action, missing=False):
    journal.journal_path = lambda p=Path(tempfile.mkdtemp(dir=BOOT)) / "journal": p
    s = world()
    if action == "remove":
        s["rows"].append({**copy.deepcopy(s["rows"][0]), "id": 902, "is_parent": False})
    c = client(s)
    plan = flow.build_plan(c, "SR-123", action=action, **opts(action))
    s["missing"] = missing
    result = flow.send(plan, c, reviewed_hash=plan.hash)
    return {"plan": snap(plan), "writes": s["writes"],
            "result": {k: result.get(k) for k in ("item_id", "fields", "verified") if k in result}}


def refusal(mode, call):
    s = world()
    if mode:
        s[mode] = True
    try:
        call(client(s))
    except (ReviewedWriteError, ValueError) as exc:
        return {"kind": type(exc).__name__, "message": str(exc)}
    return None


def main():
    doc = {action: wire(action) for action in ("edit", "stage", "add", "remove")}
    doc["edit-missing"] = wire("edit", missing=True)
    doc["refusals"] = {
        "blocked": refusal("blocked", lambda c: flow.build_plan(c, 123, action="edit", **opts("edit"))),
        "rules_fail": refusal("rules_fail", lambda c: flow.build_plan(c, 123, action="edit", **opts("edit"))),
        "parent_remove": refusal(None, lambda c: flow.build_plan(c, 123, action="remove", item=901)),
        "stage_3": refusal(None, lambda c: flow.build_plan(c, 123, action="stage", item=901, stage=3)),
        "hidden_type": refusal(None, lambda c: flow.build_plan(
            c, 123, action="edit", item=901, answers={"nope": "x"})),
        "blank": refusal(None, lambda c: flow.build_plan(
            c, 123, action="edit", item=901, answers={"title": "  "})),
        "add_bad": refusal(None, lambda c: flow.build_plan(
            c, 123, action="add", catalog_item="x71", answers={"title": "t"})),
        "bad_action": refusal(None, lambda c: flow.build_plan(c, 123, action="move", item=901)),
    }
    json.dump(stamped(doc), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
