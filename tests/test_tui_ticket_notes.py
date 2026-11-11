"""Ticket-note embedding: exact review, distinct public consent, no generic note wire."""
import asyncio

import pytest
from textual.widgets import TextArea

from fsv import ticket_notes
from fsv.resources import TICKETS
from fsv.tui.app import FsvApp
from fsv.tui.modals import ConfirmScreen, FailureScreen, NoteScreen
from test_tui_integration import offline  # noqa: F401


@pytest.mark.parametrize("public,cancel", [(False, True), (False, False), (True, True), (True, False)])
def test_ticket_note_review_and_visibility(offline, monkeypatch, public, cancel):
    calls = []
    def send(plan, client, **kw):
        calls.append((plan, kw))
        return {"note_id": 9000001010, "verified": True}
    monkeypatch.setattr(ticket_notes, "send", send)
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._selected = {"id": 9000001001, "display_id": 21, "_resource": TICKETS}
            await pilot.press("n")
            assert isinstance(app.screen, NoteScreen)
            app.screen.query_one(TextArea).text = "Acme <evidence>"
            if public:
                await pilot.press("ctrl+t")
            await pilot.press("ctrl+s")
            assert isinstance(app.screen, ConfirmScreen)
            review = "\n".join(app.screen._lines)
            assert "&lt;evidence&gt;" in review and '"notify_emails": []' in review
            assert calls == []
            if cancel and not public:
                await pilot.press("escape")
            else:
                await pilot.press("y")
                if public:
                    assert isinstance(app.screen, ConfirmScreen)
                    assert app.screen._title == "Confirm PUBLIC visibility"
                    assert calls == []
                    await pilot.press("escape" if cancel else "y")
            await app.workers.wait_for_complete()
            await pilot.pause()
    asyncio.run(go())
    assert len(calls) == (0 if cancel else 1)
    if calls:
        plan, kw = calls[0]
        assert plan.ticket_id == 21 and plan.public == public
        assert kw == {"reviewed_hash": plan.hash, "public_confirmed": public, "read_only": False}
    assert TICKETS.has_notes is False


@pytest.mark.parametrize("unknown", [False, True])
def test_note_failure_preserves_outcome(offline, monkeypatch, unknown):
    calls = []
    def send(*args, **kw):
        calls.append(args)
        if unknown:
            raise ticket_notes.NoteError("outcome unknown")
        return {"note_id": 9000001020, "verified": False, "warning": "Readback denied; do not resend"}
    monkeypatch.setattr(ticket_notes, "send", send)
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._review_ticket_note({"id": 21}, "Acme evidence", False)
            await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, FailureScreen)
            assert "unknown" in app.screen.message if unknown else "9000001020" in app.screen.message
    asyncio.run(go())
    assert len(calls) == 1
