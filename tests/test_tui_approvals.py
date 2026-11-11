"""Approval pickers dispatch only the reviewed, hash-bound shared plans."""
import asyncio

import pytest
from textual.widgets import Input

from fsv import change_approvals, change_approval_actions, config
from fsv.resources import CHANGES
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, FailureScreen, OutcomeScreen, ValueScreen
from test_tui_integration import offline  # noqa: F401

ITEM = {"id": 9000001001, "display_id": 11, "_resource": CHANGES}
ROW = {"id": 9000001002, "status": 0, "member_id": 9000001003}
GROUP = {"id": 9000001004, "name": "Acme", "approval_status": 0, "approvals": [ROW]}


@pytest.mark.parametrize("rule_index", range(4))
@pytest.mark.parametrize("confirm", [False, True])
def test_request_rule_review_and_cancel(offline, monkeypatch, rule_index, confirm):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    sent = []
    monkeypatch.setattr(change_approvals, "send", lambda plan, client, **kw: sent.append((plan, kw)) or
                        {"verified": True, "group_name_verified": True})
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._compose_approval_request(ITEM)
            await pilot.pause()
            for value in ("Acme group", "Please review", "9000001003"):
                assert isinstance(app.screen, ValueScreen)
                app.screen.query_one(Input).value = value
                await pilot.press("enter")
            assert isinstance(app.screen, ChoiceScreen)
            await pilot.press(*(["down"] * rule_index), "enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            assert any(list(change_approvals.RULES.values())[rule_index][1] in line for line in app.screen._lines)
            assert not sent
            await pilot.press("y" if confirm else "escape")
            await app.workers.wait_for_complete()
            await pilot.pause()
    asyncio.run(go())
    assert len(sent) == int(confirm)
    if confirm:
        plan, kw = sent[0]
        assert plan.rule == list(change_approvals.RULES)[rule_index]
        assert kw["reviewed_hash"] == plan.hash
        assert plan.approver_ids == (9000001003,)


@pytest.mark.parametrize("kind,operation", [("approval-respond", "approve"), ("approval-respond", "reject"),
                                             ("approval-cancel", "cancel")])
@pytest.mark.parametrize("confirm", [False, True])
def test_decision_and_group_cancel_review(offline, monkeypatch, kind, operation, confirm):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    sent = []
    monkeypatch.setattr(change_approval_actions, "send", lambda plan, client, **kw: sent.append((plan, kw)) or {"verified": True})
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._pick_approval_target(kind, ITEM, [GROUP if operation == "cancel" else ROW])
            await pilot.press("enter")
            if operation != "cancel":
                assert isinstance(app.screen, ChoiceScreen)
                await pilot.press(*(["down"] if operation == "reject" else []), "enter")
                assert isinstance(app.screen, ValueScreen)
                if operation == "reject":
                    app.screen.query_one(Input).value = "Needs work"
                await pilot.press("enter")
            assert isinstance(app.screen, ConfirmScreen)
            assert any(str(GROUP["id"] if operation == "cancel" else ROW["id"]) in line for line in app.screen._lines)
            assert sent == []
            await pilot.press("y" if confirm else "escape")
            await app.workers.wait_for_complete()
            await pilot.pause()
    asyncio.run(go())
    assert len(sent) == int(confirm)
    if confirm:
        plan, kw = sent[0]
        assert plan.operation == operation
        assert plan.target_id == (GROUP["id"] if operation == "cancel" else ROW["id"])
        assert kw["reviewed_hash"] == plan.hash


def test_group_inventory_read_and_failure(offline, monkeypatch):
    def read(cid, client):
        assert cid == 11
        return [GROUP]
    monkeypatch.setattr(change_approvals, "get_change_approval_groups", read)
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._load_approval_targets("approval-groups", ITEM)
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ChoiceScreen)
            await pilot.press("enter")
            assert isinstance(app.screen, OutcomeScreen)
            assert str(GROUP["id"]) in app.screen.message
    asyncio.run(go())


def test_approval_failure_is_not_retried(offline, monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    sent = []
    def fail(*args, **kwargs):
        sent.append(1)
        raise change_approvals.ApprovalError("outcome unknown; do not resubmit")
    monkeypatch.setattr(change_approval_actions, "send", fail)
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._prepare_approval_action(ITEM, ROW, "approve", None)
            await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, FailureScreen)
            assert "do not resubmit" in app.screen.message
    asyncio.run(go())
    assert sent == [1]
