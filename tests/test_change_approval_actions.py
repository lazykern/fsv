"""Offline synthetic tests of reviewed decision writes and fail-closed cancellation."""
import copy
import json
from dataclasses import FrozenInstanceError, replace
from typing import Any

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_approval_actions as actions, cli, config, journal
from fsv.change_approvals import ApprovalError
from test_change_approvals import rows
from test_ticket_notes import mock_client

AID = 310000000001
BASE = ["changes", "approval-respond", "CHN-123", str(AID)]
CANCEL = ["changes", "approval-cancel", "CHN-123", "310000000099"]


def plan(operation="approve", remark=None):
    return actions.build_plan("CHN-123", AID, operation, remark)


def events():
    return [r["event"] for r in journal.read() if r.get("kind") == "plan:change_approval_respond"]


@pytest.mark.parametrize("tenant", ["acme.example.com", "globex.example.com"])
@pytest.mark.parametrize("operation,remark,status", [
    ("approve", None, {"id": 7, "name": "approved"}),
    ("approve", "", {"id": 7, "name": "approved"}),
    ("approve", "Acme <review> & confirm\nDone", {"id": 7, "name": "approved"}),
    ("reject", "Needs Globex review", {"id": 9, "name": "rejected"}),
])
@pytest.mark.parametrize("envelope", [False, True])
def test_exact_single_put_and_named_decision(monkeypatch, tenant, operation, remark, status, envelope):
    monkeypatch.setattr(config, "DOMAIN", tenant)
    monkeypatch.setattr(config, "API_INT", f"https://{tenant}/api/_")
    seen = []
    before = rows()
    after = copy.deepcopy(before)
    after[0]["status"] = status
    def handler(req):
        seen.append(req)
        if req.method == "PUT":
            return httpx.Response(200, json={"status": "not trusted"})
        assert req.url.path == "/api/_/changes/123/approvals"
        data = before if len(seen) == 1 else after
        return httpx.Response(200, json={"approvals": data} if envelope else data)
    c = mock_client(monkeypatch, handler)
    try:
        p = plan(operation, remark)
        result = actions.send(p, c, reviewed_hash=p.hash)
        assert result["verified"] is True and result["decision_verified"] is True
        assert result["verification_basis"] == "named_decision"
        assert result["observed_status"] == status
        assert result["before_status"] == {"id": 0, "name": "requested"}
        assert result["readback"] == after[0]
        assert [r.method for r in seen] == ["GET", "PUT", "GET"]
        req = seen[1]
        assert str(req.url) == f"https://{tenant}/api/_/changes/123/approvals/{AID}"
        assert req.headers["x-csrf-token"] == "synthetic-token"
        body = {"approval_status": 1 if operation == "approve" else 2}
        if remark is not None:
            body["remark"] = remark
        assert json.loads(req.content) == body
        assert events() == ["started", "done"]
        assert "Globex review" not in journal.journal_path().read_text()
        with pytest.raises(ApprovalError, match="do not resubmit"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert len(seen) == 3
    finally:
        c.close()


@pytest.mark.parametrize("status", [8, {"id": 8}, {"id": 8, "name": "unobserved_state"}])
def test_raw_changed_status_no_invented_mapping(monkeypatch, status):
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "PUT":
            return httpx.Response(200, json={})
        data = rows()
        if len(seen) > 1:
            data[0]["status"] = status
        return httpx.Response(200, json=data)
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        result = actions.send(p, c, reviewed_hash=p.hash)
        assert result["verified"] is True
        assert result["decision_verified"] is False
        assert result["observed_status"] == status
        assert result["verification_basis"] == "changed_raw_status_only"
        assert "mapping unobserved" in result["warning"]
    finally:
        c.close()


@pytest.mark.parametrize("case", ["unchanged", "wrong-decision", "cancelled", "missing-record", "wrong-record",
    "partial", "read-error", "malformed", "duplicate", "wrong-member", "no-member", "conflicting-member",
    "wrong-change", "missing-status", "boolean-status", "boolean-id", "still-requested", "zero-id", "empty-name"])
def test_ambiguous_readback_blocks_duplicate(monkeypatch, case):
    seen = []
    after = rows()
    after[0]["status"] = {"id": 7, "name": "approved"}
    if case == "unchanged":
        after = rows()
    elif case in ("wrong-decision", "cancelled", "still-requested", "empty-name"):
        after[0]["status"]["name"] = {"wrong-decision": "rejected", "cancelled": "cancelled",
                                        "still-requested": "requested", "empty-name": ""}[case]
    elif case == "missing-record":
        after = after[1:]
    elif case == "wrong-record":
        after[0]["id"] += 99
    elif case == "duplicate":
        after += [after[0]]
    elif case == "wrong-member":
        after[0]["member_id"] = 9000003999
    elif case == "no-member":
        after[0].pop("member_id")
    elif case == "conflicting-member":
        after[0]["member"]["id"] = 9000003999
    elif case == "wrong-change":
        after[0]["change_id"] = 999
    elif case == "missing-status":
        after[0].pop("status")
    elif case == "boolean-status":
        after[0]["status"] = True
    elif case in ("boolean-id", "zero-id"):
        after[0]["status"]["id"] = True if case == "boolean-id" else 0
    def handler(req):
        seen.append(req)
        if req.method == "PUT":
            return httpx.Response(200, json={"status": {"id": 1, "name": "approved"}})
        if len(seen) == 1:
            return httpx.Response(200, json=rows())
        if case == "read-error":
            raise httpx.ReadTimeout("synthetic read failure", request=req)
        return httpx.Response(200, json={"approvals": None} if case == "malformed" else
                              {"approvals": after, "meta": {"partial": True}} if case == "partial" else after)
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        with pytest.raises(ApprovalError, match="outcome unknown"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert events() == ["started"]
        with pytest.raises(ApprovalError, match="do not resubmit"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert [r.method for r in seen] == ["GET", "PUT"] + ["GET"] * (3 if case == "read-error" else 1)
    finally:
        c.close()


@pytest.mark.parametrize("failure", [400, 401, 403, 408, 429, 500, "timeout"])
def test_no_replay_on_write_failure(monkeypatch, failure):
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "GET":
            return httpx.Response(200, json=rows())
        if failure == "timeout":
            raise httpx.ReadTimeout("synthetic unknown", request=req)
        return httpx.Response(failure, json={"error": "synthetic rejection"})
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        with pytest.raises(ApprovalError, match="rejected|unknown"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert [r.method for r in seen] == ["GET", "PUT"]
        assert events() == (["started", "failed"] if failure in (400, 403) else ["started"])
        if failure not in (400, 403):
            with pytest.raises(ApprovalError, match="do not resubmit"):
                actions.send(p, c, reviewed_hash=p.hash)
            assert len(seen) == 2
    finally:
        c.close()


@pytest.mark.parametrize("case", ["missing", "decided", "partial", "duplicate", "total", "non-object",
                                  "no-member", "wrong-member", "wrong-change", "csrf", "journal"])
def test_preparation_fail_closed(monkeypatch, case):
    seen = []
    data = rows()
    if case == "missing":
        data = []
    elif case == "decided":
        data[0]["status"] = {"id": 7, "name": "approved"}
    elif case == "duplicate":
        data += [data[0]]
    elif case == "non-object":
        data += [None]
    elif case == "no-member":
        data[0].pop("member_id")
    elif case == "wrong-member":
        data[0]["member"]["id"] = 9000003999
    elif case == "wrong-change":
        data[0]["change_id"] = 999
    def handler(req):
        seen.append(req)
        assert req.method == "GET"
        meta = {"partial": True} if case == "partial" else {"total_count": 99} if case == "total" else {}
        return httpx.Response(200, json={"approvals": data, "meta": meta})
    c = mock_client(monkeypatch, handler)
    if case == "csrf":
        monkeypatch.setattr(c, "_get_csrf", lambda: (_ for _ in ()).throw(RuntimeError()))
    if case == "journal":
        monkeypatch.setattr(journal, "record_plan", lambda *a, **k: None)
    try:
        p = plan()
        with pytest.raises(ApprovalError, match="no approval PUT attempted"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert len(seen) == (2 if case == "total" else 1) and journal.read() == []
    finally:
        c.close()


@pytest.mark.parametrize("operation", ["approve", "reject", "cancel"])
@pytest.mark.parametrize("mode", ["cancel_prompt", "read_only", "bad_hash", "tenant", "changed"])
def test_local_guards_no_io(monkeypatch, operation, mode):
    monkeypatch.setattr(actions, "get_client", lambda: pytest.fail("client forbidden"))
    p = plan(operation, "Needs review" if operation == "reject" else None)
    kw: dict[str, Any] = {"reviewed_hash": p.hash}
    if mode in ("dry_run", "read_only"):
        kw[mode] = True
    elif mode == "cancel_prompt":
        kw["reviewed_hash"] = None
    elif mode == "bad_hash":
        kw["reviewed_hash"] = "bad"
    elif mode == "tenant":
        monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    else:
        p = replace(p, target_id=AID + 1)
    if mode in ("dry_run", "cancel_prompt"):
        assert actions.send(p, **kw)["status"] in ("dry_run", "cancelled")
    else:
        with pytest.raises(ApprovalError):
            actions.send(p, **kw)
    assert journal.read() == []


@pytest.mark.parametrize("remark", [None, "", " ", "\n\t"])
def test_reject_requires_nonempty_remark(remark):
    with pytest.raises(ValueError, match="nonempty --remark"):
        plan("reject", remark)


@pytest.mark.parametrize("cid,aid,op,remark", [(True, AID, "approve", None), (0, AID, "approve", None),
    ("SR-123", AID, "approve", None), ("123/x", AID, "approve", None), (123, True, "approve", None),
    (123, "1/x", "approve", None), (123, "0", "approve", None), (123, AID, "unknown", None),
    (123, AID, "cancel", ""), (123, AID, "approve", "bad\x00"), (123, AID, "approve", 42)])
def test_invalid_plans(cid, aid, op, remark):
    with pytest.raises(ValueError):
        actions.build_plan(cid, aid, op, remark)


def test_immutable_plan_and_hash_binds_every_field(monkeypatch):
    p = plan("approve", "")
    original = p.hash
    payload = p.payload
    assert payload is not None
    payload["remark"] = "changed copy"
    assert p.hash == original
    with pytest.raises(FrozenInstanceError):
        p.remark = "new"  # type: ignore[misc]
    for changed in (replace(p, change_id=124), replace(p, target_id=AID + 1),
                    replace(p, operation="reject", remark="Needs review"), replace(p, remark=None),
                    replace(p, tenant="globex.example.com")):
        assert changed.hash != p.hash
    review = p.review()
    assert "notify people" in review["warning"] and "irreversible" in review["warning"]


def test_cancel_bodyless_plan():
    p = actions.build_plan(123, 310000000099, "cancel")
    review = p.review()
    assert review["path"] == "/api/_/changes/123/approval-groups/310000000099/cancel"
    assert review["payload"] is None and review["body_present"] is False
    assert review["send_enabled"] is True


@pytest.mark.parametrize("base", [BASE + ["--approve"], BASE + ["--reject", "--remark", "Needs review"]])
@pytest.mark.parametrize("args,input_", [(["--dry-run", "--json"], None), ([], "n\n"),
    (["--no-input"], None), (["--yes"], None), (["--read-only"], None),
    (["--yes", "--expect-plan", "bad"], None)])
def test_cli_review_guards(monkeypatch, base, args, input_):
    monkeypatch.setattr(actions, "get_client", lambda: pytest.fail("client forbidden"))
    result = CliRunner().invoke(cli.app, base + args, input=input_)
    assert result.exit_code == (0 if not args or "--dry-run" in args else 2), result.output
    if "--dry-run" in args:
        assert json.loads(result.stdout)["method"] == "PUT"
    assert journal.read() == []


@pytest.mark.parametrize("args", [[], ["--approve", "--reject"], ["--reject"], ["--reject", "--remark", ""]])
def test_cli_invalid_decision_no_io(monkeypatch, args):
    monkeypatch.setattr(actions, "get_client", lambda: pytest.fail("client forbidden"))
    result = CliRunner().invoke(cli.app, BASE + args + ["--dry-run"])
    assert result.exit_code == 2, result.output
    assert journal.read() == []


@pytest.mark.parametrize("json_out", [False, True])
@pytest.mark.parametrize("operation", ["approve", "reject"])
def test_cli_actual_reviewed_send(monkeypatch, json_out, operation):
    base = BASE + [f"--{operation}"] + (["--remark", "Needs review"] if operation == "reject" else [])
    runner = CliRunner()
    draft = runner.invoke(cli.app, base + ["--dry-run", "--json"])
    assert draft.exit_code == 0, draft.output
    review = json.loads(draft.stdout)
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "PUT":
            return httpx.Response(200, json={})
        data = rows()
        if len(seen) > 1:
            data[0]["status"] = {"id": 7, "name": "approved" if operation == "approve" else "rejected"}
        return httpx.Response(200, json=data)
    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(actions, "get_client", lambda: c)
    try:
        args = base + ["--yes", "--expect-plan", review["plan_hash"], "--no-input"]
        if json_out:
            args.append("--json")
        result = runner.invoke(cli.app, args)
        assert result.exit_code == 0, result.output
        if json_out:
            assert json.loads(result.stdout)["decision_verified"] is True
        else:
            assert "named_decision" in result.stdout
        assert [r.method for r in seen] == ["GET", "PUT", "GET"]
    finally:
        c.close()


@pytest.mark.parametrize("complete", [True, False])
def test_paginated_inventory(monkeypatch, complete):
    seen, written = [], False
    def handler(req):
        nonlocal written
        seen.append(req)
        if req.method == "PUT":
            written = True
            return httpx.Response(200, json={})
        page = int(req.url.params.get("page", "1"))
        if page == 1:
            data = rows()[:1]
            if written:
                data[0]["status"] = {"id": 7, "name": "approved"}
            return httpx.Response(200, json={"approvals": data, "meta": {"has_next": True}})
        return httpx.Response(200, json={"approvals": rows()[1:], "meta": {"has_next": False}}
                              if complete else {"unexpected": []})
    c = mock_client(monkeypatch, handler)
    try:
        p = plan()
        if complete:
            assert actions.send(p, c, reviewed_hash=p.hash)["verified"] is True
            assert [r.method for r in seen] == ["GET", "GET", "PUT", "GET", "GET"]
        else:
            with pytest.raises(ApprovalError, match="no approval PUT attempted"):
                actions.send(p, c, reviewed_hash=p.hash)
            assert written is False
    finally:
        c.close()
