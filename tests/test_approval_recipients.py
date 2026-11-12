import json
from types import SimpleNamespace
import pytest
from typer.testing import CliRunner
from fsv import approval_recipients as a, change_approvals, config
from fsv.reviewed_write import ReviewedWriteError


def client():
    state = {"members": [81, 82], "writes": []}
    def get(path, params=None):
        if path == "bootstrap/agents_groups":
            return {"agents": [], "groups": [{"id": 71, "name": "Acme reviewers", "agent_ids": [999]}]}
        if path == "cabs":
            return {"cabs": [{"id": 91, "name": "Release review"}]}
        ids = state["members"] if "/groups/" in path else [82, 83]
        return {"approval_users": [{"id": i, "selected": False} for i in ids], "meta": {"count": len(ids), "page": 1, "per_page": 30}}
    return SimpleNamespace(int_get=get, state=state)


def test_flatten_eligible_users_not_bootstrap_agent_ids():
    c = client()
    ids, context = a.resolve(c, 23, ["Acme reviewers"], ["91"])
    assert ids == [81, 82, 83]
    assert 999 not in ids and 71 not in ids and 91 not in ids
    a.validate(c, 23, context)
    c.state["members"] = [81]
    with pytest.raises(ReviewedWriteError, match="membership changed"):
        a.validate(c, 23, context)


def test_empty_and_unknown_sources_fail_closed():
    c = client()
    c.state["members"] = []
    with pytest.raises(ReviewedWriteError, match="empty"):
        a.resolve(c, 23, ["71"], [])
    with pytest.raises(ReviewedWriteError, match="resolve"):
        a.resolve(c, 23, [], ["missing"])


def test_cli_group_cab_review_and_membership_hash(monkeypatch):
    import fsv.cli as cli
    c = client()
    monkeypatch.setattr(change_approvals, "get_client", lambda: c)
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    r = CliRunner().invoke(cli.app, ["changes", "approval-request", "CHN-23", "--name", "Acme review", "--message", "Review", "--approver-group", "Acme reviewers", "--cab", "Release review", "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    review = json.loads(r.stdout)
    assert review["payload"]["approval_groups"][0]["notified_to"] == [81, 82, 83]
    assert len(review["recipient_sources"]) == 2
    c.state["members"] = [81]
    r = CliRunner().invoke(cli.app, ["changes", "approval-request", "CHN-23", "--name", "Acme review", "--message", "Review", "--approver-group", "Acme reviewers", "--cab", "Release review", "--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)["plan_hash"] != review["plan_hash"]
