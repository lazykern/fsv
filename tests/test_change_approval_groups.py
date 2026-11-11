"""Offline approval-chain reads and irreversible, single-attempt cancellation."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_approval_actions as actions, cli, journal
from fsv.change_approvals import ApprovalError, get_change_approval_groups
from test_change_approvals import rows
from test_ticket_notes import mock_client

GID = 310000000099
BASE = ["changes", "approval-cancel", "CHN-123", str(GID)]


def chains():
    return {"approval_chains": [{"id": 310000000098, "approval_status": 0,
            "approval_type": 1, "approval_groups": [{"id": GID, "approval_status": 0,
            "approval_type": 1, "name": "Acme review", "approvals": rows()}]}], "meta": {"triggerable_level": 1}}


def events():
    return [r["event"] for r in journal.read() if r.get("kind") == "plan:change_approval_cancel"]


@pytest.mark.parametrize("gone", [False, True])
@pytest.mark.parametrize("status", [8, {"id": 8}, {"id": 8, "name": "unobserved_state"}])
def test_cancel_exact_bodyless_put_and_verification(monkeypatch, gone, status):
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "PUT":
            assert req.url.path == f"/api/_/changes/123/approval-groups/{GID}/cancel"
            assert req.content == b"" and req.headers["content-type"] == "application/json"
            assert req.headers["x-csrf-token"] == "synthetic-token"
            return httpx.Response(200, json={"not": "evidence"})
        assert req.url.path == "/api/_/changes/123/approval-chains"
        data = chains()
        if len(seen) > 1:
            if gone:
                data["approval_chains"] = []
            else:
                for row in data["approval_chains"][0]["approval_groups"][0]["approvals"]:
                    row["status"] = status
        return httpx.Response(200, json=data)
    c = mock_client(monkeypatch, handler)
    try:
        p = actions.build_plan(123, GID, "cancel")
        result = actions.send(p, c, reviewed_hash=p.hash)
        assert result["verified"] is True and result["group_id"] == GID
        assert result["verification_basis"] == ("group_gone" if gone else "changed_raw_statuses_only")
        assert result["observed_status"] is None if gone else all(r["status"] == status for r in result["observed_status"])
        assert [r.method for r in seen] == ["GET", "PUT", "GET"]
        assert events() == ["started", "done"]
        with pytest.raises(ApprovalError, match="do not resubmit"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert len(seen) == 3
    finally:
        c.close()


@pytest.mark.parametrize("case", ["unchanged", "one-requested", "missing-record", "wrong-member", "malformed", "partial", "error"])
def test_cancel_unknown_readback_blocks_replay(monkeypatch, case):
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "PUT":
            return httpx.Response(200, json={})
        data = chains()
        if len(seen) > 1:
            group = data["approval_chains"][0]["approval_groups"][0]
            for r in group["approvals"]:
                r["status"] = {"id": 8}
            if case == "unchanged":
                data = chains()
            elif case == "one-requested":
                group["approvals"][0]["status"] = {"id": 0, "name": "requested"}
            elif case == "missing-record":
                group["approvals"].pop()
            elif case == "wrong-member":
                group["approvals"][0]["member_id"] += 99
            elif case == "malformed":
                data = {"approval_chains": None}
            elif case == "partial":
                data["meta"]["partial"] = True
            elif case == "error":
                return httpx.Response(403, json={})
        return httpx.Response(200, json=data)
    c = mock_client(monkeypatch, handler)
    try:
        p = actions.build_plan(123, GID, "cancel")
        with pytest.raises(ApprovalError, match="outcome unknown"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert events() == ["started"]
        with pytest.raises(ApprovalError, match="do not resubmit"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert sum(r.method == "PUT" for r in seen) == 1
    finally:
        c.close()


@pytest.mark.parametrize("failure", [403, 408, 429, 500, "timeout"])
def test_cancel_write_no_replay(monkeypatch, failure):
    seen = []
    def handler(req):
        seen.append(req)
        if req.method == "GET":
            return httpx.Response(200, json=chains())
        if failure == "timeout":
            raise httpx.ReadTimeout("synthetic", request=req)
        return httpx.Response(failure, json={})
    c = mock_client(monkeypatch, handler)
    try:
        p = actions.build_plan(123, GID, "cancel")
        with pytest.raises(ApprovalError, match="rejected|unknown"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert [r.method for r in seen] == ["GET", "PUT"]
        assert events() == (["started", "failed"] if failure == 403 else ["started"])
    finally:
        c.close()


@pytest.mark.parametrize("case", ["shape", "chain", "group", "record", "member", "status", "duplicate", "partial", "total", "page", "missing", "denied", "nested-partial", "nested-total"])
def test_reader_and_cancel_preparation_fail_closed(monkeypatch, case):
    data = chains()
    group = data["approval_chains"][0]["approval_groups"][0]
    if case == "shape":
        data = []
    elif case == "chain":
        data["approval_chains"][0]["id"] = True
    elif case == "group":
        group["id"] = 0
    elif case == "record":
        group["approvals"].append(None)
    elif case == "member":
        group["approvals"][0]["member"]["id"] += 99
    elif case == "status":
        group["approvals"][0]["status"] = True
    elif case == "duplicate":
        group["approvals"].append(copy.deepcopy(group["approvals"][0]))
    elif case == "partial":
        data["meta"]["partial"] = True
    elif case == "nested-partial":
        group["approval_group_meta"] = {"has_more": True}
    elif case == "nested-total":
        data["approval_chains"][0]["approval_chain_meta"] = {"total_count": 99}
    elif case == "total":
        data["meta"]["total_count"] = 99
    elif case == "page":
        data["meta"]["has_next"] = True
    elif case == "missing":
        data["approval_chains"] = []
    seen = []
    def handler(req):
        seen.append(req)
        assert req.method == "GET"
        if case == "page" and len(seen) > 1:
            return httpx.Response(200, json={"wrong": []})
        return httpx.Response(403 if case == "denied" else 200, json=data)
    c = mock_client(monkeypatch, handler)
    try:
        p = actions.build_plan(123, GID, "cancel")
        with pytest.raises(ApprovalError, match="no cancel PUT attempted"):
            actions.send(p, c, reviewed_hash=p.hash)
        assert events() == []
    finally:
        c.close()


def test_reader_complete_pagination(monkeypatch):
    def handler(req):
        page = int(req.url.params["page"])
        data = chains()
        data["meta"] = {"has_next": page == 1, "total_count": 2}
        if page == 2:
            data["approval_chains"][0]["id"] += 1
            data["approval_chains"][0]["approval_groups"] = []
        return httpx.Response(200, json=data)
    c = mock_client(monkeypatch, handler)
    try:
        assert get_change_approval_groups(123, c) == chains()["approval_chains"][0]["approval_groups"]
    finally:
        c.close()


@pytest.mark.parametrize("format_", ["table", "json", "csv"])
def test_groups_read_cli(monkeypatch, format_):
    def handler(req):
        assert req.method == "GET" and req.url.path.endswith("/approval-chains")
        return httpx.Response(200, json=chains())
    c = mock_client(monkeypatch, handler)
    from fsv import create
    monkeypatch.setattr(create, "get_client", lambda: c)
    try:
        r = CliRunner().invoke(cli.app, ["changes", "approvals", "CHN-123", "--groups", "--output", format_])
        assert r.exit_code == 0, r.output
        if format_ == "json":
            assert json.loads(r.stdout) == chains()["approval_chains"][0]["approval_groups"]
        else:
            assert str(GID) in r.stdout and "9000003001" in r.stdout
        assert events() == []
    finally:
        c.close()


@pytest.mark.parametrize("args", [["--read-only"], ["--yes"], ["--no-input"],
                                   ["--yes", "--expect-plan", "bad"]])
def test_cancel_cli_local_guards_no_io(monkeypatch, args):
    monkeypatch.setattr(actions, "get_client", lambda: pytest.fail("client forbidden"))
    result = CliRunner().invoke(cli.app, BASE + args)
    assert result.exit_code == 2, result.output
    assert events() == []


def test_cancel_cli_review_and_send(monkeypatch):
    written = False
    def handler(req):
        nonlocal written
        if req.method == "PUT":
            written = True
            return httpx.Response(200, json={})
        data = chains()
        if written:
            data["approval_chains"] = []
        return httpx.Response(200, json=data)
    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(actions, "get_client", lambda: c)
    try:
        runner = CliRunner()
        r = runner.invoke(cli.app, BASE + ["--dry-run", "--read-only", "--json"])
        assert r.exit_code == 0, r.output
        review = json.loads(r.stdout)
        assert review["group"]["approvals"][0]["member_id"] == 9000003001
        assert review["group"]["approvals"][0]["status"] == {"id": 0, "name": "requested"}
        assert not written and events() == []
        r = runner.invoke(cli.app, BASE + ["--yes", "--expect-plan", review["plan_hash"], "--no-input", "--json"])
        assert r.exit_code == 0, r.output
        assert json.loads(r.stdout)["verified"] is True
        assert events() == ["started", "done"]
    finally:
        c.close()
