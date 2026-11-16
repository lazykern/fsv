"""assets search / candidates --per-page: the page size is sent, bounded
1..100, and page, per_page, pages and total_rows are printed, so paging can
be proven with a few devices."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, config, integrated_assets as assets

DEVICES = [{"name": {"id": 100 + i, "label": f"acme-device-{i}"}, "device_pk": 100 + i} for i in range(5)]


class Fake:
    def __init__(self, legacy=False):
        self.requests = []
        self.legacy = legacy

    def _request(self, method, url, params=None, **kw):
        self.requests.append(params)
        size, page = params["page_size"], params["page"]
        rows = DEVICES[(page - 1) * size: page * size]
        if not rows and page > 1:
            return {"error": "Requested page is out of range.", "data": [], "page": 1, "total_rows": len(DEVICES)}
        return {"data": rows, "page": page, "total_rows": len(DEVICES)}

    def int_get(self, path, params=None):
        if path == "changes/7/associated-cis":
            from fsv.errors import APIError
            raise APIError(404, "{}")
        if path == "changes/7/assets":
            return {"associated_assets": []}
        if path == "changes/7":
            return {"change": {"id": 7, "requester_id": 3}}
        assert path == "assets-to-associate"
        self.requests.append(params)
        size, page = params["per_page"], params["page"]
        rows = [{"display_id": i + 1, "name": f"acme-asset-{i}"} for i in range(5)][(page - 1) * size: page * size]
        return {"assets": rows, "meta": {"total_count": 5}}


@pytest.fixture(autouse=True)
def tenant(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")


def test_per_page_is_sent_and_paging_is_reported(monkeypatch):
    c = Fake()
    monkeypatch.setattr(cli, "_client", lambda: c)
    r = CliRunner().invoke(cli.app, ["assets", "search", "acme", "--per-page", "2", "--page", "3", "--json"])
    assert r.exit_code == 0, r.output
    out = json.loads(r.stdout)
    assert (out["page"], out["per_page"], out["pages"], out["total_rows"]) == (3, 2, 3, 5)
    assert [d["name"]["id"] for d in out["data"]] == [104] and out["complete"] is False
    assert c.requests[-1]["page_size"] == 2
    r = CliRunner().invoke(cli.app, ["assets", "search", "--per-page", "2", "--page", "4", "--json"])
    assert r.exit_code != 0 and "3 page(s) of 2" in r.output


def test_default_page_size_is_unchanged_and_bounds_are_enforced(monkeypatch):
    c = Fake()
    out = assets.search(c, "")
    assert out["per_page"] == assets.PAGE_SIZE == 50 and c.requests[-1]["page_size"] == 50 and out["complete"]
    for bad in (0, 101):
        with pytest.raises(ValueError, match="1 to 100"):
            assets.search(c, "", per_page=bad)
    monkeypatch.setattr(cli, "_client", lambda: c)
    assert CliRunner().invoke(cli.app, ["assets", "search", "--per-page", "101"]).exit_code == 2


def test_legacy_picker_reads_every_page_at_the_chosen_size(monkeypatch):
    c = Fake(legacy=True)
    out = assets.candidates(c, "changes", 7, per_page=2)
    assert out["driver"] == assets.LEGACY and [r["display_id"] for r in out["data"]] == [1, 2, 3, 4, 5]
    assert (out["per_page"], out["pages"], out["total_rows"]) == (2, 3, 5)
    assert [p["per_page"] for p in c.requests] == [2, 2, 2] and [p["page"] for p in c.requests] == [1, 2, 3]
    c.requests.clear()
    assert assets.candidates(c, "changes", 7)["per_page"] == assets.LEGACY_PAGE_SIZE == 30
    assert [p["per_page"] for p in c.requests] == [30]
