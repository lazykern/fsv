"""Main-TUI task CREATE uses the reviewed sender, never the update action."""
import asyncio

import pytest
from textual.widgets import Input

from fsv import change_tasks, journal
from fsv.resources import CHANGES, TICKETS
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, FailureScreen, ValueScreen
from test_tui_integration import offline  # noqa: F401


@pytest.mark.parametrize("owner_mode,cancel", [("none", False), ("none", True), ("explicit", False), ("explicit", True)])
def test_main_tui_task_review(offline, monkeypatch, owner_mode, cancel):
    calls, refresh = [], []
    def send(plan, **kw):
        calls.append((plan, kw))
        return {"task_id": 9000002010, "verified": True}
    monkeypatch.setattr(change_tasks, "send", send)
    monkeypatch.setattr(FsvApp, "_after_write", lambda *a: refresh.append(a))
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            item = {"id": 9000001001, "display_id": 123, "_resource": CHANGES}
            app._selected = item
            await pilot.press("a")
            assert isinstance(app.screen, ChoiceScreen)
            await pilot.press("enter")  # Add task, no task inventory/target picker needed
            assert isinstance(app.screen, ValueScreen)
            app.screen.query_one(Input).value = "Acme task review"
            await pilot.press("enter")
            assert isinstance(app.screen, ChoiceScreen)
            if owner_mode == "explicit":
                await pilot.press("down", "enter")
                assert isinstance(app.screen, ValueScreen)
                app.screen.query_one(Input).value = "9000002001"
                await pilot.press("enter")
            else:
                await pilot.press("enter")
            assert isinstance(app.screen, ChoiceScreen)
            await pilot.press("enter")  # Review with optional fields omitted
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            review = "\n".join(app.screen._lines)
            assert '"title": "Acme task review"' in review
            assert '"change_id": 123' in review and '"notify_before": 0' in review
            assert "plan_hash" in review and "workflows" in review
            assert calls == []
            await pilot.press("escape" if cancel else "y")
            await app.workers.wait_for_complete()
            await pilot.pause()
    asyncio.run(go())
    assert len(calls) == (0 if cancel else 1)
    assert len(refresh) == (0 if cancel else 1)
    if calls:
        plan, kw = calls[0]
        assert plan.change_id == 123
        assert plan.owner_id == (9000002001 if owner_mode == "explicit" else None)
        assert kw == {"reviewed_hash": plan.hash, "read_only": False}


@pytest.mark.parametrize("failure", ["unknown", "unverified"])
def test_task_failure_no_second_send(offline, monkeypatch, failure):
    calls = []
    def send(plan, **kw):
        calls.append(plan)
        if failure == "unknown":
            raise change_tasks.TaskError("task outcome unknown; do not resubmit")
        return {"task_id": 9000002010, "verified": False, "warning": "Do not resend"}
    monkeypatch.setattr(change_tasks, "send", send)
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._review_change_task({"id": 123, "_resource": CHANGES}, change_tasks.build_plan(123, "Acme task"))
            await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, FailureScreen)
            assert "unknown" in app.screen.message if failure == "unknown" else "9000002010" in app.screen.message
            await pilot.press("escape")
            await app.workers.wait_for_complete()
    asyncio.run(go())
    assert len(calls) == 1


def test_read_only_blocks_menu_composition_review_and_sender(offline, monkeypatch):
    monkeypatch.setattr(change_tasks, "get_client", lambda: pytest.fail("read-only constructed client"))
    monkeypatch.setattr(change_tasks, "send", lambda *a, **k: pytest.fail("read-only sent task"))
    async def go():
        app = FsvApp(read_only=True)
        async with app.run_test() as pilot:
            item = {"id": 123, "_resource": CHANGES}
            plan = change_tasks.build_plan(123, "Acme task")
            app._selected = item
            await pilot.press("a")
            app._prepare_action("task-add", item)
            app._compose_change_task(item)
            app._change_task_options(item, "Acme task", None, {"note": "Draft"})
            app._review_change_task(item, plan)
            app._prepare_change_task(item, "Acme task", "agent@example.com")
            app._send_change_task(item, plan)
            await app.workers.wait_for_complete()
            assert not isinstance(app.screen, (ChoiceScreen, ValueScreen, ConfirmScreen))
    asyncio.run(go())
    assert journal.read() == []


def test_tui_all_optional_fields_review_and_hash(offline, monkeypatch):
    monkeypatch.setattr(change_tasks, "get_client", lambda: pytest.fail("numeric draft should be local"))
    monkeypatch.setattr(change_tasks, "send", lambda *a, **k: pytest.fail("review/cancel sent task"))
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            item = {"id": 123, "_resource": CHANGES}
            app._change_task_options(item, "Globex task", None)
            await pilot.pause()

            async def choose(label):
                assert isinstance(app.screen, ChoiceScreen)
                app.screen.query_one(Input).value = label
                await pilot.pause()
                await pilot.press("enter")

            for label, value in [("Group ID", "9000004001"), ("Note (plain text)", "<literal> & check"),
                                 ("Planned start", "2026-12-05T00:00Z"),
                                 ("Planned end", "2026-12-06T00:00Z"), ("Planned effort", "01:30")]:
                await choose(label)
                assert isinstance(app.screen, ValueScreen)
                app.screen.query_one(Input).value = value
                await pilot.press("enter")
            await choose("Status")
            await choose("In Progress")
            await choose("Notify Before")
            await choose("45 Minutes")
            await choose("Review exact task")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            import json
            review = json.loads(app.screen._lines[0])
            assert review["payload"] == {"title": "Globex task", "status": 2, "notify_before": 2700,
                "group_id": 9000004001, "description": "<div><p>&lt;literal&gt; &amp; check</p></div>",
                "planned_start_date": "2026-12-05T00:00:00+00:00",
                "planned_end_date": "2026-12-06T00:00:00+00:00", "planned_effort": "1h 30m"}
            assert review["plan_hash"] == change_tasks.build_plan(123, "Globex task", group_id=9000004001,
                status="In Progress", note="<literal> & check", notify_before="45 Minutes",
                planned_start="2026-12-05T00:00Z", planned_end="2026-12-06T00:00Z", planned_effort="01:30").hash
            await pilot.press("escape")
    asyncio.run(go())
    assert journal.read() == []


def test_tui_optional_fields_clear_and_cancel(offline, monkeypatch):
    monkeypatch.setattr(change_tasks, "get_client", lambda: pytest.fail("cancel constructed client"))
    monkeypatch.setattr(change_tasks, "send", lambda *a, **k: pytest.fail("cancel sent task"))
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._change_task_options({"id": 123, "_resource": CHANGES}, "Acme task", None,
                                     {"note": "Draft", "planned_effort": "2h"})
            await pilot.pause()
            app.screen.query_one(Input).value = "Clear optional fields"
            await pilot.pause()
            await pilot.press("enter")
            assert isinstance(app.screen, ChoiceScreen)
            assert all("Draft" not in row.name and "2h" not in row.name for row in app.screen._views)
            await pilot.press("escape")
            await app.workers.wait_for_complete()
    asyncio.run(go())
    assert journal.read() == []


def test_no_task_create_for_ticket(offline):
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._selected = {"id": 123, "_resource": TICKETS}
            await pilot.press("a")
            assert isinstance(app.screen, ChoiceScreen)
            assert all(row.value != "task-add" for row in app.screen._views)
    asyncio.run(go())
