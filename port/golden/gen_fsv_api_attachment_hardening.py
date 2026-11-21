"""Golden attachment plan identity with prior guards/requester, initial-owner and sender readbacks.

    mkdir -p /tmp/fsv-py-57a236e
    git -C /Users/phusit.somb/lab/work/fsv archive 57a236e | tar -x -C /tmp/fsv-py-57a236e
    cd /tmp/fsv-py-57a236e && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-57a236e FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 57a236e) \\
        uv run --offline --project /tmp/fsv-py-57a236e \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_attachment_hardening.py
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-ath-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import attachments as flow  # noqa: E402
from fsv import config  # noqa: E402
from fsv.reviewed_write import WritePlan  # noqa: E402
from fsv.ticket_conversation import sender_readback  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

PAYLOAD = {"subject": "Acme", "email": "jane.doe@example.com", "planned_start_date": "2030-01-02T10:00:00-05:00",
           "cc_emails": ["person@example.com"]}


def base(path="tickets"):
    return WritePlan("ticket_create_attachments", 0, config.DOMAIN, "POST", path,
                     json.dumps(PAYLOAD, sort_keys=True), "synthetic")


def plan(path="tickets", guards=(), requester=None):
    return flow.AttachedPlan(base(path), (), 4096, prior_guards=guards, requester_id=requester)


class Client:
    def __init__(self, row):
        self.row = row

    def int_get(self, path, params=None):
        assert path in {"tickets/123", "changes/123"}, path
        return {path.split("/")[0][:-1]: self.row}


ROW = {"id": 123, "attachments": [], "subject": "Acme", "requester_id": 81, "email": "Jane.Doe@Example.com",
       "planned_start_date": "2030-01-02T15:00:00Z", "cc_emails": ["Person <PERSON@example.com>"]}


def readback(name, row_changes, requester=81, path="tickets", drop=()):
    row = {**ROW, **row_changes}
    for key in drop:
        row.pop(key, None)
    p = plan(path, requester=requester)
    key = "ticket" if path == "tickets" else "change"
    try:
        got = {"result": flow.readback(Client(row), p, {key: {"id": 123}}, [])}
    except Exception as exc:  # noqa: BLE001
        got = {"error": type(exc).__name__}
    return {"name": name, "path": path, "requester": requester, "row": row, "outcome": got}


def main():
    guards = (("create", "a" * 64), ("ticket_create_attachments", "b" * 64))
    identities = []
    for name, p in [("plain", plan()), ("guards", plan(guards=guards)), ("requester", plan(requester=81)),
                    ("both", plan(guards=guards, requester=81))]:
        identities.append({"name": name, "hash": p.hash, "review": p.review()})
    readbacks = [
        readback("normalized", {}),
        readback("wrong-requester", {"requester_id": 999}),
        readback("conflicting-requester", {"requester": {"id": 81, "email": "wrong@example.com"}}),
        readback("nested-only", {"requester": {"id": 81, "email": "JANE.DOE@example.com"}}, drop=("requester_id", "email")),
        readback("no-requester-fields", {}, drop=("requester_id", "email")),
        readback("bad-requester-id", {"requester_id": "x"}),
        readback("no-plan-requester", {}, requester=None),
        readback("change-owner", {}, path="changes"),
        readback("cc-mismatch", {"cc_emails": ["other@example.com"]}),
        readback("instant-mismatch", {"planned_start_date": "2030-01-02T10:00:00Z"}),
    ]
    senders = []
    for row, expected in [(None, "support@example.com"), ({}, "support@example.com"),
                          ({"from_email": '"Acme Support" <SUPPORT@example.com>'}, "support@example.com"),
                          ({"from_email": "other@example.com"}, "support@example.com"),
                          ({"from_email": "support@example.com"}, "not an email")]:
        senders.append({"row": row, "expected": expected, "state": sender_readback(row, expected)})
    json.dump(stamped({"identities": identities, "readbacks": readbacks, "senders": senders}), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
