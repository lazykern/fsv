"""Pilot drives ticket options, exact review, public consent and no resend."""
import asyncio

import pytest
from textual.widgets import Input, TextArea

from fsv import ticket_conversation as conv
from fsv.resources import TICKETS
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, NoteScreen, OutcomeScreen, ValueScreen
from test_tui_integration import offline  # noqa: F401


@pytest.mark.parametrize("reply,cancel", [(True, True), (True, False), (False, True), (False, False)])
def test_ticket_options_recipients_review_and_public_consent(offline, monkeypatch, reply, cancel):
    calls = []
    monkeypatch.setattr(conv, "recipients", lambda c, emails: tuple(emails))
    monkeypatch.setattr(conv, "send", lambda p, c, **kw: calls.append((p, kw)) or {"conversation_id": 456, "verified": False})
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._selected = {"id": 9000001001, "display_id": 123, "_resource": TICKETS}
            if reply:
                app._focus_pane("detail")
            await pilot.press("r" if reply else "n")
            assert isinstance(app.screen, NoteScreen)
            app.screen.query_one(TextArea).text = "Acme update"
            await pilot.press("ctrl+o")
            assert isinstance(app.screen, ChoiceScreen)
            app.screen.query_one(Input).value = "CC emails" if reply else "Notify-to emails"
            await pilot.pause()
            await pilot.press("enter")
            assert isinstance(app.screen, ValueScreen)
            app.screen.query_one(Input).value = "person@example.com"
            await pilot.press("enter")
            assert isinstance(app.screen, ChoiceScreen)
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            review = "\n".join(app.screen._lines)
            assert "person@example.com" in review and "plan_hash" in review
            assert '"cc_emails"' in review if reply else '"notify_emails"' in review
            assert not calls
            await pilot.press("escape" if cancel else "y")
            if not cancel and reply:
                assert isinstance(app.screen, ConfirmScreen)
                assert "PUBLIC" in app.screen._title and not calls
                await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            if not cancel:
                assert isinstance(app.screen, OutcomeScreen)
                assert "456" in app.screen.message
                await pilot.press("escape")
    asyncio.run(go())
    assert len(calls) == (0 if cancel else 1)
    if calls:
        p, kw = calls[0]
        assert p.ticket_id == 123 and kw["reviewed_hash"] == p.hash
        assert kw["public_confirmed"] is reply


def test_read_only_blocks_ticket_options_sender(offline, monkeypatch):
    monkeypatch.setattr(conv, "send", lambda *a, **kw: pytest.fail("read-only send"))
    async def go():
        app = FsvApp(read_only=True)
        async with app.run_test() as pilot:
            app._ticket_conversation_options({"id": 123}, "Acme", True, True)
            app._prepare_ticket_conversation({"id": 123}, "Acme", True, True, {})
            await app.workers.wait_for_complete()
            assert not isinstance(app.screen, (ChoiceScreen, ConfirmScreen))
    asyncio.run(go())
