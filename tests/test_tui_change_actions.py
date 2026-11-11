"""Selected change actions: synthetic exact requests and Pilot review gates."""
from __future__ import annotations

import asyncio
from types import SimpleNamespace

import pytest
from textual.widgets import Input

from fsv import create, views as V
from fsv.resources import CHANGES, TICKETS
from fsv.tui import actions
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, FailureScreen, ValueScreen
from test_tui_integration import offline  # noqa: F401

ITEM = {"id": 9000001001, "display_id": 11, "_resource": CHANGES}
STATUS_FIELDS = [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 3, "value": "Completed"}]}]
TARGET = {"id": 9000001002, "display_id": 22, "title": "Acme task", "subject": "Acme access", "status": 1}


class Client:
    def __init__(self):
        self.calls = []
    def int_put(self, path, body):
        self.calls.append(("PUT", path, body))
        return {"task": {}}
    def int_delete(self, path, **kw):
        self.calls.append(("DELETE", path, kw))


@pytest.mark.parametrize("kind,body,expected", [
    ("task-update", {"status": 3}, ("PUT", "changes/9000001001/tasks/9000001002", {"status": 3})),
    ("task-delete", {}, ("PUT", "changes/9000001001/tasks/update", {"id": "9000001002", "action": "delete"})),
])
def test_exact_helper_one_send(kind, body, expected):
    client = Client()
    actions.apply_change_action(kind, ITEM["id"], TARGET, body, client)
    assert client.calls == [expected]


@pytest.mark.parametrize("kind", ["task-update", "task-delete", "link", "unlink", "download"])
@pytest.mark.parametrize("confirm", [False, True])
def test_selected_action_review(offline, monkeypatch, kind, confirm):
    from fsv import task_status
    monkeypatch.setattr(task_status, "fields_from", lambda client: STATUS_FIELDS)
    sent = []
    monkeypatch.setattr(create, "get_task_for_edit", lambda *a, **kw: TARGET.copy())
    monkeypatch.setattr(actions, "apply_change_action", lambda *args: sent.append(args) or {"verified": True})
    monkeypatch.setattr(actions, "download_selected", lambda *args: sent.append(args) or {"status": "downloaded", "path": "acme.txt"})
    def tab(*args):
        rows = [] if kind in ("task-delete", "unlink") else [{**TARGET, "status": 3}]
        return V.TabData("tasks", "Tasks", data=rows, parts={"Tickets": rows}, meta={"complete": True},
                         parts_meta={"Tickets": {"complete": True}})
    monkeypatch.setattr(V, "load_tab", tab)
    async def go():
        app = FsvApp()
        app._selected = ITEM.copy()
        async with app.run_test(size=(120, 40)) as pilot:
            app._pick_action_target(kind, ITEM, [TARGET], [])
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            if kind == "task-update":
                assert isinstance(app.screen, ChoiceScreen)
                await pilot.press("down", "enter")  # status
                app.screen.query_one(Input).value = "3"
                await pilot.press("enter")
            if kind == "download":
                assert isinstance(app.screen, ValueScreen)
                await pilot.press("enter")
            assert isinstance(app.screen, ConfirmScreen)
            if kind in ("link", "unlink"):
                from fsv.change_associations import build_plan
                plan = build_plan(11, [TARGET["id"]], unlink=kind == "unlink")
                assert any(plan.hash in line for line in app.screen._lines)
            assert sent == []
            await pilot.press("y" if confirm else "escape")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert len(sent) == (1 if confirm else 0)
            if confirm:
                assert not isinstance(app.screen, FailureScreen), app._write_failure
    asyncio.run(go())


def test_action_menu_navigation_and_read_only(offline):
    async def go():
        for readonly in (False, True):
            app = FsvApp(read_only=readonly)
            app._selected = ITEM.copy()
            async with app.run_test() as pilot:
                app._selected = ITEM.copy()
                await pilot.press("a")
                if readonly:
                    assert not isinstance(app.screen, ChoiceScreen)
                else:
                    assert isinstance(app.screen, ChoiceScreen)
                    await pilot.press("escape")
    asyncio.run(go())


@pytest.mark.parametrize("stage", ["write", "readback"])
def test_action_failure_no_retry(offline, monkeypatch, stage):
    sends = []
    def send(*args):
        sends.append(args)
        if stage == "write":
            raise TimeoutError("outcome unknown")
    monkeypatch.setattr(actions, "apply_change_action", send)
    monkeypatch.setattr(V, "load_tab", lambda *a: V.TabData("tasks", "Tasks", data=[], meta={"complete": False}))
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._confirm_action("task-delete", ITEM, TARGET, {})
            await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, FailureScreen)
            assert "unknown" in app.screen.message or "unverified" in app.screen.message
            await pilot.press("escape", "!")
            assert isinstance(app.screen, FailureScreen)
    asyncio.run(go())
    assert len(sends) == 1


def test_task_diff_only_and_validation():
    assert actions.task_patch({"status": 3}, "status", "3", STATUS_FIELDS) == {}
    assert actions.task_patch({"status": 1}, "status", "3", STATUS_FIELDS) == {"status": 3}
    assert actions.task_patch({}, "owner_id", "9000001003") == {"owner_id": 9000001003}
    with pytest.raises(ValueError):
        actions.task_patch({}, "status", "9")
    with pytest.raises(ValueError):
        actions.task_patch({}, "custom_fields", "anything")


def test_initiating_tickets_denied_is_not_empty():
    with pytest.raises(ValueError, match="unavailable"):
        actions.target_inventory(V.TabData("associations", "Associations", parts={"Problems": []}), False)


def test_download_bytes_skip_and_safe_path(tmp_path):
    calls = []
    client = SimpleNamespace(_client=SimpleNamespace(get=lambda *a, **kw: calls.append(a) or
                             SimpleNamespace(status_code=200, content=b"Acme evidence")))
    att = {"id": 9000001005, "name": "acme.txt", "attachment_url": "https://example.com/acme.txt"}
    result = actions.download_selected(client, att, str(tmp_path))
    assert result["status"] == "downloaded"
    assert (tmp_path / "acme.txt").read_bytes() == b"Acme evidence"
    assert actions.download_selected(client, att, str(tmp_path))["status"] == "exists"
    assert len(calls) == 1
    assert actions.download_selected(client, {**att, "name": "../outside.txt"}, str(tmp_path))["status"] == "rejected"


@pytest.mark.parametrize("kind", ["task-delete", "unlink", "link", "download"])
def test_target_loader_errors_and_cancel(offline, monkeypatch, kind):
    monkeypatch.setattr(V, "load_tab", lambda *a: V.TabData("tasks", "Tasks", missing=V.Missing("denied", 403)))
    def denied(*a, **kw):
        raise ValueError("denied")
    monkeypatch.setattr(create, "search_change_tickets", denied)
    monkeypatch.setattr(actions, "download_choices", denied)
    monkeypatch.setattr(actions, "apply_change_action", lambda *a: pytest.fail("no write"))
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._load_action_targets(kind, ITEM, "Acme")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, FailureScreen)
            assert "denied" in app.screen.message
    asyncio.run(go())


def test_download_inventory_all_sources_and_partial(monkeypatch):
    main = {"id": 9000001010, "name": "main.txt"}
    field = {"id": 9000001011, "name": "item.txt", "attachment_url": "https://example.com/item.txt"}
    thread = {"id": 9000001012, "name": "thread.txt"}
    page = SimpleNamespace(record={"attachments": [main]}, requested_items=[{"custom_fields": {"file": [field]}}],
                           notes=lambda: [])
    monkeypatch.setattr(V, "load_detail", lambda *a: page)
    calls = []
    def conversations(c, rid, **kw):
        calls.append((rid, kw))
        return [{"id": 9000001020, "attachments": [thread]}], {"complete": False}, V.Missing("thread page two", 403)
    monkeypatch.setattr(V, "load_conversations", conversations)
    rows, warnings = actions.download_choices(object(), TICKETS, ITEM)
    assert {r["name"] for r in rows} == {"main.txt", "item.txt", "thread.txt"}
    assert calls == [(11, {"all_pages": True})]
    assert any("partial" in note for note in warnings)
    client = SimpleNamespace(int_get=lambda path: {"change_planning_fields": [{"label": "Acme plan", "attachments": [thread]}]})
    rows, _ = actions.download_choices(client, CHANGES, ITEM)
    assert any(r.get("source") == "planning: Acme plan" for r in rows)
