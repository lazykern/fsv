"""Golden `filter_proof`: filterable-field guard, exact-row predicate proofs and list contradictions.

    mkdir -p /tmp/fsv-py-368a2d4
    git -C /Users/phusit.somb/lab/work/fsv archive 368a2d4 | tar -x -C /tmp/fsv-py-368a2d4
    cd /tmp/fsv-py-368a2d4 && uv sync --offline
    FSV_PY_SRC=/tmp/fsv-py-368a2d4 FSV_PY_COMMIT=$(git -C /Users/phusit.somb/lab/work/fsv rev-parse 368a2d4) \\
        uv run --offline --project /tmp/fsv-py-368a2d4 \\
        python /Users/phusit.somb/lab/work/fsv-rust/port/golden/gen_fsv_api_filter_proof.py \\
        > /Users/phusit.somb/lab/work/fsv-rust/crates/fsv-api/src/testdata/filter_proof_oracle.json
"""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

BOOT = tempfile.mkdtemp(prefix="fsv-proof-")
CFG = Path(BOOT) / "config"
CFG.mkdir()
(CFG / "config.json").write_text(json.dumps({"domain": "acme.freshservice.com"}))
os.environ["FSV_CONFIG_DIR"] = str(CFG)
os.environ["FSV_CACHE_DIR"] = str(Path(BOOT) / "cache")
os.environ["XDG_STATE_HOME"] = str(Path(BOOT) / "state")
os.environ["FSV_NO_KEYCHAIN"] = "1"

from fsv import config  # noqa: E402
from fsv.filter_proof import contradictions, matches, name_valued_choice, require_filterable_field  # noqa: E402
from fsv.query import build_query_hash_from_schema  # noqa: E402
from fsv.resources import TICKETS  # noqa: E402
from provenance import stamped  # noqa: E402

config.reload()

FIELDS = [
    {"name": "subject", "field_type": "default_subject"},
    {"name": "description", "field_type": "default_description"},
    {"name": "cf_note", "field_type": "custom_text"},
    {"name": "cf_tier", "field_type": "custom_text", "choices": [["Gold", 1]]},
    {"name": "cf_tier", "field_type": "custom_text", "choices": []},
    {"name": "cf_pick", "field_type": "nested_dropdown"},
    {"name": "cf_count", "field_type": "custom_number"},
    {"name": "priority", "field_type": "default_priority"},
    {"name": "status", "field_type": "default_status"},
    {"name": "group", "field_type": "custom_text"},
    {"name": "requester"},
    {"field_type": "custom_text"},
    {"name": 7, "field_type": None},
    {"name": "x", "field_type": 0},
]

PRI = {"condition": "priority", "operator": "is_in", "type": "default", "value": [1]}
NOT_PRI = {"condition": "priority", "operator": "is_not_in", "type": "default", "value": [1, "4"]}
CF = {"condition": "cf_tier", "operator": "is_in", "type": "custom_field", "value": ["Gold"]}

MATCHES = [
    ({"priority": 1}, [PRI]),
    ({"priority": "1"}, [PRI]),
    ({"priority": 4}, [PRI]),
    ({"priority": 2}, [NOT_PRI]),
    ({"priority": 4}, [NOT_PRI]),
    ({"priority": {"id": 1, "name": "Low"}}, [PRI]),
    ({"priority": {"id": 1, "label": "Low"}}, [PRI]),
    ({"priority": {"name": "Low"}}, [PRI]),
    ({"priority": None}, [PRI]),
    ({"priority": True}, [PRI]),
    ({"priority": 1.0}, [PRI]),
    ({"priority": [1]}, [PRI]),
    ({}, [PRI]),
    ({"priority": 1}, []),
    ({"priority": 1, "custom_fields": {"cf_tier": "Gold"}}, [PRI, CF]),
    ({"priority": 1, "custom_fields": {"cf_tier": "Silver"}}, [PRI, CF]),
    ({"priority": 1, "custom_fields": None}, [CF]),
    ({"priority": 1, "cf_tier": "Gold"}, [CF]),
    ({"priority": 1}, [{**PRI, "value": []}]),
    ({"priority": 1}, [{**PRI, "value": 1}]),
    ({"priority": 1}, [{**PRI, "value": [None]}]),
    ({"priority": 1}, [{**PRI, "value": [1.5]}]),
    ({"priority": 1}, [{**PRI, "operator": "custom_date"}]),
    ({"priority": 1}, [{**PRI, "type": "other"}]),
    ({"priority": 1}, [{**PRI, "condition": 3}]),
    ({"created_at": "x"}, [{"condition": "created_at", "operator": "custom_date"}]),
]

CONTRA = [
    ([{"id": 1, "priority": 1}, {"id": 2, "priority": 4}, {"priority": 3}, {"id": 4},
      {"id": 5, "priority": None}, {"id": None, "priority": 2}], [PRI]),
    ([{"id": 1, "priority": 1}], []),
]


TYPE = {"name": "ticket_type", "label": "Type", "default_field": True, "field_type": "default_ticket_type",
        "choices": [{"id": 3, "value": "Incident"}, {"id": 4, "value": "Service Request"},
                    {"id": 5, "value": "Retired", "deleted": True}]}
NAMED = [
    (TYPE, "Service Request"), (TYPE, " service request "), (TYPE, "4"), (TYPE, " 4 "), (TYPE, "Case"),
    (TYPE, "5"), (TYPE, "Retired"), (TYPE, "9"), (TYPE, ""),
    ({**TYPE, "choices": [{"id": 3, "value": "Incident"}, {"id": 6, "value": "incident"}]}, "Incident"),
    ({**TYPE, "choices": [{"id": 3, "value": "Incident"}, {"id": 6, "value": "Request"}]}, "Case"),
    ({**TYPE, "choices": [{"id": 3, "value": "Incident"}, {"id": "Incident", "value": "Request"}]}, "Incident"),
    ({**TYPE, "choices": [{"id": True, "value": "Incident"}]}, "True"),
    ({**TYPE, "choices": [{"id": 3, "value": " "}, {"id": 4, "value": 7}, "x", {"id": 8, "value": "Ops",
                                                                                "deactivated": 1}]}, "3"),
    ({**TYPE, "choices": [{"id": 3.0, "value": "Incident"}]}, "3.0"),
    ({**TYPE, "label": None}, "Case"),
    ({**TYPE, "label": "", "name": None}, "Case"),
    ({**TYPE, "choices": None}, "Incident"),
    ({**TYPE, "field_type": "default_status"}, "Incident"),
    ({**TYPE, "field_type": None}, "Incident"),
]
SR = {"condition": "ticket_type", "operator": "is_in", "type": "default", "value": ["Service Request"]}
TYPE_MATCHES = [
    ({"type": "Service Request"}, [SR]), ({"ticket_type": "Service Request"}, [SR]), ({"type": "Incident"}, [SR]),
    ({"type": "Service Request", "ticket_type": "Incident"}, [SR]), ({}, [SR]),
    ({"type": "service_request"}, [{**SR, "value": ["service request"]}]),
    ({"type": "service_request"}, [{**SR, "value": ["service_request"]}]),
    ({"type": "Incident"}, [{**SR, "operator": "is_not_in"}]),
    ({"type": "Incident"}, [{**SR, "value": [3]}]),
    ({"type": "Incident"}, [{**SR, "type": "custom_field"}]),
    ({"type": "Incident", "custom_fields": {"ticket_type": "Incident"}},
     [{**SR, "type": "custom_field", "value": ["Incident"]}]),
]
QUERIES = ["type=Service Request", "Type=service request", "ticket_type=4", "type!=Incident", "type=Case",
           "type=5", "type=Retired", "type=9"]


def run(fn):
    try:
        return {"ok": fn()}
    except ValueError as e:
        return {"err": str(e)}


def main() -> None:
    cases = {
        "filterable": [{"field": f, **run(lambda f=f: require_filterable_field(f))} for f in FIELDS],
        "matches": [{"row": r, "clauses": c, **run(lambda r=r, c=c: matches(r, c))} for r, c in MATCHES],
        "contradictions": [{"rows": r, "clauses": c, "ids": contradictions(r, c)} for r, c in CONTRA],
        "name_valued": [{"field": f, "raw": r, **run(lambda f=f, r=r: name_valued_choice(f, r))} for f, r in NAMED],
        "type_matches": [{"row": r, "clauses": c, **run(lambda r=r, c=c: matches(r, c))} for r, c in TYPE_MATCHES],
        "type_queries": [{"expr": e, **run(lambda e=e: json.loads(
            build_query_hash_from_schema(None, TICKETS, {"fields": [TYPE]}, [e])))} for e in QUERIES],
    }
    json.dump(stamped(cases), sys.stdout, indent=2)


if __name__ == "__main__":
    main()
