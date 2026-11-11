"""Two invented tenants for the site-like list/detail view tests.

* ``acme``: workspaces, edit-form business rules, a change lifecycle,
  planning fields, the tabs API, saved views, list headers, a service
  request with requested items and a conversation thread.
* ``globex``: a single-workspace account (no ``/ws/<n>/`` paths), no
  business rules, no lifecycle, no planning fields, non-English labels and a
  restricted role that gets 403 on the optional endpoints.

Everything (names, labels, ids, e-mails) is made up; the response shapes
follow the web's internal ``/api/_`` endpoints. ``World`` is an
``httpx.MockTransport`` handler behind a real :class:`fsv.client.Client`,
so the tests exercise real URLs and query strings. No network.
"""
from __future__ import annotations

import copy
import json
from typing import Any
from urllib.parse import parse_qs

import httpx

import acme_change_form as F
from fsv import client as client_mod

NOW = "2030-06-15T12:00:00+00:00"

ME = {"id": 700001, "first_name": "Robin", "last_name": "Agent", "email": "robin.agent@example.com",
      "time_zone": "Tokyo"}
GROUPS = [{"id": 7101, "name": "Platform Ops"}, {"id": 7102, "name": "Service Desk"}]
AGENTS = [{"id": 7301, "user_id": 700001, "name": "Robin Agent"}, {"id": 7302, "user_id": 700002, "name": "Kim Lee"}]


def _person(pid: int, name: str, **kw: Any) -> dict[str, Any]:
    return {"id": pid, "name": name, "email": name.lower().replace(" ", ".") + "@example.com", **kw}


JANE = _person(700101, "Jane Doe", job_title="Product Owner")
SAM = _person(700102, "Sam Rivera")


# ----------------------------------------------------------------- acme --
def acme_change_fields() -> dict[str, Any]:
    fields = F.fields()["change_fields"]
    dept = F._f("department", "Department", "default_department", 2.5,
                choices=[{"id": 7201, "value": "Engineering"}, {"id": 7202, "value": "Finance"}])
    reason = F._f("change_reason", "Reason for change", "default_change_reason", None, helptext="Why now")
    fields.insert(2, dept)
    fields.append(reason)
    return {"change_fields": fields}


def acme_change(cid: int = 501, **over: Any) -> dict[str, Any]:
    rec = F.record(id=cid, display_id=cid, human_display_id=f"CHN-{cid}")
    rec.update({
        "workspace_id": 2, "state_flow_id": 8001, "state_traversal": [1, 2], "status": 2, "status_name": "Planning",
        "requester": JANE, "requester_id": JANE["id"], "agent_id": 700001, "agent": {"id": 700001, "name": "Robin Agent"},
        "group_id": 7101, "group": {"id": 7101, "name": "Platform Ops"}, "department_id": 7201,
        "department": {"id": 7201, "name": "Engineering"},
        "created_at": "2030-06-13T09:00:00+00:00", "due_by": "2030-06-30T09:00:00+00:00", "approval_status": 4,
        "description": "<p>Rotate the <b>billing</b> keys.</p><ul><li>staging first</li><li>then live</li></ul>",
        "change_status": {"id": 2, "name": "Planning"},
    })
    rec.update(over)
    return rec


ACME_CHANGES = {
    501: acme_change(501),
    502: acme_change(502, subject="Search: reindex catalogue", agent_id=None, agent=None, group_id=None, group=None,
                     department_id=None, department=None, status=1, status_name="Open",
                     change_status={"id": 1, "name": "Open"}, requester=SAM, requester_id=SAM["id"],
                     created_at="2030-06-14T09:00:00+00:00"),
}

ACME_TICKET_FIELDS = {"ticket_fields": [
    F._f("requester", "Requester", "default_requester", 1, required=True),
    F._f("subject", "Subject", "default_subject", 2, required=True),
    F._f("source", "Source", "default_source", 3, choices=[{"id": 1, "value": "Email"}, {"id": 3, "value": "Phone"}]),
    F._f("status", "Status", "default_status", 4, choices=[{"id": 2, "value": "Open"}, {"id": 5, "value": "Closed"}]),
    F._f("priority", "Priority", "default_priority", 5, choices=[{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}]),
    F._f("ticket_type", "Type", "default_ticket_type", 6,
         choices=[{"id": 7401, "value": "Incident"}, {"id": 7402, "value": "Service Request"}]),
    F._f("group", "Group", "default_group", 7),
    F._f("agent", "Agent", "default_agent", 8),
    F._f("affected_app", "Affected App", "custom_dropdown", 9,
         choices=F._cc((7501, 7511, "Billing"), (7502, 7512, "Search"))),
    F._f("description", "Description", "default_description", 10),
]}

ACME_TICKETS = {
    601: {"id": 601, "display_id": 601, "human_display_id": "INC-601", "subject": "VPN drops", "status": 2,
          "status_name": "Open", "ticket_status": {"id": 2, "name": "Open"}, "priority": 1, "state": "new",
          "requester": SAM, "requester_id": SAM["id"], "group_id": 7102, "group": {"id": 7102, "name": "Service Desk"},
          "responder_id": None, "responder": None, "created_at": "2030-06-15T10:00:00+00:00", "type": "Incident",
          "workspace_id": 2, "custom_fields": {}},
    602: {"id": 602, "display_id": 602, "human_display_id": "SR-602", "subject": "Access to analytics", "status": 5,
          "status_name": "Closed", "ticket_status": {"id": 5, "name": "Closed"}, "priority": 2, "state": "elapsed",
          "requester": JANE, "requester_id": JANE["id"], "requested_for_id": JANE["id"], "source": 3,
          "group_id": 7102, "group": {"id": 7102, "name": "Service Desk"}, "responder_id": 700001,
          "responder": {"id": 700001, "name": "Robin Agent"}, "type": "Service Request", "workspace_id": 2,
          "created_at": "2030-05-14T09:00:00+00:00", "due_by": "2030-05-20T09:00:00+00:00",
          "stats": {"resolved_at": "2030-05-25T09:00:00+00:00", "closed_at": "2030-05-28T09:00:00+00:00",
                    "resolution_time_in_secs": 950400},
          "description": "", "custom_fields": {"affected_app": "Billing"},
          "attachments": [{"id": 7601, "name": "screenshot.png", "size": 2048}]},
}

ACME_ITEM = {
    "id": 7701, "stage": {"id": 1, "name": "Requested"}, "quantity": 1,
    "item": {"id": 7702, "name": "Analytics workspace access", "display_id": 12},
}
ACME_ITEM_MORE = {"requested_item": {
    "id": 7701, "item_id": 7702,
    "custom_fields": {"access_kind": "Add permission", "workspace": "Sales", "reason_text": "Quarterly report",
                      "unused": None},
    "item": {"custom_fields": [
        {"name": "access_kind", "label": "Request type", "field_type": "custom_dropdown", "sections": [
            {"name": "Add permission", "fields": [{"name": "workspace", "label": "Workspace", "nested_fields": []}]}],
         "nested_fields": []},
        {"name": "reason_text", "label": "Reason", "field_type": "custom_paragraph", "nested_fields": []},
        {"name": "unused", "label": "Unused", "nested_fields": []}]},
}}


def _conv(i: int, *, private: bool = False, incoming: bool = False, source: int = 0) -> dict[str, Any]:
    who = SAM if incoming else {"id": 700001, "name": "Robin Agent"}
    return {"id": 7800 + i, "body": f"<div>Message {i}</div>", "body_text": f"Message {i}", "private": private,
            "incoming": incoming, "source": source, "user": who, "created_at": f"2030-05-1{5 + i}T09:00:00+00:00",
            "attachments": []}


ACME_CONVERSATIONS = [_conv(0, incoming=True), _conv(1), _conv(2, private=True, source=2), _conv(3, incoming=True),
                      _conv(4, source=2)]


# --------------------------------------------------------------- globex --
def globex_change_fields() -> dict[str, Any]:
    def f(name: str, label: str, ftype: str, pos: Any, **kw: Any) -> dict[str, Any]:
        out = F._f(name, label, ftype, pos, **kw)
        out.pop("workspace_id", None)
        return out
    return {"change_fields": [
        f("requester", "Anforderer", "default_requester", 1, required=True),
        f("subject", "Betreff", "default_subject", 2, required=True),
        f("status", "Status", "default_status", 3, choices=[{"id": 1, "value": "Offen"}, {"id": 2, "value": "Geplant"}]),
        f("priority", "Priorität", "default_priority", 4,
          choices=[{"id": 1, "value": "Niedrig"}, {"id": 2, "value": "Mittel"}, {"id": 3, "value": "Hoch"}]),
        f("department", "Abteilung", "default_department", 5, choices=[{"id": 8201, "value": "Vertrieb"}]),
        f("group", "Gruppe", "default_group", 6),
        f("agent", "Bearbeiter", "default_agent", 7),
        f("umgebung", "Umgebung", "custom_dropdown", 8, choices=F._cc((8301, 8311, "Produktion"), (8302, 8312, "Test"))),
        f("description", "Beschreibung", "default_description", 9),
    ]}


GLOBEX_CHANGES = {
    77: {"id": 77, "display_id": 77, "human_display_id": "CHN-77", "subject": "Drucker-Treiber aktualisieren",
         "status": 1, "change_status": {"id": 1, "name": "Offen"}, "priority": 3, "requester_id": 800001,
         "requester": {"id": 800001, "name": "Lena Vogel"}, "agent_id": 800002,
         "agent": {"id": 800002, "name": "Max Weber"}, "group_id": None, "department_id": 8201,
         "department": {"id": 8201, "name": "Vertrieb"}, "created_at": "2030-06-14T08:00:00+00:00",
         "due_by": "2030-06-20T08:00:00+00:00", "custom_fields": {"umgebung": "Test"},
         "description": "<p>Neuer Treiber.</p>"},
}
GLOBEX_TICKET_FIELDS = {"ticket_fields": [
    {k: v for k, v in F._f("requester", "Anforderer", "default_requester", 1).items() if k != "workspace_id"},
    {k: v for k, v in F._f("subject", "Betreff", "default_subject", 2).items() if k != "workspace_id"},
    {k: v for k, v in F._f("status", "Status", "default_status", 3,
                           choices=[{"id": 2, "value": "Offen"}, {"id": 4, "value": "Gelöst"}]).items()
     if k != "workspace_id"},
    {k: v for k, v in F._f("priority", "Priorität", "default_priority", 4,
                           choices=[{"id": 1, "value": "Niedrig"}]).items() if k != "workspace_id"},
]}
GLOBEX_TICKETS = {
    88: {"id": 88, "display_id": 88, "human_display_id": "SR-88", "subject": "Neuer Laptop", "status": 2,
         "ticket_status": {"id": 2, "name": "Offen"}, "priority": 1, "state": "overdue",
         "requester": {"id": 800001, "name": "Lena Vogel"}, "requester_id": 800001, "requested_for_id": 800003,
         "responder_id": None, "group_id": None, "created_at": "2030-06-10T08:00:00+00:00", "source": 2,
         "type": "Service Request", "custom_fields": {}, "description": "<p>Bitte bis Freitag.</p>"},
}


class World:
    """MockTransport handler for one tenant; records every request."""

    def __init__(self, tenant: str = "acme") -> None:
        assert tenant in ("acme", "globex")
        self.tenant = tenant
        self.requests: list[httpx.Request] = []

    # helpers -------------------------------------------------------------
    def client(self) -> client_mod.Client:
        c = client_mod.Client(cookies={"_x_m": "cookie", "fw-session-id": "session"})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(self), follow_redirects=False)
        c._csrf = "tok"
        return c

    def paths(self) -> list[str]:
        return [r.url.path for r in self.requests]

    def calls(self, path: str) -> list[dict[str, str]]:
        return [{k: v[0] for k, v in parse_qs(r.url.query.decode(), keep_blank_values=True).items()}
                for r in self.requests if r.url.path == path]

    def writes(self) -> list[httpx.Request]:
        return [r for r in self.requests if r.method != "GET"]

    def __call__(self, req: httpx.Request) -> httpx.Response:
        self.requests.append(req)
        if req.method != "GET":
            return httpx.Response(405, json={"message": "read-only test world"})
        p = req.url.path
        q = {k: v[0] for k, v in parse_qs(req.url.query.decode(), keep_blank_values=True).items()}
        if p == "/a/dashboard":  # session probe after a 403: the session is fine
            return httpx.Response(200, text='<html><meta name="csrf-token" content="tok"></html>',
                                  headers={"content-type": "text/html"})
        out = self._acme(p, q) if self.tenant == "acme" else self._globex(p, q)
        if out is None:
            return httpx.Response(404, json={"message": f"no route GET {p}"})
        if isinstance(out, int):
            return httpx.Response(out, json={"message": "Access denied"})
        return httpx.Response(200, json=copy.deepcopy(out))

    # acme ------------------------------------------------------------------
    def _acme(self, p: str, q: dict[str, str]) -> Any:  # noqa: C901
        api = "/api/_/"
        if p == "/api/v2/agents/me":
            return {"agent": ME}
        if p in ("/api/_/change_form_fields", "/api/v2/change_form_fields"):
            return acme_change_fields()
        if p in ("/api/_/ticket_form_fields", "/api/v2/ticket_form_fields"):
            return ACME_TICKET_FIELDS
        if p == "/api/_/ws/2/business_rules":
            return F.rules() if q.get("module_type") == "change" else {"business_rules": [], "user": None}
        if p == "/api/_/bootstrap/agents_groups":
            return {"groups": GROUPS, "agents": AGENTS}
        if p == "/api/_/change-filters":
            return {"change_filters": [
                {"id": "my_open", "name": "New & My Open Changes", "default": True},
                {"id": "all", "name": "All Changes", "default": True},
                {"id": 9001, "name": "My Team's Changes", "user_id": 700001, "default": False, "advanced": False}],
                "meta": {"editable_filter_ids": []}}
        if p == "/api/_/change-filters/my_open":
            return {"change_filter": {"id": "my_open", "name": "New & My Open Changes", "default": True,
                                      "order_by": "created_at", "order_type": "desc", "query_hash": [
                                          {"condition": "status", "operator": "is_in", "value": [1], "type": "default"}]}}
        if p.startswith("/api/_/change-filters/"):
            return {"change_filter": {"id": p.rsplit("/", 1)[1], "order_by": "updated_at", "order_type": "asc"}}
        if p == "/api/_/changes":
            rows = list(ACME_CHANGES.values())
            return {"changes": rows, "meta": {"offset": 0, "has_next": False, "count": len(rows)}}
        if p == "/api/_/ticket_filters":
            return {"ticket_filters": [
                {"id": "new_and_my_open", "name": "New & My Open Tickets", "default": True},
                {"id": "all_tickets", "name": "All Tickets", "default": True}]}
        if p.startswith("/api/_/ticket_filters/"):
            return {"ticket_filter": {"id": p.rsplit("/", 1)[1], "order_by": "created_at", "order_type": "desc"}}
        if p == "/api/_/tickets/headers":
            return {"headers": [
                {"name": "display_id", "label": "Ticket ID", "selected": True, "position": 1},
                {"name": "ticket_subject", "label": "Subject", "selected": True, "position": 1},
                {"name": "requester", "label": "Requester", "selected": True, "position": 2},
                {"name": "state", "label": "State", "selected": True, "position": 3},
                {"name": "status", "label": "Status", "selected": True, "position": 4},
                {"name": "priority", "label": "Priority", "selected": True, "position": 5},
                {"name": "assigned_to", "label": "Assigned to", "selected": True, "position": 6},
                {"name": "urgency", "label": "Urgency", "selected": False, "position": 7}]}
        if p == "/api/_/tickets":
            rows = list(ACME_TICKETS.values())
            return {"tickets": rows, "meta": {"count": len(rows), "has_next": False}}
        if p.startswith("/api/_/changes/"):
            return self._acme_change(p[len(api) + len("changes/"):], q)
        if p.startswith("/api/_/tickets/"):
            return self._acme_ticket(p[len(api) + len("tickets/"):], q)
        if p == "/api/_/ws/2/change_management/8001":
            assert q.get("include") == "state_info"
            return {"state_flow": {"id": 8001, "name": "Normal", "state_list": [
                {"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 3, "value": "Awaiting Approval"},
                {"id": 6, "value": "Closed"}]}}
        if p == f"/api/_/users/{JANE['id']}":
            return {"user": JANE}
        return None

    def _acme_change(self, rest: str, q: dict[str, str]) -> Any:  # noqa: C901
        cid_s, _, sub = rest.partition("/")
        cid = int(cid_s)
        rec = ACME_CHANGES.get(cid)
        if rec is None:
            return None
        if not sub:
            return {"change": rec}
        if sub == "tabs":
            return [{"name": "devops"}, {"name": "risk"}, {"name": "notes", "count": 1}, {"name": "tasks", "count": 1},
                    {"name": "assets", "count": 0}, {"name": "impacted_services", "count": 0},
                    {"name": "associations", "count": 1, "associated_modules": []}, {"name": "approvals", "count": 1},
                    {"name": "activity"}]
        if sub == "planning-fields":
            return {"change_planning_fields": [{"name": "go_live_checklist", "attachments": [{"name": "runbook.pdf"}],
                                                "description": "<p>Smoke test <b>billing</b></p><ul><li>login</li></ul>"}]}
        if sub == "notes":
            return {"notes": [{"id": 7901, "body": "<p>CAB slot booked</p>", "private": True,
                               "user": {"id": 700001, "name": "Robin Agent"}, "created_at": "2030-06-14T10:00:00+00:00"}],
                    "meta": {"has_next": False}}
        if sub == "tasks":
            return {"tasks": [{"id": 7911, "display_id": 3, "title": "Take snapshot", "status": 2,
                               "group": {"id": 7101, "name": "Platform Ops"}, "agent": {"id": 700001, "name": "Robin Agent"},
                               "due_date": "2030-06-16T09:00:00+00:00"}], "meta": {"has_next": False}}
        if sub == "approvals":
            return {"approvals": [{"id": 7921, "level_id": 1, "status": {"id": 2, "name": "approved"},
                                   "member": {"id": 700002, "name": "Kim Lee"}, "updated_at": "2030-06-14T11:00:00+00:00",
                                   "remark": [{"data": "Fine by me"}]}], "meta": {"triggerable_level": 1}}
        if sub == "activities":
            return {"activities": [{"actor": {"id": 700001, "name": "Robin Agent"}, "content": "set Status as <b>Planning</b>",
                                    "sub_contents": None, "created_at": "2030-06-14T01:30:00Z"}]}
        if sub == "assets":
            if q.get("filter") == "impacted_services":
                return {"associated_assets": []}
            return {"associated_assets": [{"id": 7931, "display_id": 42, "name": "billing-db", "ci_type_name": "Database"}]}
        if sub == "releases":
            return {"releases": [], "meta": {"has_next": False}, "requesters": []}
        if sub == "tickets":
            rows = [ACME_TICKETS[601]] if not q.get("filter") else []
            return {"tickets": rows, "meta": {"has_next": False}, "requesters": []}
        if sub == "problems":
            return {"problems": [], "meta": {"has_next": False}, "requesters": []}
        if sub == "risk-analysis":
            return {"overall_score": None, "computed_risk": None, "basic_risk_awareness": [
                {"name": "approval_status", "value": "Not Requested", "operator": "is"},
                {"name": "impact", "value": "Low", "operator": "is"}]}
        if sub == "survey":
            assert q.get("module") == "risk" and q.get("workspace_id") == "2"
            return {}
        if sub in ("work-items", "pull-requests", "pipeline-runs"):
            return {sub.replace("-", "_"): [], "meta": {"count": 0}}
        return None

    def _acme_ticket(self, rest: str, q: dict[str, str]) -> Any:  # noqa: C901
        tid_s, _, sub = rest.partition("/")
        tid = int(tid_s)
        rec = ACME_TICKETS.get(tid)
        if rec is None:
            return None
        if not sub:
            return {"ticket": rec}
        if sub == "tabs":
            tabs = [{"name": "related_tickets", "count": 0}, {"name": "tasks", "count": 0}, {"name": "assets", "count": 0},
                    {"name": "associations", "count": 0}, {"name": "approvals", "count": 1}, {"name": "activity"},
                    {"name": "resolution_notes"}]
            if rec["type"] == "Service Request":
                tabs.insert(0, {"name": "requested_items", "count": 1})
            return tabs
        if sub == "requested_items":
            return {"requested_items": [ACME_ITEM], "meta": {}}
        if sub == "requested_items/7701":
            assert q.get("view") == "more_info"
            return ACME_ITEM_MORE
        if sub == "conversations":
            per = int(q.get("per_page", "30"))
            page = int(q.get("page", "1"))
            rows = ACME_CONVERSATIONS if tid == 602 else []
            if per == 3 and "page" not in q:  # the web's first load: the latest three
                return {"conversations": rows[-3:], "meta": {"count": len(rows), "has_more": len(rows) > 3}}
            chunk = rows[(page - 1) * per: page * per]
            return {"conversations": chunk, "meta": {"count": len(rows), "has_more": page * per < len(rows)}}
        if sub == "mappings":
            return {"predecessor_ticket": None, "successor_tickets": None}
        if sub == "approvals":
            return {"approvals": [{"id": 7941, "level_id": 1, "status": {"name": "approved"},
                                   "member": {"id": 700002, "name": "Kim Lee"}}]}
        if sub == "activities":
            return {"activities": []}
        if sub == "tasks":
            return {"tasks": [], "meta": {"has_next": False}}
        if sub == "associated_assets":
            return {"associated_assets": []}
        if sub == "changes":
            return {"changes": [], "meta": {"has_next": False}, "requesters": []}
        return None

    # globex ------------------------------------------------------------------
    def _globex(self, p: str, q: dict[str, str]) -> Any:  # noqa: C901
        assert "/ws/" not in p, f"single-workspace account: no /ws/ paths ({p})"
        if p == "/api/v2/agents/me":
            return 403
        if p in ("/api/_/change_form_fields", "/api/v2/change_form_fields"):
            return globex_change_fields()
        if p in ("/api/_/ticket_form_fields", "/api/v2/ticket_form_fields"):
            return GLOBEX_TICKET_FIELDS
        if p in ("/api/_/change-filters", "/api/_/ticket_filters", "/api/_/tickets/headers",
                 "/api/_/bootstrap/agents_groups"):
            return 403
        if p == "/api/_/changes":
            return {"changes": list(GLOBEX_CHANGES.values()), "meta": {"has_next": False}}
        if p == "/api/_/tickets":
            return {"tickets": list(GLOBEX_TICKETS.values())}
        if p.startswith("/api/_/changes/"):
            cid_s, _, sub = p[len("/api/_/changes/"):].partition("/")
            rec = GLOBEX_CHANGES.get(int(cid_s))
            if rec is None:
                return None
            if not sub:
                return {"change": rec}
            if sub == "tabs":
                return 403
            if sub == "notes":
                return {"notes": [{"id": 1, "body": "<p>Freigabe folgt</p>", "private": False,
                                   "user": {"name": "Max Weber"}, "created_at": "2030-06-14T09:00:00+00:00"}]}
            if sub in ("tasks", "approvals"):
                return 403
            return None
        if p.startswith("/api/_/tickets/"):
            tid_s, _, sub = p[len("/api/_/tickets/"):].partition("/")
            rec = GLOBEX_TICKETS.get(int(tid_s))
            if rec is None:
                return None
            if not sub:
                return {"ticket": rec}
            if sub in ("tabs", "requested_items"):
                return 403
            if sub == "conversations":
                return {"conversations": [{"id": 2, "body": "<p>Erledigt?</p>", "incoming": True,
                                           "user": {"name": "Lena Vogel"}, "created_at": "2030-06-11T08:00:00+00:00"}],
                        "meta": {"count": 1, "has_more": False}}
            return None
        if p == "/api/_/business_rules":
            return None  # 404: the plan has no business rules
        if p.startswith("/api/_/users/"):
            return 403
        return None


def dumps(obj: Any) -> str:
    return json.dumps(obj, sort_keys=True)
