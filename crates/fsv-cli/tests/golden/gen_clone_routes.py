#!/usr/bin/env python3
"""Write tests/fixtures/routes/clone_*.json: the `clone_wire` server of
Python tests/test_clone_failure_matrix.py as canned routes.

    python3 crates/fsv-cli/tests/golden/gen_clone_routes.py

`clone_ok` answers every step, `clone_noid` creates a change without an id,
and `clone_<step>_<status>` fails that one step with `status`. The Python
test patches `_get_change_schema_fields` to `[]`; an empty schema read would
fall back to `/api/v2`, so the form answers the subject field only.
"""
import json
import pathlib

ROUTES = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "routes"
DASHBOARD = {"method": "GET", "path": "/a/dashboard",
             "text": '<html><head><meta name="csrf-token" content="tok"></head></html>'}
READS = {
    "changes/123": {"change": {"id": 123, "subject": "Acme maintenance", "status": 47,
                               "attachments": [{"id": 11}], "state_flow_id": 27,
                               "attached_risk_survey_response_status": "completed"}},
    "changes/123/tasks": {"tasks": [{"id": 501, "title": "Verify Acme", "owner_id": 71, "status": 3}]},
    "changes/123/assets": {"associated_assets": [{"id": 801, "config_item": {"display_id": 701}}]},
    "changes/123/planning-fields": {"change_planning_fields": [
        {"name": "test_plan", "description": "Acme checks", "attachments": []}]},
    "change_form_fields": {"change_fields": [
        {"id": 1, "name": "subject", "label": "Subject", "field_type": "default_subject",
         "default_field": True, "required": True}]},
}
WRITES = {
    "base": ("POST", "changes"),
    "tasks": ("POST", "changes/456/tasks"),
    "assets": ("PUT", "changes/456/assets/associate"),
    "planning": ("POST", "changes/456/planning-fields"),
}
FAILURES = [("tasks", 403), ("assets", 409), ("planning", 429), ("tasks", 500)]


def routes(fail=None, status=500, missing_id=False):
    out = [{"path": f"/api/_/{p}", "json": j} for p, j in READS.items()]
    for kind, (method, path) in WRITES.items():
        r = {"method": method, "path": f"/api/_/{path}"}
        if kind == fail:
            r.update(status=status, json={"description": "synthetic denial"})
        elif kind == "base":
            r["json"] = {"change": {} if missing_id else {"id": 456, "human_display_id": "CHN-456"}}
        else:
            r["json"] = {"task": {"id": 901}}
        out.append(r)
    return out + [DASHBOARD]


def write(name, rs):
    (ROUTES / f"{name}.json").write_text(json.dumps(rs, indent=1) + "\n")


write("clone_ok", routes())
write("clone_noid", routes(missing_id=True))
for step, status in FAILURES:
    write(f"clone_{step}_{status}", routes(step, status))
