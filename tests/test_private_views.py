"""Private view lifecycle through real offline transports; enums vary by tenant."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, private_views as views
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=[("acme.example.com", 81, 901), ("globex.example.com", 97, 902)])
def wire(request, monkeypatch, tmp_path):
    domain, visibility, user = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    original = {"id": 701, "user_id": user, "default": False, "visibility": visibility,
                "visibility_name": "Me", "name": "Acme source", "workspace_id": 17,
                "query_hash": [{"condition": "status", "operator": "is_in", "type": "default", "value": ["42"]}],
                "advanced_query_hash": None, "order_by": "created_at", "order_type": "desc"}
    state = {"rows": {701: original}, "writes": [], "failure": None, "omit_id": False, "partial": False}

    def handle(req):
        leaf = req.url.path.rsplit("/", 1)[-1]
        if req.method == "GET":
            if leaf == "ticket_filters":
                return httpx.Response(200, json={"ticket_filters": list(state["rows"].values()), "has_more": state["partial"]})
            row = state["rows"].get(int(leaf))
            if row is None:
                return httpx.Response(404, text="<html>Not found</html>")
            return httpx.Response(200, json={"ticket_filter": copy.deepcopy(row)})
        body = json.loads(req.content) if req.content else None
        state["writes"].append((req.method, body))
        if state["failure"] == "disconnect":
            raise httpx.ReadError("synthetic disconnect", request=req)
        if state["failure"]:
            return httpx.Response(state["failure"], json={"error": "synthetic"})
        if req.method == "DELETE":
            del state["rows"][int(leaf)]
            return httpx.Response(204)
        vid = 702 if req.method == "POST" else int(leaf)
        state["rows"][vid] = {**original, **body, "id": vid}
        return httpx.Response(201 if req.method == "POST" else 200,
                              json={} if state["omit_id"] else {"ticket_filter": {"id": vid}})

    clients = []
    def make():
        c = Client(cookies={})
        c._client.close()
        c._client = httpx.Client(transport=httpx.MockTransport(handle))
        monkeypatch.setattr(c, "me", lambda: {"id": user})
        monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
        clients.append(c)
        return c
    state.update(client=make(), make=make, visibility=visibility)
    yield state
    for c in clients:
        c.close()


def test_exact_lifecycle_and_html_404_verification(wire):
    c = wire["client"]
    p = views.build_plan(c, 701, action="create", name="Globex clone")
    assert p.payload["visibility"] == wire["visibility"]
    assert "user_id" not in p.payload and "default" not in p.payload
    result = views.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] and result["view_id"] == 702
    q = views.build_plan(c, 702, action="edit", name="Globex renamed")
    assert views.send(q, c, reviewed_hash=q.hash)["verified"]
    d = views.build_plan(c, 702, action="delete")
    assert views.send(d, c, reviewed_hash=d.hash)["verified"]
    assert 702 not in wire["rows"] and 701 in wire["rows"]
    assert [w[0] for w in wire["writes"]] == ["POST", "PUT", "DELETE"]


@pytest.mark.parametrize("change", [{"visibility_name": None}, {"visibility_name": "All agents"},
                                      {"default": True}, {"default": None}, {"system": True}, {"user_id": 999}])
def test_not_proven_private_owned_and_nondefault_refused(wire, change):
    wire["rows"][701].update(change)
    for action in ["create", "edit", "delete"]:
        with pytest.raises(ReviewedWriteError):
            views.build_plan(wire["client"], 701, action=action, name=None if action == "delete" else "Acme")
    assert not wire["writes"]


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_single_attempt_guard_across_clients(wire, failure):
    p = views.build_plan(wire["client"], 701, action="create", name="Globex")
    wire["failure"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        views.send(p, wire["client"], reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        views.send(p, wire["make"](), reviewed_hash=p.hash)
    assert len(wire["writes"]) == 1


def test_stale_metadata_blocks_and_missing_response_id_never_guesses(wire):
    p = views.build_plan(wire["client"], 701, action="create", name="Globex")
    wire["rows"][701]["name"] = "Changed source"
    with pytest.raises(ReviewedWriteError, match="changed"):
        views.send(p, wire["client"], reviewed_hash=p.hash)
    assert not wire["writes"]
    p = views.build_plan(wire["client"], 701, action="create", name="Globex")
    wire["omit_id"] = True
    result = views.send(p, wire["client"], reviewed_hash=p.hash)
    assert not result["verified"] and "do not resend" in result["warning"]
    assert len(wire["writes"]) == 1


def test_cli_review_full_hash_gate(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    args = ["tickets", "private-views", "write", "701", "create", "--name", "Globex", "--output", "json"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2 and not wire["writes"]
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["verified"]



def test_numeric_only_visibility_refusal_names_the_checked_endpoints(wire):
    """Real tenant shape: the exact view read carries only a number (here 1)."""
    wire["rows"][701].pop("visibility_name", None)
    wire["rows"][701]["visibility"] = 1
    with pytest.raises(ReviewedWriteError) as exc:
        views.build_plan(wire["client"], 701, action="create", name="Acme")
    message = str(exc.value)
    assert "GET /api/_/ticket_filters/701 returns visibility 1 (All agents)" in message
    assert "Myself (3)" in message and "Nothing was sent" in message
    assert "\n" not in message and not wire["writes"]
