"""Offline synthetic approval REQUEST safety; no accepted live contract claimed."""
import json
from dataclasses import replace
from types import SimpleNamespace
from typing import Any

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_approvals as approvals, cli, config, journal
from test_ticket_notes import mock_client


@pytest.fixture(params=["acme.example.com", "globex.example.com"])
def tenant(request, monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", request.param)
    monkeypatch.setattr(config, "API_INT", f"https://{request.param}/api/_")
    return request.param


def plan():
    return approvals.build_plan("CHN-123", "Acme review", "Please review <Acme> & confirm.",
                                approver_ids=[9000003001, 9000003002])


def rows(gid=9000003010, name="Acme review", agents=(9000003001, 9000003002), start=9000003020):
    # Supported conservative flat read-back adapter, not observed populated wire.
    return [{"id": start + i, "approval_group": {"id": gid, "name": name},
             "member": {"id": aid, "name": "Example Agent"}} for i, aid in enumerate(agents)]


def events():
    return [r["event"] for r in journal.read() if r["kind"] == "plan:change_approval_request"]


@pytest.mark.parametrize("response", [{}, {"id": 1}, {"approval_groups": [{"id": 1}]}])
def test_exact_post_verified_only_by_inventory(monkeypatch, tenant, response):
    seen = []
    old = rows(gid=9000003009, start=9000003018)
    exact = rows()
    def handler(req):
        seen.append(req)
        if req.method == "POST":
            return httpx.Response(200, json=response)
        assert req.url.path == "/api/_/changes/123/approvals"
        return httpx.Response(200, json={"approvals": old if len(seen) == 1 else old + exact})
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        result = approvals.send(p, c, reviewed_hash=p.hash)
        assert result["verified"] is True and result["approval_group_id"] == 9000003010
        assert result["readback"] == exact
        post = [r for r in seen if r.method == "POST"]
        assert len(post) == 1
        assert str(post[0].url) == f"https://{tenant}/api/_/changes/123/approval-groups"
        assert post[0].headers["x-csrf-token"] == "synthetic-token"
        assert json.loads(post[0].content) == {"approval_groups": [{"name": "Acme review", "approval_type": 1,
            "email_content": '<p dir="ltr">Please review &lt;Acme&gt; &amp; confirm.</p>',
            "notified_to": [9000003001, 9000003002]}]}
        assert events() == ["started", "done"]
        assert "Acme review" not in journal.journal_path().read_text()
        with pytest.raises(approvals.ApprovalError, match="do not resubmit"):
            approvals.send(p, c, reviewed_hash=p.hash)
        assert len(seen) == 3
    finally:
        c.close()


@pytest.mark.parametrize("case", ["none", "multiple", "concurrent-unrelated", "wrong-name", "wrong-agents", "extra-agent",
    "missing-agent", "partial", "read-error", "malformed", "group-less", "reused-member", "duplicate-id", "old-group"])
def test_ambiguous_stays_unknown_and_blocks_retry(monkeypatch, case):
    seen = []
    old = rows(gid=9000003009, start=9000003018)
    after = old + rows()
    if case == "none":
        after = old
    elif case in ("multiple", "concurrent-unrelated"):
        after += rows(gid=9000003011, start=9000003022,
                      name="Globex review" if case == "concurrent-unrelated" else "Acme review")
    elif case == "wrong-name":
        after = old + rows(name="Globex review")
    elif case == "wrong-agents":
        after = old + rows(agents=(9000003003, 9000003004))
    elif case == "extra-agent":
        after = old + rows(agents=(9000003001, 9000003002, 9000003003))
    elif case == "missing-agent":
        after = old + rows(agents=(9000003001,))
    elif case == "group-less":
        after = [{"id": 9000003020, "member_id": 9000003001}]
    elif case == "reused-member":
        after = rows(start=9000003018)
    elif case == "duplicate-id":
        after = old + rows() + [rows()[0]]
    elif case == "old-group":
        after = old + rows(gid=9000003009)
    def handler(req):
        seen.append(req)
        if req.method == "POST":
            # A plausible returned ID is explicitly NOT trusted.
            return httpx.Response(200, json={"id": 9000003010})
        if len(seen) == 1:
            return httpx.Response(200, json={"approvals": old})
        if case == "read-error":
            raise httpx.ReadTimeout("synthetic read failure", request=req)
        return httpx.Response(200, json={"approvals": None if case == "malformed" else after,
                                       "meta": {"has_next": case == "partial"}})
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        with pytest.raises(approvals.ApprovalError, match="outcome unknown"):
            approvals.send(p, c, reviewed_hash=p.hash)
        assert events() == ["started"]
        with pytest.raises(approvals.ApprovalError, match="do not resubmit"):
            approvals.send(p, c, reviewed_hash=p.hash)
        assert len([r for r in seen if r.method == "POST"]) == 1
    finally:
        c.close()


@pytest.mark.parametrize("failure", [400, 401, 403, 408, 429, 500, "timeout"])
def test_single_attempt_no_403_replay(monkeypatch, failure):
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "GET":
            return httpx.Response(200, json={"approvals": []})
        if failure == "timeout":
            raise httpx.ReadTimeout("synthetic unknown", request=req)
        return httpx.Response(failure, json={"error": "synthetic failure"})
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        with pytest.raises(approvals.ApprovalError, match="rejected|unknown"):
            approvals.send(p, c, reviewed_hash=p.hash)
        assert len([r for r in seen if r.method == "POST"]) == 1
        assert events() == (["started", "failed"] if failure in (400, 403) else ["started"])
        if failure not in (400, 403):
            with pytest.raises(approvals.ApprovalError, match="do not resubmit"):
                approvals.send(p, c, reviewed_hash=p.hash)
            assert len([r for r in seen if r.method == "POST"]) == 1
    finally:
        c.close()


@pytest.mark.parametrize("mode", ["dry_run", "cancel", "read_only", "hash", "tenant", "edited"])
def test_guards_no_io(monkeypatch, mode):
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("client forbidden"))
    p = plan()
    kw: dict[str, Any] = {"reviewed_hash": p.hash}
    if mode in ("dry_run", "read_only"):
        kw[mode] = True
    elif mode == "cancel":
        kw["reviewed_hash"] = None
    elif mode == "hash":
        kw["reviewed_hash"] = "wrong"
    elif mode == "tenant":
        monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    else:
        p = replace(p, message="Globex review")
    if mode in ("dry_run", "cancel"):
        assert approvals.send(p, **kw)["status"] in ("dry_run", "cancelled")
    else:
        with pytest.raises(approvals.ApprovalError):
            approvals.send(p, **kw)
    assert journal.read() == []


@pytest.mark.parametrize("case", ["partial", "malformed", "group-less", "duplicate", "csrf", "journal"])
def test_preparation_failure_no_post(monkeypatch, case):
    def handler(req):
        assert req.method == "GET"
        data = rows()
        if case == "duplicate":
            data += [data[0]]
        elif case == "group-less":
            data = [{"id": 9000003020, "member_id": 9000003001}]
        return httpx.Response(200, json={"approvals": None if case == "malformed" else data,
                                       "meta": {"has_next": case == "partial"}})
    c = mock_client(monkeypatch, handler)
    if case == "csrf":
        monkeypatch.setattr(c, "_get_csrf", lambda: (_ for _ in ()).throw(RuntimeError()))
    elif case == "journal":
        monkeypatch.setattr(journal, "record_plan", lambda *a, **k: None)
    try:
        p = plan()
        with pytest.raises(approvals.ApprovalError, match="no approval"):
            approvals.send(p, c, reviewed_hash=p.hash)
        assert journal.read() == []
    finally:
        c.close()


def test_escape_immutable_and_review():
    p = approvals.build_plan(123, "Acme review", '<script> & "quote"\r\nnext\n\nlast', approver_ids=[9000003001])
    assert p.payload["approval_groups"][0]["email_content"] == (
        '<p dir="ltr">&lt;script&gt; &amp; &quot;quote&quot;<br>next<br><br>last</p>')
    original = p.hash
    p.payload["approval_groups"][0]["notified_to"].append(9000003002)
    assert p.hash == original
    assert "irreversible" in p.review()["warning"] and "cancel/revoke" in p.review()["warning"]
    assert p.review()["recipients"] == [9000003001]
    assert p.review()["approval_rule"] == "Everyone"


@pytest.mark.parametrize("cid,name,message,ids", [(True, "Acme", "review", [1]), (0, "Acme", "review", [1]),
    ("SR-123", "Acme", "review", [1]), ("123/x", "Acme", "review", [1]), (123, "", "review", [1]),
    (123, "Acme\n", "review", [1]), (123, "Acme", "", [1]), (123, "Acme", "x\x00", [1]),
    (123, "Acme", "review", []), (123, "Acme", "review", [True]), (123, "Acme", "review", [0]),
    (123, "Acme", "review", ["1"]), (123, "Acme", "review", [1, 1]), (123, "Acme", "review", "1")])
def test_invalid_plan(cid, name, message, ids):
    with pytest.raises(ValueError):
        approvals.build_plan(cid, name, message, approver_ids=ids)


def test_resolve_exact_active_agent_not_user_id(monkeypatch):
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("implicit lookup"))
    assert approvals.resolve_approvers(["9000003001"]) == (9000003001,)
    row = {"id": 9000003001, "user_id": 9000003099, "email": "agent@example.com"}
    c = SimpleNamespace(autocomplete=lambda *a: [row])
    assert approvals.resolve_approvers(["AGENT@example.com"], c) == (9000003001,)
    for data in ([], [row, row], [{**row, "id": None}], [{**row, "deactivated": True}],
                 [{**row, "deleted": True}], [{**row, "email": "other@example.com"}],
                 [{**row, "value": "Agent (Deactivated)"}], {}):
        c.autocomplete = lambda *a: data
        with pytest.raises(approvals.ApprovalError):
            approvals.resolve_approvers(["agent@example.com"], c)
    c.autocomplete = lambda *a: (_ for _ in ()).throw(RuntimeError("synthetic private session details"))
    with pytest.raises(approvals.ApprovalError) as exc:
        approvals.resolve_approvers(["agent@example.com"], c)
    assert "private session" not in str(exc.value)


@pytest.mark.parametrize("values", [[], None, "agent@example.com", ["me"], ["0"], ["-1"],
                                      ["agent@example.com\n"], ["9000003001", "9000003001"]])
def test_invalid_recipients_no_io(monkeypatch, values):
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("lookup forbidden"))
    with pytest.raises(ValueError):
        approvals.resolve_approvers(values)


BASE = ["changes", "approval-request", "CHN-123", "--name", "Acme review", "--message", "Please review", "--approver", "9000003001"]


@pytest.mark.parametrize("args,input_", [(["--dry-run", "--json"], None), ([], "n\n"),
    (["--no-input"], None), (["--yes"], None), (["--read-only"], None),
    (["--yes", "--expect-plan", "wrong"], None)])
def test_cli_review_guards(monkeypatch, args, input_):
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("client forbidden"))
    r = CliRunner().invoke(cli.app, [*BASE, *args], input=input_)
    assert r.exit_code == (0 if not args or "--dry-run" in args else 2), r.output
    if "--dry-run" in args:
        draft = json.loads(r.stdout)
        assert draft["recipients"] == [9000003001]
        assert "irreversible" in draft["warning"]
    assert journal.read() == []


def test_cli_explicit_recipients_required(monkeypatch):
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("client forbidden"))
    r = CliRunner().invoke(cli.app, BASE[:-2] + ["--dry-run"])
    assert r.exit_code == 2 and "--approver" in r.output


def test_cli_reviewed_send_repeatable(monkeypatch):
    runner = CliRunner()
    base = [*BASE, "--approver", "9000003002"]
    draft = runner.invoke(cli.app, [*base, "--dry-run", "--json"])
    assert draft.exit_code == 0, draft.output
    review = json.loads(draft.stdout)
    seen = []
    def send(p, **kw):
        seen.append((p, kw))
        return {"approval_group_id": 9000003010, "verified": True}
    monkeypatch.setattr(approvals, "send", send)
    r = runner.invoke(cli.app, [*base, "--yes", "--expect-plan", review["plan_hash"], "--no-input", "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)["verified"] is True
    assert seen[0][0].approver_ids == (9000003001, 9000003002)
    assert seen[0][1] == {"reviewed_hash": review["plan_hash"]}


@pytest.mark.parametrize("case", ["total", "bad-total", "non-object", "bad-id", "partial", "complete-false"])
@pytest.mark.parametrize("after_post", [False, True])
def test_raw_inventory_failures_cannot_be_hidden(monkeypatch, case, after_post):
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "POST":
            return httpx.Response(200, json={})
        if after_post and len(seen) == 1:
            return httpx.Response(200, json={"approvals": []})
        data = rows()
        meta: dict[str, Any] = {"has_next": False}
        if case == "total":
            meta["total_count"] = 99
        elif case == "bad-total":
            meta["count"] = True
        elif case == "non-object":
            data += [None]
        elif case == "bad-id":
            data += [{"id": False}]
        elif case == "partial":
            meta["partial"] = "synthetic incomplete response"
        else:
            meta["complete"] = False
        return httpx.Response(200, json={"approvals": data, "meta": meta})
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        with pytest.raises(approvals.ApprovalError, match="unknown" if after_post else "no approval POST"):
            approvals.send(p, c, reviewed_hash=p.hash)
        assert len([r for r in seen if r.method == "POST"]) == (1 if after_post else 0)
        assert events() == (["started"] if after_post else [])
    finally:
        c.close()


def test_cli_read_only_blocks_even_email_lookup(monkeypatch):
    monkeypatch.setattr(approvals, "get_client", lambda: pytest.fail("lookup forbidden"))
    r = CliRunner().invoke(cli.app, [*BASE[:-1], "agent@example.com", "--read-only"])
    assert r.exit_code == 2 and "read-only" in r.output
    assert journal.read() == []


@pytest.mark.parametrize("complete", [True, False])
def test_complete_paginated_before_after(monkeypatch, complete):
    seen, posted = [], False
    old = rows(gid=9000003009, start=9000003018)
    def handler(req):
        nonlocal posted
        seen.append(req)
        if req.method == "POST":
            posted = True
            return httpx.Response(200, json={})
        page = int(req.url.params.get("page", "1"))
        if page == 1:
            return httpx.Response(200, json={"approvals": old, "meta": {"has_next": True}})
        if not complete:
            return httpx.Response(200, json={"unexpected": []})
        return httpx.Response(200, json={"approvals": rows() if posted else [], "meta": {"has_next": False}})
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        if complete:
            assert approvals.send(p, c, reviewed_hash=p.hash)["verified"] is True
            assert [r.method for r in seen] == ["GET", "GET", "POST", "GET", "GET"]
        else:
            with pytest.raises(approvals.ApprovalError, match="no approval POST"):
                approvals.send(p, c, reviewed_hash=p.hash)
            assert posted is False and journal.read() == []
    finally:
        c.close()
