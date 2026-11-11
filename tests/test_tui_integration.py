"""Main application integration, offline synthetic Acme targets only."""
from __future__ import annotations

import asyncio
from types import SimpleNamespace

import pytest
from textual.screen import ModalScreen
from textual.widgets import Input, OptionList

from fsv import catalog, formui
from fsv.formui import session as form_sessions
from fsv.resources import CHANGES, TICKETS
from fsv.tui import actions
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, FailureScreen


@pytest.fixture
def offline(monkeypatch):
    monkeypatch.setattr(FsvApp, "_load_list", lambda *a, **k: None)
    monkeypatch.setattr(FsvApp, "_load_detail", lambda *a, **k: None)
    monkeypatch.setattr(FsvApp, "_get_default_filters", lambda *a: [])
    monkeypatch.setattr(actions.config, "load_config", lambda: {})
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: object())


class CancelForm(ModalScreen):
    BINDINGS = [("escape", "cancel")]

    def __init__(self, session):
        super().__init__()
        self.session = session

    def action_cancel(self):
        self.dismiss(SimpleNamespace(action="cancel"))


@pytest.mark.parametrize("kind,keys", [("change", ("enter",)), ("incident", ("down", "enter")),
                                      ("sr", ("down", "down", "enter"))])
def test_create_dispatch_cancel(offline, monkeypatch, kind, keys):
    seen = []
    def session(name, **kw):
        seen.append((name, kw))
        return SimpleNamespace(send=lambda *_: pytest.fail("cancel must not send"))
    monkeypatch.setattr(formui, "FormScreen", CancelForm)
    monkeypatch.setattr(formui, "change_session", lambda c: session("change"))
    monkeypatch.setattr(form_sessions, "ticket_session", lambda c: session("incident"))
    monkeypatch.setattr(form_sessions, "sr_session", lambda c, **kw: session("sr", **kw))
    monkeypatch.setattr(catalog, "list_items", lambda c: [{"id": 9000001001, "display_id": 77, "name": "Acme Access", "category_id": 9}])
    monkeypatch.setattr(catalog, "list_categories", lambda c: [{"id": 9, "name": "Access"}])
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            await pilot.press("N")
            assert isinstance(app.screen, ChoiceScreen)
            assert app.screen.query_one(Input).placeholder == "type to filter choices"
            options = app.screen.query_one(OptionList)
            assert all("custom" not in str(options.get_option_at_index(i).prompt)
                       for i in range(options.option_count))
            await pilot.press(*keys)
            await app.workers.wait_for_complete()
            await pilot.pause()
            if kind == "sr":
                await pilot.press("enter")  # all categories
                await pilot.press(*"Acme", "enter")
                await app.workers.wait_for_complete()
                await pilot.pause()
            assert isinstance(app.screen, CancelForm)
            await pilot.press("escape")
            assert not isinstance(app.screen, ModalScreen)
    asyncio.run(go())
    assert seen == [(kind, {"item_ref": 77} if kind == "sr" else {})]


@pytest.mark.parametrize("per_page", [1, 100])
def test_real_list_loader_preserves_short_and_terminal_metadata(offline, monkeypatch, per_page):
    from fsv import service
    calls = []
    def listing(res, **kw):
        page = kw["page"]
        calls.append(page)
        return [{"id": page, "display_id": page, "_resource": TICKETS}], {"meta": {"has_next": page == 1}}
    monkeypatch.setattr(service, "list_page", listing)
    async def go():
        app = FsvApp()
        app._schemas = {name: {"fields": []} for name in ("tickets", "changes", "problems")}
        app._views = {"tickets": []}
        app._headers = {"tickets": []}
        app._me = {}
        app._per_page = per_page
        async with app.run_test() as pilot:
            await asyncio.to_thread(app._fetch_list, False, 0, "tickets", 1)
            assert app._has_more
            await asyncio.to_thread(app._fetch_list, True, 0, "tickets", 2)
            assert not app._has_more and app._page == 2
    asyncio.run(go())
    assert calls == [1, 2]


def test_read_only_and_chooser_cancel(offline, monkeypatch):
    monkeypatch.setattr(form_sessions, "ticket_session", lambda c: pytest.fail("no session"))
    async def go():
        for readonly in (False, True):
            app = FsvApp(read_only=readonly)
            async with app.run_test() as pilot:
                await pilot.press("N")
                if readonly:
                    assert not isinstance(app.screen, ChoiceScreen)
                else:
                    await pilot.press("escape")
                assert not isinstance(app.screen, ModalScreen)
    asyncio.run(go())


def test_persistent_partial_failure_and_single_send(offline):
    from fsv.change_flow import PartialFailure
    sends = []
    def send(plan):
        sends.append(plan)
        raise PartialFailure(9000001002, [], [{"step": "planning", "ok": False}], RuntimeError("x" * 700))
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._send_form(SimpleNamespace(send=send), SimpleNamespace(plan="reviewed"), "new", None)
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, FailureScreen)
            assert "9000001002" in app.screen.message and "x" * 700 in app.screen.message
            await pilot.press("escape")
            assert "planning" in app._write_failure
    asyncio.run(go())
    assert sends == ["reviewed"]


@pytest.mark.parametrize("mode,out,expected", [
    ("new", {"id": 9000001002, "result": {"change": {"display_id": 23}}}, ("Change", "CHN-23")),
    ("incident", {"id": 9000001003, "result": {"human_display_id": "INC-24"}}, ("Incident", "INC-24")),
    ("sr", {"created": {"id": 9000001004, "display_id": 25}}, ("Service Request", "SR-25")),
])
def test_result_identity(mode, out, expected):
    assert actions.form_result(mode, out) == expected


def test_list_authority_retry_dedup_and_stale(offline):
    row = {"id": 9000001001, "display_id": 1, "_resource": TICKETS}
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._accept_list(0, "tickets", 1, [row], None, True, False)
            assert app._has_more and "of 1+" in app._view_header()
            app._load_more()
            assert app._page == 1
            app._list_failed(0, "tickets", "temporary error")
            assert not app._loading_more and app._page == 1
            app._accept_list(0, "tickets", 2, [row, {**row, "id": 9000001002, "display_id": 2}], 2, False, True)
            assert len(app._items) == 2 and not app._has_more and app._page == 2
            app._list_epoch += 1
            app._accept_list(0, "tickets", 3, [{**row, "id": 9000001003}], 3, True, True)
            assert len(app._items) == 2 and app._page == 2
            app.entity = "changes"
            app._accept_list(1, "tickets", 3, [row], 3, True, True)
            assert app._page == 2
    asyncio.run(go())
