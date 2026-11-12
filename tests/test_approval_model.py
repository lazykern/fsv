"""Approval model detection: legacy flat-approval tenants (approval-chains
answers 400/404) refuse chains/groups-only actions with a clear message and
list the flat approvals read-only. Synthetic tenants and ids only."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import approval_model, change_approval_actions as actions, change_approvals as approvals, cli, config, journal
from fsv import sr_approvals
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client

FLAT = [{"id": 710000000001, "member_id": 9000003001, "user_id": 9000004001, "level_id": 1,
         "approval_level_status": None, "remark": [], "status": {"id": 0, "name": "requested"},
         "type": {"id": 1, "name": "everyone"}, "last_reminded_at": None, "change_id": 123,
         "member": {"id": 9000003001, "name": "Acme Approver", "email": "approver@example.com"}}]


@pytest.fixture(params=[("acme.example.com", 400), ("globex.example.com", 404)])
def legacy(request, monkeypatch):
    domain, status = request.param
    monkeypatch.setattr(config, "DOMAIN", domain)
    monkeypatch.setattr(config, "API_INT", f"https://{domain}/api/_")
    seen = []

    def handler(req):
        seen.append((req.method, req.url.path))
        if req.url.path.endswith("/approval-chains"):
            return httpx.Response(status, json={"errors": "Parallel approvals feature needs to be enabled"})
        if req.method == "GET" and req.url.path.endswith("/approvals"):
            return httpx.Response(200, json={"approvals": FLAT, "meta": {"triggerable_level": 1, "approval_type": 1}})
        if req.method == "GET" and req.url.path.endswith("/approval_template"):
            return httpx.Response(200, json="<p>template</p>")
        raise AssertionError(f"unexpected {req.method} {req.url.path}")

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(approvals, "get_client", lambda: c)
    monkeypatch.setattr(actions, "get_client", lambda: c)
    monkeypatch.setattr(cli, "_client", lambda: c)
    from fsv import create
    monkeypatch.setattr(create, "get_client", lambda: c)
    return c, seen, status


def writes(seen):
    return [s for s in seen if s[0] != "GET"]


def test_detect_models(legacy):
    c, _, status = legacy
    assert approval_model.detect(c, "changes", 123) == (approval_model.LEGACY, status)
    with pytest.raises(ReviewedWriteError, match="legacy flat approvals: not yet supported"):
        approval_model.require_chains(c, "tickets", 123, "sr approvals cancel")


def test_unknown_model_is_not_legacy(monkeypatch):
    c = mock_client(monkeypatch, lambda req: httpx.Response(403, json={}))
    assert approval_model.detect(c, "changes", 5) == (approval_model.UNKNOWN, 403)
    approval_model.require_chains(c, "changes", 5, "x")  # proven legacy only refuses


def test_change_request_refuses_before_post(legacy):
    c, seen, status = legacy
    p = approvals.build_plan("CHN-123", "Acme review", "Please review", approver_ids=[9000003001])
    with pytest.raises(approvals.ApprovalError, match=r"approval-request refused: .*legacy flat approvals .*HTTP "
                       + str(status) + r".*not yet supported"):
        approvals.send(p, c, reviewed_hash=p.hash)
    assert not writes(seen)
    assert journal.read() == []


@pytest.mark.parametrize("operation,remark,command", [("approve", None, "approval-respond --approve"),
                                                      ("reject", "no", "approval-respond --reject"),
                                                      ("cancel", None, "approval-cancel")])
def test_change_decisions_and_cancel_refuse(legacy, operation, remark, command):
    c, seen, _ = legacy
    p = actions.build_plan("CHN-123", 710000000001, operation, remark)
    with pytest.raises(approvals.ApprovalError, match=f"{command} refused: .*legacy flat approvals: not yet supported"):
        actions.send(p, c, reviewed_hash=p.hash)
    assert not writes(seen)


def test_change_cancel_cli_refuses_even_on_dry_run(legacy):
    r = CliRunner().invoke(cli.app, ["changes", "approval-cancel", "CHN-123", "710000000001", "--dry-run"])
    assert r.exit_code == 2 and "legacy flat approvals: not yet supported" in r.output


def test_change_groups_listing_names_model_and_flat_listing_works(legacy):
    r = CliRunner().invoke(cli.app, ["changes", "approvals", "CHN-123", "--groups", "--json"])
    assert r.exit_code != 0 and "legacy flat approvals" in r.output
    r = CliRunner().invoke(cli.app, ["changes", "approvals", "CHN-123", "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)[0]["member_id"] == 9000003001


@pytest.mark.parametrize("operation", ["request", "approve", "reject", "cancel", "remind"])
def test_sr_writes_refuse(legacy, operation):
    c, seen, _ = legacy
    kwargs = ({"name": "Acme", "message": "m", "approvers": ["9000004001"]} if operation == "request"
              else {"ident": 710000000001, "remark": "r"})
    with pytest.raises(ReviewedWriteError, match=f"sr approvals {operation} refused: .*not yet supported"):
        sr_approvals.build_plan(c, "SR-123", operation, **kwargs)
    assert not writes(seen)


def test_sr_listing_uses_flat_list(legacy):
    r = CliRunner().invoke(cli.app, ["sr", "approvals", "list", "SR-123", "--json"])
    assert r.exit_code == 0, r.output
    out = json.loads(r.stdout)
    assert out["model"] == "legacy" and "legacy flat approvals" in out["note"]
    assert [a["id"] for a in out["approvals"]] == [710000000001]
