"""Offline synthetic change-task request contract and uncertain-outcome tests."""
import json
from dataclasses import replace
from types import SimpleNamespace

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_tasks as tasks, cli, config, journal
from test_ticket_notes import mock_client


@pytest.fixture(params=["acme.example.com", "globex.example.com"])
def tenant(request, monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", request.param)
    monkeypatch.setattr(config, "API_INT", f"https://{request.param}/api/_")
    return request.param


@pytest.mark.parametrize("owner", [None, 9000002001])
@pytest.mark.parametrize("envelope", ["task", "bare", "missing"])
def test_one_exact_post_and_readback(monkeypatch, tenant, owner, envelope):
    calls = []
    exact = {"id": 9000002010, "title": "Acme verification", "owner_id": owner}
    old = {**exact, "id": 9000002009}
    def handler(req):
        calls.append(req)
        if req.method == "POST":
            body = {"task": exact} if envelope == "task" else exact if envelope == "bare" else {}
            return httpx.Response(200, json=body)
        return httpx.Response(200, json={"tasks": [old] if len(calls) == 1 else [old, exact]})
    c = mock_client(monkeypatch, handler)
    try:
        plan = tasks.build_plan("CHN-123", exact["title"], status_id=1, owner_id=owner)
        result = tasks.send(plan, c, reviewed_hash=plan.hash)
        posts = [r for r in calls if r.method == "POST"]
        assert len(posts) == 1 and str(posts[0].url) == f"https://{tenant}/api/_/changes/123/tasks"
        assert posts[0].headers["x-csrf-token"] == "synthetic-token"
        expected = {"title": exact["title"], "status": 1, "notify_before": 0}
        if owner is not None:
            expected["owner_id"] = owner
        assert json.loads(posts[0].content) == expected
        assert result["task_id"] == exact["id"] and result["verified"] is True
        assert result["readback"] == exact
        events = [r for r in journal.read() if r["kind"] == "plan:change_task_create"]
        assert [r["event"] for r in events] == ["started", "done"]
        assert "Acme verification" not in journal.journal_path().read_text()
        with pytest.raises(tasks.TaskError, match="sent or its outcome is unknown"):
            tasks.send(plan, c, reviewed_hash=plan.hash)
        assert len(calls) == 3
    finally:
        c.close()


@pytest.mark.parametrize("case", ["none", "ambiguous", "wrong-owner", "wrong-title", "partial", "read-error", "malformed"])
def test_missing_response_id_never_guesses_or_resubmits(monkeypatch, case):
    calls = []
    row = {"id": 9000002010, "title": "Acme review", "owner_id": 9000002001}
    def handler(req):
        calls.append(req)
        if req.method == "POST":
            return httpx.Response(200, json={"task": {"id": False}})
        if len(calls) == 1:
            return httpx.Response(200, json={"tasks": [{**row, "id": 9000002009}]})
        if case == "read-error":
            raise RuntimeError("synthetic read failure")
        rows = [row]
        if case == "none":
            rows = [{**row, "id": 9000002009}]
        elif case == "ambiguous":
            rows += [{**row, "id": 9000002011}]
        elif case == "wrong-owner":
            rows = [{**row, "owner_id": 9000002002}]
        elif case == "wrong-title":
            rows = [{**row, "title": "Globex review"}]
        if case == "malformed":
            return httpx.Response(200, json={"tasks": None})
        return httpx.Response(200, json={"tasks": rows, "meta": {"has_next": case == "partial"}})
    c = mock_client(monkeypatch, handler)
    try:
        plan = tasks.build_plan(123, row["title"], status_id=1, owner_id=row["owner_id"])
        with pytest.raises(tasks.TaskError, match="outcome unknown"):
            tasks.send(plan, c, reviewed_hash=plan.hash)
        assert [r["event"] for r in journal.read() if r["kind"] == "plan:change_task_create"] == ["started"]
        with pytest.raises(tasks.TaskError, match="do not resubmit"):
            tasks.send(plan, c, reviewed_hash=plan.hash)
        assert len([r for r in calls if r.method == "POST"]) == 1
    finally:
        c.close()


@pytest.mark.parametrize("failure", [400, 401, 403, 408, 429, 500, "timeout"])
def test_post_failure_single_attempt(monkeypatch, failure):
    calls = []
    def handler(req):
        calls.append(req)
        if req.method == "GET":
            return httpx.Response(200, json={"tasks": []})
        if failure == "timeout":
            raise httpx.ReadTimeout("synthetic uncertainty", request=req)
        return httpx.Response(failure, json={"error": "synthetic rejection"})
    c = mock_client(monkeypatch, handler)
    try:
        plan = tasks.build_plan(123, "Acme review", status_id=1)
        with pytest.raises(tasks.TaskError, match="rejected|unknown"):
            tasks.send(plan, c, reviewed_hash=plan.hash)
        assert len([r for r in calls if r.method == "POST"]) == 1
        events = [r["event"] for r in journal.read() if r["kind"] == "plan:change_task_create"]
        assert events == (["started", "failed"] if failure in (400, 403) else ["started"])
        if failure not in (400, 403):
            with pytest.raises(tasks.TaskError, match="do not resubmit"):
                tasks.send(plan, c, reviewed_hash=plan.hash)
            assert len([r for r in calls if r.method == "POST"]) == 1
    finally:
        c.close()


@pytest.mark.parametrize("readback", [[], [{"id": 9000002010, "title": "Other", "owner_id": None}],
                                      [{"id": 9000002011, "title": "Acme review", "owner_id": None}]])
def test_returned_id_not_replaced_with_title_match(monkeypatch, readback):
    calls = []
    def handler(req):
        calls.append(req)
        return httpx.Response(200, json={"task": {"id": "9000002010"}} if req.method == "POST"
                              else {"tasks": [] if len(calls) == 1 else readback})
    c = mock_client(monkeypatch, handler)
    try:
        plan = tasks.build_plan(123, "Acme review", status_id=1)
        result = tasks.send(plan, c, reviewed_hash=plan.hash)
        assert result["task_id"] == 9000002010 and result["verified"] is False
        assert "do not resend" in result["warning"]
        with pytest.raises(tasks.TaskError):
            tasks.send(plan, c, reviewed_hash=plan.hash)
        assert len(calls) == 3
    finally:
        c.close()


@pytest.mark.parametrize("mode", ["dry_run", "read_only", "cancel", "hash", "tenant"])
def test_pre_send_guards_no_io(monkeypatch, mode):
    monkeypatch.setattr(tasks, "get_client", lambda: pytest.fail("client should not be constructed"))
    plan = tasks.build_plan(123, "Acme review", status_id=1)
    kw = {"reviewed_hash": plan.hash}
    if mode in ("dry_run", "read_only"):
        kw[mode] = True
    elif mode == "cancel":
        kw["reviewed_hash"] = None
    elif mode == "hash":
        plan = replace(plan, title="Globex review")
    else:
        monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    if mode in ("dry_run", "cancel"):
        assert tasks.send(plan, **kw)["status"] in ("dry_run", "cancelled")
    else:
        with pytest.raises(tasks.TaskError):
            tasks.send(plan, **kw)
    assert journal.read() == []


@pytest.mark.parametrize("case", ["inventory", "csrf", "journal"])
def test_preparation_failure_no_post(monkeypatch, case):
    c = SimpleNamespace(_get_csrf=lambda: "synthetic-token", _request=lambda *a, **k: pytest.fail("POST forbidden"))
    monkeypatch.setattr(tasks, "_inventory", lambda *a: [])
    if case == "inventory":
        monkeypatch.setattr(tasks, "_inventory", lambda *a: (_ for _ in ()).throw(RuntimeError()))
    elif case == "csrf":
        c._get_csrf = lambda: (_ for _ in ()).throw(RuntimeError())
    else:
        monkeypatch.setattr(journal, "record_plan", lambda *a, **k: None)
    plan = tasks.build_plan(123, "Acme review", status_id=1)
    with pytest.raises(tasks.TaskError, match="no task"):
        tasks.send(plan, c, reviewed_hash=plan.hash)


@pytest.mark.parametrize("cid,title,owner", [(True, "x", None), (0, "x", None), ("SR-123", "x", None),
    ("123/x", "x", None), (123, "", None), (123, " \n", None), (123, "x\x00", None),
    (123, "x", True), (123, "x", 0), (123, "x", "123")])
def test_invalid_plan(cid, title, owner):
    with pytest.raises(ValueError):
        tasks.build_plan(cid, title, status_id=1, owner_id=owner)


def test_owner_resolution_exact_and_explicit(monkeypatch):
    monkeypatch.setattr(tasks, "get_client", lambda: pytest.fail("implicit lookup"))
    assert tasks.resolve_owner(None) is None
    assert tasks.resolve_owner("9000002001") == 9000002001
    with pytest.raises(ValueError):
        tasks.resolve_owner("me")
    row = {"email": "agent@example.com", "user_id": 9000002001, "id": 9000002002}
    c = SimpleNamespace(autocomplete=lambda *a: [row])
    assert tasks.resolve_owner("agent@example.com", c) == row["user_id"]
    for rows in ([], [row, row], [{**row, "email": "other@example.com"}], [{**row, "deactivated": True}], [{**row, "user_id": None}]):
        c.autocomplete = lambda *a: rows
        with pytest.raises(tasks.TaskError):
            tasks.resolve_owner("agent@example.com", c)


@pytest.mark.parametrize("args,input_", [(["--dry-run", "--json"], None), ([], "n\n"),
    (["--no-input"], None), (["--yes"], None), (["--read-only"], None),
    (["--yes", "--expect-plan", "wrong"], None)])
def test_cli_review_and_guards(monkeypatch, args, input_):
    monkeypatch.setattr(tasks, "resolve_status", lambda value, client=None: {"Open": 1, "In Progress": 2}[value])
    r = CliRunner().invoke(cli.app, ["changes", "tasks-add", "CHN-123", "--title", "Acme review", *args], input=input_)
    assert r.exit_code == (0 if args == ["--dry-run", "--json"] or not args else 2), r.output
    if "--dry-run" in args:
        review = json.loads(r.stdout)
        assert review["payload"] == {"title": "Acme review", "status": 1, "notify_before": 0}
    assert journal.read() == []


def test_cli_reviewed_hash_send(monkeypatch):
    monkeypatch.setattr(tasks, "resolve_status", lambda value, client=None: 1)
    runner = CliRunner()
    base = ["changes", "tasks-add", "CHN-123", "--title", "Globex review", "--owner", "9000002001"]
    draft = runner.invoke(cli.app, [*base, "--dry-run", "--json"])
    assert draft.exit_code == 0, draft.output
    review = json.loads(draft.stdout)
    calls = []
    def send(plan, **kw):
        calls.append((plan, kw))
        return {"task_id": 9000002010, "verified": True}
    monkeypatch.setattr(tasks, "send", send)
    r = runner.invoke(cli.app, [*base, "--yes", "--expect-plan", review["plan_hash"], "--no-input", "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)["task_id"] == 9000002010
    assert len(calls) == 1 and calls[0][1] == {"reviewed_hash": review["plan_hash"]}


@pytest.mark.parametrize("baseline_complete", [True, False])
def test_pagination_baseline_and_readback(monkeypatch, baseline_complete):
    calls, posted = [], False
    old = {"id": 9000002009, "title": "Acme paginated task", "owner_id": 9000002001}
    exact = {**old, "id": 9000002010}
    def handler(req):
        nonlocal posted
        calls.append(req)
        if req.method == "POST":
            posted = True
            return httpx.Response(200, json={})
        page = int(req.url.params.get("page", "1"))
        if page == 1:
            return httpx.Response(200, json={"tasks": [{**old, "id": 9000002008}], "meta": {"has_next": True}})
        if not baseline_complete:
            return httpx.Response(200, json={"unexpected": []})
        return httpx.Response(200, json={"tasks": [old, exact] if posted else [old], "meta": {"has_next": False}})
    c = mock_client(monkeypatch, handler)
    try:
        plan = tasks.build_plan(123, old["title"], status_id=1, owner_id=old["owner_id"])
        if baseline_complete:
            result = tasks.send(plan, c, reviewed_hash=plan.hash)
            assert result["task_id"] == exact["id"] and result["verified"] is True
            assert [r.method for r in calls] == ["GET", "GET", "POST", "GET", "GET"]
        else:
            with pytest.raises(tasks.TaskError, match="no task POST was attempted"):
                tasks.send(plan, c, reviewed_hash=plan.hash)
            assert not posted and journal.read() == []
    finally:
        c.close()


def test_unknown_owner_readback_is_not_unassigned(monkeypatch):
    monkeypatch.setattr(tasks, "_inventory", lambda *a: [] if not journal.read() else [
        {"id": 9000002010, "title": "Acme review"}])
    c = SimpleNamespace(_get_csrf=lambda: "synthetic-token", _request=lambda *a, **k: {})
    plan = tasks.build_plan(123, "Acme review", status_id=1)
    with pytest.raises(tasks.TaskError, match="outcome unknown"):
        tasks.send(plan, c, reviewed_hash=plan.hash)


def test_owner_lookup_failure_is_safe(monkeypatch):
    def denied(*a):
        raise RuntimeError("synthetic private session details")
    c = SimpleNamespace(autocomplete=denied)
    with pytest.raises(tasks.TaskError, match="could not resolve") as exc:
        tasks.resolve_owner("agent@example.com", c)
    assert "private session details" not in str(exc.value)
    assert journal.read() == []
