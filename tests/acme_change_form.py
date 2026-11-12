"""Invented "Acme" change form for the CLI face tests.

Shapes mirror ``GET /api/_/change_form_fields`` and
``GET /api/_/ws/<n>/business_rules``; every name, label and id is made up
(ids 900000xxxx, account 900000777, example.com people).
"""
from __future__ import annotations

import copy
import itertools
from typing import Any

ACCOUNT = "900000777"
_ids = itertools.count(9000006001)


def _f(name: str, label: str, field_type: str, position: int | None, **kw: Any) -> dict[str, Any]:
    default = field_type.startswith("default_")
    out = {
        "id": next(_ids), "name": name, "label": label, "description": "", "field_type": field_type,
        "required": kw.pop("required", False), "required_for_closure": False, "position": position,
        "default_field": default, "choices": kw.pop("choices", []), "nested_fields": [],
        "placeholder": kw.pop("placeholder", ""), "helptext": kw.pop("helptext", ""), "hint": False,
        "content": None, "displayed_to_agents": True, "data_source": kw.pop("data_source", 0),
        "lookup_config": kw.pop("lookup_config", None), "workspace_id": 1 if default else 2,
        "updated_at": kw.pop("updated_at", "2030-01-02T03:04:05Z"),
    }
    out.update(kw)
    return out


def _cc(*pairs: tuple[int, int, str]) -> list[dict[str, Any]]:
    return [{"id": i, "display_id": d, "value": v, "label": v} for i, d, v in pairs]


SERVICES = ["Billing", "Search", "Reports", "Ledger", "Mail", "Chat", "Docs", "Wiki", "Pager", "Auth",
            "Queue", "Cache", "Storage", "Metrics"]


def fields() -> dict[str, Any]:
    return {"change_fields": [
        _f("requester", "Requester", "default_requester", 1, required=True),
        _f("subject", "Subject", "default_subject", 2, required=True, placeholder="Service: objective"),
        _f("change_type", "Change Type", "default_change_type", 3, required=True,
           choices=[{"id": 1, "value": "Normal"}, {"id": 2, "value": "Standard"}, {"id": 4, "value": "Emergency"}]),
        _f("cloud_only", "Cloud Only", "custom_dropdown", 4, required=True,
           helptext="Only the cloud team performs this change",
           choices=_cc((9000002101, 9000003101, "Yes"), (9000002102, 9000003102, "No"))),
        _f("stage", "Stage", "custom_dropdown", 5,
           choices=_cc((9000002111, 9000003111, "Live"), (9000002112, 9000003112, "Staging"))),
        _f("staging_start", "Staging Start", "custom_date", 6),
        _f("staging_end", "Staging End", "custom_date", 7),
        _f("services", "Services", "custom_multi_select_dropdown", 8,
           choices=_cc(*[(9000002200 + i, 9000003200 + i, s) for i, s in enumerate(SERVICES)])),
        _f("notify_ops", "Notify Ops Team", "custom_dropdown", 9,
           choices=_cc((9000002301, 9000003301, "Yes"), (9000002302, 9000003302, "No"))),
        _f("reviewer", "Reviewer", "custom_lookup", 10, data_source=5,
           lookup_config={"data_source": 5, "link": f"/lookup_choices?field_name=reviewer_{ACCOUNT}&module=change"}),
        _f("downtime_window", "Downtime Window", "custom_text", 11, placeholder="22:00 - 22:30"),
        _f("legacy_flag", "Legacy Flag", "custom_checkbox", 12),
        _f("description", "Description", "default_description", 13, required=True, helptext="What and why"),
        _f("priority", "Priority", "default_priority", 14, required=True,
           helptext="Low - Can wait || Medium - Planned work || High - Revenue at risk || Urgent - Outage",
           choices=[{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"},
                    {"id": 4, "value": "Urgent"}]),
        _f("impact", "Impact", "default_impact", 15, required=True,
           choices=[{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"}]),
        _f("risk", "Risk", "default_risk", 16, required=True,
           choices=[{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"}]),
        _f("group", "Group", "default_group", 17),
        _f("agent", "Agent", "default_agent", 18),
        _f("planned_start_date", "Planned Start Date", "default_planned_start_date", 19, required=True),
        _f("planned_end_date", "Planned End Date", "default_planned_end_date", 20, required=True),
        _f("status", "Status", "default_status", 21, required=True,
           choices=[{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 3, "value": "Closed"}]),
        _f("go_live_checklist", "Go-Live Checklist", "planning_field", None),
    ]}


def _rule(rid: int, name: str, cond: Any, actions: list[dict[str, Any]], *, rev: bool = False,
          form: str = "both", validation: bool = False) -> dict[str, Any]:
    return {"id": rid, "name": name, "workspace_id": 2, "module_type": "change", "item_id": None,
            "deleted": False, "user_type": "both", "form_type": form, "state": "active",
            "rule_type": "client_side", "description": "", "condition_data": cond,
            "action_data": {"action": actions}, "is_reversible": rev, "is_validation": validation,
            "updated_at": "2030-01-03T00:00:00Z"}


def _c(name: str, op: str, value: Any = None) -> dict[str, Any]:
    out = {"name": name, "operator": op, "evaluate_on": "change"}
    if value is not None:
        out["value"] = value
    return out


def _a(prop: str, names: list[str], values: Any = None) -> dict[str, Any]:
    out: dict[str, Any] = {"name": names, "property": prop}
    if values is not None:
        out["values"] = values
    return out


CF = "customFields."


def rules() -> dict[str, Any]:
    return {"business_rules": [
        _rule(9000004101, "Cloud only: no", {"all": [_c(CF + "cloud_only", "is", "No")]},
              [_a("populate", [CF + "stage"], [9000003112]),
               _a("show", [CF + "staging_start", CF + "staging_end"]),
               _a("mandate", [CF + "staging_start", CF + "staging_end"])]),
        _rule(9000004102, "Cloud only: yes", {"all": [_c(CF + "cloud_only", "is", "Yes")]},
              [_a("populate", [CF + "stage"], [9000003111]),
               _a("hide", [CF + "staging_start", CF + "staging_end"])]),
        _rule(9000004103, "Hide legacy flag", {"all": None}, [_a("hide", [CF + "legacy_flag"])]),
        _rule(9000004104, "Urgent priority", {"all": [_c("priority", "is", "4")]},
              [_a("message", ["Urgent changes need the duty lead's approval"])], validation=True),
        _rule(9000004105, "Edit: live needs a window", {"all": [_c(CF + "stage", "is", "Live")]},
              [_a("mandate", [CF + "downtime_window"])], form="edit_form"),
    ], "user": {"first_name": "Robin", "last_name": "Agent", "email": "robin.agent@example.com",
                "time_zone": "Tokyo"}}


PEOPLE = [
    {"id": 9000007001, "value": "Jane Doe", "email": "jane.doe@example.com", "details": "jane.doe@example.com"},
    {"id": 9000007002, "value": "Jane Doerr", "email": "jane.doerr@example.com", "details": "jane.doerr@example.com"},
]
AGENTS = [{"id": 9000008001, "user_id": 9000007101, "value": "Alex Kim", "email": "alex.kim@example.com"}]
GROUPS = [{"id": 9000009001, "name": "Platform Ops"}]


def record(**over: Any) -> dict[str, Any]:
    """An existing change as ``GET /api/_/changes/<id>`` returns it."""
    rec = {
        "id": 900000501, "human_display_id": "CHN-501", "subject": "Billing: rotate keys",
        "description": "<p>Rotate keys.</p>", "requester_id": 9000007001, "change_type": 1, "priority": 2,
        "impact": 1, "risk": 1, "status": 1, "group_id": 9000009001, "agent_id": None,
        "planned_start_date": "2030-11-26T12:00:00Z", "planned_end_date": "2030-11-26T15:00:00Z",
        "custom_fields": {"cloud_only": "Yes", "stage": "Live", "services": ["Billing"], "notify_ops": "No",
                          "downtime_window": "22:00 - 22:30", "reviewer": None, "legacy_flag": False,
                          "staging_start": None, "staging_end": None},
    }
    rec.update(over)
    return rec


class FakeClient:
    """Records every call; serves the Acme metadata. No network."""

    def __init__(self, *, rules_shape: str = "plural", change: dict[str, Any] | None = None) -> None:
        self.calls: list[tuple[str, str, Any]] = []
        self.rules_shape = rules_shape
        self.fields = fields()
        self.rules = rules()
        self.change = change or record()
        self.fail_upload = False
        self.next_att = itertools.count(9000005001)

    # reads
    def int_get(self, path: str, params: Any = None) -> Any:
        self.calls.append(("GET", path, params))
        if path == "change_form_fields":
            return copy.deepcopy(self.fields)
        if path.endswith("business_rules"):
            r = copy.deepcopy(self.rules)
            if self.rules_shape == "singular":
                r["business_rule"] = r.pop("business_rules")
            return r
        if path == "bootstrap/agents_groups":
            return {"groups": GROUPS}
        if path.endswith("/planning-fields"):
            return {"change_planning_fields": []}
        if path.startswith("changes/"):
            return {"change": copy.deepcopy(self.change)}
        return {}

    def autocomplete(self, kind: str, query: str, params: Any = None) -> list[dict[str, Any]]:
        self.calls.append(("AUTO", kind, query))
        q = query.casefold()
        pool = PEOPLE if kind == "requesters" else AGENTS
        return [p for p in pool if q in p["value"].casefold() or q in p["email"]]

    def lookup_choices(self, link: str, query: str) -> list[dict[str, Any]]:
        q = query.casefold()
        return [{"id": p["id"], "name": p["value"]} for p in PEOPLE if q in p["value"].casefold()]

    # writes
    def int_post(self, path: str, body: Any) -> Any:
        self.calls.append(("POST", path, body))
        if path == "changes":
            return {"change": {"id": 900000502, "human_display_id": "CHN-502"}}
        if path == "attachments":
            if self.fail_upload:
                raise RuntimeError("upload refused")
            return {"id": next(self.next_att), "content_file_name": body["content_file_name"],
                    "content_file_size": body["content_file_size"]}
        return {"ok": True}

    def int_put(self, path: str, body: Any) -> Any:
        self.calls.append(("PUT", path, body))
        return {"change": {"id": int(path.split("/")[1]) if path.split("/")[1].isdigit() else 0}}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kwargs):
        import json
        from urllib.parse import urlsplit
        path = urlsplit(url).path.removeprefix("/api/_/")
        assert method == "PUT" and path.startswith("changes/")
        body = json.loads(kwargs["content"])
        result = self.int_put(path, body)
        for key, value in body.items():
            if key == "custom_fields":
                self.change.setdefault(key, {}).update(value)
            else:
                self.change[key] = value
        return result

    def writes(self) -> list[tuple[str, str, Any]]:
        return [c for c in self.calls if c[0] in ("POST", "PUT")]
