"""Caller-level legacy transport checks, not durable-plan/readback certification.

These wrappers predate reviewed plans. Tests pin no automatic replay and CSRF
preparation at each caller; transport audit rows alone are not restart guards.
"""
import json

import httpx
import pytest

from fsv import config, create, journal, service
from fsv.client import Client
from fsv.errors import APIError
from fsv.resources import CHANGES


CASES = [
    ("note", "POST", "changes/123/notes", {"body": "Acme evidence", "private": True}),
    ("status", "PUT", "changes/123", {"status": 47}),
    ("asset-add", "PUT", "changes/123/assets/associate", {"item_ids": [701]}),
    ("asset-remove", "PUT", "changes/123/assets/detach", {"cmdb_request_id": 801}),
    ("task-update", "PUT", "changes/123/tasks/701", {"title": "Acme revised"}),
    ("task-delete", "PUT", "changes/123/tasks/update", {"id": "701", "action": "delete"}),
    ("clone-task", "POST", "changes/456/tasks", None),
]


@pytest.fixture
def wire(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    state = {"mutations": [], "failure": None}
    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "changes/123/assets":
                return httpx.Response(200, json={"associated_assets": [{"id": 801, "config_item": {"display_id": 701}}]})
            if path == "changes/123/tasks":
                return httpx.Response(200, json={"tasks": [{"id": 701, "title": "Acme task"}, {"id": 702, "title": "Globex task"}]})
            raise AssertionError("unexpected read " + path)
        state["mutations"].append(req)
        if state["failure"] == "disconnect":
            raise httpx.ReadError("synthetic disconnect", request=req)
        if state["failure"]:
            return httpx.Response(state["failure"], json={"error": "synthetic denial"})
        return httpx.Response(200, json={"note": {"id": 901}, "task": {"id": 902}, "change": {"id": 123}})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic-token")
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    state["client"] = c
    yield state
    c.close()


def invoke(kind, client):
    if kind == "note":
        return service.add_note(CHANGES, 123, "Acme evidence", client=client)
    if kind == "status":
        return service.update_item(CHANGES, 123, {"status": 47}, client=client)
    if kind == "asset-add":
        return create.associate_assets(123, [701], c=client)
    if kind == "asset-remove":
        return create.dissociate_assets(123, [701], c=client)
    if kind == "task-update":
        return create.update_task(123, 701, {"title": "Acme revised"}, c=client)
    if kind == "task-delete":
        return create.delete_task(123, 701, c=client)
    return create.clone_tasks(123, 456, c=client)


@pytest.mark.parametrize("kind,method,path,payload", CASES)
@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_each_legacy_caller_never_automatically_replays(wire, kind, method, path, payload, failure):
    wire["failure"] = failure
    with pytest.raises((APIError, httpx.ReadError)):
        invoke(kind, wire["client"])
    assert len(wire["mutations"]) == 1
    req = wire["mutations"][0]
    assert req.method == method and req.url.path.endswith("/" + path)
    assert req.headers["x-csrf-token"] == "synthetic-token"
    if payload is not None:
        assert json.loads(req.content) == payload
    rows = journal.read()
    audit = [row for row in rows if row.get("method") == method]
    assert audit and audit[-1]["ok"] is False
    # No claim that these old wrappers have the reviewed sender's plan guard.
    assert not any(row.get("kind", "").startswith("plan:") for row in rows)


@pytest.mark.parametrize("kind,method,path,payload", CASES)
def test_csrf_preparation_failure_prevents_each_legacy_mutation(wire, monkeypatch, kind, method, path, payload):
    def fail():
        raise RuntimeError("synthetic session preparation failure")
    monkeypatch.setattr(wire["client"], "_get_csrf", fail)
    with pytest.raises(RuntimeError, match="preparation"):
        invoke(kind, wire["client"])
    assert not wire["mutations"]


@pytest.mark.parametrize("kind,method,path,payload", CASES)
def test_legacy_success_contracts_are_not_confused_with_independent_readback(wire, kind, method, path, payload):
    invoke(kind, wire["client"])
    expected = 2 if kind == "clone-task" else 1
    assert len(wire["mutations"]) == expected
    assert all(req.method == method and req.url.path.endswith("/" + path) for req in wire["mutations"])
    if payload is not None:
        assert json.loads(wire["mutations"][0].content) == payload
    if kind == "clone-task":
        # Preserve the existing envelope; this is a compatibility test, not a
        # claim of accepted native task-clone wire or wider task field parity.
        body = json.loads(wire["mutations"][0].content)
        assert body["change_task"]["title"] == "Acme task"
    assert not any(row.get("kind", "").startswith("plan:") for row in journal.read())
