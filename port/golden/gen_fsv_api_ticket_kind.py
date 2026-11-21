"""Golden `ticket_kind.exact_ticket_type` over native and echo type keys.

    mkdir -p /tmp/fsv-py-97deaf9
    git -C /Users/phusit.somb/lab/work/fsv archive 97deaf9 | tar -x -C /tmp/fsv-py-97deaf9
    cd /tmp/fsv-py-97deaf9 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-97deaf9 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 97deaf9) \\
        uv run --offline --project /tmp/fsv-py-97deaf9 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_ticket_kind.py \\
        > /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-api/src/testdata/ticket_kind_oracle.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-kind-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import config  # noqa: E402
from fsv.reviewed_write import ReviewedWriteError  # noqa: E402
from fsv.ticket_kind import exact_ticket_type  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

RECORDS = [
    {"type": "Incident"}, {"ticket_type": "Incident"}, {"type": "Incident", "ticket_type": "incident"},
    {"type": "Service Request"}, {"type": "Incident", "ticket_type": "Service Request"},
    {}, {"type": ""}, {"type": 1}, {"type": "   "}, {"type": None}, {"type": None, "ticket_type": "Incident"},
    {"type": "service_request", "ticket_type": "Service  Request"}, {"type": " INCIDENT "},
    {"type": "Incident", "ticket_type": ""}, {"type": "Incident", "ticket_type": 7},
    {"type": ["Incident"]}, {"type": "Straße"}, {"type": "STRASSE", "ticket_type": "straße"},
    None, [], "Incident",
]


def run(record):
    try:
        return {"ok": exact_ticket_type(record)}
    except ReviewedWriteError as exc:
        return {"err": str(exc)}


def main():
    cases = [{"record": r, **run(r)} for r in RECORDS]
    json.dump(stamped({"cases": cases}), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
