"""Asset association endpoint detection: integrated-ITAM associated-cis vs
the legacy associated-assets list, probed by read-only GETs (404 = absent).
Synthetic tenants and ids only."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, integrated_assets as assets
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client

LEGACY_ROWS = [{"id": 880000000002, "change_id": 41,
                "config_item": {"id": 770000000002, "name": "Globex laptop", "display_id": 3802, "asset_type": "Laptop"}},
               {"id": 880000000001, "change_id": 41,
                "config_item": {"id": 770000000001, "name": "Acme server", "display_id": 3801, "asset_type": "Server"}}]
INTEGRATED = {"data": [{"item_id": 501, "itam_cmdb_request_id": 601, "category": "Device"}],
              "meta": {"total_count": 1, "total_pages": 1}}


def tenant(monkeypatch, present: dict[str, int]):
    seen = []

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        seen.append((req.method, path))
        assert req.method == "GET", "no writes in detection"
        status = present.get(path.rsplit("/", 1)[-1], 404)
        if status != 200:
            return httpx.Response(status, json={"errors": "absent"})
        body = INTEGRATED if path.endswith("associated-cis") else {"associated_assets": LEGACY_ROWS}
        return httpx.Response(200, json=body)
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    return mock_client(monkeypatch, handler), seen


@pytest.mark.parametrize("module,legacy_leaf,target", [("tickets", "associated_assets", "SR-41"),
                                                       ("changes", "assets", "CHN-41")])
def test_legacy_list_when_associated_cis_is_absent(monkeypatch, module, legacy_leaf, target):
    c, seen = tenant(monkeypatch, {legacy_leaf: 200})
    out = assets.listing(c, module, target)
    assert out["driver"] == "legacy" and out["complete"] is True
    assert [r["config_item"]["display_id"] for r in out["data"]] == [3801, 3802]
    assert (("GET", f"{module}/41/associated-cis")) in seen


def test_integrated_driver_is_kept_where_it_exists(monkeypatch):
    c, _ = tenant(monkeypatch, {"associated-cis": 200, "associated_assets": 200})
    out = assets.listing(c, "tickets", 41)
    assert out["driver"] == "integrated" and out["data"][0]["item_id"] == 501


def test_neither_endpoint_fails_closed(monkeypatch):
    c, _ = tenant(monkeypatch, {})
    with pytest.raises(ReviewedWriteError, match="no asset association endpoint found"):
        assets.listing(c, "tickets", 41)


def test_non_404_probe_failure_is_not_absence(monkeypatch):
    c, _ = tenant(monkeypatch, {"associated-cis": 403})
    with pytest.raises(APIError):
        assets.driver(c, "tickets", 41)


def test_legacy_rows_need_display_ids_and_services_are_refused(monkeypatch):
    c, _ = tenant(monkeypatch, {"associated_assets": 200})
    with pytest.raises(ReviewedWriteError, match="business services"):
        assets.listing(c, "tickets", 41, kind="services")
    LEGACY_ROWS.append({"id": 1, "config_item": {"name": "no display id"}})
    try:
        with pytest.raises(ReviewedWriteError, match="positive numeric identity"):
            assets.listing(c, "tickets", 41)
    finally:
        LEGACY_ROWS.pop()


def test_cli_sr_and_tickets_assets_list_on_the_legacy_driver(monkeypatch):
    c, _ = tenant(monkeypatch, {"associated_assets": 200})
    monkeypatch.setattr(cli, "_client", lambda: c)
    from fsv import cli_sr
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    for args in (["sr", "assets", "list", "SR-41", "--json"], ["tickets", "assets", "list", "INC-41", "--json"]):
        r = CliRunner().invoke(cli.app, args)
        assert r.exit_code == 0, r.output
        assert json.loads(r.stdout)["driver"] == "legacy"
