"""assets candidates: legacy assets-to-associate only on legacy records; a 403
(plan without CMDB), 404 or integrated/problem record uses the integrated
inventory search, marking rows already associated with the exact record."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, cli_sr, config, integrated_assets as assets, journal
from fsv.errors import APIError
from test_ticket_notes import mock_client

DEVICES = [{"device_pk": 81, "name": {"id": 81, "label": "Acme laptop"}, "type": "Laptop"},
           {"device_pk": 82, "name": {"id": 82, "label": "Globex phone"}, "type": "Phone"}]


@pytest.fixture
def tenant(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"integrated": True, "legacy_status": 403, "searches": [], "writes": []}

    def handler(req):
        if req.method != "GET":
            s["writes"].append(req.url.path)
            return httpx.Response(500)
        path = req.url.path
        if path.startswith("/itam/core/"):
            s["searches"].append((path, dict(req.url.params)))
            return httpx.Response(200, json={"page": int(req.url.params["page"]), "error": None,
                                             "total_rows": 2, "data": DEVICES})
        path = path.removeprefix("/api/_/")
        if path.endswith("associated-cis"):
            if not s["integrated"]:
                return httpx.Response(404, json={})
            return httpx.Response(200, json={"data": [{"item_id": 82, "itam_cmdb_request_id": 5, "category": "Device"}],
                                             "meta": {"total_count": 1, "total_pages": 1}})
        if path in {"tickets/41/associated_assets", "changes/41/assets"}:
            return httpx.Response(200, json={"associated_assets": []})
        if path in {"tickets/41", "changes/41"}:
            return httpx.Response(200, json={path.split("/")[0][:-1]: {"id": 41, "requester_id": 7}})
        if path == "assets-to-associate":
            if s["legacy_status"] != 200:
                return httpx.Response(s["legacy_status"], json={"description": "feature not in plan"})
            return httpx.Response(200, json={"assets": [{"id": 1, "display_id": 3811}], "meta": {"total_count": 1}})
        raise AssertionError(path)

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    return c, s


def test_integrated_record_uses_inventory_search_and_marks_associated(tenant):
    c, s = tenant
    out = assets.candidates(c, "tickets", "INC-41", "acme")
    assert out["driver"] == "integrated" and [r["id"] for r in out["data"]] == [81, 82]
    assert [r["associated"] for r in out["data"]] == [False, True] and out["complete"] is True
    assert s["searches"][0][0] == "/itam/core/device/api/" and s["searches"][0][1]["q"] == "acme"
    assert not s["writes"]


@pytest.mark.parametrize("status", [403, 404])
@pytest.mark.parametrize("module,target", [("tickets", "SR-41"), ("changes", "CHN-41")])
def test_legacy_403_or_404_falls_back_to_integrated_search(tenant, status, module, target):
    c, s = tenant
    s.update(integrated=False, legacy_status=status)
    out = assets.candidates(c, module, target)
    assert out["driver"] == "integrated" and f"HTTP {status}" in out["note"]
    assert {r["associated"] for r in out["data"]} == {None} and "unknown" in out["associated_note"]


def test_legacy_record_with_working_list_keeps_legacy_and_other_errors_propagate(tenant):
    c, s = tenant
    s.update(integrated=False, legacy_status=200)
    out = assets.candidates(c, "tickets", "SR-41")
    assert out["driver"] == "legacy" and out["data"][0]["display_id"] == 3811 and not s["searches"]
    s["legacy_status"] = 500
    with pytest.raises(APIError):
        assets.candidates(c, "tickets", "SR-41")


def test_problem_without_association_endpoint_and_cli(tenant):
    c, s = tenant
    s["integrated"] = False
    out = assets.candidates(c, "problems", "PRB-41", kind="services")
    assert out["kind"] == "services" and s["searches"][-1][0] == "/itam/core/businessservice/api/"
    runner = CliRunner()
    for group, target in [("problems", "PRB-41"), ("tickets", "INC-41"), ("sr", "SR-41"), ("changes", "CHN-41")]:
        r = runner.invoke(cli.app, [group, "assets", "candidates", target, "--search", "acme", "--json"])
        assert r.exit_code == 0, r.output
        assert json.loads(r.stdout)["driver"] == "integrated"
    assert not s["writes"]
