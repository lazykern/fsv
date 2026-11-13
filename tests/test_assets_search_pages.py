"""assets search --page N past the end says 'no results on page N' for every
page-2 answer shape (devices and services), using page 1's total."""
import httpx
import pytest

from fsv import config, integrated_assets as assets
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client

ROWS = [{"device_pk": 81, "name": {"id": 81, "label": "Acme laptop"}},
        {"device_pk": 82, "name": {"id": 82, "label": "Globex phone"}}]


def tenant(monkeypatch, page2, total=2):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    calls = []

    def handler(req):
        page = int(req.url.params["page"])
        calls.append((req.url.path, page))
        if page == 1:
            return httpx.Response(200, json={"page": 1, "error": None, "total_rows": total, "data": ROWS})
        status, body = page2
        return httpx.Response(status, json=body)
    return mock_client(monkeypatch, handler), calls


@pytest.mark.parametrize("kind", ["devices", "services"])
@pytest.mark.parametrize("page2", [
    (404, {"detail": "Invalid page."}),
    (400, {"error": "bad page"}),
    (200, {"error": "Requested page is out of range.", "data": []}),
    (200, {"page": 1, "error": None, "total_rows": 2, "data": ROWS}),
    (200, {"page": 2, "error": None, "total_rows": 2, "data": []}),
])
def test_past_the_end_shapes(monkeypatch, kind, page2):
    c, calls = tenant(monkeypatch, page2)
    with pytest.raises(ValueError, match=f"no results on page 2: the {kind} inventory has 2 row"):
        assets.search(c, "acme", kind=kind, page=2)
    assert {p for _, p in calls} == {1, 2}


def test_unreadable_page_inside_the_inventory_stays_unavailable(monkeypatch):
    c, _ = tenant(monkeypatch, (200, {"error": "internal"}), total=120)
    with pytest.raises(ReviewedWriteError, match="unavailable"):
        assets.search(c, "", page=2)


def test_real_second_page_is_returned(monkeypatch):
    c, calls = tenant(monkeypatch, (200, {"page": 2, "error": None, "total_rows": 52, "data": ROWS}), total=52)
    out = assets.search(c, "", page=2)
    assert [r["name"]["id"] for r in out["data"]] == [81, 82] and out["complete"] is False
    assert calls == [("/itam/core/device/api/", 2)]
