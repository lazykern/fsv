"""Golden change-relationship wires for every captured kind and action.

    mkdir -p /tmp/fsv-py-1490984
    git -C /Users/phusit.somb/lab/work/fsv archive 1490984 | tar -x -C /tmp/fsv-py-1490984
    cd /tmp/fsv-py-1490984 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-1490984 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 1490984) \\
        uv run --offline --project /tmp/fsv-py-1490984 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_change_relationships.py
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-rel-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import change_relationships as flow  # noqa: E402
from fsv import config  # noqa: E402
from fsv.client import Client  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

PROJECT = {
    "name": "Acme project",
    "description": "Acme work",
    "start_date": "2026-01-01",
    "end_date": "2026-01-02",
    "owner_email": "person@example.com",
    "project_category": "1",
}


def seed(kind, action):
    rows = {name: [] for name in flow.KINDS}
    if kind == "project-task":
        rows["project"] = [{"id": 81, "key": "ACME", "name": "Acme project"}]
    if action == "remove":
        rows[kind] = [{
            "id": 81 if kind == "project" else 91 if kind == "project-task" else 71,
            "key": "ACME" if kind == "project" else "ACME-1",
            "name": "Acme task",
            "project_id": 81,
        }]
    return rows


def client(rows):
    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method != "GET":
            return httpx.Response(404, json={"error": path})
        if path == "changes/123":
            return httpx.Response(200, json={"change": {"id": 123, "subject": "Acme change"}})
        if path.startswith("freshrelease"):
            kind = "project" if path.endswith("fetch_linked_fr_projects") else "project-task"
            key = flow.KINDS[kind][0]
            return httpx.Response(200, json={
                "status": 200,
                "body": {key: rows[kind], "meta": {"page": 1, "total": len(rows[kind]), "total_pages": 1}},
            })
        if path.startswith("changes/123/"):
            key = path.rsplit("/", 1)[1]
            kind = next(k for k, pair in flow.KINDS.items() if pair[0] == key)
            return httpx.Response(200, json={key: rows[kind], "meta": {"has_next": False}})
        module, ident = path.split("/")
        return httpx.Response(200, json={module[:-1]: {"id": int(ident), "ticket_type": "Incident"}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    c._csrf = "synthetic"
    return c


def options(kind, action):
    if kind == "project":
        if action == "remove":
            return {"item": "ACME"}
        return {"project": PROJECT}
    if kind == "project-task":
        return {
            "item": "ACME-1",
            "project_key": "ACME",
            "name": "Acme task" if action == "add" else None,
        }
    return {"item": "71"}


def snap(kind, action):
    plan = flow.build_plan(client(seed(kind, action)), "CHN-123", kind=kind, action=action, **options(kind, action))
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


def main():
    doc = {}
    for kind in flow.KINDS:
        for action in ("add", "remove"):
            doc[f"{kind}-{action}"] = snap(kind, action)
    json.dump(stamped(doc), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
