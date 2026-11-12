"""Golden required change-task fields (Python 780c002).

    cd /tmp/fsv-py-780c002 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=780c002... uv run --offline python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_change_tasks_required.py \
        > crates/fsv-api/src/testdata/change_tasks_required_oracle.json
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

from fsv import change_tasks as flow  # noqa: E402
from fsv import config  # noqa: E402
from provenance import stamped  # noqa: E402

config.DOMAIN = "acme.example.com"

FIELDS = [
    {"name": "title", "label": "Title", "default": True, "field_options": {"required_for_create": "true"}},
    {"name": "workspace_id", "label": "Workspace", "default": True, "field_options": {"required_for_create": "true"}},
    {"name": "cf_system", "label": "System", "default": False, "field_options": {"required_for_create": "true"}, "choices": [{"id": "17", "value": "Acme"}]},
    {"name": "cf_environment", "label": "Environment", "default": False, "field_options": {"required_for_create": "true"}, "choices": [{"id": "29", "value": "Test"}]},
    {"name": "planned_start_date", "label": "Planned Start", "default": True, "field_options": {"required_for_create": "true"}},
    {"name": "planned_end_date", "label": "Planned End", "default": True, "field_options": {"required_for_create": "true"}},
    {"name": "status", "choices": [{"id": "7", "value": "Open"}]},
    {"name": "notify_before", "choices": [{"id": "0", "value": "Never"}]},
]


def plan_out(plan):
    return {
        "hash": plan.hash,
        "payload": plan.payload,
        "contract": plan.review()["contract"],
        "fields_json": plan.fields_json,
        "custom_fields_json": plan.custom_fields_json,
    }


def main():
    missing = None
    try:
        flow.build_plan(23, "Acme", status_id=7, fields=FIELDS)
    except Exception as exc:  # noqa: BLE001
        missing = {"error": {"type": type(exc).__name__, "msg": str(exc)}}
    kw = dict(
        status_id=7,
        fields=FIELDS,
        custom_fields={"system": 17, "environment": 29},
        planned_start="2026-12-01T10:00:00Z",
        planned_end="2026-12-01T11:00:00Z",
    )
    plan = flow.build_plan(23, "Acme", **kw)
    changed = copy.deepcopy(FIELDS)
    changed[2]["choices"].append({"id": "31", "value": "Globex"})
    other = flow.build_plan(23, "Acme", **{**kw, "fields": changed})
    bare = flow.build_plan(23, "Acme", status_id=7)
    empty = flow.build_plan(23, "Acme", status_id=7, fields=[])
    doc = {
        "fields": FIELDS,
        "missing": missing,
        "bound": plan_out(plan),
        "changed_hash": other.hash,
        "bare_hash": bare.hash,
        "empty_fields_hash": empty.hash,
    }
    json.dump(stamped(doc), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
