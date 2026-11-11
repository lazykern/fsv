"""Expected values for the fsv-api reviewed-write tests (reviewed_write, watchers,
problem_known_error, planning_text, ticket_time, bulk_priority, ticket_conversation,
ticket_properties answers).

Run from the Python checkout:

    cd /Users/phusit.somb/lab/work/fsv && uv run python \
        ../fsv-rust/port/golden/gen_fsv_api_reviewed.py \
        > ../fsv-rust/crates/fsv-api/src/testdata/reviewed_oracle.json
"""
from __future__ import annotations

import json
import os
import tempfile

os.environ.setdefault("FSV_CACHE_DIR", tempfile.mkdtemp())

from fsv import (bulk_priority, config, formmeta, planning_text, problem_known_error,  # noqa: E402
                 ticket_conversation, ticket_properties, ticket_time, watchers)
from fsv.errors import APIError  # noqa: E402
from fsv.reviewed_write import WritePlan  # noqa: E402

formmeta._save = lambda *a, **k: None
formmeta.configured_workspace = lambda: None
DOMAIN = "acme.freshservice.com"


def form_field(id_, name, label, field_type, position, **extra):
    """Mirror of fsv-api `service::testing::form_field`."""
    default = field_type.startswith("default_")
    f = {"id": id_, "name": name, "label": label, "description": "", "field_type": field_type,
         "required": False, "required_for_closure": False, "position": position,
         "default_field": default, "choices": [], "nested_fields": [],
         "placeholder": "", "helptext": "", "hint": False,
         "content": None, "displayed_to_agents": True, "data_source": 0,
         "lookup_config": None, "workspace_id": 1 if default else 2,
         "updated_at": "2030-01-02T03:04:05Z"}
    f.update(extra)
    return f


class Fake:
    """Route table: exact path -> reply (callable or value); `me` returns `user`."""

    def __init__(self, routes, user=901):
        self.routes = routes
        self.user = user

    def int_get(self, path, params=None):
        for key, reply in self.routes.items():
            if path == key or (key.startswith("*") and path.endswith(key[1:])):
                return reply() if callable(reply) else reply
        raise APIError(404, "missing", "GET", path)

    def autocomplete(self, kind, query, params=None):
        return self.int_get(f"autocomplete/{kind}")

    def me(self):
        return {"id": self.user}


def write_plan():
    p = WritePlan("tickets_self_watch", 123, DOMAIN, "POST", "tickets/123/watch",
                  json.dumps({"user_id": 901}), "ctx")
    return {"hash": p.hash, "review": p.review()}


def gen_watchers():
    out = {}
    for module in ["tickets", "changes"]:
        members = [902]
        c = Fake({f"{module}/123/watchers": lambda: {"watcher_ids": list(members)}})
        w = watchers.build_plan(c, module, "123", watch=True)
        members.append(901)
        u = watchers.build_plan(c, module, "123", watch=False)
        out[module] = {"watch_hash": w.hash, "unwatch_hash": u.hash, "unwatch_payload_json": u.payload_json}
    return out


def known_error():
    c = Fake({"problems/123": {"problem": {"id": 123, "known_error": False, "subject": "Acme problem"}}})
    return {"hash": problem_known_error.build_plan(c, "123").hash}


def planning():
    schema = {"fields": [{"name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "id": 901}]}
    planning_text.schema.load = lambda *a, **k: schema
    out = {}
    cases = [("changes", "Rollout Plan", "acme_plan")] + [
        ("problems", key, key) for key in ("problem_cause", "problem_impact", "problem_symptom")]
    for module, section, key in cases:
        key_name = "change_planning_fields" if module == "changes" else "problem_planning_fields"
        c = Fake({f"{module}/123/planning-fields": {key_name: []}})
        out[key] = {"hash": planning_text.build_plan(c, module, "123", section, "<div>Acme evidence</div>").hash}
    return out


def gen_ticket_time():
    rows = []
    c = Fake({"tickets/123/time_entries": lambda: {"time_entries": [dict(r) for r in rows], "has_more": False}})
    create = ticket_time.build_plan(c, "INC-123", action="create", spent="00:01",
                                    executed_at="2030-01-02T10:00:00-05:00", note="Acme")
    rows.append({"id": 456, **create.payload})
    update = ticket_time.build_plan(c, "123", action="update", entry_id=456, note="Globex")
    rows[0].update(update.payload)
    delete = ticket_time.build_plan(c, "123", action="delete", entry_id=456)
    return {"create_payload_json": create.payload_json, "create_hash": create.hash,
            "update_hash": update.hash, "delete_hash": delete.hash}


def gen_bulk_priority():
    out = {}
    for domain, cid, high in [("acme.example.com", 123, 87), ("globex.example.com", 742, 26)]:
        config.DOMAIN = domain
        fields = {"ticket_fields": [
            form_field(cid + 9000000000, "subject", "Subject", "default_subject", 1),
            form_field(cid + 9000000001, "priority", "Priority", "default_priority", 2,
                       choices=[{"id": 1, "value": "Low"}, {"id": high, "value": "High"}]),
        ]}
        c = Fake({"ticket_form_fields": fields, "*business_rules": {"business_rules": []},
                  f"tickets/{cid}": {"ticket": {"id": cid, "subject": "Acme one", "priority": 1}},
                  f"tickets/{cid + 1}": {"ticket": {"id": cid + 1, "subject": "Globex two", "priority": 1}}})
        p = bulk_priority.build_plan(c, [str(cid), str(cid + 1)], "High")
        out[domain] = {"hash": p.hash, "review": p.review()}
    config.DOMAIN = DOMAIN
    return out


def ticket_form(status_id, extra=()):
    return {"ticket_fields": [
        form_field(1, "subject", "Subject", "default_subject", 1),
        form_field(2, "status", "Status", "default_status", 2,
                   choices=[{"id": 2, "value": "Open"}, {"id": status_id, "value": "Pending"}]),
        form_field(3, "planned_start_date", "Start", "default_planned_start_date", 3),
        form_field(4, "planned_end_date", "End", "default_planned_end_date", 4),
        *extra,
    ]}


def conversation():
    c = Fake({"ticket_form_fields": ticket_form(3), "ws/1/business_rules": {},
              "tickets/123": {"ticket": {"id": 123, "subject": "Test", "status": 2}},
              "autocomplete/requesters": [{"id": 11, "value": "Acme User", "email": "person@example.com"}]})
    out = {}
    reply = ticket_conversation.build_plan(c, "123", "Acme update", kind="reply",
                                           bcc=("person@example.com",), sender="support@example.com")
    forward = ticket_conversation.build_plan(c, "123", "Acme update", kind="forward",
                                             notify=("person@example.com",), sender="support@example.com")
    for kind, p in [("reply", reply), ("forward", forward)]:
        out[kind] = {"hash": p.hash, "payload_json": p.payload_json}
    return out


def properties():
    rich = [
        form_field(5, "priority", "Priority", "default_priority", 5, choices=[{"id": 73, "value": "High"}]),
        form_field(6, "cost_code", "Cost Code", "custom_text", 6, workspace_id=1),
        form_field(7, "requested_for", "Requested For", "default_requested_for", 7),
    ]
    row = {"id": 123, "subject": "Test", "status": 2, "priority": 1, "tags": ["old"],
           "planned_start_date": None, "planned_end_date": None,
           "custom_fields": {"cost_code": "old", "other": "retained"}}
    c = Fake({"ticket_form_fields": ticket_form(3, rich), "ws/1/business_rules": {"business_rules": []},
              "tickets/123": {"ticket": row}})
    full = ticket_properties.build_plan(c, "SR-123", answers={"Subject": "Globex", "Priority": "High",
                                                               "Cost Code": "new"})
    legacy = ticket_properties.build_plan(c, 123, tags=["Acme"])
    return {"full": {"hash": full.hash, "body_json": full.body_json, "answers_json": full.answers_json,
                     "metadata_hash": full.metadata_hash},
            "legacy": {"hash": legacy.hash, "metadata_hash": legacy.metadata_hash}}


def send_failure():
    """`catalog.SendFailure` text plus `cli_sr.place`'s conversion, copied
    from its `except cat.SendFailure` block."""
    from fsv import catalog as cat
    from fsv.errors import SessionError, exit_code_for
    cases = [("upload", [], RuntimeError("boom")),
             ("upload", [{"id": 9, "name": "a.txt"}], ValueError("bad")),
             ("place-request", [{"id": 9, "name": "a.txt"}], APIError(422, "no")),
             ("place-request", [], APIError(409, "dup")),
             ("place-request", [], APIError(403, "denied")),
             ("place-request", [], APIError(500, "x")),
             ("place-request", [], SessionError("expired")),
             ("place-request", [], RuntimeError("response has no exact request ID; outcome unknown"))]
    out = []
    for step, up, error in cases:
        e = cat.SendFailure(step, up, error)
        note = ("uploaded attachment ids (not attached to any request): "
                + ", ".join(f"{u['id']} ({u['name']})" for u in e.uploaded)) if e.uploaded else None
        if e.step == "upload":
            what = "nothing was placed"
        elif isinstance(e.error, APIError) and 400 <= e.error.status < 500 and e.error.status not in (403, 408, 429):
            what = "the server refused it; nothing was placed"
        else:
            what = "the outcome is unknown: check `fsv journal tail` and the requester's requests before retrying"
        detail = f"HTTP {e.error.status}" if isinstance(e.error, APIError) else type(e.error).__name__
        out.append({"display": str(e), "note": note, "code": exit_code_for(e.error),
                    "message": f"{e.step} failed: {detail}; {what}"})
    return out


def merge_same_conditions():
    from fsv.cli import _merge_same_conditions
    q = [{"condition": "status", "operator": "is_in", "type": "default", "value": [2]},
         {"condition": "priority", "operator": "is", "type": "default", "value": 3},
         {"condition": "status", "operator": "is_in", "type": "default", "value": [3, 4]},
         {"condition": "status", "operator": "is_in", "type": "custom", "value": [5]},
         {"condition": "status", "operator": "is_not_in", "type": "default", "value": None},
         {"condition": "status", "operator": "is_not_in", "type": "default", "value": "ab"},
         {"operator": "is_in", "value": {"k": 1}},
         {"operator": "is_in"},
         {"condition": "status", "operator": "is_in", "type": "default", "value": []}]
    return {"input": q, "output": _merge_same_conditions(json.loads(json.dumps(q)))}


def main():
    config.DOMAIN = DOMAIN
    doc = {
        "write_plan": write_plan(),
        "watchers": gen_watchers(),
        "known_error": known_error(),
        "planning": planning(),
        "ticket_time": gen_ticket_time(),
        "bulk_priority": gen_bulk_priority(),
        "conversation": conversation(),
        "properties": properties(),
        "send_failure": send_failure(),
        "merge_same_conditions": merge_same_conditions(),
    }
    print(json.dumps(doc, indent=1, sort_keys=True, ensure_ascii=False))


if __name__ == "__main__":
    main()
