"""Incident CC is reviewed and included in the exact form plan."""
import json

import pytest
from typer.testing import CliRunner

import fsv.change_flow as flow
import fsv.cli as cli
from fsv import ticket_flow
from test_ticket_create import Client, WHO, draft, meta


@pytest.mark.parametrize("recipients", [("a@example.com",), ("a@globex.example.com", "b@globex.example.com")])
def test_cc_wire_and_hash(recipients):
    base = ticket_flow.build_create_plan(meta(), draft(), resolver=None, requester=WHO)
    plan = ticket_flow.build_create_plan(meta(), draft(), resolver=None, requester=WHO, cc_emails=recipients)
    assert plan.ok, plan.errors
    assert plan.payload["cc_emails"] == list(recipients)
    assert plan.payload["email"] == base.payload["email"]
    assert plan.hash != base.hash
    assert ("CC", ", ".join(recipients)) in plan.review.rows


@pytest.mark.parametrize("recipient", ["bad", "Name <a@example.com>", "a@example.com\nb@example.com"])
def test_invalid_cc_blocks_plan(recipient):
    p = ticket_flow.build_create_plan(meta(), draft(), resolver=None, requester=WHO, cc_emails=(recipient,))
    assert not p.ok and not p.hash


def test_cli_cc_review_and_one_write(monkeypatch, tmp_path):
    from fsv import journal
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "j")
    c, m = Client(), meta()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr("fsv.ticket_cli.load_meta", lambda *a, **kw: m)
    monkeypatch.setattr(flow, "load_meta", lambda *a, **kw: m)
    args = ["tickets", "create", "--requester", "jane.doe@example.com", "--set", "Subject=Example", "--set", "Description=Help", "--set", "Cloud Only=Yes", "--set", "Stage=Live", "--cc", "a@example.com", "--cc", "b@example.com"]
    runner = CliRunner()
    result = runner.invoke(cli.app, args + ["--dry-run", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["payload"]["cc_emails"] == ["a@example.com", "b@example.com"]
    assert not c.writes
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", data["plan_hash"]])
    assert result.exit_code == 0, result.output
    assert len(c.writes) == 1 and c.writes[0][2]["cc_emails"] == data["payload"]["cc_emails"]
