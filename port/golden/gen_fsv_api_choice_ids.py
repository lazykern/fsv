"""Golden runtime choice identities and the merge Closed status over string ids.

    mkdir -p /tmp/fsv-py-7acec01
    git -C /Users/phusit.somb/lab/work/fsv archive 7acec01 | tar -x -C /tmp/fsv-py-7acec01
    cd /tmp/fsv-py-7acec01 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-7acec01 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 7acec01) \\
        uv run --offline --project /tmp/fsv-py-7acec01 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_choice_ids.py \\
        > /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-api/src/testdata/choice_ids_oracle.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile

BOOT = tempfile.mkdtemp(prefix="fsv-cid-")
os.makedirs(os.path.join(BOOT, "config"))
with open(os.path.join(BOOT, "config", "config.json"), "w") as fh:
    json.dump({"domain": "acme.freshservice.com"}, fh)
os.environ["FSV_CONFIG_DIR"] = os.path.join(BOOT, "config")
os.environ["FSV_CACHE_DIR"] = os.path.join(BOOT, "cache")
os.environ["XDG_STATE_HOME"] = os.path.join(BOOT, "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import config, formmeta, ticket_merge as flow  # noqa: E402
from fsv.choice_ids import runtime_choice_id  # noqa: E402
from fsv.client import APIError  # noqa: E402
from fsv.reviewed_write import ReviewedWriteError  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()
formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: 1

IDS = [(1, 1), ("1", 1), ("0", 0), (0, 0), ("0", 1), (0, 1), ("900", 0), (True, 0), (False, 0), ("-1", 0),
       (-1, 0), (" 1", 0), ("1.0", 0), (1.0, 0), ("\u0661", 0), (None, 0), ("", 0), ("007", 1), ("+1", 0)]


def status_field(ident):
    return {"id": 9000007101, "name": "status", "label": "Status", "description": "", "field_type": "default_status",
            "required": False, "required_for_closure": False, "position": 1, "default_field": True,
            "choices": [{"id": "31", "value": "Open"}, {"id": ident, "value": "Closed"}], "nested_fields": [],
            "placeholder": "", "helptext": "", "hint": False, "content": None, "displayed_to_agents": True,
            "data_source": 0, "lookup_config": None, "workspace_id": 1, "updated_at": "2030-01-02T03:04:05Z"}


class Fake:
    def __init__(self, ident):
        self.ident = ident

    def int_get(self, path, params=None):
        if path == "ticket_form_fields":
            return {"ticket_fields": [status_field(self.ident)]}
        if path.endswith("business_rules"):
            return {"business_rules": []}
        raise APIError(404, "missing", "GET", path)


def closed(ident):
    try:
        status, _ = flow._closed(Fake(ident), 123)
        return {"ok": status}
    except ReviewedWriteError as exc:
        return {"err": str(exc)}


def main():
    out = {
        "ids": [{"value": v, "minimum": m, "expected": runtime_choice_id(v, minimum=m)} for v, m in IDS],
        "closed": [{"id": i, **closed(i)} for i in ["89", 89, "0", "8a", True, "-89", 89.0, None]],
    }
    json.dump(stamped(out), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
