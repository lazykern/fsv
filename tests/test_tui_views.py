"""TUI list and detail panes mirror the web (pilot tests over the invented
tenants; mocked httpx, no network)."""
from __future__ import annotations

import asyncio
import datetime as dt
import sys
import types

import pytest
from rich.text import Text
from textual.screen import ModalScreen
from textual.widgets import DataTable

import acme_views as W
from fsv import views as V
from fsv.tui import app as tui_app
from fsv.tui.app import FsvApp
from fsv.tui.modals import ViewPickerScreen

FIXED_NOW = dt.datetime(2030, 6, 15, 12, tzinfo=dt.timezone.utc)


def _world(monkeypatch, tenant: str) -> W.World:
    w = W.World(tenant)
    c = w.client()
    monkeypatch.setattr(tui_app, "get_client", lambda: c)
    monkeypatch.setattr(V, "_now", lambda: FIXED_NOW)
    monkeypatch.setattr(tui_app.config, "get_default_where", lambda entity: [])
    if tenant == "globex":
        monkeypatch.setenv("FSV_TZ", "Europe/Berlin")
    else:
        monkeypatch.delenv("FSV_TZ", raising=False)
    return w


def text_of(widget) -> str:
    content = widget.content
    if isinstance(content, Text):
        return content.plain
    return str(content)


async def settle(app, pilot, rounds: int = 6) -> None:
    for _ in range(rounds):
        await pilot.pause(0.25)
        await app.workers.wait_for_complete()


def columns(app) -> list[str]:
    table = app.query_one("#list", DataTable)
    return [str(col.label) for col in table.columns.values()]


def run(coro):
    return asyncio.run(coro)


def test_ticket_list_and_service_request_detail(monkeypatch):
    w = _world(monkeypatch, "acme")

    async def go():
        app = FsvApp()
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            assert columns(app) == ["", "Subject", "Requester", "State", "Status", "Priority", "Assigned to"]
            header = text_of(app.query_one("#header"))
            assert "New & My Open Tickets · 1-2 of 2 · Sort by: Created Date" in header
            await pilot.press("j")  # SR-602
            await settle(app, pilot)
            assert "#SR-602 Access to analytics  Closed (late)" in text_of(app.query_one("#detail-bar"))
            assert "via Phone" in text_of(app.query_one("#reporter-line"))
            tabs = text_of(app.query_one("#detail-tabs"))
            assert tabs.split() == ["1:Details", "2:Related", "tickets", "3:Tasks", "4:Assets", "5:Associations",
                                    "6:Approvals", "(1)", "7:Activity", "8:Resolution"]
            main = text_of(app.query_one("#detail-content"))
            assert "Requested items" in main and "Request type  Add permission" in main
            assert "Conversations (5)" in main
            side = text_of(app.query_one("#detail-props"))
            assert "Properties" in side and "Affected App" in side and app.query_one("#detail-side").display
    run(go())
    assert not w.writes()


def test_change_detail_tabs_follow_the_server(monkeypatch):
    w = _world(monkeypatch, "acme")

    async def go():
        app = FsvApp()
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            await pilot.press("g", "c")
            await settle(app, pilot, 8)
            assert columns(app) == ["", "Subject", "Requester", "Status", "Priority", "Assigned to", "Department"]
            assert "New & My Open Changes" in text_of(app.query_one("#header"))
            table = app.query_one("#list", DataTable)
            assert "Platform Ops / Robin Agent (Me)" in [str(c) for c in table.get_row_at(0)]
            await settle(app, pilot)
            assert "Planning" in text_of(app.query_one("#state-flow-bar"))
            assert "Jane Doe reported 2 days ago" in text_of(app.query_one("#reporter-line"))
            tabs = text_of(app.query_one("#detail-tabs"))
            assert [t for t in tabs.split() if ":" in t][:4] == ["1:Details", "2:Devops", "3:Risk", "4:Notes"]
            assert "Go-Live Checklist" in text_of(app.query_one("#detail-content"))
            assert "Downtime Window*" in text_of(app.query_one("#detail-props"))
            await pilot.press("tab", "4")  # Notes
            await settle(app, pilot)
            assert "CAB slot booked" in text_of(app.query_one("#detail-content"))
            assert not app.query_one("#detail-side").display
            await pilot.press("l")  # Tasks
            await settle(app, pilot)
            assert "Take snapshot" in text_of(app.query_one("#detail-content"))
    run(go())


def test_view_picker_switches_the_list_by_name(monkeypatch):
    w = _world(monkeypatch, "acme")

    async def go():
        app = FsvApp()
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            await pilot.press("g", "c")
            await settle(app, pilot, 8)
            await pilot.press("v")
            assert isinstance(app.screen, ViewPickerScreen)
            await pilot.press(*"team", "enter")
            await settle(app, pilot, 8)
            assert "My Team's Changes · 1-2 of 2 · Sort by: Last Modified" in text_of(app.query_one("#header"))
    run(go())
    q = w.calls("/api/_/changes")[-1]
    assert (q["filter"], q["order_by"], q["order_type"]) == ("9001", "updated_at", "asc")


def test_new_and_edit_hooks_without_form_ui(monkeypatch):
    _world(monkeypatch, "acme")
    monkeypatch.setitem(sys.modules, "fsv.formui", None)  # import fails
    import fsv
    monkeypatch.delattr(fsv, "formui", raising=False)

    async def go():
        app = FsvApp()
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            await pilot.press("N")
            await pilot.pause()
            assert any("form UI not available" in str(n.message) for n in app._notifications)
    run(go())


def test_edit_hook_pushes_the_form_screen(monkeypatch):
    _world(monkeypatch, "acme")
    seen: list = []

    class FakeScreen(ModalScreen):
        def __init__(self, session):
            super().__init__()
            seen.append(session)

    fake = types.ModuleType("fsv.formui")
    fake.FormScreen = FakeScreen
    fake.change_session = lambda client, mode="new", target=None, record=None: {
        "mode": mode, "target": target, "subject": (record or {}).get("subject")}
    monkeypatch.setitem(sys.modules, "fsv.formui", fake)
    import fsv
    monkeypatch.setattr(fsv, "formui", fake, raising=False)

    async def go():
        app = FsvApp()
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            await pilot.press("e")  # a ticket is selected: edit is for changes
            await pilot.pause()
            assert any("Select a change" in str(n.message) for n in app._notifications)
            await pilot.press("g", "c")
            await settle(app, pilot, 8)
            await pilot.press("e")
            await settle(app, pilot, 2)
            assert isinstance(app.screen, FakeScreen), [str(n.message) for n in app._notifications]
            app.pop_screen()
            await pilot.press("N")
            await settle(app, pilot, 2)
            assert isinstance(app.screen, FakeScreen)
    run(go())
    assert seen == [{"mode": "edit", "target": 501, "subject": "Billing: rotate keys"},
                    {"mode": "new", "target": None, "subject": None}]


def test_read_only_blocks_form_hooks(monkeypatch):
    _world(monkeypatch, "acme")

    async def go():
        app = FsvApp(read_only=True)
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            await pilot.press("N")
            await pilot.pause()
            assert any("Read-only" in str(n.message) for n in app._notifications)
    run(go())


@pytest.mark.parametrize("entity_keys", [(), ("g", "c")])
def test_globex_renders_without_optional_features(monkeypatch, entity_keys):
    w = _world(monkeypatch, "globex")

    async def go():
        app = FsvApp()
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            if entity_keys:
                await pilot.press(*entity_keys)
                await settle(app, pilot, 8)
            cols = columns(app)
            assert cols[1:3] == ["Betreff", "Anforderer"]
            header = text_of(app.query_one("#header"))
            assert ("All changes" if entity_keys else "All tickets") in header
            await settle(app, pilot)
            bar = text_of(app.query_one("#detail-bar"))
            assert bar.startswith("#CHN-77" if entity_keys else "#SR-88")
            assert not app.query_one("#state-flow-bar").display  # no lifecycle
            tabs = text_of(app.query_one("#detail-tabs"))
            assert "1:Details" in tabs
            content = text_of(app.query_one("#detail-content"))
            assert "tabs: not permitted for your role (403)" in content
            await pilot.press("v")
            await pilot.pause()
            assert not isinstance(app.screen, ViewPickerScreen)  # no views on this account
    run(go())
    assert not any("/ws/" in p for p in w.paths())
