"""Golden custom-field readbacks and attachment result identities on initial owners.

    mkdir -p /tmp/fsv-py-291e5cd
    git -C /Users/phusit.somb/lab/work/fsv archive 291e5cd | tar -x -C /tmp/fsv-py-291e5cd
    cd /tmp/fsv-py-291e5cd && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-291e5cd FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 291e5cd) \\
        uv run --offline --project /tmp/fsv-py-291e5cd \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_attachment_identities.py \\
        > /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-api/src/testdata/attachment_identities_oracle.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-aid-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import attachments as flow  # noqa: E402
from fsv import config  # noqa: E402
from fsv.readback import custom_fields_match  # noqa: E402
from fsv.reviewed_write import WritePlan  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

PAYLOAD = {"subject": "Acme", "email": "jane.doe@example.com",
           "custom_fields": {"count": 12, "requester_lookup": "991"}}
UPLOADED = [501, 502]


class Client:
    def __init__(self, row):
        self.row = row

    def int_get(self, path, params=None):
        assert path in {"tickets/123", "changes/123"}, path
        return {path.split("/")[0][:-1]: self.row}


ROW = {"id": 123, "attachments": [{"id": 501}, {"id": 502}], "subject": "Acme", "requester_id": 81,
       "email": "jane.doe@example.com",
       "custom_fields": {"count": "12.0", "requester_lookup": {"id": 991, "value": "Alex Acme"},
                         "extra": "server default"}}


def readback(name, custom, path="tickets", requester=81, drop=()):
    row = {**ROW, "custom_fields": custom}
    for key in drop:
        row.pop(key, None)
    base = WritePlan("ticket_create_attachments", 0, config.DOMAIN, "POST", path,
                     json.dumps(PAYLOAD, sort_keys=True), "synthetic")
    p = flow.AttachedPlan(base, (), 4096, requester_id=requester)
    key = "ticket" if path == "tickets" else "change"
    try:
        got = {"result": flow.readback(Client(row), p, {key: {"id": 123}}, UPLOADED)}
    except Exception as exc:  # noqa: BLE001
        got = {"error": type(exc).__name__}
    return {"name": name, "path": path, "requester": requester, "row": row, "outcome": got}


MATCH = [
    ({"lookup": {"id": 991, "value": "Alex"}}, {"lookup": "991"}),
    ({"lookup": {"id": 991, "value": "Alex"}}, {"lookup": 991}),
    ({}, {"lookup": "991"}),
    ({"lookup": {"id": 992, "value": "991"}}, {"lookup": "991"}),
    ({"lookup": True}, {"lookup": "991"}),
    ({"lookup": None}, {"lookup": "991"}),
    ({"lookup": {"id": 991, "value": "Acme", "unknown": "shape"}}, {"lookup": "991"}),
    ({"lookup": {"id": 991, "value": "Alex"}}, {"lookup": {"id": 991}}),
    ({"lookup": {"id": 991, "value": "Alex"}}, {"lookup": None}),
    ({"lookup": {"id": True, "value": "Alex"}}, {"lookup": "True"}),
    ({"count": "12.0", "extra": 1}, {"count": 12}),
    ({"count": "12.0"}, {}),
    (None, {"count": 12}),
    ({"count": 12}, None),
    ([], []),
]


def main():
    cases = {
        "matches": [{"actual": a, "expected": e, "matched": custom_fields_match(a, e)} for a, e in MATCH],
        "readbacks": [
            readback("normalized", ROW["custom_fields"]),
            readback("change-owner", ROW["custom_fields"], path="changes"),
            readback("no-plan-requester", ROW["custom_fields"], requester=None),
            readback("wrong-lookup", {**ROW["custom_fields"], "requester_lookup": {"id": 992, "value": "991"}}),
            readback("missing-key", {"count": 12}),
            readback("not-object", "x"),
            readback("absent", None, drop=("custom_fields",)),
        ],
    }
    json.dump(stamped(cases), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
