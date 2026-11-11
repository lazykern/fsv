"""The interactive service-request form over the mocked-httpx Acme catalog:
requester first (never the logged-in user), a choice revealing its
section's field, file-field validation, and ``fsv sr create ITEM`` sending
uploads only after the review is confirmed."""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import acme_catalog as AC
import fsv.change_cli as change_cli
import fsv.change_flow as flow
import fsv.cli as cli
import fsv.formui.app as formui_app
from formui_drive import drive, focus, run_app, settle, text_of, type_text
from fsv import config
from fsv.formui.session import ATTACHMENTS, REQUESTER, sr_session
from fsv.forms import LookupValue

runner = CliRunner()
JANE = LookupValue(9000001, "Jane Doe", AC.JANE)


@pytest.fixture
def cat(monkeypatch, tmp_path):
    t = AC.AcmeCatalog()
    c = t.client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    (tmp_path / "deck.pptx").write_bytes(b"deck-bytes")
    (tmp_path / "brief.pdf").write_bytes(b"pdf-bytes")
    (tmp_path / "notes.txt").write_bytes(b"notes")
    return t, c, tmp_path


def fill_sr(s, base):
    s.set(REQUESTER, JANE)
    s.set("title_100", "Dark mode")
    s.set("paragraph_101", "Please add dark mode.")
    s.set("date_103", __import__("datetime").date(2030, 12, 3))
    s.set("goal", "Lower cost")
    s.set("goal_2_kpi", "-10% support tickets")
    s.set("priority", "Medium - planned work")
    s.set("overview_deck", str(base / "deck.pptx"))
    s.set(ATTACHMENTS, str(base / "notes.txt"))


def test_requester_comes_first_and_is_never_the_logged_in_user(cat):
    _t, c, _base = cat
    s = sr_session(c, 77)
    assert s.items[0].key == REQUESTER and s.requester is None
    assert "Requester is required" in s.problems()


def test_config_default_requester_is_used_when_set(cat):
    _t, c, _base = cat
    config.save_config({**config.load_config(), "defaults": {"sr_requester": AC.JANE}})
    s = sr_session(c, 77)
    assert s.requester is not None and s.requester.email == AC.JANE
    assert any("defaults.sr_requester" in n for n in s.load_notes)


def test_choice_reveals_its_section_field(cat):
    _t, c, _base = cat
    s = sr_session(c, 77)

    async def script(app, pilot):
        rows = app.screen.rows
        assert not rows["goal_1_kpi"].display and not rows["goal_2_kpi"].display
        await focus(app, pilot, "goal")
        await type_text(pilot, "lower")
        await pilot.press("enter")
        await settle(app, pilot)
        assert rows["goal_2_kpi"].display and not rows["goal_1_kpi"].display
        assert "+ 1.2 Cost target (required) appeared because 1. Expected benefit = Lower cost" in \
            text_of(app.screen.query_one("#form-notes"))
        await focus(app, pilot, "goal")
        assert any("Picking More revenue  (Total USD) will show 1.1 Revenue target (required)" in e or
                   "will show 1.1 Revenue target" in e for e in s.effects("goal"))

    run_app(s, script)


def test_file_field_validates_type_and_count(cat):
    _t, c, base = cat
    s = sr_session(c, 77)

    async def script(app, pilot):
        await focus(app, pilot, "overview_deck")
        await type_text(pilot, str(base / "brief.pdf"))
        await pilot.press("enter")
        await settle(app, pilot)
        err = text_of(app.screen.rows["overview_deck"].query_one(".field-error"))
        assert "allowed file types are .pptx" in err
        ed = app.screen.rows["overview_deck"].editor
        ed.input.value = f"{base / 'deck.pptx'}; {base / 'deck.pptx'}"
        await pilot.press("enter")
        await settle(app, pilot)
        assert "at most 1 file" in text_of(app.screen.rows["overview_deck"].query_one(".field-error"))
        ed.input.value = str(base / "deck.pptx")
        await pilot.press("enter")
        await settle(app, pilot)
        assert text_of(app.screen.rows["overview_deck"].query_one(".field-error")) == ""
        assert "deck.pptx" in text_of(ed.query_one(".file-list"))

    run_app(s, script)
    assert s.state.get("overview_deck") == (str(base / "deck.pptx"),)


def test_general_attachment_is_required_by_the_item(cat):
    _t, c, base = cat
    s = sr_session(c, 77)
    fill_sr(s, base)
    s.set(ATTACHMENTS, "")
    assert "Attachments: at least one file is required" in s.problems()
    assert s.view(ATTACHMENTS).required


def test_sr_create_opens_the_item_form_and_uploads_only_after_submit(cat, monkeypatch):
    t, _c, base = cat
    monkeypatch.setattr(flow, "interactive", lambda: True)
    monkeypatch.setattr(change_cli, "form_tty", lambda: True)
    seen: dict = {}

    async def script(app, pilot):
        fill_sr(app.screen.session, base)
        app.screen.refresh_rows()
        await pilot.press("ctrl+s")
        await settle(app, pilot, 0.1)
        assert t.writes() == []  # nothing uploaded while reviewing
        banner = text_of(app.screen.query_one("#rv-banner"))
        assert "This sends a real service request" in banner
        assert str(app.screen.query_one("#rv-submit").label) == "Place request"
        await pilot.click("#rv-submit")
        await settle(app, pilot)

    def fake(session):
        seen["s"] = session
        return drive(session, script)

    monkeypatch.setattr(formui_app, "run_form", fake)
    r = runner.invoke(cli.app, ["sr", "create", "77"])
    assert r.exit_code == 0, r.output
    w = t.writes()
    assert [p for _m, p, _b in w] == ["/api/_/attachments", "/api/_/attachments",
                                      "/api/_/catalog/service-items/77/place-request"]
    body = w[-1][2]
    assert body["email"] == AC.JANE and body["custom_fields"]["goal_2_kpi"] == "-10% support tickets"
    assert body["custom_fields"]["overview_deck"] == [9000005001] and body["attachments"] == [9000005002]
    assert "placed" in r.output


def test_sr_create_cancel_uploads_nothing(cat, monkeypatch):
    t, _c, _base = cat
    monkeypatch.setattr(flow, "interactive", lambda: True)
    monkeypatch.setattr(change_cli, "form_tty", lambda: True)

    async def script(app, pilot):
        await pilot.press("escape")
        await settle(app, pilot)

    monkeypatch.setattr(formui_app, "run_form", lambda s: drive(s, script))
    r = runner.invoke(cli.app, ["sr", "create", "77"])
    assert r.exit_code == 1 and "nothing was uploaded or sent" in r.output
    assert t.writes() == []


def test_sr_saved_draft_round_trips(cat, monkeypatch):
    _t, c, base = cat
    s = sr_session(c, 77)
    fill_sr(s, base)
    plan = s.build_plan()
    assert plan.ok, plan.errors
    path = s.save_draft(str(base / "sr.yaml"), plan)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    r = runner.invoke(cli.app, ["sr", "create", "77", "--from", path, "--dry-run", "-o", "json"])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output)["plan_hash"] == plan.hash
    assert s.command_for(path, plan) == f"fsv sr create 77 --from {path} --yes --expect-plan {plan.hash[:16]}"
