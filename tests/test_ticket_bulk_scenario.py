"""Bulk scenario: exact name/ID resolution (never by position), explicit IDs,
the review names the scenario and warns, 202 async with unverified effects."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, ticket_bulk
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client

SCENARIOS = [{"id": 501, "name": "Acme convert to request", "visibility": "shared"},
             {"id": 502, "name": "Assign to Acme Team", "visibility": "shared"},
             {"id": 503, "name": "assign to acme team", "visibility": "personal"},
             {"id": 504, "name": "502", "visibility": "shared"}]


@pytest.fixture
def tenant(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"writes": [], "status": 202}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "scenario_automations":
                return httpx.Response(200, json={"scenario_automations": SCENARIOS})
            tid = int(path.split("/")[1])
            return httpx.Response(200, json={"ticket": {"id": tid, "subject": f"Acme {tid}", "spam": False, "deleted": False}})
        s["writes"].append((req.method, path, json.loads(req.content)))
        return httpx.Response(s["status"], json={} if s["status"] < 400 else {"errors": "synthetic"})

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return c, s


def test_exact_name_resolution_and_ambiguity(tenant):
    c, _ = tenant
    assert ticket_bulk.resolve_scenario(c, "Acme convert to request")["id"] == 501
    assert ticket_bulk.resolve_scenario(c, "501")["id"] == 501
    assert ticket_bulk.resolve_scenario(c, "Assign to Acme Team")["id"] == 502  # exact wins over case-folded
    with pytest.raises(ReviewedWriteError, match="exactly one"):
        ticket_bulk.resolve_scenario(c, "ASSIGN TO ACME TEAM")
    with pytest.raises(ReviewedWriteError, match="exactly one"):
        ticket_bulk.resolve_scenario(c, "502")  # an ID and another scenario's name
    with pytest.raises(ReviewedWriteError, match="none"):
        ticket_bulk.resolve_scenario(c, "Assign")


def test_plan_payload_review_and_unverified_202(tenant):
    c, s = tenant
    plan = ticket_bulk.build_plan(c, ["INC-31", "30"], action="scenario", scenario="Acme convert to request")
    assert plan.write.payload == {"scenario_id": 501, "ids": [30, 31]}
    review = plan.review()
    assert review["scenario"] == {"id": 501, "name": "Acme convert to request"}
    assert "cannot be proven" in review["warning"] and "Acme convert to request" in review["warning"]
    result = ticket_bulk.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] is False and result["identity_verified"] == {"30": True, "31": True}
    assert s["writes"] == [("PUT", "tickets/execute_scenario", {"scenario_id": 501, "ids": [30, 31]})]
    with pytest.raises(ReviewedWriteError, match="sent"):
        ticket_bulk.send(plan, c, reviewed_hash=plan.hash)


def test_filters_and_option_mismatch_refused(tenant):
    c, s = tenant
    with pytest.raises(ValueError, match="explicit"):
        ticket_bulk.build_plan(c, [], action="scenario", where=["status=Open"], select_all=True, scenario="501")
    with pytest.raises(ValueError, match="--scenario"):
        ticket_bulk.build_plan(c, ["30"], action="scenario")
    with pytest.raises(ValueError, match="--scenario"):
        ticket_bulk.build_plan(c, ["30"], action="spam", scenario="501")
    assert not s["writes"]


def test_cli_prints_scenario_and_gates(tenant):
    c, s = tenant
    run = CliRunner().invoke
    args = ["tickets", "bulk", "scenario", "30", "31", "--scenario", "Assign to Acme Team"]
    review = run(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    assert "Scenario to run: 502 (Assign to Acme Team)" in review.stderr
    digest = json.loads(review.stdout)["plan_hash"]
    assert run(cli.app, args + ["--yes"]).exit_code == 2 and not s["writes"]
    done = run(cli.app, args + ["--yes", "--expect-plan", digest])
    assert done.exit_code == 4 and json.loads(done.stdout)["scenario"]["scenario_id"] == 502
    assert len(s["writes"]) == 1
