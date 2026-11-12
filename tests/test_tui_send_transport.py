"""Headless main-TUI send audit with real Client/httpx transport and journals.

Builders and senders are not mocked. Only metadata is synthetic. Every enabled
path is exercised at the UI confirmation boundary, through HTTP, and on replay.
"""
import asyncio
import copy
import json

import httpx
import pytest
from textual.widgets import Static

import acme_change_form as acme
from fsv import (change_approval_actions, change_approvals, change_flow, change_form_update,
                 change_tasks, config, formmeta, journal, planning_text, ticket_properties)
from fsv.client import Client
from fsv.formmeta import FormMeta
from fsv.resources import CHANGES, PROBLEMS, TICKETS
from fsv.tui.app import FsvApp
from fsv.tui.modals import ConfirmScreen, FailureScreen, OutcomeScreen
from test_tui_integration import offline  # noqa: F401

PATHS = ["reply", "note", "public-note", "problem-note", "approval-request", "approve", "reject", "cancel-approval",
         "task-add", "task-update", "task-delete", "planning-change", "planning-problem", "ticket-property", "change-status"]


@pytest.fixture
def transport(offline, monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    fields = [acme._f("subject", "Subject", "default_subject", 1),
              acme._f("status", "Status", "default_status", 2, choices=[{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}])]
    metas = {kind: FormMeta(kind, "edit_form", {kind + "_fields": copy.deepcopy(fields)}, {"business_rules": []}, 1, 0, "fixture", 123)
             for kind in ("ticket", "change")}
    def metadata(client, kind, *args, **kwargs):
        return metas[kind]
    for module in (formmeta, change_flow, change_form_update, ticket_properties):
        monkeypatch.setattr(module, "load_meta", metadata)
    monkeypatch.setattr(planning_text.schema, "load", lambda *args, **kwargs: {"fields": [
        {"name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "id": 901}]})
    task_fields = [{"name": "title"}, {"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Completed"}]},
                   {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}]}]
    state = {"mode": "confirm", "path": None, "requests": [], "writes": [], "conversations": [], "notes": [], "planning": [],
             "ticket": {"id": 123, "subject": "Acme original", "status": 1, "custom_fields": {}},
             "change": {"id": 123, "display_id": 123, "subject": "Acme original", "status": 1, "custom_fields": {}},
             "problem": {"id": 123, "subject": "Acme investigation"}, "tasks": [], "approvals": []}
    def handle(req):
        state["requests"].append(req)
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "change_task_fields":
                data = {"change_task_fields": task_fields}
            elif path == "tickets/123/conversations":
                data = {"conversations": state["conversations"], "meta": {"count": len(state["conversations"])}}
            elif path.endswith("/tasks"):
                data = {"tasks": state["tasks"], "meta": {"count": len(state["tasks"])}}
            elif path == "changes/123/approvals":
                data = {"approvals": state["approvals"], "meta": {"count": len(state["approvals"])}}
            elif path == "changes/123/approval-chains":
                groups = [{"id": 601, "name": "Acme review", "approval_status": 0, "approvals": state["approvals"]}] if state["approvals"] else []
                data = {"approval_chains": [{"id": 701, "approval_groups": groups}], "meta": {"count": 1}}
            elif path.endswith("/planning-fields"):
                data = {"change_planning_fields" if path.startswith("changes") else "problem_planning_fields": state["planning"]}
            elif path == "problems/123/notes":
                data = {"notes": state["notes"], "meta": {"count": len(state["notes"])}}
            else:
                key = {"tickets/123": "ticket", "changes/123": "change", "problems/123": "problem"}.get(path)
                assert key, "Unexpected GET: " + path
                data = {key: state[key]}
            return httpx.Response(200, json=copy.deepcopy(data))
        state["writes"].append(req)
        mode = state["mode"]
        if mode == "disconnect":
            raise httpx.ReadError("synthetic uncertain outcome", request=req)
        if mode in ("403", "429", "500"):
            return httpx.Response(int(mode), json={"error": "synthetic uncertain outcome"})
        body = json.loads(req.content) if req.content else {}
        persist = mode != "unverified"
        row = {"id": 501, **(body or {})}
        if path in {"tickets/123/notes", "tickets/123/reply"}:
            if persist:
                state["conversations"] = [copy.deepcopy(row)]
            result = {"note": row}
        elif path == "problems/123/notes":
            if persist:
                state["notes"] = [copy.deepcopy(row)]
            result = {"note": row}
        elif path == "changes/123/tasks":
            row.setdefault("owner_id", None)
            if persist:
                state["tasks"] = [copy.deepcopy(row)]
            result = {"task": row}
        elif path == "changes/123/tasks/501":
            if persist:
                state["tasks"][0].update(body)
            result = {"task": row}
        elif path == "changes/123/tasks/update":
            if persist:
                state["tasks"] = []
            result = {}
        elif path == "changes/123/approval-groups":
            row = {"id": 501, "member_id": 81, "status": {"id": 0, "name": "requested"}, "type": {"id": 1, "name": "everyone"}}
            if persist:
                state["approvals"] = [row]
            result = {}
        elif path == "changes/123/approvals/501":
            if persist:
                state["approvals"][0]["status"] = {"id": body["approval_status"], "name": "approved" if body["approval_status"] == 1 else "rejected"}
            result = {}
        elif path == "changes/123/approval-groups/601/cancel":
            if persist:
                state["approvals"] = []
            result = {}
        elif "/planning-fields" in path:
            key = "acme_plan" if path.startswith("changes") else "problem_cause"
            if persist:
                state["planning"] = [{"name": key, "description": body["description"], "attachments": []}]
            result = {}
        elif path in {"tickets/123", "changes/123"}:
            key = "ticket" if path.startswith("tickets") else "change"
            if persist:
                state[key].update({k: v for k, v in body.items() if k != "br_validation_excludes"})
            result = {key: row}
        else:
            raise AssertionError("Unexpected write: " + path)
        return httpx.Response(200, json=result)
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    c._csrf = "synthetic"
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: c)
    monkeypatch.setattr(change_tasks, "get_client", lambda: c)
    yield c, state
    c.close()


def open_review(app, operation, state):
    item = {"id": 9000000123, "display_id": 123, "_resource": TICKETS}
    app._selected = item
    if operation == "reply":
        app._prepare_ticket_conversation(item, "Acme reply", True, True, {})
    elif operation in {"note", "public-note"}:
        app._review_ticket_note(item, "Acme note", operation == "public-note")
    elif operation == "problem-note":
        item["_resource"] = PROBLEMS
        app._prepare_problem_note(item, "Acme evidence")
    elif operation in {"approval-request", "approve", "reject", "cancel-approval"}:
        item["_resource"] = CHANGES
        if operation == "approval-request":
            plan = change_approvals.build_plan(123, "Acme review", "Please review", approver_ids=(81,))
        else:
            plan = change_approval_actions.build_plan(123, 601 if operation == "cancel-approval" else 501,
                                                    "cancel" if operation == "cancel-approval" else operation,
                                                    "Acme decision" if operation == "reject" else None)
        app._review_approval(item, plan, None)
    elif operation == "task-add":
        item["_resource"] = CHANGES
        app._prepare_change_task(item, "Acme task", None)
    elif operation in {"task-update", "task-delete"}:
        item["_resource"] = CHANGES
        app._confirm_action(operation, item, {"id": 501}, {"title": "Acme revised"} if operation == "task-update" else {})
    elif operation.startswith("planning-"):
        item["_resource"] = CHANGES if operation == "planning-change" else PROBLEMS
        app._prepare_planning_text(item, "Rollout Plan" if operation == "planning-change" else "Root Cause", "Acme plan")
    elif operation == "ticket-property":
        app._prepare_ticket_property(item, "subject", "Acme revised")
    elif operation == "change-status":
        item["_resource"] = CHANGES
        app._prepare_change_status(item, 2)


@pytest.mark.parametrize("operation", PATHS)
@pytest.mark.parametrize("mode", ["cancel", "confirm", "disconnect", "unverified", "403", "429", "500"])
def test_pilot_exact_review_confirm_single_http_and_no_replay(transport, operation, mode):
    c, state = transport
    state.update(mode=mode, path=operation)
    if operation in {"task-update", "task-delete"}:
        state["tasks"] = [{"id": 501, "title": "Acme original", "status": 1}]
    if operation in {"approve", "reject", "cancel-approval"}:
        state["approvals"] = [{"id": 501, "member_id": 81, "status": {"id": 0, "name": "requested"}}]
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            open_review(app, operation, state)
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen), getattr(app, "_write_failure", None)
            review = json.loads(app.screen._lines[0])
            assert review["plan_hash"] and "payload" in review and not state["writes"]
            await pilot.press("escape" if mode == "cancel" else "y")
            if operation in {"reply", "public-note"} and mode != "cancel":
                assert isinstance(app.screen, ConfirmScreen) and not state["writes"]
                await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert len(state["writes"]) == (0 if mode == "cancel" else 1)
            if mode == "cancel":
                return
            req = state["writes"][0]
            assert (json.loads(req.content) if req.content else None) == review["payload"]
            assert req.headers["content-type"] == "application/json"
            assert isinstance(app.screen, (FailureScreen, OutcomeScreen))
            rendered = " ".join(str(widget.render()) for widget in app.screen.query(Static))
            if mode == "confirm":
                assert any(r.method == "GET" for r in state["requests"][state["requests"].index(req)+1:])
                assert isinstance(app.screen, OutcomeScreen), rendered
                assert '"verified": true' in rendered.lower(), rendered
                return
            assert "unknown" in rendered.lower() or "unverified" in rendered.lower() or '"verified": false' in rendered.lower()
            await pilot.press("escape")
            open_review(app, operation, state)
            await app.workers.wait_for_complete()
            await pilot.pause()
            await pilot.press("y")
            if operation in {"reply", "public-note"} and isinstance(app.screen, ConfirmScreen):
                await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert len(state["writes"]) == 1
            assert isinstance(app.screen, FailureScreen)
    asyncio.run(go())


@pytest.mark.parametrize("operation", PATHS)
def test_readonly_after_review_blocks_every_sender(transport, operation):
    c, state = transport
    if operation in {"task-update", "task-delete"}:
        state["tasks"] = [{"id": 501, "title": "Acme original", "status": 1}]
    if operation in {"approve", "reject", "cancel-approval"}:
        state["approvals"] = [{"id": 501, "member_id": 81, "status": {"id": 0, "name": "requested"}}]
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            open_review(app, operation, state)
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            app.read_only = True
            await pilot.press("y")
            if isinstance(app.screen, ConfirmScreen):
                await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert not state["writes"]
    asyncio.run(go())


@pytest.mark.parametrize("operation", ["reply", "public-note"])
def test_cancel_second_public_consent_sends_nothing(transport, operation):
    c, state = transport
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            open_review(app, operation, state)
            await app.workers.wait_for_complete()
            await pilot.pause()
            await pilot.press("y")
            assert isinstance(app.screen, ConfirmScreen)
            await pilot.press("escape")
            await app.workers.wait_for_complete()
            assert not state["writes"]
    asyncio.run(go())


@pytest.mark.parametrize("mode", ["cancel", "confirm", "disconnect", "unverified"])
def test_main_tui_full_change_property_form_uses_single_reviewed_transport(transport, mode):
    from fsv.formui.screens import ReviewScreen
    c, state = transport
    state["mode"] = mode
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            item = {"id": 9000000123, "display_id": 123, "_resource": CHANGES}
            app._selected = item
            await pilot.press("e")
            await app.workers.wait_for_complete()
            await pilot.pause()
            app.screen.rows["subject"].editor.focus()
            app.screen.rows["subject"].editor.value = "Acme revised"
            await pilot.pause(0.4)
            await pilot.press("ctrl+s")
            await pilot.pause()
            assert isinstance(app.screen, ReviewScreen)
            assert app.screen.plan.payload == {"subject": "Acme revised"}, app.screen.plan.to_json()
            assert not state["writes"]
            await pilot.click("#rv-cancel" if mode == "cancel" else "#rv-submit")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert len(state["writes"]) == (0 if mode == "cancel" else 1), app._write_failure
            if mode != "cancel":
                assert isinstance(app.screen, FailureScreen if mode == "disconnect" else OutcomeScreen)
                assert json.loads(state["writes"][0].content) == {"subject": "Acme revised"}
    asyncio.run(go())


def test_legacy_tui_entrypoints_fail_closed_without_transport(transport):
    from fsv.tui import actions
    c, state = transport
    for fn, args in [(actions.apply_status, (CHANGES, 123, 2)),
                     (actions.apply_note, (CHANGES, 123, "Acme", False)),
                     (actions.apply_reply, (TICKETS, 123, "Acme")),
                     (actions.apply_change_action, ("task-update", 123, {"id": 501}, {"title": "Acme"}, c))]:
        with pytest.raises(ValueError, match="reviewed"):
            fn(*args)
    async def go():
        app = FsvApp()
        async with app.run_test() as pilot:
            app._selected = {"id": 123, "_resource": CHANGES}
            await pilot.press("n")
            assert isinstance(app.screen, FailureScreen)
            assert "disabled" in app.screen.message
            app._do_write("status", CHANGES, app._selected, 2, "Planning")
            await app.workers.wait_for_complete()
            assert not state["writes"]
    asyncio.run(go())
