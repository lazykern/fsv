"""Golden `ticket_bulk` wires for spam, unspam and delete.

Explicit ids only. Status and assign stay on the real builders in Rust; this
generator does not monkeypatch them, so it does not record those wires.

    mkdir -p /tmp/fsv-py-e116be9
    git -C /Users/phusit.somb/lab/work/fsv archive e116be9 | tar -x -C /tmp/fsv-py-e116be9
    cd /tmp/fsv-py-e116be9 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-e116be9 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse e116be9) \\
        uv run --offline --project /tmp/fsv-py-e116be9 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_ticket_bulk.py
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-bulk-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import config  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.ticket_bulk import build_plan  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

ROWS = {
    71: {"id": 71, "status": 2, "spam": False, "deleted": False},
    72: {"id": 72, "status": 2, "spam": False, "deleted": False},
}


def client(rows):
    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET" and path.startswith("tickets/"):
            ticket_id = int(path.rsplit("/", 1)[1])
            return httpx.Response(200, json={"ticket": rows[ticket_id]})
        return httpx.Response(404, json={"error": path})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    c._csrf = "synthetic"
    return c


def snap(action, ids):
    plan = build_plan(client(ROWS), ids, action=action)
    return {
        "hash": plan.hash,
        "write_hash": plan.write.hash,
        "action": plan.write.action,
        "method": plan.write.method,
        "path": plan.write.path,
        "payload": plan.write.payload_json,
        "tenant": plan.write.tenant,
        "target": plan.write.target,
        "context": plan.write.context_hash,
    }


def main():
    doc = {
        "spam": snap("spam", ["INC-72", "INC-71"]),
        "unspam": snap("unspam", ["71"]),
        "delete": snap("delete", ["71", "72"]),
    }
    json.dump(stamped(doc), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
