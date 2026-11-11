"""Offline association contracts and complete, error-honest inventories."""
import json
from dataclasses import replace
from types import SimpleNamespace

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_associations as assoc, cli, config, create, journal
from fsv.tui import actions
from test_ticket_notes import mock_client

TID = 9000003101


@pytest.mark.parametrize("unlink", [False, True])
@pytest.mark.parametrize("response", [{}, {"ticket": {"id": False}}, ["unobserved"]])
def test_exact_single_request_complete_readback(monkeypatch, unlink, response):
    calls = []
    wrote = False
    def handler(req):
        nonlocal wrote
        calls.append(req)
        if req.method != "GET":
            wrote = True
            return httpx.Response(200, json=response)
        rows = [{"id": TID}] if wrote != unlink else []
        # Exact ticket appears on page two; read all pages on both sides.
        page = int(req.url.params.get("page", 1))
        return httpx.Response(200, json={"tickets": [] if page == 1 else rows,
                                       "meta": {"has_next": page == 1}})
    c = mock_client(monkeypatch, handler)
    try:
        plan = assoc.build_plan("CHN-123", [TID], unlink=unlink)
        out = assoc.send(plan, c, reviewed_hash=plan.hash)
        assert out["verified"] is True and out["status"] == "submitted"
        writes = [r for r in calls if r.method != "GET"]
        assert len(writes) == 1
        req = writes[0]
        assert req.method == ("DELETE" if unlink else "PUT")
        assert req.url.path == f"/api/_/changes/123/tickets/{TID if unlink else 'associate'}"
        assert dict(req.url.params) == ({"module_type": "ticket"} if unlink else {})
        assert json.loads(req.content) == ({"type": "unlink"} if unlink else {"item_ids": [TID], "module_type": "ticket"})
        assert req.headers["x-csrf-token"] == "synthetic-token"
        assert [r["event"] for r in journal.read() if r["kind"] == "plan:change_ticket_association"] == ["started", "done"]
        with pytest.raises(assoc.AssociationError, match="do not resubmit"):
            assoc.send(plan, c, reviewed_hash=plan.hash)
        assert len([r for r in calls if r.method != "GET"]) == 1
    finally:
        c.close()


@pytest.mark.parametrize("unlink", [False, True])
@pytest.mark.parametrize("failure", [403, 408, 429, 500, "timeout", "readback", "wrong-state", "malformed-inventory", "page2"])
def test_failure_no_replay_or_false_success(monkeypatch, unlink, failure):
    calls = []
    wrote = False
    def handler(req):
        nonlocal wrote
        calls.append(req)
        if req.method != "GET":
            wrote = True
            if failure == "timeout":
                raise httpx.ReadTimeout("synthetic uncertainty", request=req)
            return httpx.Response(failure if isinstance(failure, int) else 200, json={})
        rows = [{"id": TID}] if unlink else []
        if wrote:
            if failure == "readback" or (failure == "page2" and req.url.params.get("page") == "2"):
                return httpx.Response(403, json={"error": "synthetic denial"})
            if failure == "malformed-inventory":
                return httpx.Response(200, json={"tickets": [None]})
            if failure != "wrong-state":
                rows = [] if unlink else [{"id": TID}]
        return httpx.Response(200, json={"tickets": rows, "meta": {"has_next": wrote and failure == "page2"}})
    c = mock_client(monkeypatch, handler)
    try:
        plan = assoc.build_plan(123, [TID], unlink=unlink)
        with pytest.raises(assoc.AssociationError, match="rejected|unknown"):
            assoc.send(plan, c, reviewed_hash=plan.hash)
        assert len([r for r in calls if r.method != "GET"]) == 1
        events = [r["event"] for r in journal.read() if r["kind"] == "plan:change_ticket_association"]
        assert events == (["started", "failed"] if failure == 403 else ["started"])
        if failure != 403:
            with pytest.raises(assoc.AssociationError, match="do not resubmit"):
                assoc.send(plan, c, reviewed_hash=plan.hash)
            assert len([r for r in calls if r.method != "GET"]) == 1
    finally:
        c.close()


@pytest.mark.parametrize("case", ["page2", "cycle", "limit", "malformed", "invalid-id"])
def test_inventory_partial_rows_retained(case):
    calls = []
    def get(path, params):
        calls.append((path, params))
        key = path.rsplit("/", 1)[-1]
        if key != "tickets":
            return {key: []}
        page = int(params.get("page", 1))
        if page > 1:
            if case == "page2":
                raise RuntimeError("synthetic denial")
            if case == "malformed":
                return {key: [False]}
        if case == "invalid-id":
            return {key: [{"id": False}]}
        if case == "cycle":
            return {key: [{"id": TID}], "next_page_url": "?page=2"}
        return {key: [{"id": TID}], "meta": {"has_next": True}}
    out = create.get_change_associations(123, SimpleNamespace(int_get=get))
    assert out["complete"] is False and out["warnings"]
    assert out["tickets"] == ([] if case == "invalid-id" else [{"id": TID}])
    assert out["meta"]["tickets"]["complete"] is False
    assert len(calls) <= 52


def test_review_guards_and_wrappers(monkeypatch):
    monkeypatch.setattr(assoc, "get_client", lambda: pytest.fail("no client"))
    plan = assoc.build_plan(123, [TID])
    assert assoc.send(plan, reviewed_hash=None)["status"] == "cancelled"
    assert create.associate_ticket(123, [TID])["status"] == "cancelled"
    assert create.dissociate_ticket(123, TID)["status"] == "cancelled"
    assert assoc.send(plan, reviewed_hash=None, dry_run=True)["plan_hash"] == plan.hash
    for options in ({"reviewed_hash": "wrong"}, {"reviewed_hash": plan.hash, "read_only": True}):
        with pytest.raises(assoc.AssociationError):
            assoc.send(plan, **options)
    with pytest.raises(assoc.AssociationError):
        assoc.send(replace(plan, tenant="globex.example.com"), reviewed_hash=plan.hash)
    with pytest.raises(assoc.AssociationError):
        assoc.build_plan(123, [TID, TID + 1], unlink=True)
    with pytest.raises(ValueError, match="exact reviewed plan"):
        actions.apply_change_action("link", 123, {"id": TID}, {}, object())
    assert journal.read() == []


def test_tui_dispatch_uses_exact_plan(monkeypatch):
    calls = []
    monkeypatch.setattr(assoc, "send", lambda *args, **kw: calls.append((args, kw)) or {"verified": True})
    for unlink in (False, True):
        plan = assoc.build_plan(123, [TID], unlink=unlink)
        out = actions.apply_change_action("unlink" if unlink else "link", 123,
                                          {"id": TID, "_association_plan": plan}, {}, "client")
        assert out["verified"] is True
        assert calls[-1] == ((plan, "client"), {"reviewed_hash": plan.hash})


def test_cli_review_and_exact_hash(monkeypatch):
    runner = CliRunner()
    monkeypatch.setattr(cli, "_resolve_change_ticket_id", lambda value: TID)
    calls = []
    monkeypatch.setattr(assoc, "get_client", lambda: pytest.fail("no client"))
    dry = runner.invoke(cli.app, ["changes", "associations", "add", "CHN-123", "SR-456", "--dry-run"])
    assert dry.exit_code == 0, dry.output
    review = json.loads(dry.stdout)
    assert review["ticket_ids"] == [TID]
    assert journal.read() == []
    for extra in (["--yes"], ["--yes", "--expect-plan", "wrong"], ["--read-only"]):
        result = runner.invoke(cli.app, ["changes", "associations", "add", "CHN-123", "SR-456", *extra])
        assert result.exit_code != 0
    monkeypatch.setattr(assoc, "send", lambda plan, **kw: calls.append((plan, kw)) or {"verified": True, "status": "submitted"})
    result = runner.invoke(cli.app, ["changes", "associations", "add", "CHN-123", "SR-456", "--yes", "--expect-plan", review["plan_hash"], "--json"])
    assert result.exit_code == 0, result.output
    assert json.loads(result.stdout)["verified"] is True
    assert calls[0][1] == {"reviewed_hash": review["plan_hash"], "read_only": False}


@pytest.mark.parametrize("unlink", [False, True])
@pytest.mark.parametrize("case", ["preflight", "journal", "noop"])
def test_no_send_when_preparation_fails_or_state_already_matches(monkeypatch, unlink, case):
    def get(path, params):
        if case == "preflight":
            raise RuntimeError("synthetic unavailable")
        present = not unlink if case == "noop" else unlink
        return {"tickets": [{"id": TID}] if present else []}
    c = SimpleNamespace(int_get=get, _get_csrf=lambda: "synthetic-token",
                        _request=lambda *a, **kw: pytest.fail("no write"))
    plan = assoc.build_plan(123, [TID], unlink=unlink)
    if case == "journal":
        monkeypatch.setattr(journal, "record_plan", lambda *a, **kw: None)
    if case == "noop":
        out = assoc.send(plan, c, reviewed_hash=plan.hash)
        assert out["verified"] is True and out["status"] == "already_in_requested_state"
    else:
        with pytest.raises(assoc.AssociationError, match="no write attempted"):
            assoc.send(plan, c, reviewed_hash=plan.hash)
    assert journal.read() == []


def test_tui_inventory_page_two_failure_preserves_rows():
    from fsv import views
    from fsv.resources import CHANGES
    def get(path, params):
        key = path.rsplit("/", 1)[-1]
        if key != "tickets" or params.get("filter"):
            return {key: []}
        if int(params.get("page", 1)) == 2:
            raise RuntimeError("synthetic interruption")
        return {key: [{"id": TID}], "meta": {"has_next": True}}
    tab = views.load_tab(SimpleNamespace(int_get=get), CHANGES, {"display_id": 123}, "associations")
    rows, warnings, complete = actions.target_inventory(tab, False)
    assert rows == [{"id": TID}] and warnings and complete is False


def test_cli_list_unavailable_not_empty(monkeypatch):
    monkeypatch.setattr(create, "get_change_associations", lambda *args: {
        "tickets": [{"id": TID, "subject": "Acme access"}], "problems": [], "releases": [],
        "meta": {"tickets": {"complete": False}}, "complete": False, "warnings": ["tickets: incomplete"]})
    runner = CliRunner()
    result = runner.invoke(cli.app, ["changes", "associations", "ls", "CHN-123", "--json"])
    assert result.exit_code == 0, result.output
    out = json.loads(result.stdout)
    assert out["complete"] is False and out["tickets"][0]["id"] == TID
    assert "tickets: incomplete" in result.stderr
