"""Real Client/MockTransport reviewed property writes on two tenant schemas."""
from __future__ import annotations

import json
from types import SimpleNamespace

import httpx
import pytest

from fsv import config, formmeta, journal, ticket_properties as props
from test_ticket_notes import mock_client


@pytest.fixture(params=[("acme.example.com", 123, 2, 3, 9000008101),
                        ("globex.example.com", 742, 2, 3, 9000008201)])
def wire(request, monkeypatch):
    domain, cid, opened, pending, fid = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    monkeypatch.setattr(formmeta, "configured_workspace", lambda: 1)
    w = SimpleNamespace(cid=cid, pending=pending, requests=[], failure=None, readback=None,
        row={"id": cid, "subject": "Acme review", "status": opened, "tags": ["before"],
             "planned_start_date": None, "planned_end_date": None},
        fields=[{"id": fid, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1},
                {"id": fid + 1, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
                 "choices": [{"id": opened, "value": "Open"}, {"id": pending, "value": "Pending"},
                             {"id": 4, "value": "Resolved"}, {"id": 5, "value": "Closed"}]}],
        conv_id=cid + 9000000000, conversation=None, conv_readback=None, conv_transform=None,
        lookup_name=domain.split(".")[0].title() + " Operator")

    def handler(req):
        w.requests.append(req)
        path = req.url.path.removeprefix("/api/_/")
        if req.method in {"PUT", "POST"}:
            if w.failure == "disconnect":
                raise httpx.ReadError("synthetic disconnect", request=req)
            if w.failure:
                return httpx.Response(w.failure, json={"error": "synthetic denial"})
            payload = json.loads(req.content)
            if req.method == "PUT":
                w.row.update(payload)
                return httpx.Response(200, json={"ticket": {"id": cid}})
            w.conversation = {"id": w.conv_id, "body": payload["body"]}
            for key in ("cc_emails", "bcc_emails", "private", "from_email"):
                if key in payload:
                    w.conversation[key] = payload[key]
            if "notify_emails" in payload:
                w.conversation["to_emails"] = payload["notify_emails"]
            if "helpdesk_ticket" in payload:
                w.row["status"] = payload["helpdesk_ticket"]["status"]
            return httpx.Response(200, json={"conversation": {"id": w.conv_id}})
        if path == "ticket_form_fields":
            return httpx.Response(200, json={"ticket_fields": w.fields})
        if path.endswith("business_rules"):
            return httpx.Response(200, json={"business_rules": []})
        if "/search/autocomplete/" in path:
            return httpx.Response(200, json={"results": [{"id": cid + 8000, "value": w.lookup_name,
                "email": req.url.params.get("q")} ]})
        if path == f"tickets/{cid}/conversations":
            conversation = dict(w.conversation) if w.conversation else None
            if conversation and w.conv_transform:
                w.conv_transform(conversation)
            if conversation and w.conv_readback == "wrong-id":
                conversation["id"] += 1
            elif conversation and w.conv_readback:
                if w.conv_readback == "contradict":
                    conversation["body"] = "different"
                else:
                    conversation.pop(w.conv_readback, None)
            return httpx.Response(200, json={"conversations": [conversation] if conversation else [], "meta": {"count": 1}})
        if path == f"tickets/{cid}":
            row = dict(w.row)
            mutated = any(r.method in {"PUT", "POST"} for r in w.requests)
            if mutated and w.readback == "wrong-id":
                row["id"] = cid + 1
            if mutated and w.readback == "missing":
                row.pop("tags", None)
            if mutated and w.readback == "contradict":
                row["tags"] = ["other"]
            return httpx.Response(200, json={"ticket": row})
        pytest.fail(f"unexpected request {req.method} {req.url}")
    w.client = mock_client(monkeypatch, handler)
    yield w
    w.client.close()


def test_exact_property_http_headers_dates_token_and_metadata(wire):
    w = wire
    p = props.build_plan(w.client, w.cid, tags=["after"], planned_start="2030-01-02T10:00:00+07:00",
        planned_end="2030-01-02T12:00:00+07:00", answers={"Status": "Pending"})
    assert not [r for r in w.requests if r.method == "PUT"]
    assert p.payload["status"] == w.pending
    result = props.send(p, w.client, reviewed_hash=p.hash)
    assert result["verified"], result
    writes = [r for r in w.requests if r.method == "PUT"]
    assert len(writes) == 1 and str(writes[0].url) == f"{config.API_INT}/tickets/{w.cid}"
    assert writes[0].headers["content-type"] == "application/json"
    assert writes[0].headers["x-csrf-token"] == "synthetic-token"
    assert json.loads(writes[0].content) == p.payload
    assert "+07:00" in p.payload["planned_start_date"] and p.payload["br_validation_excludes"]


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_http_ambiguous_failure_never_replayed_even_with_new_client(wire, monkeypatch, failure):
    w = wire
    p = props.build_plan(w.client, w.cid, tags=["after"])
    w.failure = failure
    with pytest.raises(props.PropertyError, match="unknown"):
        props.send(p, w.client, reviewed_hash=p.hash)
    replacement = mock_client(monkeypatch, lambda r: pytest.fail("restart may not resend"))
    try:
        with pytest.raises(props.PropertyError, match="unknown"):
            props.send(p, replacement, reviewed_hash=p.hash)
    finally:
        replacement.close()
    assert len([r for r in w.requests if r.method == "PUT"]) == 1
    assert [r for r in journal.read() if r.get("kind") == "plan:ticket_properties"][-1]["event"] == "started"


@pytest.mark.parametrize("readback", ["wrong-id", "missing", "contradict"])
def test_bad_readback_stays_unverified_without_resend(wire, readback):
    w = wire
    w.readback = readback
    p = props.build_plan(w.client, w.cid, tags=["after"])
    result = props.send(p, w.client, reviewed_hash=p.hash)
    assert result["verified"] is False and "do not resend" in result["warning"]
    assert len([r for r in w.requests if r.method == "PUT"]) == 1


def test_changed_state_or_metadata_blocks_write(wire):
    w = wire
    p = props.build_plan(w.client, w.cid, answers={"Status": "Pending"})
    w.fields[1]["choices"].append({"id": 9, "value": "Assigned"})
    with pytest.raises(props.PropertyError, match="changed"):
        props.send(p, w.client, reviewed_hash=p.hash)
    assert not [r for r in w.requests if r.method == "PUT"]
