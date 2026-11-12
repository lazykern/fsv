"""Legacy asset association writes (capture-based, not live-proven): item ids
are asset display ids, candidates come from assets-to-associate, only an
independent read of the legacy list verifies, removal is refused."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, cli_sr, config, integrated_assets as assets, journal
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client

CANDIDATES = [{"id": 770000000011, "display_id": 3811, "name": "Acme laptop", "used_by": "Example Requester"},
              {"id": 770000000012, "display_id": 3812, "name": "Globex phone", "used_by": "Example Requester"}]


@pytest.fixture(params=[("changes", "CHN-41", "changes/41/assets", "changes/41/assets/associate"),
                        ("tickets", "SR-41", "tickets/41/associated_assets", "tickets/41/associate_assets")])
def tenant(request, monkeypatch, tmp_path):
    module, target, read, write = request.param
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    monkeypatch.setattr(config, "API_INT", "https://globex.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"assoc": [], "writes": [], "effect": True, "candidate_reads": []}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path.endswith("associated-cis"):
                return httpx.Response(404, json={})
            if path == read:
                return httpx.Response(200, json={"associated_assets": copy.deepcopy(s["assoc"])})
            if path == f"{module}/41":
                return httpx.Response(200, json={module[:-1]: {"id": 41, "requester_id": 9000004001}})
            if path == "assets-to-associate":
                s["candidate_reads"].append(dict(req.url.params))
                return httpx.Response(200, json={"assets": CANDIDATES, "meta": {"total_count": 2, "labels": []}})
            raise AssertionError(path)
        body = json.loads(req.content)
        s["writes"].append((req.method, path, body))
        if s["effect"]:
            s["assoc"] += [{"id": 880000000000 + i, "config_item": {"id": 1, "display_id": i, "name": "x"}}
                           for i in body["item_ids"]]
        return httpx.Response(200, json={"untrusted": True})
    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    return c, s, module, target, write


def test_add_puts_display_ids_and_verifies_by_readback_only(tenant):
    c, s, module, target, write = tenant
    plan = assets.build_plan(c, module, target, [3812, 3811])
    assert plan.method == "PUT" and plan.path == write and plan.payload == {"item_ids": [3811, 3812]}
    params = s["candidate_reads"][0]
    assert params["entity"] == module[:-1] and params["user_id"] == "9000004001" and params["entity_id"] == "41"
    result = assets.send(plan, c, module, [3811, 3812], reviewed_hash=plan.hash)
    assert result["verified"] is True and "not yet live-proven" in result["provenance"]
    assert len(s["writes"]) == 1
    with pytest.raises(ReviewedWriteError, match="was sent"):
        assets.send(plan, c, module, [3811, 3812], reviewed_hash=plan.hash)


def test_unknown_outcome_is_unverified_and_not_resent(tenant):
    c, s, module, target, _ = tenant
    s["effect"] = False
    plan = assets.build_plan(c, module, target, [3811])
    result = assets.send(plan, c, module, [3811], reviewed_hash=plan.hash)
    assert result["verified"] is False and len(s["writes"]) == 1


def test_guards_removal_noncandidates_duplicates(tenant):
    c, s, module, target, _ = tenant
    with pytest.raises(ReviewedWriteError, match="no captured wire"):
        assets.build_plan(c, module, target, [3811], remove=True)
    with pytest.raises(ReviewedWriteError, match="not among the assets"):
        assets.build_plan(c, module, target, [9999])
    s["assoc"] = [{"id": 1, "config_item": {"id": 1, "display_id": 3811, "name": "x"}}]
    with pytest.raises(ReviewedWriteError, match="already associated"):
        assets.build_plan(c, module, target, [3811])
    assert not s["writes"]


def test_cli_add_dry_run_and_candidates(tenant):
    c, s, module, target, write = tenant
    group = "sr" if target.startswith("SR") else module
    r = CliRunner().invoke(cli.app, [group, "assets", "add", target, "3811", "--dry-run"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.stdout)["path"] == "/api/_/" + write
    r = CliRunner().invoke(cli.app, [group, "assets", "candidates", target, "--json"])
    assert r.exit_code == 0, r.output and [a["display_id"] for a in json.loads(r.stdout)["data"]] == [3811, 3812]
    assert not s["writes"]
