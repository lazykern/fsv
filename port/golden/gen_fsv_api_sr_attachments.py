"""Golden `cli_sr._attachment_rows` (Python 25bf9d7) for fsv-api
`downloads::attachment_rows`, which the TUI download picker calls with one
empty label map per requested item.

    cd /tmp/fsv-py-25bf9d7 && FSV_PY_SRC=$PWD FSV_PY_COMMIT=25bf9d7... uv run python \
        /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_sr_attachments.py
"""
from __future__ import annotations

import json
import sys

from fsv import cli_sr
from provenance import stamped

GENERAL = {"id": 9901, "name": "overview.txt", "size": 11, "attachment_url": "https://example.com/file"}
FIELD = {"id": 9902, "name": "field.txt", "size": 12}
CASES = {
    "python_test": ({"attachments": []}, [{"attachments": [GENERAL, FIELD], "custom_fields": {"brief": [FIELD]}}]),
    "dedupe": ({"attachments": [{"id": 1, "name": "a"}]},
               [{"attachments": [{"id": 1, "name": "dup"}, {"name": "noid"}, "junk", {"id": 2, "name": "b"}],
                 "custom_fields": {"f": [{"id": 2, "name": "b2"}, 7]}},
                {"attachments": [{"id": 2, "name": "again"}, {"id": None, "name": "nullid"}]}]),
    "no_general": ({"attachments": [{"id": 5, "name": "r"}]}, [{"custom_fields": {"f": [{"id": 5}]}}]),
}
out = {name: {"record": rec, "items": items,
              "rows": cli_sr._attachment_rows(rec, items, [{} for _ in items])}
       for name, (rec, items) in CASES.items()}
json.dump(stamped(out), sys.stdout, indent=1, ensure_ascii=False)
