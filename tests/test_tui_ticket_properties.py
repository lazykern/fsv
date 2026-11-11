"""Main-TUI schema-backed ticket/SR property editing and beneficiary wire."""
import asyncio

import pytest
from textual.widgets import Input

from fsv import formmeta, ticket_properties as props
from fsv.resources import TICKETS
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, OutcomeScreen, ValueScreen
from test_ticket_properties import Client, rich_metadata, setup as property_setup  # noqa: F401
from test_tui_integration import offline  # noqa: F401


@pytest.mark.parametrize("cancel", [True, False])
def test_tui_property_field_picker_review_and_outcome(offline, property_setup, monkeypatch, cancel):
    c = Client()
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: c)
    monkeypatch.setattr(formmeta, "load_meta", lambda *a, **kw: rich_metadata())
    monkeypatch.setattr(props, "load_meta", lambda *a, **kw: rich_metadata())
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._selected = {"id": 9000001001, "display_id": 123, "_resource": TICKETS}
            await pilot.press("e")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ChoiceScreen)
            assert "ticket/SR property" in app.screen._title
            app.screen.query_one(Input).value = "Subject"
            await pilot.pause()
            await pilot.press("enter")
            assert isinstance(app.screen, ValueScreen)
            app.screen.query_one(Input).value = "Globex maintenance"
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            review = app.screen._lines[0]
            assert "Globex maintenance" in review and "before" in review and "plan_hash" in review
            assert not c.writes
            await pilot.press("escape" if cancel else "y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            if not cancel:
                assert isinstance(app.screen, OutcomeScreen)
    asyncio.run(go())
    assert len(c.writes) == (0 if cancel else 1)


def test_requested_for_update_is_property_only_and_resolves_current_user(property_setup, monkeypatch):
    m = rich_metadata()
    m.form_fields["ticket_fields"].append({"id": 9000009001, "name": "requested_for", "label": "Beneficiary",
        "field_type": "default_requested_for", "position": 7, "default_field": True})
    monkeypatch.setattr(props, "load_meta", lambda *a, **kw: m)
    c = Client()
    c.row["requested_for_id"] = 901
    monkeypatch.setattr(c, "autocomplete", lambda *a, **kw: [{"id": 902, "name": "Acme User", "email": "person@example.com"}], raising=False)
    p = props.build_plan(c, "SR-123", answers={"Beneficiary": "person@example.com"})
    assert p.payload["requested_for_id"] == 902
    assert not m.spec.field("requested_for").sent  # no accidental create/change wire expansion
    assert props.send(p, c, reviewed_hash=p.hash)["verified"]


def test_beneficiary_edit_never_reuses_previous_person_rule_context(property_setup, monkeypatch):
    from test_catalog_safety import rule
    m = rich_metadata()
    m.form_fields["ticket_fields"].append({"id": 9000009001, "name": "requested_for", "label": "Beneficiary",
        "field_type": "default_requested_for", "position": 7, "default_field": True})
    r = rule([{"name": "department_id", "operator": "is", "value": ["9"], "evaluate_on": "requested_for"}],
             [{"name": ["subject"], "property": "hide"}])
    r.pop("item_id")
    r["module_type"] = "ticket"
    m.rules = {"business_rules": [r], "requested_for": {"department_id": "9"}}
    monkeypatch.setattr(props, "load_meta", lambda *a, **kw: m)
    c = Client()
    c.row["requested_for_id"] = 901
    monkeypatch.setattr(c, "autocomplete", lambda *a, **kw: [{"id": 902, "name": "Acme User", "email": "person@example.com"}], raising=False)
    with pytest.raises(props.PropertyError, match="unavailable fields or context"):
        props.build_plan(c, "SR-123", answers={"Beneficiary": "person@example.com"})
    assert not c.writes


def test_readonly_property_picker_never_prepares(offline, property_setup, monkeypatch):
    monkeypatch.setattr(formmeta, "load_meta", lambda *a, **kw: pytest.fail("read-only metadata fetch"))
    async def go():
        app = FsvApp(read_only=True)
        async with app.run_test() as pilot:
            app._selected = {"id": 123, "_resource": TICKETS}
            await pilot.press("e")
            app._load_ticket_property_fields(app._selected)
            await app.workers.wait_for_complete()
            app._prepare_ticket_property(app._selected, "subject", "Acme")
            await app.workers.wait_for_complete()
            assert not isinstance(app.screen, (ChoiceScreen, ConfirmScreen))
    asyncio.run(go())
