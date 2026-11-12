"""Read-only CI/asset picker: exact IDs, HTTP params, real Pilot selection."""
import asyncio
import json

import httpx
import pytest
from textual.widgets import Input, Static
from typer.testing import CliRunner

from fsv import asset_picker, cli, config
from fsv.client import Client
from fsv.resources import CHANGES
from fsv.reviewed_write import ReviewedWriteError
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, OutcomeScreen, ValueScreen
from test_tui_integration import offline  # noqa: F401


@pytest.fixture
def wire(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    state = {"rows": [{"device_pk": 701, "name": {"id": 701, "label": "Globex laptop"}, "type": "Computer"}], "requests": []}
    def handle(req):
        state["requests"].append(req)
        assert req.method == "GET" and req.url.path.endswith("/itam/core/device/api/")
        return httpx.Response(200, json={"page": int(req.url.params["page"]), "error": None, "total_rows": len(state["rows"]), "data": state["rows"]})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    state["client"] = c
    yield state
    c.close()


def test_cli_exact_display_id_and_search_params(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire["client"])
    result = CliRunner().invoke(cli.app, ["changes", "asset-picker", "CHN-123", "--search", "Globex", "--select", "701", "--page", "2", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["selected"]["inventory"]["name"]["id"] == 701 and data["mutation"] == "none"
    assert "not a complete inventory" in data["scope"]
    req = wire["requests"][0]
    assert dict(req.url.params) == {"order": "", "page": "2", "page_size": "50", "q": "Globex"}
    result = asset_picker.candidates(wire["client"], 123, "")
    with pytest.raises(ReviewedWriteError, match="not unique"):
        asset_picker.select(result, 9000007001)
    assert all(req.method == "GET" for req in wire["requests"])


def test_ambiguous_identity_blocks_selection(wire):
    wire["rows"] *= 2
    with pytest.raises(ReviewedWriteError, match="duplicate"):
        asset_picker.candidates(wire["client"], 123, "")


@pytest.mark.parametrize("mode", ["select", "cancel-search", "cancel-picker", "empty"])
def test_pilot_readonly_search_selection_and_cancel(offline, wire, monkeypatch, mode):
    import fsv.tui.app as tui
    monkeypatch.setattr(tui, "get_client", lambda: wire["client"])
    if mode == "empty":
        wire["rows"] = []
    async def go():
        app = FsvApp(read_only=True)
        async with app.run_test(size=(120, 40)) as pilot:
            app._selected = {"id": 9000000123, "display_id": 123, "_resource": CHANGES}
            await pilot.press("i")
            assert isinstance(app.screen, ValueScreen)
            if mode == "cancel-search":
                await pilot.press("escape")
                assert not wire["requests"]
                return
            app.screen.query_one(Input).value = "Globex"
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            if mode == "empty":
                assert isinstance(app.screen, OutcomeScreen)
            else:
                assert isinstance(app.screen, ChoiceScreen)
                copy = " ".join(str(widget.render()) for widget in app.screen.query(Static))
                assert "Asset candidates" in copy and "page 1" in copy and "read-only" in copy
                assert "saved view" not in copy.lower()
                await pilot.press("escape" if mode == "cancel-picker" else "enter")
                if mode == "select":
                    assert isinstance(app.screen, OutcomeScreen)
                    text = " ".join(str(widget.render()) for widget in app.screen.query(Static))
                    assert "no association" in text and "701" in text
                else:
                    assert not isinstance(app.screen, OutcomeScreen)
            assert wire["requests"] and all(req.method == "GET" for req in wire["requests"])
    asyncio.run(go())
