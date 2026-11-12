"""Synthetic native relationship envelopes and project proxy nested outcomes."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, change_relationships as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"rows": {kind: [] for kind in flow.KINDS}, "writes": [], "partial": False,
         "nested_fail": False, "failure": False, "no_effect": False}
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "changes/123":
                return httpx.Response(200, json={"change": {"id": 123, "subject": "Acme change"}})
            if path.startswith("freshrelease"):
                kind = "project" if path.endswith("fetch_linked_fr_projects") else "project-task"
                key = flow.KINDS[kind][0]
                assert req.url.params["placeholder[resourceId]"] == "123"
                return httpx.Response(200, json={"status": 200, "body": {key: copy.deepcopy(s["rows"][kind]),
                    "meta": {"page": 1, "total": len(s["rows"][kind]), "total_pages": 2 if s["partial"] else 1}}})
            if path.startswith("changes/123/"):
                key = path.rsplit("/", 1)[1]
                kind = next(k for k, pair in flow.KINDS.items() if pair[0] == key)
                if kind == "caused-incident":
                    assert req.url.params["filter"] == "associated_tickets_caused"
                return httpx.Response(200, json={key: copy.deepcopy(s["rows"][kind]), "meta": {"has_next": s["partial"]}})
            module, ident = path.split("/")
            return httpx.Response(200, json={module[:-1]: {"id": int(ident), "ticket_type": "Incident"}})
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body, dict(req.url.params)))
        if s["failure"]:
            raise httpx.ReadError("disconnect", request=req)
        if path.startswith("freshrelease"):
            if s["nested_fail"]:
                return httpx.Response(200, json={"status": 500, "body": {"error": "failed"}})
            kind = "project" if path.endswith("fr_project") else "project-task"
            if "unlink" in path:
                if not s["no_effect"]:
                    s["rows"][kind] = []
                return httpx.Response(200, json={"status": 200, "body": {"success": True}})
            row = {"id": 81 if kind == "project" else 91, "key": "ACME" if kind == "project" else "ACME-1",
                   "name": body["project"]["name"] if kind == "project" else body["activity_data"]["name"], "project_id": 81}
            if not s["no_effect"]:
                s["rows"][kind].append(row)
            return httpx.Response(200, json={"status": 200, "body": {"project" if kind == "project" else "issue": row}})
        key = path.split("/")[2]
        kind = next(k for k, pair in flow.KINDS.items() if pair[0] == key)
        adding = path.endswith("associate") or body.get("type") == "link"
        if not s["no_effect"]:
            s["rows"][kind] = [{"id": 71, "subject": "Acme record"}] if adding else []
        return httpx.Response(200, json={key: s["rows"][kind]}) if adding else httpx.Response(204, headers={"content-type": "application/json"})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s
    c.close()


def options(kind, action):
    if kind == "project":
        if action == "remove":
            return {"item": "ACME"}
        return {"project": {"name": "Acme project", "description": "Acme work", "start_date": "2026-01-01",
                            "end_date": "2026-01-02", "owner_email": "person@example.com", "project_category": "1"}}
    if kind == "project-task":
        return {"item": "ACME-1", "project_key": "ACME", "name": "Acme task" if action == "add" else None}
    return {"item": "71"}


@pytest.mark.parametrize("kind", list(flow.KINDS))
@pytest.mark.parametrize("action", ["add", "remove"])
def test_relationship_wire_and_exact_readback(api, kind, action):
    c, s = api
    if kind == "project-task":
        s["rows"]["project"] = [{"id": 81, "key": "ACME", "name": "Acme project"}]
    if action == "remove":
        s["rows"][kind] = [{"id": 81 if kind == "project" else 91 if kind == "project-task" else 71,
                            "key": "ACME" if kind == "project" else "ACME-1", "name": "Acme task", "project_id": 81}]
    p = flow.build_plan(c, "CHN-123", kind=kind, action=action, **options(kind, action))
    assert not s["writes"]
    r = flow.send(p, c, reviewed_hash=p.hash)
    assert r["verified"], r
    assert len(s["writes"]) == 1
    method, path, body, query = s["writes"][0]
    if kind == "release":
        assert method == "PUT" and body == {"type": "link" if action == "add" else "unlink"}
    elif kind in {"problem", "caused-incident"}:
        if action == "add":
            assert body == {"item_ids": [71], "module_type": "problem" if kind == "problem" else "ticket_cause"}
        else:
            assert method == "DELETE" and body == {"type": "unlink"}
            assert query == ({"module_type": "ticket_cause"} if kind == "caused-incident" else {})
    else:
        assert method == "POST"
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


@pytest.mark.parametrize("failure", ["nested_fail", "failure", "no_effect"])
def test_project_unknown_or_unverified_never_replays(api, failure):
    c, s = api
    p = flow.build_plan(c, 123, kind="project", action="add", **options("project", "add"))
    s[failure] = True
    if failure == "failure":
        with pytest.raises(ReviewedWriteError, match="unknown"):
            flow.send(p, c, reviewed_hash=p.hash)
    else:
        assert not flow.send(p, c, reviewed_hash=p.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


def test_unproven_existing_project_link_refused(api):
    c, s = api
    with pytest.raises(ValueError, match="create-and-associate"):
        flow.build_plan(c, 123, kind="project", action="add", item="81")
    assert not s["writes"]


def test_partial_project_inventory_refused(api):
    c, s = api
    s["partial"] = True
    with pytest.raises(ReviewedWriteError, match="complete"):
        flow.build_plan(c, 123, kind="project", action="add", **options("project", "add"))
    assert not s["writes"]


def test_relationship_cli_review(api):
    c, s = api
    r = CliRunner().invoke(cli.app, ["changes", "relationships", "write", "CHN-123", "release", "add",
                                      "--item", "71", "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    p = json.loads(r.stdout)
    assert p["method"] == "PUT" and p["payload"] == {"type": "link"}
    assert not s["writes"]
