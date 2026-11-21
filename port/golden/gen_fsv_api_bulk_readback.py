"""Golden bounded bulk readback: lagging exact reads re-run only the verifier.

Spam and delete over explicit ids. The first `lag` post-write reads return
the pre-write row; sleeps are recorded, never slept.

    mkdir -p /tmp/fsv-py-aa8df39
    git -C /Users/phusit.somb/lab/work/fsv archive aa8df39 | tar -x -C /tmp/fsv-py-aa8df39
    cd /tmp/fsv-py-aa8df39 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-aa8df39 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse aa8df39) \\
        uv run --offline --project /tmp/fsv-py-aa8df39 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_bulk_readback.py \\
        > /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-api/src/testdata/bulk_readback_oracle.json
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
from pathlib import Path
from urllib.parse import parse_qs

BOOT = tempfile.mkdtemp(prefix="fsv-bulkrb-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

import httpx  # noqa: E402

from fsv import config, journal, readback  # noqa: E402
from fsv.client import Client  # noqa: E402
from fsv.reviewed_write import ReviewedWriteError  # noqa: E402
from fsv.ticket_bulk import build_plan, send  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()


def world(lag):
    s = {"rows": {i: {"id": i, "status": 2, "spam": False, "deleted": False} for i in (71, 72)},
         "writes": [], "stale": lag, "post_reads": 0, "sleeps": []}

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET" and path.startswith("tickets/"):
            row = copy.deepcopy(s["rows"][int(path.rsplit("/", 1)[1])])
            if s["stale"] and s["writes"]:
                s["stale"] -= 1
                row.update(status=2, spam=False, deleted=False)
            s["post_reads"] += bool(s["writes"])
            return httpx.Response(200, json={"ticket": row})
        s["writes"].append((req.method, path))
        if path.endswith("delete_multiple"):
            ids, change = parse_qs(req.url.query.decode())["ids"][0].split(","), {"deleted": True}
        else:
            ids, change = json.loads(req.content)["ids"], {"spam": path.endswith("/spam_multiple")}
        for i in ids:
            s["rows"][int(i)].update(change)
        return httpx.Response(200, json={})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    c._csrf = "synthetic"
    readback._sleep = lambda seconds: s["sleeps"].append(seconds)
    journal.journal_path = lambda p=Path(tempfile.mkdtemp(dir=BOOT)) / "journal": p
    return c, s


def case(action, lag):
    c, s = world(lag)
    plan = build_plan(c, ["71", "72"], action=action)
    result = send(plan, c, reviewed_hash=plan.hash, confirm_destructive=True)
    try:
        send(plan, c, reviewed_hash=plan.hash, confirm_destructive=True)
        replay = None
    except ReviewedWriteError as exc:
        replay = str(exc)
    return {"action": action, "lag": lag, "hash": plan.hash, "write_hash": plan.write.hash,
            "result": result, "sleeps": s["sleeps"], "post_reads": s["post_reads"],
            "writes": len(s["writes"]), "replay": replay}


def main():
    cases = [case(action, lag) for action in ("spam", "delete") for lag in (0, 2, 100)]
    json.dump(stamped({"cases": cases}), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
