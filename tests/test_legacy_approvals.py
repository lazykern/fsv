"""Legacy flat-approval driver (capture-based, not live-proven): synthetic
tenant where approval-chains answers 400. Wire shapes follow blocked-submit
captures; write responses are untrusted and only readback verifies."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import change_approvals, cli, cli_sr, config, journal, legacy_approvals as legacy
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client

CAB_MEMBERS = [9000003101, 9000003102]


def row(ident, member, *, status=(0, "requested"), rtype=(1, "everyone"), reminded=None, module_key="change_id"):
    return {"id": ident, "member_id": member, "user_id": 9000009001, "level_id": 1, "approval_level_status": None,
            "remark": [], "status": {"id": status[0], "name": status[1]}, "type": {"id": rtype[0], "name": rtype[1]},
            "last_reminded_at": reminded, module_key: 41,
            "member": {"id": member, "name": "Example Approver", "email": "approver@example.com"}}


@pytest.fixture
def tenant(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    monkeypatch.setattr(config, "API_INT", "https://globex.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"approvals": {"changes": [row(710000000001, 9000003999, status=(1, "approved"))], "tickets": []},
         "writes": [], "post_effect": "create", "seen": []}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        s["seen"].append((req.method, path))
        if req.method == "GET":
            if path.endswith("/approval-chains"):
                return httpx.Response(400, json={"errors": "Parallel approvals feature needs to be enabled"})
            if path == "cabs":
                return httpx.Response(200, json={"cabs": [{"id": 9000007001, "name": "Globex CAB"}]})
            if path == "changes/41/cab-members":
                assert req.url.params["cab_id"] == "9000007001"
                return httpx.Response(200, json={"approval_agents": [{"id": i, "name": "A", "selected": False,
                                                                      "email": "a@example.com"} for i in CAB_MEMBERS]})
            if path == "approvals/approval_template":
                return httpx.Response(200, json="<p>Hi {{approval.member.name}}, please review {{ticket.subject}}</p>")
            if "search/autocomplete/requesters" in path:
                return httpx.Response(200, json={"results": [{"id": 9000004001, "email": "requester@example.com",
                                                               "value": "Example Requester"}]})
            if path in ("changes/41/approvals", "tickets/41/approvals"):
                module = path.split("/")[0]
                return httpx.Response(200, json={"approvals": copy.deepcopy(s["approvals"][module]),
                                                 "meta": {"triggerable_level": 1, "approval_type": 1}})
            raise AssertionError(path)
        body = json.loads(req.content) if req.content else None
        s["writes"].append((req.method, path, body))
        module = path.split("/")[0]
        if path.endswith("/resend"):
            for r in s["approvals"][module]:
                if path.endswith(f"/{r['id']}/resend"):
                    r["last_reminded_at"] = "2030-01-02T03:04:05Z"
        elif s["post_effect"] == "create":
            key = "change_id" if module == "changes" else "ticket_id"
            s["approvals"][module] += [row(720000000000 + i, m, rtype=(body["approval_type"], "x"), module_key=key)
                                       for i, m in enumerate(body["notified_to"])]
        return httpx.Response(201, json={"untrusted": True})
    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(change_approvals, "get_client", lambda: c)
    from fsv import change_approval_actions
    monkeypatch.setattr(change_approval_actions, "get_client", lambda: c)
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    return c, s


def test_change_cab_request_expands_members_client_side(tenant):
    c, s = tenant
    r = CliRunner().invoke(cli.app, ["changes", "approval-request", "CHN-41", "--cab", "Globex CAB", "--dry-run"])
    assert r.exit_code == 0, r.output
    review = json.loads(r.stdout)
    assert review["driver"] == "legacy" and "not yet live-proven" in review["provenance"]
    assert review["method"] == "POST" and review["path"] == "/api/_/changes/41/approvals"
    assert review["payload"] == {"approval_type": 1, "notified_to": CAB_MEMBERS}  # no CAB id/name/message
    assert not s["writes"]


def test_change_request_send_is_verified_only_by_readback_and_never_replayed(tenant):
    c, s = tenant
    plan = legacy.build_request(c, "changes", 41, CAB_MEMBERS)
    result = legacy.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] is True and result["approval_ids"] == [720000000000, 720000000001]
    assert [w[:2] for w in s["writes"]] == [("POST", "changes/41/approvals")]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        legacy.send(plan, c, reviewed_hash=plan.hash)
    assert len(s["writes"]) == 1


def test_unknown_outcome_is_unverified_not_resent(tenant):
    c, s = tenant
    s["post_effect"] = "nothing"
    plan = legacy.build_request(c, "changes", 41, CAB_MEMBERS, rule="first-responder")
    assert plan.payload["approval_type"] == 4
    result = legacy.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] is False and "do not resend" in result["warning"]
    assert len(s["writes"]) == 1


def test_ticket_request_uses_template_and_requester_ids(tenant):
    c, s = tenant
    r = CliRunner().invoke(cli.app, ["sr", "approvals", "request", "SR-41", "--approver", "requester@example.com",
                                     "--dry-run"])
    assert r.exit_code == 0, r.output
    payload = json.loads(r.stdout)["payload"]
    assert payload == {"approval_type": 1, "notified_to": [9000004001],
                       "email_content": "<p>Hi {{approval.member.name}}, please review {{ticket.subject}}</p>"}
    r = CliRunner().invoke(cli.app, ["sr", "approvals", "request", "SR-41", "--approver", "9000004001",
                                     "--name", "x", "--message", "y", "--dry-run"])
    assert r.exit_code == 2 and "do not apply to legacy" in r.output


@pytest.mark.parametrize("module", ["changes", "tickets"])
def test_remind_is_bodyless_and_verified_by_last_reminded_at(tenant, module):
    c, s = tenant
    key = "change_id" if module == "changes" else "ticket_id"
    s["approvals"][module] = [row(730000000001, 9000003101, module_key=key)]
    plan = legacy.build_remind(c, module, 41, 730000000001)
    assert plan.method == "PUT" and plan.path == f"{module}/41/approvals/730000000001/resend" and plan.payload is None
    result = legacy.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] is True and s["writes"] == [("PUT", plan.path, None)]


def test_remind_refuses_a_decided_approval_and_cli_routes(tenant):
    c, s = tenant
    with pytest.raises(ReviewedWriteError, match="only a requested approval"):
        legacy.build_remind(c, "changes", 41, 710000000001)
    s["approvals"]["changes"].append(row(730000000002, 9000003101))
    r = CliRunner().invoke(cli.app, ["changes", "approval-remind", "CHN-41", "730000000002", "--dry-run"])
    assert r.exit_code == 0, r.output and json.loads(r.stdout)["driver"] == "legacy"


@pytest.mark.parametrize("args", [["changes", "approval-cancel", "CHN-41", "710000000001", "--dry-run"],
                                  ["sr", "approvals", "approve", "SR-41", "710000000001", "--dry-run"],
                                  ["sr", "approvals", "cancel", "SR-41", "710000000001", "--dry-run"]])
def test_cancel_and_decisions_refuse_on_legacy(tenant, args):
    c, s = tenant
    r = CliRunner().invoke(cli.app, args)
    assert r.exit_code == 2 and "legacy flat approvals" in r.output
    assert not s["writes"]


def test_unverified_rules_are_refused_on_legacy(tenant):
    c, _ = tenant
    with pytest.raises(ValueError, match="observed rules"):
        legacy.build_request(c, "changes", 41, CAB_MEMBERS, rule="majority")
