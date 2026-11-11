"""Synthetic service-catalog tenant for the ``fsv sr`` tests.

Everything is invented (acme domain, made-up names, labels and ids) but
mirrors the shapes of the agent catalog endpoints. ``AcmeCatalog`` is an
``httpx.MockTransport`` handler behind a real :class:`fsv.client.Client`, so
the tests exercise the real URLs, query strings and request bodies; no test
reaches a network.
"""
from __future__ import annotations

import copy
import itertools
import json
from typing import Any
from urllib.parse import parse_qs

import httpx

from fsv import client as client_mod

DOMAIN = "acme.freshservice.com"
ACCOUNT = "4242"
WS = 3
JANE = "jane.doe@example.com"

PEOPLE = [
    {"id": 9000001, "value": "Jane Doe", "email": JANE},
    {"id": 9000002, "value": "Sam Lee", "email": "sam.lee@example.com"},
    {"id": 9000003, "value": "Sam Leeds", "email": "sam.leeds@example.com"},
]

CATEGORIES = {"service_categories": [
    {"id": 7001, "name": "Engineering", "description": "Product work", "position": 1, "workspace_id": WS,
     "hierarchy_info": {"has_sub_category": True, "order": "az"}},
    {"id": 7004, "name": "Engineering Tools", "description": "Build tools", "position": 2, "workspace_id": WS,
     "parent_id": 7001, "hierarchy_info": {"has_sub_category": None, "order": None}},
    {"id": 7002, "name": "Access", "description": "Accounts and permissions", "position": 3, "workspace_id": WS,
     "hierarchy_info": {"has_sub_category": None, "order": "az"}},
    {"id": 7003, "name": "Hardware", "description": "Devices", "position": 4, "workspace_id": WS,
     "hierarchy_info": {"has_sub_category": None, "order": None}},
]}


def _summary(id_: int, did: int, name: str, short: str, cat: int, **kw: Any) -> dict[str, Any]:
    out = {"id": id_, "display_id": did, "name": name, "short_description": short, "category_id": cat,
           "deleted": False, "workspace_id": WS, "allow_attachments": True, "allow_quantity": False,
           "configs": {"attachment_mandatory": False, "subject": "", "auto_gen_document": False},
           "icon_detail": {"name": "icon.png", "type": "image/png", "url": "https://files.acme.example/icon.png"}}
    out.update(kw)
    return out


ITEMS = [
    _summary(4400001, 77, "Feature Request", "Ask for a product change", 7001),
    _summary(4400002, 78, "VPN Access", "Remote access to the office network", 7002, allow_attachments=False),
    _summary(4400003, 79, "Laptop", "A new or replacement laptop", 7003),
    _summary(4400004, 80, "Build Agent", "A CI build agent", 7004),
]


def _cf(id_: str, name: str, type_: int, pos: int, label: str, **kw: Any) -> dict[str, Any]:
    out = {"id": id_, "name": name, "type": type_, "position": pos, "label": label, "required": kw.pop("required", False),
           "visible": True, "deleted": False, "account_id": ACCOUNT, "placeholder": kw.pop("placeholder", ""),
           "field_options": kw.pop("field_options", {}), "choices": kw.pop("choices", []),
           "fields": kw.pop("fields", []), "parent_id": kw.pop("parent_id", None)}
    out.update(kw)
    return out


def feature_request() -> dict[str, Any]:
    kid_a, kid_b = "f-goal-rev", "f-goal-cost"
    return {"service_item": {
        "id": 4400001, "display_id": 77, "name": "Feature Request", "short_description": "Ask for a product change",
        "category_id": 7001, "category_name": "Engineering", "workspace_id": WS,
        "updated_at": "2030-01-15T10:00:00+09:00",
        "description": "<p>Use this form for <b>feature</b> work.</p>",
        "allow_attachments": True, "allow_quantity": False, "attachment_mandatory": True,
        "configs": {"attachment_mandatory": True, "subject": "{{custom_field.cf_title_100}}", "auto_gen_document": False},
        "custom_fields": [
            _cf("f-notice", "notice", 1024, 1, "&lt;p&gt;Attach the &lt;b&gt;overview deck&lt;/b&gt;.&lt;/p&gt;",
                field_options={"content_name": "Notice"}),
            _cf("f-title", "title_100", 1001, 2, "Title", required=True, placeholder="Short name"),
            _cf("f-details", "paragraph_101", 1008, 3, "Details", required=True),
            _cf("f-sys", "paragraph_102", 1008, 4, "Affected system (if known)"),
            _cf("f-date", "date_103", 1007, 5, "Needed by", required=True, field_options={"date_only": "true"}),
            _cf("f-goal", "goal", 1003, 6, "1. Expected benefit", required=True,
                field_options={"section_info": json.dumps([
                    {"sectionFields": [kid_a], "sectionChoices": ["More revenue  (Total USD)"]},
                    {"sectionFields": [kid_b], "sectionChoices": ["Lower cost"]}])},
                choices=[
                    {"id": "c-rev", "value": "More revenue  (Total USD)", "label": "More revenue  (Total USD)",
                     "position": 1, "dependent_ids": {"field": [kid_a]}},
                    {"id": "c-cost", "value": "Lower cost", "label": "Lower cost", "position": 2,
                     "dependent_ids": {"field": [kid_b]}},
                    {"id": "c-none", "value": "Other", "label": "Other", "position": 3, "dependent_ids": {}}],
                fields=[
                    _cf(kid_a, "goal_1_kpi", 1001, 1, "1.1 Revenue target", required=True, parent_id="f-goal"),
                    _cf(kid_b, "goal_2_kpi", 1001, 2, "1.2 Cost target", required=True, parent_id="f-goal")]),
            _cf("f-prio", "priority", 1003, 7, "Priority", required=True, choices=[
                {"id": "p1", "value": "Critical - blocks customers", "label": "Critical - blocks customers", "position": 1},
                {"id": "p3", "value": "Medium - planned work", "label": "Medium - planned work", "position": 2},
                {"id": "p4", "value": "Low - cosmetic, reports", "label": "Low - cosmetic, reports", "position": 3}]),
            _cf("f-deck", "overview_deck", 1014, 8, "Overview deck (.pptx)", required=True,
                placeholder="&lt;span class=\"btn-link\"&gt;Upload files&lt;/span&gt; or drop here",
                field_options={"allowed_file_types": "[\".pptx\"]", "max_attachment_limit": "1"}),
        ],
    }}


def vpn_access() -> dict[str, Any]:
    return {"service_item": {
        "id": 4400002, "display_id": 78, "name": "VPN Access", "short_description": "Remote access to the office network",
        "category_id": 7002, "category_name": "Access", "workspace_id": WS, "updated_at": "2030-01-10T10:00:00+09:00",
        "description": "", "allow_attachments": False, "allow_quantity": False, "attachment_mandatory": False,
        "configs": {"attachment_mandatory": False, "subject": "", "auto_gen_document": False},
        "custom_fields": [
            _cf("v-reason", "reason_200", 1001, 1, "Reason", required=True),
            _cf("v-level", "access_level", 1003, 2, "Access level", required=True, choices=[
                {"id": "l1", "value": "Read only", "label": "Read only", "position": 1},
                {"id": "l2", "value": "Read  write", "label": "Read  write", "position": 2},
                {"id": "l3", "value": "Admin", "label": "Admin", "position": 3}]),
            _cf("v-why", "justification", 1008, 3, "Justification"),
        ],
    }}


def rules_for(item_id: int) -> dict[str, Any]:
    rules: list[dict[str, Any]] = []
    if item_id == 4400002:
        rules.append({
            "id": 9100001, "name": "Admin needs a justification", "workspace_id": WS,
            "module_type": "service_request", "item_id": 4400002, "deleted": False, "user_type": "both",
            "form_type": "both", "state": "active", "rule_type": "client_side", "description": "",
            "condition_data": {"all": [{"name": "customField.sr_access_level", "operator": "is", "value": ["Admin"],
                                        "evaluate_on": "change"}]},
            "action_data": {"action": [{"name": ["customField.sr_justification"], "property": "show"},
                                       {"name": ["customField.sr_justification"], "property": "mandate"}]},
            "is_reversible": True, "is_validation": False,
        })
    return {"business_rules": rules, "user": {"first_name": "Test", "last_name": "Agent", "time_zone": "Tokyo"}}


# requested items of SR-123 (read side)
TICKET = {"ticket": {
    "id": 123, "display_id": 123, "human_display_id": "SR-123", "subject": "Dark mode", "type": "Service Request",
    "attachments": [
        {"id": 801, "name": "deck.pptx", "size": 11, "content_type": "application/octet-stream",
         "attachment_url": f"https://{DOMAIN}/helpdesk/attachments/801"},
        {"id": 802, "name": "../escape.sh", "size": 5, "attachment_url": f"https://{DOMAIN}/helpdesk/attachments/802"},
        {"id": 803, "name": "brief.pdf", "size": 9, "attachment_url": f"https://{DOMAIN}/helpdesk/attachments/803"},
    ],
}}
REQUESTED = {"requested_items": [{
    "id": 5, "stage": {"name": "Requested"},
    "item": {"name": "Feature Request", "display_id": 77, "custom_fields": [
        {"name": "title_100", "label": "Title"},
        {"name": "goal", "label": "1. Expected benefit", "fields": [{"name": "goal_1_kpi", "label": "1.1 Revenue target"}]},
    ]},
    "custom_fields": {"title_100": "Dark mode", "paragraph_101": "Please add dark mode.", "goal": "More revenue  (Total USD)",
                      "goal_1_kpi": "+5%", "date_103": "2030-12-03", "paragraph_102": None, "overview_deck": [801]},
}]}
DOWNLOADS = {801: b"deck-bytes!", 802: b"nope!", 803: b"pdf-bytes"}


class AcmeCatalog:
    """MockTransport handler for the catalog, rules, people, uploads,
    place-request and the read side. Records every request."""

    def __init__(self) -> None:
        self.requests: list[httpx.Request] = []
        self.items = {77: feature_request(), 78: vpn_access()}
        self.item_gets: dict[int, int] = {}
        self.next_att = itertools.count(9000005001)
        self.fail_upload = False
        self.fail_place: int | None = None
        self.on_second_item_get: Any = None  # callable(item json) -> item json

    # helpers -----------------------------------------------------------
    def client(self) -> client_mod.Client:
        c = client_mod.Client(cookies={"_x_m": "cookie", "fw-session-id": "session"})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(self), follow_redirects=False)
        c._csrf = "tok"
        return c

    def writes(self) -> list[tuple[str, str, Any]]:
        return [(r.method, r.url.path, json.loads(r.content or b"null")) for r in self.requests
                if r.method in ("POST", "PUT", "PATCH", "DELETE")]

    def paths(self) -> list[str]:
        return [f"{r.method} {r.url.path}" for r in self.requests]

    # handler -------------------------------------------------------------
    def __call__(self, req: httpx.Request) -> httpx.Response:
        self.requests.append(req)
        p = req.url.path
        q = {k: v[0] for k, v in parse_qs(req.url.query.decode()).items()}
        if req.method == "GET":
            return self._get(p, q)
        body = json.loads(req.content or b"{}")
        if p == "/api/_/attachments":
            if self.fail_upload:
                return httpx.Response(500, json={"message": "upload refused"})
            return httpx.Response(200, json={"id": next(self.next_att), "content_file_name": body["content_file_name"],
                                             "content_file_size": body["content_file_size"]})
        if p.endswith("/place-request"):
            if self.fail_place:
                return httpx.Response(self.fail_place, json={"description": "Validation failed",
                                                             "errors": [{"field": "custom_fields", "message": "invalid"}]})
            return httpx.Response(200, json={"service_request": {"id": 9100000123, "display_id": 123,
                                                                 "human_display_id": "SR-123"}})
        return httpx.Response(404, json={"message": f"no route {req.method} {p}"})

    def _get(self, p: str, q: dict[str, str]) -> httpx.Response:
        if p == "/api/_/catalog/service-categories":
            assert q.get("category_hierarchy") == "true"
            return httpx.Response(200, json=CATEGORIES)
        if p == "/api/_/catalog/service-items":
            if q.get("filter") == "frequently_requested":
                return httpx.Response(200, json={"service_items": [ITEMS[1]]})
            page = int(q.get("page", "1"))
            rows = ITEMS[:3] if page == 1 else ITEMS[3:]
            return httpx.Response(200, json={"service_items": rows,
                                             "meta": {"total_items": len(ITEMS), "total_pages": 2, "current_page": page}})
        if p.startswith("/api/_/catalog/service-items/"):
            did = int(p.rsplit("/", 1)[1])
            if did not in self.items:
                return httpx.Response(404, json={"message": "Record not found"})
            n = self.item_gets[did] = self.item_gets.get(did, 0) + 1
            data = copy.deepcopy(self.items[did])
            if n >= 2 and self.on_second_item_get is not None:
                data = self.on_second_item_get(data)
            return httpx.Response(200, json=data)
        if p == f"/api/_/ws/{WS}/business_rules":
            assert q.get("module_type") == "service_request"
            return httpx.Response(200, json=rules_for(int(q["item_id"])))
        if p == "/search/autocomplete/requesters":
            needle = q.get("q", "").casefold()
            rows = [x for x in PEOPLE if needle in x["value"].casefold() or needle in x["email"]]
            return httpx.Response(200, json={"results": rows})
        if p == "/api/_/tickets/123":
            return httpx.Response(200, json=TICKET)
        if p == "/api/_/tickets/123/requested_items":
            return httpx.Response(200, json=REQUESTED)
        if p == "/api/_/tickets/123/requested_items/5":
            return httpx.Response(200, json={"requested_item": copy.deepcopy(REQUESTED["requested_items"][0])})
        if p.startswith("/helpdesk/attachments/"):
            aid = int(p.rsplit("/", 1)[1])
            return httpx.Response(200, content=DOWNLOADS[aid], headers={"content-type": "application/octet-stream"})
        return httpx.Response(404, json={"message": f"no route GET {p}"})
