"""Offline synthetic approval REQUEST safety using per-member read-back shape."""
import copy
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


def rows(agents=(9000003001, 9000003002), start=310000000001):
    # Synthetic values preserving the observed populated list shape: no group.
    return [{"id": start + i, "created_at": "2026-01-01T00:00:00Z",
             "member_id": aid, "user_id": aid, "level_id": 1,
             "approval_level_status": None, "remark": [],
             "response_details": {"response_time": None, "first_responder": None},
             "status": {"id": 0, "name": "requested"},
             "type": {"id": 1, "name": "everyone"},
             "triggered_at": "2026-01-01T00:00:00Z", "change_id": 123,
             "user": {"id": aid, "name": "Example Agent", "email": "agent@example.com"},
             "member": {"id": aid, "name": "Example Agent", "email": "agent@example.com"},
             "delegatee": None} for i, aid in enumerate(agents)]


def events():
    return [r["event"] for r in journal.read() if r["kind"] == "plan:change_approval_request"]


@pytest.mark.parametrize("response", [{}, {"id": 1}, {"approval_groups": [{"id": 1}]}])
@pytest.mark.parametrize("envelope", [False, True])
@pytest.mark.parametrize("populated_baseline", [False, True])
def test_exact_post_verified_only_by_inventory(monkeypatch, tenant, response, envelope, populated_baseline):
    seen = []
    old = rows(start=310000000011) if populated_baseline else []
    # Old decisions may already be approved, and have no group identity either.
    for row in old:
        row["status"] = {"id": 1, "name": "approved"}
    exact = rows()
    def handler(req):
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
        seen.append(req)
        if req.method == "POST":
            return httpx.Response(200, json=response)
        assert req.url.path == "/api/_/changes/123/approvals"
        data = old if len(seen) == 1 else old + exact
        return httpx.Response(200, json={"approvals": data} if envelope else data)
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        result = approvals.send(p, c, reviewed_hash=p.hash)
        assert result["verified"] is True
        assert result["approval_ids"] == [310000000001, 310000000002]
        assert result["group_name_verified"] is False
        assert "approval_group_id" not in result
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


@pytest.mark.parametrize("case", ["match", "paginated", "reverse-members", "wrong-name", "missing-name", "wrong-members",
    "duplicate-member", "wrong-records", "swapped-record-members", "changed-status", "existing-group",
    "missing-baseline", "extra-group", "duplicate-group", "partial", "malformed", "denied",
    "baseline-denied", "baseline-partial", "flat-concurrent", "flat-missing-baseline"])
def test_request_named_group_enrichment_keeps_record_guards(monkeypatch, tenant, case):
    old = rows(start=310000000011)
    fresh = rows()
    def group(gid, name, records):
        return {"id": gid, "name": name, "approval_status": 0, "approvals": records}
    baseline = [group(310000000090, "Acme earlier review", old)]
    named = group(310000000099, "Acme review", copy.deepcopy(fresh))
    after_groups = copy.deepcopy(baseline) + [named]
    if case == "reverse-members":
        named["approvals"].reverse()
    elif case == "wrong-name":
        named["name"] = "acme review"  # Exact, not case-insensitive.
    elif case == "missing-name":
        named.pop("name")
    elif case == "wrong-members":
        named["approvals"] = rows(agents=(9000003003, 9000003004))
    elif case == "duplicate-member":
        named["approvals"] = rows(agents=(9000003001, 9000003001))
    elif case == "wrong-records":
        named["approvals"] = rows(start=310000000021)
    elif case == "swapped-record-members":
        named["approvals"][0]["id"], named["approvals"][1]["id"] = (
            named["approvals"][1]["id"], named["approvals"][0]["id"])
    elif case == "changed-status":
        named["approvals"][0]["status"] = {"id": 1, "name": "approved"}
    elif case == "existing-group":
        baseline = copy.deepcopy(after_groups)
    elif case == "missing-baseline":
        after_groups = [named]
    elif case == "extra-group":
        after_groups.append(group(310000000097, "Globex review", rows(start=310000000021)))
    elif case == "duplicate-group":
        after_groups.append(copy.deepcopy(named))
    posted, seen = False, []
    def handler(req):
        nonlocal posted
        seen.append(req)
        if req.method == "POST":
            posted = True
            return httpx.Response(200, json={"id": 1, "name": "untrusted"})
        if req.url.path.endswith("/approval-chains"):
            if case == ("denied" if posted else "baseline-denied"):
                return httpx.Response(403, json={})
            if posted and case == "malformed":
                return httpx.Response(200, json={"approval_chains": None})
            data: Any = {"approval_chains": [{"id": 310000000098,
                    "approval_groups": after_groups if posted else baseline}]}
            if case == ("partial" if posted else "baseline-partial"):
                data["meta"] = {"partial": True}
            if case == "paginated":
                page = int(req.url.params["page"])
                data = {"approval_chains": [{"id": 310000000098 + page,
                        "approval_groups": (after_groups if posted else baseline)[page - 1:page]}],
                        "meta": {"has_next": page == 1, "total_count": 2}}
            return httpx.Response(200, json=data)
        assert req.url.path.endswith("/approvals")
        data = old + fresh if posted else old
        if posted and case == "flat-concurrent":
            data += rows(start=310000000031, agents=(9000003003,))
        elif posted and case == "flat-missing-baseline":
            data = fresh
        return httpx.Response(200, json=data)
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        if case.startswith("flat-"):
            with pytest.raises(approvals.ApprovalError, match="outcome unknown"):
                approvals.send(p, c, reviewed_hash=p.hash)
            assert events() == ["started"]
            with pytest.raises(approvals.ApprovalError, match="do not resubmit"):
                approvals.send(p, c, reviewed_hash=p.hash)
        else:
            result = approvals.send(p, c, reviewed_hash=p.hash)
            assert result["verified"] is True and result["readback"] == fresh
            assert result["approval_ids"] == [r["id"] for r in fresh]
            assert result["group_name_verified"] is (case in ("match", "paginated", "reverse-members"))
            if result["group_name_verified"]:
                assert result["group_id"] == named["id"]
            else:
                assert "group_id" not in result
            assert events() == ["started", "done"]
            assert [r.method for r in seen] == (
                ["GET", "GET", "GET", "POST", "GET", "GET", "GET"] if case == "paginated" else
                ["GET", "GET", "POST", "GET"] + ([] if case.startswith("baseline-") else ["GET"]))
        assert sum(r.method == "POST" for r in seen) == 1
    finally:
        c.close()


@pytest.mark.parametrize("case", ["none", "multiple", "concurrent-unrelated", "wrong-agents", "extra-agent",
    "missing-agent", "partial", "read-error", "malformed", "missing-fields", "reused-id", "duplicate-id",
    "duplicate-member", "conflicting-member", "missing-member-id", "user-not-member", "missing-baseline",
    "wrong-status", "wrong-rule", "missing-status", "missing-rule", "boolean-status", "boolean-rule",
    "conflicting-status", "conflicting-rule"])
def test_ambiguous_stays_unknown_and_blocks_retry(monkeypatch, case):
    seen = []
    old = rows(start=310000000011)
    fresh = rows()
    if case == "none":
        fresh = []
    elif case == "multiple":
        fresh += rows(start=310000000021)
    elif case == "concurrent-unrelated":
        fresh += rows(start=310000000021, agents=(9000003003,))
    elif case == "wrong-agents":
        fresh = rows(agents=(9000003003, 9000003004))
    elif case == "extra-agent":
        fresh = rows(agents=(9000003001, 9000003002, 9000003003))
    elif case == "missing-agent":
        fresh = rows(agents=(9000003001,))
    elif case == "missing-fields":
        fresh = [{"id": 310000000001, "member_id": 9000003001}]
    elif case == "reused-id":
        fresh[0]["id"] = old[0]["id"]
    elif case == "duplicate-id":
        fresh += [fresh[0]]
    elif case == "duplicate-member":
        fresh = rows(agents=(9000003001, 9000003001))
    elif case == "conflicting-member":
        fresh[0]["member"]["id"] = 9000003003
    elif case in ("missing-member-id", "user-not-member"):
        del fresh[0]["member_id"]
        if case == "user-not-member":
            del fresh[0]["member"]
    elif case in ("wrong-status", "conflicting-status", "boolean-status"):
        fresh[0]["status"] = {"id": False if case == "boolean-status" else 1,
                              "name": "requested" if case != "wrong-status" else "approved"}
    elif case in ("wrong-rule", "conflicting-rule", "boolean-rule"):
        fresh[0]["type"] = {"id": True if case == "boolean-rule" else 2,
                            "name": "everyone" if case != "wrong-rule" else "anyone"}
    elif case == "missing-status":
        del fresh[0]["status"]
    elif case == "missing-rule":
        del fresh[0]["type"]
    after = ([] if case == "missing-baseline" else old[1:] if case == "reused-id" else old) + fresh
    def handler(req):
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
        seen.append(req)
        if req.method == "POST":
            # A plausible returned ID is explicitly NOT trusted.
            return httpx.Response(200, json={"id": 9000003010})
        if len(seen) == 1:
            return httpx.Response(200, json=old)
        if case == "read-error":
            raise httpx.ReadTimeout("synthetic read failure", request=req)
        data = {"approvals": None} if case == "malformed" else after
        if case == "partial":
            data = {"approvals": after, "meta": {"partial": True}}
        return httpx.Response(200, json=data)
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
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
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
        assert events() == (["started", "failed"] if failure == 400 else ["started"])
        if failure != 400:
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


@pytest.mark.parametrize("case", ["partial", "malformed", "bad-id", "duplicate", "csrf", "journal"])
def test_preparation_failure_no_post(monkeypatch, case):
    def handler(req):
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
        assert req.method == "GET"
        data = rows()
        if case == "duplicate":
            data += [data[0]]
        elif case == "bad-id":
            data = [{"id": False, "member_id": 9000003001}]
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
    assert "exactly one new matching group" in p.review()["verification"]


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
        return {"approval_ids": [310000000001, 310000000002], "verified": True}
    monkeypatch.setattr(approvals, "send", send)
    r = runner.invoke(cli.app, [*base, "--yes", "--expect-plan", review["plan_hash"], "--no-input", "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)["verified"] is True
    assert seen[0][0].approver_ids == (9000003001, 9000003002)
    assert seen[0][1] == {"reviewed_hash": review["plan_hash"]}


@pytest.mark.parametrize("case", ["total", "bad-total", "non-object", "bad-id", "partial", "complete-false",
                                  "bare-non-object", "bare-bad-id", "bare-duplicate"])
@pytest.mark.parametrize("after_post", [False, True])
def test_raw_inventory_failures_cannot_be_hidden(monkeypatch, case, after_post):
    seen = []
    def handler(req):
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
        seen.append(req)
        if req.method == "POST":
            return httpx.Response(200, json={})
        if after_post and len(seen) == 1:
            return httpx.Response(200, json={"approvals": []})
        data = rows()
        meta: dict[str, Any] = {"has_next": False}
        if case.startswith("bare-"):
            data += [None if case == "bare-non-object" else {"id": False} if case == "bare-bad-id" else data[0]]
            return httpx.Response(200, json=data)
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
    old = rows(start=310000000011)
    def handler(req):
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
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


@pytest.mark.parametrize("envelope", [False, True])
@pytest.mark.parametrize("empty", [False, True])
@pytest.mark.parametrize("format_", ["table", "json", "csv"])
def test_cli_approvals_list_populated_shape(monkeypatch, envelope, empty, format_):
    from fsv import create

    data = [] if empty else rows()
    seen = []
    def handler(req):
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
        seen.append(req)
        assert req.method == "GET" and req.url.path == "/api/_/changes/123/approvals"
        return httpx.Response(200, json={"approvals": data} if envelope else data)
    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(create, "get_client", lambda: c)
    try:
        result = CliRunner().invoke(cli.app, ["changes", "approvals", "CHN-123", "--output", format_])
        assert result.exit_code == 0, result.output
        if format_ == "json":
            assert json.loads(result.stdout) == data
        elif not empty:
            assert "Example Agent" in result.stdout and "requested" in result.stdout
            assert "approved" not in result.stdout
        assert len(seen) == 1
    finally:
        c.close()


@pytest.mark.parametrize("format_", ["text", "json"])
@pytest.mark.parametrize("exact", [False, True])
def test_cli_actual_send_readback(monkeypatch, format_, exact):
    seen = []
    def handler(req):
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(403, json={})
        seen.append(req)
        if req.method == "POST":
            return httpx.Response(200, json={"id": 99})
        return httpx.Response(200, json=rows(agents=(9000003001,)) if exact and len(seen) > 1 else [])
    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(approvals, "get_client", lambda: c)
    try:
        draft = CliRunner().invoke(cli.app, [*BASE, "--dry-run"])
        assert draft.exit_code == 0, draft.output
        review = json.loads(draft.stdout)
        args = [*BASE, "--yes", "--expect-plan", review["plan_hash"], "--no-input"]
        if format_ == "json":
            args.append("--json")
        result = CliRunner().invoke(cli.app, args)
        assert result.exit_code == (0 if exact else 2), result.output
        if not exact:
            assert "outcome unknown" in result.output
            retry = CliRunner().invoke(cli.app, args)
            assert retry.exit_code == 2 and "do not resubmit" in retry.output
        elif format_ == "json":
            assert json.loads(result.stdout)["approval_ids"] == [310000000001]
            assert json.loads(result.stdout)["group_name_verified"] is False
        else:
            assert "310000000001" in result.stdout
            assert "group name verified: False" in result.stdout
        assert [req.method for req in seen] == ["GET", "POST", "GET"]
    finally:
        c.close()
