"""Form UI regressions from the QA review (P1 #1-#6), on the invented Acme
tenant over a mocked transport; no network."""
from __future__ import annotations

import datetime as dt
from zoneinfo import ZoneInfo

import pytest

import acme_change_form as A
from formui_drive import JANE, fill_change, focus, run_app, settle, type_text
from formui_tenants import AcmeChanges
from fsv.forms import FormSpec, FormState, ValueParseError
from fsv.formmeta import FormMeta
from fsv.formui import when
from fsv.formui.screens import ConfirmDialog, DatePickerScreen, SavePathDialog
from fsv.formui.session import Suggestion, change_session

BERLIN = ZoneInfo("Europe/Berlin")


@pytest.fixture
def session(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    return change_session(AcmeChanges().client())


# P1 #1 ------------------------------------------------------------------------
def test_rejected_date_blocks_the_review_instead_of_sending_the_old_one(session):
    fill_change(session)
    seen = {}

    async def script(app, pilot):
        await focus(app, pilot, "planned_start_date")
        ed = app.screen.rows["planned_start_date"].editor
        ed.input.value = "nonsense"
        await pilot.pause()
        await pilot.press("ctrl+s")
        await settle(app, pilot)
        rv = app.screen
        seen["screen"] = type(rv).__name__
        seen["ok"] = rv.plan.ok
        seen["errors"] = list(rv.plan.errors)
        seen["submit_disabled"] = rv.query_one("#rv-submit").disabled
        await pilot.press("escape")  # back to edit
        await settle(app, pilot)
        seen["kept_text"] = ed.input.value
        ed.input.value = "2030-11-26 18:00"
        ed.commit()
        await settle(app, pilot)
        seen["fixed_errors"] = session.input_errors()

    run_app(session, script)
    assert seen["screen"] == "ReviewScreen"
    assert not seen["ok"] and seen["submit_disabled"]
    assert any(e.startswith("Planned Start Date:") for e in seen["errors"])
    assert seen["kept_text"] == "nonsense"
    assert seen["fixed_errors"] == []
    assert session.state.get("planned_start_date").hour == 18


# P1 #2 ------------------------------------------------------------------------
def test_enter_never_picks_a_result_of_the_previous_search(session):
    seen = {}

    async def script(app, pilot):
        await focus(app, pilot, "requester")
        ed = app.screen.rows["requester"].editor
        await type_text(pilot, "jane")
        await settle(app, pilot, 0.6)
        seen["first"] = [o.label for o in ed._options]
        ed.input.value = "not-a-person"
        await pilot.pause(0.01)
        seen["options_now"] = list(ed._options)
        await pilot.press("enter")
        await pilot.pause(0.01)
        seen["requester_after_enter"] = session.requester
        # a late answer for the old query is ignored; a click on stale rows too
        ed._found("jane", [Suggestion("Jane Doe", "", JANE)], "")
        seen["after_late"] = list(ed._options)
        await settle(app, pilot, 0.6)

    run_app(session, script)
    assert any("Jane Doe" in x for x in seen["first"])
    assert seen["options_now"] == [] and seen["after_late"] == []
    assert seen["requester_after_enter"] is None


# P1 #3 ------------------------------------------------------------------------
def test_calendar_keeps_the_instant_of_an_explicit_offset(session):
    seen = {}
    value = dt.datetime.fromisoformat("2031-03-02T18:00:00+07:00")

    async def script(app, pilot):
        pk = DatePickerScreen("Start", value, date_only=False, tz=BERLIN, tz_source="")
        app.push_screen(pk)
        await pilot.pause()
        seen["value"] = pk.value()
        app.pop_screen()

    run_app(session, script)
    assert seen["value"] == value  # 12:00 +01:00, the same instant
    assert seen["value"].utcoffset() == dt.timedelta(hours=1)


@pytest.mark.parametrize("value", [
    dt.datetime.fromisoformat("2031-03-02T18:00:00+07:00"),
    dt.datetime.fromisoformat("2031-03-02T18:00:30+01:00"),
    dt.datetime(2031, 7, 1, 9, 15, tzinfo=BERLIN),
])
def test_text_round_trips_to_the_same_instant(value):
    text = when.to_text(value, BERLIN)
    back = when.parse_when(text, tz=BERLIN).value
    assert back == value and back.second == value.second


def test_form_zone_values_are_written_without_an_offset():
    assert when.to_text(dt.datetime(2031, 7, 1, 9, 15, tzinfo=BERLIN), BERLIN) == "2031-07-01 09:15"


# P1 #4 ------------------------------------------------------------------------
def test_daylight_saving_gap_and_overlap_are_refused():
    gap = when.parse_when("2031-03-30 02:30", tz=BERLIN)
    assert not gap.ok and "does not exist" in gap.error
    twice = when.parse_when("2031-10-26 02:30", tz=BERLIN)
    assert not twice.ok and "happens twice" in twice.error and "+02:00" in twice.error and "+01:00" in twice.error
    chosen = when.parse_when("2031-10-26 02:30 +01:00", tz=BERLIN)
    assert chosen.ok and chosen.value.utcoffset() == dt.timedelta(hours=1)
    tokyo = when.parse_when("2031-03-30 02:30", tz=ZoneInfo("Asia/Tokyo"))  # no daylight saving
    assert tokyo.ok and tokyo.value.utcoffset() == dt.timedelta(hours=9)


def test_engine_refuses_daylight_saving_gaps_too():
    meta = FormMeta("change", "new_form", A.fields(), A.rules(), 2, 0.0, "server")
    state = FormState(FormSpec.from_form_fields(A.fields()), meta.engine(), mode="new", tz=BERLIN)
    with pytest.raises(ValueParseError, match="does not exist"):
        state.set("Planned Start Date", "2031-03-30 02:30")
    state.set("Planned Start Date", "2031-03-30 03:30")
    assert state.get("planned_start_date").utcoffset() == dt.timedelta(hours=2)


# P1 #5 ------------------------------------------------------------------------
def test_bad_draft_path_keeps_the_form_and_the_dialog(session, tmp_path):
    fill_change(session)
    seen = {}

    async def script(app, pilot):
        await pilot.press("f4")
        await settle(app, pilot)
        dlg = app.screen
        seen["dialog"] = type(dlg).__name__
        dlg.query_one("#dlg-path").value = str(tmp_path / "no-such-dir" / "draft.yaml")
        await pilot.press("enter")
        await settle(app, pilot)
        seen["still"] = type(app.screen).__name__
        seen["note"] = str(app.screen.query_one(".dlg-note").content)
        dlg.query_one("#dlg-path").value = "   "
        await pilot.press("enter")
        await settle(app, pilot)
        seen["blank"] = str(app.screen.query_one(".dlg-note").content)
        dlg.query_one("#dlg-path").value = str(tmp_path / "draft.yaml")
        await pilot.press("enter")
        await settle(app, pilot)

    app = run_app(session, script)
    assert seen["dialog"] == "SavePathDialog" and seen["still"] == "SavePathDialog"
    assert "does not exist" in seen["note"] and "type a file path" in seen["blank"]
    assert app.result is not None and app.result.action == "save_draft"
    assert (tmp_path / "draft.yaml").exists()


def test_save_error_is_shown_in_the_dialog(session, tmp_path):
    seen = {}

    def refuse(path):
        raise PermissionError(13, "Permission denied", path)

    async def script(app, pilot):
        app.push_screen(SavePathDialog(str(tmp_path / "x.yaml"), save=refuse))
        await settle(app, pilot)
        await pilot.press("enter")
        await settle(app, pilot)
        seen["screen"] = type(app.screen).__name__
        seen["note"] = str(app.screen.query_one(".dlg-note").content)

    run_app(session, script)
    assert seen["screen"] == "SavePathDialog" and "Permission denied" in seen["note"]


# P1 #6 ------------------------------------------------------------------------
@pytest.mark.parametrize("key,text", [("subject", "unsaved text"), ("requester", "jan")])
def test_escape_right_after_typing_asks_before_discarding(session, key, text):
    seen = {}

    async def script(app, pilot):
        await focus(app, pilot, key)
        ed = app.screen.rows[key].editor
        target = ed.input if hasattr(ed, "input") else ed
        target.value = text
        await pilot.pause(0.02)  # inside the debounce
        if key == "requester":
            await pilot.press("escape")  # first Esc closes the suggestion list
            await pilot.pause(0.02)
        await pilot.press("escape")
        await pilot.pause(0.05)
        seen["screen"] = app.screen
        seen["result"] = app.result

    run_app(session, script)
    assert isinstance(seen["screen"], ConfirmDialog)
    assert seen["result"] is None


def test_escape_on_an_untouched_form_still_cancels(session):
    seen = {}

    async def script(app, pilot):
        await pilot.press("escape")  # closes the focused requester's suggestion list
        await pilot.pause(0.05)
        await pilot.press("escape")
        await pilot.pause(0.05)
        seen["result"] = app.result

    run_app(session, script)
    assert seen["result"] is not None and seen["result"].action == "cancel"
