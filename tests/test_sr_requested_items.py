"""Synthetic requested-item accepted wires, catalog validation and exact readback."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli_sr, config, journal, sr_requested_items as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    fields = [{"id": i, "name": name, "label": name.title(), "field_type": typ, "type": 1001 if name == "title" else 1008, "required": name == "title",
               "field_options": {}, "position": i} for i, name, typ in [(1, "title", "custom_text"), (2, "details", "custom_paragraph")]]
    item = {"id": 801, "display_id": 71, "name": "Acme service", "workspace_id": 2, "custom_fields": fields}
    s = {"rows": [{"id": 901, "ticket_id": 123, "item_id": 801, "item": item, "is_parent": True,
                   "stage": {"id": 1, "name": "Requested"}}],
         "answers": {901: {"title": "Original", "details": "Original details"}}, "writes": [],
         "blocked": False, "failure": False, "missing": False, "rules_fail": False}
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "tickets/123":
                # Native exact ticket reads expose ``type``; echoes may use ``ticket_type``.
                return httpx.Response(200, json={"ticket": {"id": 123, **s.get("kind_fields", {"type": "Service Request"})}})
            if "business_rules" in path:
                return httpx.Response(403, json={"error": "denied"}) if s["rules_fail"] else httpx.Response(200, json={"business_rules": []})
            if path == "catalog/service-items/71":
                return httpx.Response(200, json={"service_item": item})
            if path == "tickets/123/requested_items":
                return httpx.Response(200, json={"requested_items": copy.deepcopy(s["rows"]), "meta": {"e_signature_limit_reached": False}})
            ident = int(path.rsplit("/", 1)[1])
            answers = copy.deepcopy(s["answers"][ident])
            if s["missing"] and s["writes"]:
                answers.pop("details", None)
            return httpx.Response(200, json={"requested_item": {"id": ident, "item_id": 71, "custom_fields": answers,
                "item": {"custom_fields": fields, "description": "Acme item"}}, "business_rules": {
                    "hidden_req_item_fields": {str(ident): ["details"] if s["blocked"] else []},
                    "non_editable_item_fields": {str(ident): []}}})
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body))
        if s["failure"]:
            raise httpx.ReadError("disconnect", request=req)
        if req.method == "POST":
            assert body["item_id"] == "71"
            s["rows"].append({"id": 902, "ticket_id": 123, "item_id": 801, "item": item, "is_parent": False,
                              "stage": {"id": 1, "name": "Requested"}})
            s["answers"][902] = body["custom_fields"]
            return httpx.Response(200, json={"requested_item": {"id": 902, "item_id": 71}})
        ident = int(path.split("/")[3])
        if req.method == "DELETE":
            s["rows"] = [r for r in s["rows"] if r["id"] != ident]
            return httpx.Response(204, headers={"content-type": "application/json"})
        if req.method == "PATCH":
            next(r for r in s["rows"] if r["id"] == ident)["stage"] = {"id": 2, "name": "Delivered"}
        else:
            s["answers"][ident].update(body["custom_fields"])
        return httpx.Response(200, json={"requested_item": {"id": ident, "item_id": 801}})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    yield c, s
    c.close()


def opts(action):
    if action == "add":
        return {"catalog_item": "71", "answers": {"title": "Acme request", "details": "Additional item"}}
    if action == "edit":
        return {"item": 901, "answers": {"title": "Acme edited", "details": "Updated details"}}
    return {"item": 902 if action == "remove" else 901, **({"stage": 2} if action == "stage" else {})}


@pytest.mark.parametrize("action,method", [("edit", "PUT"), ("stage", "PATCH"), ("add", "POST"), ("remove", "DELETE")])
def test_exact_requested_item_wire(api, action, method):
    c, s = api
    if action == "remove":
        s["rows"].append({**copy.deepcopy(s["rows"][0]), "id": 902, "is_parent": False})
    p = flow.build_plan(c, "SR-123", action=action, **opts(action))
    assert not s["writes"]
    result = flow.send(p, c, reviewed_hash=p.hash)
    assert result["verified"], result
    assert len(s["writes"]) == 1 and s["writes"][0][0] == method
    assert s["writes"][0][2] == p.payload
    assert any(r["id"] == 901 for r in s["rows"])
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("mode", ["blocked", "rules_fail"])
def test_editability_and_rule_failure_never_write(api, mode):
    c, s = api
    s[mode] = True
    with pytest.raises(ReviewedWriteError):
        flow.build_plan(c, 123, action="edit", **opts("edit"))
    assert not s["writes"]


def test_parent_removal_and_unproven_stage_fail_closed(api):
    c, s = api
    with pytest.raises(ReviewedWriteError, match="non-parent"):
        flow.build_plan(c, 123, action="remove", item=901)
    with pytest.raises(ValueError, match="stage 2"):
        flow.build_plan(c, 123, action="stage", item=901, stage=3)
    assert not s["writes"]


@pytest.mark.parametrize("failure", ["failure", "missing"])
def test_ambiguous_or_unverified_answer_never_replays(api, failure):
    c, s = api
    p = flow.build_plan(c, 123, action="edit", **opts("edit"))
    s[failure] = True
    if failure == "failure":
        with pytest.raises(ReviewedWriteError, match="unknown"):
            flow.send(p, c, reviewed_hash=p.hash)
    else:
        assert not flow.send(p, c, reviewed_hash=p.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


def test_sr_item_cli_review(api):
    c, s = api
    args = ["items", "write", "SR-123", "edit", "--item", "901", "--answers-json", '{"title":"Acme edited"}', "--json"]
    runner = CliRunner()
    review = runner.invoke(cli_sr.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    p = json.loads(review.stdout)
    sent = runner.invoke(cli_sr.app, args + ["--yes", "--expect-plan", p["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.stdout)["verified"]


@pytest.mark.parametrize("fields,allowed", [
    ({"type": "Service Request"}, True), ({"ticket_type": "Service Request"}, True),
    ({"type": "Service Request", "ticket_type": "service_request"}, True),
    ({"type": "Incident"}, False), ({"type": "Service Request", "ticket_type": "Incident"}, False),
    ({}, False), ({"type": None}, False)])
@pytest.mark.parametrize("action", ["edit", "stage", "add"])
def test_parent_type_proof_uses_native_exact_read(api, fields, allowed, action):
    c, s = api
    s["kind_fields"] = fields
    if allowed:
        assert flow.build_plan(c, "SR-123", action=action, **opts(action)).method in {"PUT", "PATCH", "POST"}
    else:
        with pytest.raises(ReviewedWriteError, match="service request|ticket type"):
            flow.build_plan(c, "SR-123", action=action, **opts(action))
    assert not s["writes"]
