"""Golden strict mutation inventories: rejected raw pages, multi-page and single-page shapes.

    mkdir -p /tmp/fsv-py-57a236e
    git -C /Users/phusit.somb/lab/work/fsv archive 57a236e | tar -x -C /tmp/fsv-py-57a236e
    cd /tmp/fsv-py-57a236e && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-57a236e FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 57a236e) \\
        uv run --offline --project /tmp/fsv-py-57a236e \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_reviewed_inventory.py
"""
from __future__ import annotations

import copy
import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-inv-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import config  # noqa: E402
from fsv.reviewed_inventory import complete_inventory, single_page_inventory  # noqa: E402
from fsv.reviewed_write import ReviewedWriteError  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()


class Pages:
    def __init__(self, pages):
        self.pages = pages
        self.requests = []

    def int_get(self, path, params=None):
        self.requests.append([path, dict(params or {})])
        page = (params or {}).get("page", 1)
        return copy.deepcopy(self.pages[page - 1])

    v2_get = int_get


REJECTED = [
    {"rows": [{"id": 71}, "malformed"], "has_more": False},
    {"rows": [{"id": 71}, {"id": 71}], "has_more": False},
    {"rows": [{"id": True}], "has_more": False},
    {"rows": [{"id": "superscript"}], "has_more": False},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total": 2}},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total": "1"}},
    {"rows": [{"id": 71}], "has_more": "false"},
    {"rows": [{"id": 71}], "has_more": False, "meta": "malformed"},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"has_more": True}},
    {"rows": [{"id": 71}], "has_more": True, "next_page_url": None},
    {"rows": [{"id": 71}], "has_more": False, "next_page_url": "?page=2"},
    {"rows": [{"id": 71}], "next_page": 2},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"page": True}},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total_pages": 2}},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"total_pages": 0}},
    {"rows": [{"id": 0}], "has_more": False},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"page": 2}},
    {"rows": [{"id": 71}], "has_more": False, "meta": {"next_cursor": ""}},
    {"rows": [{"id": 71}], "has_more": False, "next_cursor": "a", "meta": {"next_cursor": "b"}},
    {"rows": [{"id": 71}], "has_more": True, "meta": {"has_next": False}},
]

ACCEPTED = [
    {"rows": [{"id": "72"}, {"id": 71}], "has_more": False, "meta": {"total": 2, "page": 1, "total_pages": 1}},
    {"rows": [], "meta": {"page": 1, "total_pages": 0, "total": 0}},
    {"rows": [{"id": 71}]},
]


def outcome(call):
    try:
        return {"rows": call()}
    except ReviewedWriteError as exc:
        return {"error": str(exc)}


def paged(name, pages, params, api="internal"):
    c = Pages(pages)
    got = outcome(lambda: complete_inventory(c, "example", "rows", params, api=api))
    return {"name": name, "pages": pages, "params": params, "api": api, "requests": c.requests, "outcome": got}


def main():
    cases = []
    for i, raw in enumerate(REJECTED + ACCEPTED):
        cases.append(paged(f"single-page-{i}", [raw], {"page": 1, "per_page": 100}))
    for second in [{"id": 71}, {"id": 72}]:
        cases.append(paged(f"conflicting-{second['id']}",
                           [{"rows": [{"id": 71}], "has_more": True, "total": 2},
                            {"rows": [second], "has_more": False, "total": 3}],
                           {"page": 1, "per_page": 1}))
    for api in ("internal", "v2"):
        cases.append(paged(f"multi-{api}",
                           [{"rows": [{"id": 72}], "has_more": True, "meta": {"total_count": 2}},
                            {"rows": [{"id": 71}], "has_more": False, "meta": {"total_count": 2}}],
                           {"page": 1, "per_page": 1, "query_hash": "synthetic"}, api))
    cases.append(paged("no-page-param",
                       [{"rows": [{"id": 71}], "has_more": False, "meta": {"page": 1}}],
                       {"include": "user", "per_page": 100}))
    for i, metadata in enumerate([{"total_count": 2}, {"has_more": True}, {"next_page_url": "?page=2"},
                                  {"total_pages": 2}]):
        raw = {"rows": [{"id": 71}], "meta": metadata}
        cases.append({"name": f"single-claims-{i}", "single": True, "raw": raw,
                      "outcome": outcome(lambda raw=raw: single_page_inventory(raw, "rows"))})
    for i, raw in enumerate([{"rows": [{"id": 71}]}, {"rows": [], "meta": {"page": 1, "total_pages": 0, "total": 0}},
                             {"rows": [{"id": 9}, {"id": 3}], "meta": {"e_signature_limit_reached": False}},
                             {"other": []}]):
        cases.append({"name": f"single-ok-{i}", "single": True, "raw": raw,
                      "outcome": outcome(lambda raw=raw: single_page_inventory(raw, "rows"))})
    json.dump(stamped({"cases": cases}), sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
