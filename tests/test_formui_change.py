"""The interactive change form (Textual pilot) over a mocked-httpx Acme
tenant: type-to-filter, live rule reveals, restricted choices, requester
search, dates and the calendar, the review, and the CLI wiring for
``fsv changes create`` / ``fsv changes update ID``."""
from __future__ import annotations

import datetime as dt
import json

import pytest
from typer.testing import CliRunner

import fsv.change_cli as change_cli
import fsv.change_flow as flow
import fsv.cli as cli
import fsv.formui.app as formui_app
from formui_drive import JANE, drive, fill_change, focus, run_app, settle, text_of, type_text
from formui_tenants import AcmeChanges
from fsv.formui.session import change_session
from fsv.formui.widgets import ComboBox

runner = CliRunner()


@pytest.fixture
def acme(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    t = AcmeChanges()
    c = t.client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    return t, c


@pytest.fixture
def tty(monkeypatch):
    monkeypatch.setattr(flow, "interactive", lambda: True)
    monkeypatch.setattr(change_cli, "form_tty", lambda: True)


def use_script(monkeypatch, script, seen: dict | None = None):
    def fake_run_form(session):
        if seen is not None:
            seen["session"] = session
        return drive(session, script)

    monkeypatch.setattr(formui_app, "run_form", fake_run_form)


# ── widgets ────────────────────────────────────────────────────────────────
def test_type_to_filter_picks_a_choice(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        await focus(app, pilot, "change_type")
        combo = app.screen.rows["change_type"].editor
        assert combo.is_open and [o.label for o in combo._options] == ["Normal", "Standard", "Emergency"]
        await type_text(pilot, "emer")
        await pilot.pause()
        assert [o.label for o in combo._options] == ["Emergency"]
        await pilot.press("enter")
        await settle(app, pilot)
        assert not combo.is_open and combo.input.value == "Emergency"

    run_app(s, script)
    assert s.state.display("change_type") == "Emergency"


def test_arrows_and_multi_select_checklist(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        await focus(app, pilot, "services")
        await type_text(pilot, "s")  # Search, Storage, ... (prefix first)
        await pilot.press("enter")  # toggles the first match
        await type_text(pilot, "bill")
        await pilot.press("enter")
        await settle(app, pilot)
        combo = app.screen.rows["services"].editor
        assert combo.is_open  # a checklist stays open
        assert "Search, Billing" in text_of(combo.query_one(".combo-chosen"))
        await type_text(pilot, "bill")
        await pilot.press("enter")  # toggles Billing off again
        await settle(app, pilot)

    run_app(s, script)
    assert s.state.get("services") == ("Search",)


def test_answer_reveals_fields_live_with_a_note(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        rows = app.screen.rows
        await focus(app, pilot, "cloud_only")
        await type_text(pilot, "yes")
        await pilot.press("enter")
        await settle(app, pilot)
        assert not rows["staging_start"].display and not rows["staging_end"].display
        await focus(app, pilot, "cloud_only")
        await pilot.press("backspace", "backspace", "backspace")
        await type_text(pilot, "no")
        await pilot.press("enter")
        await settle(app, pilot)
        assert rows["staging_start"].display
        label = text_of(rows["staging_start"].query_one(".field-label"))
        assert label.startswith("Staging Start") and label.rstrip().endswith("*")
        notes = text_of(app.screen.query_one("#form-notes"))
        assert "+ Staging Start (required), Staging End (required) appeared because Cloud Only = No" in notes

    run_app(s, script)


def test_restricted_dropdown_only_offers_the_allowed_choice(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        await focus(app, pilot, "cloud_only")
        await type_text(pilot, "no")
        await pilot.press("enter")
        await settle(app, pilot)
        await focus(app, pilot, "stage")
        combo = app.screen.rows["stage"].editor
        await type_text(pilot, "l")
        await pilot.pause()
        assert [o.label for o in combo._options] == []  # Live is not offered
        assert "no choice matches" in text_of(combo.query_one(".combo-note"))
        assert "choices limited by rule Cloud only: no" in text_of(app.screen.rows["stage"].query_one(".field-note"))
        help_text = text_of(app.screen.query_one("#help-text"))
        assert "Choices" in help_text and "Staging" in help_text and "Live" not in help_text

    run_app(s, script)
    assert s.state.get("stage") == "Staging"  # the single allowed choice was preselected


def test_requester_search_is_debounced_and_picks(acme, monkeypatch):
    t, c = acme
    s = change_session(c)
    import fsv.formui.widgets as W

    assert W.DEBOUNCE_LOOKUP == 0.25
    monkeypatch.setattr(W, "DEBOUNCE_LOOKUP", 0.6)  # generous on a loaded test machine

    async def script(app, pilot):
        await focus(app, pilot, "requester")
        await type_text(pilot, "jane")
        await pilot.pause(1.0)
        await settle(app, pilot)
        combo = app.screen.rows["requester"].editor
        assert [o.label for o in combo._options] == ["Jane Doe <jane.doe@example.com>",
                                                     "Jane Doerr <jane.doerr@example.com>"]
        await pilot.press("down", "enter")
        await settle(app, pilot)

    run_app(s, script)
    assert t.gets("/search/autocomplete/requesters") == [{"q": "jane", "qf": "details"}]  # one call, after the pause
    assert s.requester is not None and s.requester.email == "jane.doerr@example.com"


def test_group_and_agent_filter_the_bootstrap_list_locally(acme):
    t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        await focus(app, pilot, "agent")
        await settle(app, pilot)
        await type_text(pilot, "ana")
        await settle(app, pilot, 0.4)
        combo = app.screen.rows["agent"].editor
        assert [o.label for o in combo._options] == ["Ana Lopez <ana.lopez@example.com>"]
        await pilot.press("enter")
        await focus(app, pilot, "group")
        await settle(app, pilot)
        await type_text(pilot, "ops")
        await settle(app, pilot, 0.4)
        await pilot.press("enter")
        await settle(app, pilot)

    run_app(s, script)
    assert len(t.gets("/api/_/bootstrap/agents_groups")) == 1
    assert s.state.get("agent").id == 9000007102 and s.state.get("group").id == 9000009001


def test_typed_date_previews_and_commits(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        await focus(app, pilot, "planned_start_date")
        await type_text(pilot, "2030-11-26 19:00")
        await pilot.pause()
        ed = app.screen.rows["planned_start_date"].editor
        assert text_of(ed.query_one(".date-preview")) == "Tue 26 Nov 2030 19:00 (+09:00 from your Freshservice profile tz)"
        await pilot.press("enter")
        await settle(app, pilot)
        await focus(app, pilot, "planned_end_date")
        await type_text(pilot, "2030-11-26")
        await pilot.press("tab")
        await settle(app, pilot)
        assert "add a time" in text_of(app.screen.rows["planned_end_date"].editor.query_one(".date-preview"))

    run_app(s, script)
    tokyo = s.tz
    assert s.state.get("planned_start_date") == dt.datetime(2030, 11, 26, 19, 0, tzinfo=tokyo)
    assert s.state.get("planned_end_date") is None


def test_calendar_picks_day_and_time(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        await focus(app, pilot, "planned_start_date")
        await pilot.press("f2")
        await settle(app, pilot)
        assert type(app.screen).__name__ == "DatePickerScreen"
        await pilot.press("right", "enter")  # tomorrow, then the time box
        await pilot.press("backspace", "backspace", "backspace", "backspace", "backspace")
        await type_text(pilot, "19:00")
        await pilot.press("enter")
        await settle(app, pilot)
        assert type(app.screen).__name__ == "FormScreen"

    run_app(s, script)
    tomorrow = dt.datetime.now(s.tz).date() + dt.timedelta(days=1)
    assert s.state.get("planned_start_date") == dt.datetime.combine(tomorrow, dt.time(19, 0), tzinfo=s.tz)


def test_required_fields_block_submit(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        assert app.screen.query_one("#btn-review").disabled
        assert "required field(s) missing: Requester, Subject" in text_of(app.screen.query_one("#form-status"))
        await pilot.press("ctrl+s")
        await settle(app, pilot, 0.1)
        assert type(app.screen).__name__ == "ReviewScreen"
        assert app.screen.query_one("#rv-submit").disabled
        await pilot.press("escape")  # back to edit, focus on the first missing field
        await settle(app, pilot)
        assert type(app.screen).__name__ == "FormScreen"
        assert text_of(app.screen.rows["subject"].query_one(".field-error")) == "required"

    run_app(s, script)


def test_review_shows_plan_hash_and_old_review_content(acme):
    _t, c = acme
    s = change_session(c)

    async def script(app, pilot):
        fill_change(app.screen.session)
        app.screen.refresh_rows()
        assert not app.screen.query_one("#btn-review").disabled
        await pilot.press("ctrl+s")
        await settle(app, pilot, 0.1)
        body = text_of(app.screen.query_one("#rv-body"))
        plan = app.screen.plan
        assert plan.ok and plan.hash and f"plan {plan.hash}" in body
        assert "Requester" in body and "Jane Doe <jane.doe@example.com>" in body
        assert "Stage" in body and "only option allowed by rule" in body
        assert "Status on create" in body and not app.screen.query_one("#rv-submit").disabled

    run_app(s, script)


def test_narrow_terminal_moves_help_below(acme):
    _t, c = acme

    async def script(app, pilot):
        assert app.screen.has_class("-narrow")

    run_app(change_session(c), script, size=(80, 30))


# ── CLI ──────────────────────────────────────────────────────────────────────
async def _fill_and_submit(app, pilot):
    fill_change(app.screen.session)
    app.screen.refresh_rows()
    await pilot.press("ctrl+s")
    await settle(app, pilot, 0.1)
    await pilot.click("#rv-submit")
    await settle(app, pilot)


def test_changes_create_opens_the_form_and_sends_once(acme, tty, monkeypatch):
    t, _c = acme
    use_script(monkeypatch, _fill_and_submit)
    r = runner.invoke(cli.app, ["changes", "create"])
    assert r.exit_code == 0, r.output
    posts = [w for w in t.writes() if w[:2] == ("POST", "/api/_/changes")]
    assert len(posts) == 1
    body = posts[0][2]
    assert body["email"] == "Jane Doe <jane.doe@example.com>"
    assert body["custom_fields"] == {"cloud_only": "Yes", "stage": "Live"}
    assert body["planned_start_date"] == "2030-11-26T19:00:00+09:00"
    assert "<strong>billing</strong>" in body["description"]
    assert "created" in r.output and "CHN-502" in r.output
    # the metadata was re-fetched before sending (freshness check)
    assert len(t.gets("/api/_/change_form_fields")) >= 2


def test_changes_create_cancel_sends_nothing(acme, tty, monkeypatch):
    t, _c = acme

    async def script(app, pilot):
        await focus(app, pilot, "subject")
        await type_text(pilot, "half done")
        await pilot.press("tab")
        await pilot.press("escape")  # leaves the subject's combo... then the form asks
        await settle(app, pilot)
        if type(app.screen).__name__ == "FormScreen":
            await pilot.press("escape")
            await settle(app, pilot)
        assert type(app.screen).__name__ == "ConfirmDialog"
        await pilot.press("y")
        await settle(app, pilot)

    use_script(monkeypatch, script)
    r = runner.invoke(cli.app, ["changes", "create"])
    assert r.exit_code == 1 and "nothing was sent" in r.output
    assert t.writes() == []


def test_changes_create_dry_run_from_the_form_sends_nothing(acme, tty, monkeypatch):
    t, _c = acme
    seen: dict = {}

    async def script(app, pilot):
        await _fill_and_submit(app, pilot)

    use_script(monkeypatch, script, seen)
    r = runner.invoke(cli.app, ["changes", "create", "--dry-run", "-o", "json"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    assert doc["plan_hash"] and doc["payload"]["subject"] == "Billing: rotate keys"
    assert seen["session"].dry_run and t.writes() == []


def test_copy_as_command_round_trips_through_a_saved_draft(acme, tty, monkeypatch, tmp_path):
    t, c = acme
    s = change_session(c)
    fill_change(s)
    (tmp_path / "evidence.pdf").write_bytes(b"%PDF-1.4 acme")
    (tmp_path / "rollout.xlsx").write_bytes(b"sheet")
    s.set("__attachments__", str(tmp_path / "evidence.pdf"))
    s.set("planning:Go-Live Checklist", str(tmp_path / "rollout.xlsx"))
    plan = s.build_plan()
    assert plan.ok, plan.errors
    assert {a["name"] for a in plan.review.attachments} == {"evidence.pdf", "rollout.xlsx"}
    (tmp_path / "drafts").mkdir()
    path = s.save_draft(str(tmp_path / "drafts" / "draft.yaml"), plan)
    cmd = s.command_for(path, plan)
    assert cmd == f"fsv changes create --from {path} --yes --expect-plan {plan.hash[:16]}"
    monkeypatch.setattr(flow, "interactive", lambda: False)
    r = runner.invoke(cli.app, ["changes", "create", "--from", path, "--dry-run", "-o", "json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["plan_hash"] == plan.hash


def test_review_copy_as_command_dialog(acme, tmp_path, monkeypatch):
    _t, c = acme
    monkeypatch.chdir(tmp_path)
    s = change_session(c)

    async def script(app, pilot):
        fill_change(app.screen.session)
        app.screen.refresh_rows()
        await pilot.press("ctrl+s")
        await settle(app, pilot, 0.1)
        await pilot.press("f6")
        await settle(app, pilot)
        await pilot.press("enter")  # save to the default path
        await settle(app, pilot)
        assert type(app.screen).__name__ == "TextDialog"
        cmd = text_of(app.screen.query_one("#dlg-command"))
        assert cmd.startswith("fsv changes create --from change-draft.yaml --yes --expect-plan ")

    run_app(s, script)
    assert (tmp_path / "change-draft.yaml").exists()


def test_changes_update_opens_prefilled_edit_form_and_sends_only_the_diff(acme, tty, monkeypatch):
    t, _c = acme
    seen: dict = {}

    async def script(app, pilot):
        rows = app.screen.rows
        assert app.screen.session.mode == "edit"
        assert rows["subject"].editor.value == "Billing: rotate keys"
        assert rows["priority"].editor.input.value == "Medium"
        await focus(app, pilot, "priority")
        await pilot.press(*["backspace"] * 6)
        await type_text(pilot, "high")
        await pilot.press("enter")
        await settle(app, pilot)
        await pilot.press("ctrl+s")
        await settle(app, pilot, 0.1)
        body = text_of(app.screen.query_one("#rv-body"))
        assert "Priority" in body and "Medium" in body and "→" in body and "High" in body
        await pilot.click("#rv-submit")
        await settle(app, pilot)

    use_script(monkeypatch, script, seen)
    r = runner.invoke(cli.app, ["changes", "update", "900000501"])
    assert r.exit_code == 0, r.output
    puts = [w for w in t.writes() if w[0] == "PUT"]
    assert puts == [("PUT", "/api/_/changes/900000501", {"priority": 3})]
    rules_q = t.gets("/api/_/ws/2/business_rules")
    assert rules_q and rules_q[0]["form_type"] == "edit_form"


def test_update_without_flags_and_without_a_terminal_still_errors(acme):
    r = runner.invoke(cli.app, ["changes", "update", "900000501"])
    assert r.exit_code != 0 and "nothing to update" in r.output


def test_session_send_for_embedding_apps(acme):
    t, c = acme
    s = change_session(c)
    fill_change(s)
    plan = s.build_plan()
    out = s.send(plan)
    assert out["id"] == 900000502
    assert [w[:2] for w in t.writes()] == [("POST", "/api/_/changes")]
    assert ComboBox  # imported for the public widget surface
