"""Mocked-httpx tenants for the form UI tests (every name, label, id and
address is invented; no test reaches a network).

* ``AcmeChanges``: the Acme change form of ``acme_change_form`` served over
  a real :class:`fsv.client.Client` (workspace 2, business rules, the
  agents/groups bootstrap, requester autocomplete).
* ``Globex``: a small account that has *none* of the optional features:
  no workspaces (no ``/ws/<id>/`` paths, fields carry no workspace id), no
  business rules (404), no catalog sections, German labels, and a
  restricted role (403 on the agents/groups bootstrap and the agent
  search). Its agent profile has a time zone; the rules response has none.
"""
from __future__ import annotations

import copy
import itertools
import json
from typing import Any
from urllib.parse import parse_qs

import httpx

import acme_change_form as A
from fsv import client as client_mod

DASHBOARD = '<html><head><meta name="csrf-token" content="tok"></head><body>app</body></html>'


class Tenant:
    """Common MockTransport plumbing; subclasses implement ``get``."""

    def __init__(self) -> None:
        self.requests: list[httpx.Request] = []
        self.next_att = itertools.count(9000005001)

    def client(self) -> client_mod.Client:
        c = client_mod.Client(cookies={"_x_m": "cookie", "fw-session-id": "session"})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(self), follow_redirects=False)
        c._csrf = "tok"
        return c

    def __call__(self, req: httpx.Request) -> httpx.Response:
        self.requests.append(req)
        q = {k: v[0] for k, v in parse_qs(req.url.query.decode(), keep_blank_values=True).items()}
        if req.method == "GET":
            if req.url.path == "/a/dashboard":
                return httpx.Response(200, text=DASHBOARD, headers={"content-type": "text/html"})
            return self.get(req.url.path, q)
        body = json.loads(req.content or b"{}")
        if req.url.path == "/api/_/attachments":
            return httpx.Response(200, json={"id": next(self.next_att), "content_file_name": body["content_file_name"],
                                             "content_file_size": body["content_file_size"]})
        return self.write(req.method, req.url.path, body)

    def get(self, path: str, q: dict[str, str]) -> httpx.Response:  # pragma: no cover
        raise NotImplementedError

    def write(self, method: str, path: str, body: Any) -> httpx.Response:  # pragma: no cover
        raise NotImplementedError

    # helpers
    def writes(self) -> list[tuple[str, str, Any]]:
        return [(r.method, r.url.path, json.loads(r.content or b"null")) for r in self.requests
                if r.method in ("POST", "PUT", "PATCH", "DELETE")]

    def gets(self, path: str) -> list[dict[str, str]]:
        return [{k: v[0] for k, v in parse_qs(r.url.query.decode(), keep_blank_values=True).items()}
                for r in self.requests if r.method == "GET" and r.url.path == path]

    def paths(self) -> list[str]:
        return [f"{r.method} {r.url.path}" for r in self.requests]


class AcmeChanges(Tenant):
    AGENTS = [{"id": 9000008001, "user_id": 9000007101, "name": "Alex Kim", "email": "alex.kim@example.com"},
              {"id": 9000008002, "user_id": 9000007102, "name": "Ana Lopez", "email": "ana.lopez@example.com"}]

    def __init__(self) -> None:
        super().__init__()
        self.fields = A.fields()
        self.rules = A.rules()
        self.change = A.record()

    def get(self, p: str, q: dict[str, str]) -> httpx.Response:
        if p == "/api/_/change_form_fields":
            return httpx.Response(200, json=copy.deepcopy(self.fields))
        if p == "/api/_/ws/2/business_rules":
            return httpx.Response(200, json=copy.deepcopy(self.rules))
        if p == "/api/_/bootstrap/agents_groups":
            return httpx.Response(200, json={"groups": A.GROUPS, "agents": self.AGENTS})
        if p == "/search/autocomplete/requesters":
            needle = q.get("q", "").casefold()
            rows = [{**x, "details": f"{x['value']} <{x['email']}>"} for x in A.PEOPLE
                    if needle in x["value"].casefold() or needle in x["email"]]
            return httpx.Response(200, json={"results": rows})
        if p.startswith("/api/_/changes/") and p.endswith("/planning-fields"):
            return httpx.Response(200, json={"change_planning_fields": []})
        if p.startswith("/api/_/changes/"):
            return httpx.Response(200, json={"change": copy.deepcopy(self.change)})
        return httpx.Response(404, json={"message": f"no route GET {p}"})

    def write(self, method: str, p: str, body: Any) -> httpx.Response:
        if method == "POST" and p == "/api/_/changes":
            return httpx.Response(200, json={"change": {"id": 900000502, "human_display_id": "CHN-502"}})
        if method == "PUT" and p.startswith("/api/_/changes/"):
            for key, value in body.items():
                if key == "custom_fields":
                    self.change.setdefault(key, {}).update(value)
                else:
                    self.change[key] = value
            return httpx.Response(200, json={"change": {"id": int(p.rsplit("/", 1)[1])}})
        return httpx.Response(404, json={"message": f"no route {method} {p}"})


# ---------------------------------------------------------------------------
# Globex: no workspaces, no rules, no sections, German labels, restricted role


def _gf(id_: int, name: str, label: str, ftype: str, pos: int | None, **kw: Any) -> dict[str, Any]:
    return {"id": id_, "name": name, "label": label, "description": "", "field_type": ftype,
            "required": kw.pop("required", False), "position": pos, "default_field": ftype.startswith("default_"),
            "choices": kw.pop("choices", []), "nested_fields": [], "placeholder": kw.pop("placeholder", ""),
            "helptext": kw.pop("helptext", ""), "hint": False, "content": None, "displayed_to_agents": True,
            "data_source": 0, "lookup_config": None, "updated_at": "2031-02-03T04:05:06Z", **kw}


def _ch(*labels: str) -> list[dict[str, Any]]:
    return [{"id": i + 1, "value": v} for i, v in enumerate(labels)]


def globex_fields() -> dict[str, Any]:
    return {"change_fields": [
        _gf(9000010001, "requester", "Antragsteller", "default_requester", 1, required=True),
        _gf(9000010002, "subject", "Betreff", "default_subject", 2, required=True, placeholder="Kurzbeschreibung"),
        _gf(9000010003, "change_type", "Änderungstyp", "default_change_type", 3, required=True,
            choices=_ch("Normal", "Standard", "Notfall")),
        _gf(9000010004, "umgebung", "Umgebung", "custom_dropdown", 4, helptext="Produktion - Live-System || Test - Testsystem",
            choices=[{"id": 9000011001, "display_id": 1, "value": "Produktion", "label": "Produktion"},
                     {"id": 9000011002, "display_id": 2, "value": "Test", "label": "Test"}]),
        _gf(9000010005, "description", "Beschreibung", "default_description", 5, required=True),
        _gf(9000010006, "priority", "Priorität", "default_priority", 6, required=True,
            choices=_ch("Niedrig", "Mittel", "Hoch", "Dringend")),
        _gf(9000010007, "group", "Gruppe", "default_group", 7),
        _gf(9000010008, "agent", "Bearbeiter", "default_agent", 8),
        _gf(9000010009, "planned_start_date", "Geplanter Beginn", "default_planned_start_date", 9, required=True),
        _gf(9000010010, "planned_end_date", "Geplantes Ende", "default_planned_end_date", 10, required=True),
        _gf(9000010011, "status", "Status", "default_status", 11, required=True, choices=_ch("Offen", "Geschlossen")),
    ]}


def globex_item() -> dict[str, Any]:
    def cf(id_: str, name: str, type_: int, pos: int, label: str, **kw: Any) -> dict[str, Any]:
        return {"id": id_, "name": name, "type": type_, "position": pos, "label": label,
                "required": kw.pop("required", False), "visible": True, "deleted": False,
                "placeholder": "", "field_options": kw.pop("field_options", {}), "choices": kw.pop("choices", []),
                "fields": [], "parent_id": None, **kw}

    return {"service_item": {
        "id": 4500001, "display_id": 501, "name": "Laptop-Anfrage", "short_description": "Neues Gerät",
        "category_id": 7101, "category_name": "Hardware", "updated_at": "2031-02-01T09:00:00Z",
        "description": "<p>Bitte Angebot anhängen.</p>", "allow_attachments": False, "allow_quantity": False,
        "attachment_mandatory": False, "configs": {"attachment_mandatory": False, "subject": "", "auto_gen_document": False},
        "custom_fields": [
            cf("g-why", "begruendung", 1001, 1, "Begründung", required=True),
            cf("g-model", "modell", 1003, 2, "Modell", required=True, choices=[
                {"id": "m1", "value": "Standard 14 Zoll", "label": "Standard 14 Zoll", "position": 1},
                {"id": "m2", "value": "Entwickler 16 Zoll", "label": "Entwickler 16 Zoll", "position": 2}]),
            cf("g-date", "benoetigt_bis", 1007, 3, "Benötigt bis", required=True, field_options={"date_only": "true"}),
            cf("g-offer", "angebot", 1014, 4, "Angebot (.pdf)", field_options={"allowed_file_types": "[\".pdf\"]",
                                                                               "max_attachment_limit": "1"}),
        ],
    }}


GLOBEX_PEOPLE = [{"id": 9000012001, "value": "Erika Muster", "email": "erika.muster@example.com"},
                 {"id": 9000012002, "value": "Jörg Beispiel", "email": "joerg.beispiel@example.com"}]


class Globex(Tenant):
    def get(self, p: str, q: dict[str, str]) -> httpx.Response:
        if "/ws/" in p:
            raise AssertionError(f"Globex has no workspaces; fsv asked for {p}")
        if p == "/api/_/change_form_fields":
            return httpx.Response(200, json=globex_fields())
        if p == "/api/_/business_rules":
            return httpx.Response(404, json={"message": "Not found"})
        if p == "/api/_/bootstrap/agents_groups":
            return httpx.Response(403, json={"message": "You are not authorized to perform this action."})
        if p == "/search/autocomplete/agents":
            return httpx.Response(403, json={"message": "You are not authorized to perform this action."})
        if p == "/search/autocomplete/requesters":
            needle = q.get("q", "").casefold()
            rows = [x for x in GLOBEX_PEOPLE if needle in x["value"].casefold() or needle in x["email"]]
            return httpx.Response(200, json={"results": rows})
        if p == "/api/v2/agents/me":
            return httpx.Response(200, json={"agent": {"id": 9000012100, "time_zone": "Berlin", "language": "de"}})
        if p == "/api/_/catalog/service-items/501":
            return httpx.Response(200, json=globex_item())
        return httpx.Response(404, json={"message": f"no route GET {p}"})

    def write(self, method: str, p: str, body: Any) -> httpx.Response:
        if method == "POST" and p == "/api/_/changes":
            return httpx.Response(200, json={"change": {"id": 7, "human_display_id": "CHN-7"}})
        if method == "POST" and p == "/api/_/catalog/service-items/501/place-request":
            return httpx.Response(200, json={"service_request": {"id": 9100000777, "display_id": 777,
                                                                 "human_display_id": "SR-777"}})
        return httpx.Response(404, json={"message": f"no route {method} {p}"})
