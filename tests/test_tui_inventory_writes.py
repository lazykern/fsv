"""Real Pilot and HTTP transport for reviewed device/service association UX."""
import asyncio
import copy
import json

import httpx
import pytest
from textual.widgets import Input

from fsv import config, journal
from fsv.client import Client
from fsv.resources import CHANGES, PROBLEMS, TICKETS
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, FailureScreen, OutcomeScreen
from test_tui_integration import offline  # noqa: F401


@pytest.mark.parametrize("step", ["menu", "search", "picker", "read-only"])
def test_inventory_cancel_and_readonly_never_prepare(offline, monkeypatch, step):
    from fsv import integrated_assets
    monkeypatch.setattr(integrated_assets, "build_plan", lambda *a, **k: pytest.fail("unexpected plan"))
    monkeypatch.setattr(integrated_assets, "send", lambda *a, **k: pytest.fail("unexpected send"))
    monkeypatch.setattr(integrated_assets, "search", lambda *a, **k: {"data": [
        {"name": {"id": 81, "label": "Acme item"}, "type": "Computer"}]})
    async def go():
        app = FsvApp(read_only=step == "read-only")
        async with app.run_test() as pilot:
            app._selected = {"id": 123, "display_id": 123, "_resource": TICKETS}
            await pilot.press("i")
            if step == "read-only":
                app._prepare_inventory_association(app._selected, "devices", 81, False)
                app._send_inventory_association(None, "tickets", "devices", 81, False)
                await app.workers.wait_for_complete()
                await pilot.press("escape")
                return
            if step == "menu":
                await pilot.press("escape")
                return
            app.screen.query_one(Input).value = "Add device"
            await pilot.pause()
            await pilot.press("enter")
            if step == "search":
                await pilot.press("escape")
                return
            app.screen.query_one(Input).value = "Acme"
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ChoiceScreen)
            await pilot.press("escape")
    asyncio.run(go())


@pytest.mark.parametrize("module,prefix,resource", [("tickets", "INC", TICKETS), ("tickets", "SR", TICKETS),
                                                    ("problems", "PRB", PROBLEMS), ("changes", "CHN", CHANGES)])
@pytest.mark.parametrize("kind", ["devices", "services"])
@pytest.mark.parametrize("remove", [False, True])
@pytest.mark.parametrize("mode", ["cancel", "confirm", "ambiguous"])
def test_pilot_reviewed_inventory_single_send(offline, monkeypatch, tmp_path, module, prefix, resource, kind, remove, mode):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    category = "Device" if kind == "devices" else "Business service"
    row = {"item_id": 81, "itam_cmdb_request_id": 901, "category": category, "name": "Acme item"}
    state = {"rows": [row] if remove else [], "writes": [], "reads": []}
    def handle(req):
        if req.method == "GET":
            state["reads"].append(req)
            if "/itam/" in req.url.path:
                assert req.url.path == f"/itam/core/{'device' if kind == 'devices' else 'businessservice'}/api/"
                return httpx.Response(200, json={"page": 1, "total_rows": 1, "data": [
                    {"name": {"id": 81, "label": "Acme item"}, "type": category}]})
            assert req.url.path == f"/api/_/{module}/123/associated-cis"
            assert req.url.params["association_type"] == ("cis" if kind == "devices" else "services")
            return httpx.Response(200, json={"data": copy.deepcopy(state["rows"]),
                                            "meta": {"total_count": len(state["rows"]), "total_pages": 1}})
        state["writes"].append(req)
        assert req.method == "PUT"
        assert req.url.path == f"/api/_/{module}/123/{'disassociate-ci' if remove else 'associate-ci'}"
        assert json.loads(req.content) == ({"itam_cmdb_request_id": 901} if remove else
                                          {"device_ids" if kind == "devices" else "businessapp_ids": [81]})
        if mode == "ambiguous":
            raise httpx.ReadError("synthetic disconnect", request=req)
        state["rows"] = [] if remove else [row]
        return httpx.Response(200, json={})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    c._csrf = "synthetic"
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: c)
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._selected = {"id": 9000000123, "display_id": 123, "human_display_id": f"{prefix}-123", "_resource": resource}
            await pilot.press("i")
            assert isinstance(app.screen, ChoiceScreen)
            app.screen.query_one(Input).value = ("Remove associated" if remove else "Add") + (" business" if kind == "services" else " device")
            await pilot.pause()
            await pilot.press("enter")
            if not remove:
                app.screen.query_one(Input).value = "Acme"
                await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ChoiceScreen)
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            assert "plan_hash" in app.screen._lines[0]
            assert ("901" if remove else "81") in app.screen._lines[0]
            assert not state["writes"]
            await pilot.press("escape" if mode == "cancel" else "y", "y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            if mode != "cancel":
                assert isinstance(app.screen, FailureScreen if mode == "ambiguous" else OutcomeScreen)
            assert len(state["writes"]) == (0 if mode == "cancel" else 1)
            if mode == "ambiguous":
                await pilot.press("escape")
                app._prepare_inventory_association(app._selected, kind, 81, remove)
                await app.workers.wait_for_complete()
                await pilot.pause()
                await pilot.press("y")
                await app.workers.wait_for_complete()
                await pilot.pause()
                assert isinstance(app.screen, FailureScreen)
                assert len(state["writes"]) == 1
    try:
        asyncio.run(go())
    finally:
        c.close()
