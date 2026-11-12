"""TUI write actions: modals + mocked service layer (no network)."""
from __future__ import annotations

import asyncio

import pytest
from textual.widgets import Button, DataTable, OptionList, TextArea

from fsv import service
from fsv.errors import SessionError
from fsv.resources import CHANGES, TICKETS
from fsv.tui import actions
from fsv.tui.app import FsvApp
from fsv.tui.modals import ConfirmScreen, NoteScreen, StatusScreen

STATUS_SCHEMA = {"fields": [{"name": "status", "choices": [
    {"id": 1, "value": "Open"}, {"id": 2, "value": "Review"}, {"id": 3, "value": "Closed"}]}]}


def _item(res, status=1):
    return {"id": 42, "status": status, "subject": "Demo", "_resource": res,
            "display_id": 42}


@pytest.fixture
def calls(monkeypatch):
    log: list[tuple] = []
    monkeypatch.setattr(service, "update_item", lambda r, i, b, client=None: log.append(("update", r.name, i, b)) or {})
    monkeypatch.setattr(service, "add_note", lambda r, i, b, public=False, client=None: log.append(("note", r.name, i, b, public)) or {})
    monkeypatch.setattr(service, "add_reply", lambda r, i, b, client=None: log.append(("reply", r.name, i, b)) or {})
    monkeypatch.setattr(FsvApp, "_clear_detail", lambda self, message="": None)
    monkeypatch.setattr(FsvApp, "_load_list", lambda self, append=False: None)
    monkeypatch.setattr(FsvApp, "_load_detail", lambda self, item, key: None)
    monkeypatch.setattr(actions.config, "load_config", lambda: {})
    from types import SimpleNamespace
    from fsv import ticket_notes
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: object())
    def session(client, **kwargs):
        values = {}
        plan = SimpleNamespace(ok=True, errors=[], hash="synthetic-review", to_json=lambda: {
            "plan_hash": "synthetic-review", "payload": values, "warning": "change status -> Review"})
        def send(plan):
            service.update_item(CHANGES, 42, values)
            return {"verified": True}
        return SimpleNamespace(set=lambda k, v: values.update({k: v}) or SimpleNamespace(error=""),
                               build_plan=lambda: plan, send=send)
    monkeypatch.setattr("fsv.formui.change_session", session)
    monkeypatch.setattr(ticket_notes, "send", lambda plan, client, **kw:
                        log.append(("note", "tickets", plan.ticket_id,
                                    "hello" if "hello" in plan.body_html else "hi", plan.public)) or
                        {"note_id": 901, "verified": True})
    return log


def _app(res, read_only=False):
    app = FsvApp(read_only=read_only)
    app._schemas[res.name] = STATUS_SCHEMA
    app._selected = _item(res)
    return app


def run(coro):
    return asyncio.run(coro)


def test_status_cancel_makes_no_call(calls):
    async def go():
        app = _app(CHANGES)
        async with app.run_test() as pilot:
            await pilot.press("s")
            assert isinstance(app.screen, StatusScreen)
            await pilot.press("escape")
            assert not isinstance(app.screen, StatusScreen)
            await pilot.press("s", "down", "enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            await pilot.click("#cancel")
            await pilot.pause()
            assert not isinstance(app.screen, ConfirmScreen)
    run(go())
    assert calls == []


def test_status_confirm_one_call_with_safety_text(calls):
    async def go():
        app = _app(CHANGES)
        async with app.run_test() as pilot:
            await pilot.press("s", "down", "enter")  # highlighted starts at current (Open) -> Review
            await app.workers.wait_for_complete()
            await pilot.pause()
            body = str(app.screen.query_one("#confirm-body").render())
            assert "change status -> Review" in body
            await pilot.click("#confirm")
            await app.workers.wait_for_complete()
            await pilot.pause()
    run(go())
    assert calls == [("update", "changes", 42, {"status": 2})]


def test_ticket_every_status_transition_warns(calls):
    plan = actions.status_plan(TICKETS, _item(TICKETS), STATUS_SCHEMA, 2, "Review")
    assert plan.risks and "Review" in plan.risks[0]
    closed = actions.status_plan(TICKETS, _item(TICKETS), STATUS_SCHEMA, 3, "Closed")
    assert closed.risks and "Closed" in closed.risks[0]


def test_private_note_requires_confirm(calls):
    async def go():
        app = _app(TICKETS)
        async with app.run_test() as pilot:
            await pilot.press("n")
            assert isinstance(app.screen, NoteScreen)
            app.screen.query_one(TextArea).text = "hello"
            await pilot.pause()
            await pilot.click("#submit")
            assert isinstance(app.screen, ConfirmScreen)
            assert calls == []
            await pilot.click("#confirm")
            await app.workers.wait_for_complete()
            await pilot.pause()
    run(go())
    assert calls == [("note", "tickets", 42, "hello", False)]


def test_public_note_requires_confirm_and_cancel_sends_nothing(calls):
    async def go():
        app = _app(TICKETS)
        async with app.run_test() as pilot:
            await pilot.press("n")
            app.screen.query_one(TextArea).text = "hi"
            await pilot.press("ctrl+t")
            await pilot.press("ctrl+s")
            assert isinstance(app.screen, ConfirmScreen)
            assert "ticket note" in app.screen._title
            await pilot.press("escape")
            await pilot.pause()
    run(go())
    assert calls == []


def test_public_note_confirmed(calls):
    async def go():
        app = _app(TICKETS)
        async with app.run_test() as pilot:
            await pilot.press("n")
            app.screen.query_one(TextArea).text = "hi"
            await pilot.press("ctrl+t", "ctrl+s")
            await pilot.click("#confirm")
            await pilot.click("#confirm")
            await app.workers.wait_for_complete()
    run(go())
    assert calls == [("note", "tickets", 42, "hi", True)]


def test_ticket_reply_from_detail_pane(calls, monkeypatch):
    from fsv import ticket_conversation
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: object())
    monkeypatch.setattr(ticket_conversation, "send", lambda plan, client, **kw:
                        calls.append(("reviewed_reply", plan.ticket_id, plan.payload["body"])) or
                        {"conversation_id": 901, "verified": True})
    async def go():
        app = _app(TICKETS)
        async with app.run_test() as pilot:
            monkeypatch.setattr(app, "_active_pane", lambda: "detail")
            await pilot.press("r")
            assert isinstance(app.screen, NoteScreen)
            app.screen.query_one(TextArea).text = "thanks"
            await pilot.press("ctrl+s")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert "ticket conversation" in app.screen._title
            await pilot.click("#confirm")
            assert "PUBLIC" in app.screen._title and not calls
            await pilot.click("#confirm")
            await app.workers.wait_for_complete()
    run(go())
    assert calls == [("reviewed_reply", 42, "<div>thanks</div>")]


def test_empty_note_not_submitted(calls):
    async def go():
        app = _app(TICKETS)
        async with app.run_test() as pilot:
            await pilot.press("n", "ctrl+s")
            assert isinstance(app.screen, NoteScreen)
    run(go())
    assert calls == []


def test_read_only_blocks_write_keys(calls, monkeypatch):
    async def go():
        app = _app(CHANGES, read_only=True)
        async with app.run_test() as pilot:
            monkeypatch.setattr(app, "_active_pane", lambda: "detail")
            await pilot.press("s", "n", "r")
            assert not isinstance(app.screen, (StatusScreen, NoteScreen, ConfirmScreen))
    run(go())
    assert calls == []


def test_read_only_from_config(monkeypatch):
    monkeypatch.setattr(actions.config, "load_config", lambda: {"tui": {"read_only": True}})
    assert actions.resolve_read_only(False) is True
    monkeypatch.setattr(actions.config, "load_config", lambda: {})
    assert actions.resolve_read_only(False) is False
    assert actions.resolve_read_only(True) is True


def test_error_becomes_persistent_not_crash(calls, monkeypatch):
    def boom(*a, **k):
        raise SessionError("Session expired. Run: fsv login")
    monkeypatch.setattr(service, "update_item", boom)
    toasts = []

    async def go():
        app = _app(CHANGES)
        orig = app.notify
        app.notify = lambda m, **k: toasts.append((m, k.get("severity"))) or orig(m, **k)
        async with app.run_test() as pilot:
            await pilot.press("s", "down", "enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            await pilot.click("#confirm")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert app.is_running
            assert "Session expired. Run: fsv login" in app._write_failure
    run(go())


def test_status_choices_from_schema():
    assert actions.status_choices(STATUS_SCHEMA) == [(1, "Open"), (2, "Review"), (3, "Closed")]
    assert actions.status_choices({"fields": []}) == []


def test_typing_in_note_does_not_trigger_app_keys(calls):
    async def go():
        app = _app(TICKETS)
        async with app.run_test() as pilot:
            await pilot.press("n", "q", "o", "y")
            assert app.is_running and isinstance(app.screen, NoteScreen)
            assert app.screen.query_one(TextArea).text == "qoy"
    run(go())
