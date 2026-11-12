import copy
import json
from types import SimpleNamespace
import pytest
from fsv import config, sr_approvals as a
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture
def wire(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path))
    state = {"groups": [], "writes": [], "persist": True}
    def get(path, params=None):
        assert path == "tickets/23/approval-chains"
        return copy.deepcopy({"approval_chains": [{"id": 71, "approval_status": 0, "approval_groups": state["groups"]}], "meta": {"triggerable_level": 1}})
    def request(method, path, **kw):
        state["writes"].append((method, path, kw))
        if not state["persist"]:
            return {}
        if method == "POST":
            body = json.loads(kw["content"])["approval_groups"][0]
            state["groups"].append({"id": 91, "name": body["name"], "approval_type": body["approval_type"], "approval_status": 0,
                                    "approvals": [{"id": 101, "member_id": 81, "user_id": 81, "status": {"id": 0, "name": "requested"}, "remark": [], "last_reminded_at": None}]})
        else:
            group = state["groups"][0]
            row = group["approvals"][0]
            assert kw["headers"]["Content-Type"] == "application/json"
            if path.endswith("/resend"):
                assert "content" not in kw
                row["last_reminded_at"] = "2026-12-01T10:00:00Z"
            elif path.endswith("/cancel"):
                assert "content" not in kw
                group["approval_status"] = 3
                row["status"] = {"id": 3, "name": "cancelled"}
            else:
                body = json.loads(kw["content"])
                ident = body["approval_status"]
                row["status"] = {"id": ident, "name": "approved" if ident == 1 else "rejected"}
                row["remark"].append({"status": ident, "data": body["remark"]})
        return {"approval_chain": {"id": 999}}
    c = SimpleNamespace(int_get=get, _request=request, _get_csrf=lambda: "synthetic", state=state)
    return c


def create(c):
    plan = a.build_plan(c, "SR-23", "request", name="Acme review", message="Please review", approvers=["81"])
    assert a.send(plan, c, reviewed_hash=plan.hash)["verified"]


@pytest.mark.parametrize("operation", ["approve", "reject", "cancel", "remind"])
def test_accepted_shapes(wire, operation):
    create(wire)
    plan = a.build_plan(wire, 23, operation, ident=91 if operation == "cancel" else 101,
                        remark="Needs revision" if operation == "reject" else "")
    assert a.send(plan, wire, reviewed_hash=plan.hash)["verified"]
    assert len(wire.state["writes"]) == 2
    assert plan.method == "PUT"


def test_response_echo_not_proof_and_replay_blocked(wire):
    plan = a.build_plan(wire, 23, "request", name="Acme review", message="Review", approvers=["81"])
    wire.state["persist"] = False
    assert not a.send(plan, wire, reviewed_hash=plan.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="outcome unknown"):
        a.send(plan, wire, reviewed_hash=plan.hash)
    assert len(wire.state["writes"]) == 1


def test_immutable_reject_validation_and_full_hash(wire):
    create(wire)
    with pytest.raises(ValueError, match="remark"):
        a.build_plan(wire, 23, "reject", ident=101)
    plan = a.build_plan(wire, 23, "approve", ident=101)
    with pytest.raises(ReviewedWriteError, match="full review hash"):
        a.send(plan, wire, reviewed_hash=plan.hash[:12])
    assert len(wire.state["writes"]) == 1
    assert a.send(plan, wire, reviewed_hash=plan.hash)["verified"]
    wire.state["groups"][0]["approval_status"] = 1
    with pytest.raises(ReviewedWriteError, match="immutable"):
        a.build_plan(wire, 23, "cancel", ident=91)
