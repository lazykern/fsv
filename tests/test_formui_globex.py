"""Portability: the form screens on "Globex", an account with no
workspaces, no business rules, no catalog sections, German labels and a
restricted role (403 on the agents/groups list and the agent search).
Nothing may crash; what is missing is shown as a dim note."""
from __future__ import annotations

import datetime as dt
from zoneinfo import ZoneInfo

import pytest
from typer.testing import CliRunner

import fsv.change_cli as change_cli
import fsv.change_flow as flow
import fsv.cli as cli
import fsv.formui.app as formui_app
from formui_drive import drive, focus, run_app, settle, text_of, type_text
from formui_tenants import Globex
from fsv import config
from fsv.formui.session import change_session, sr_session
from fsv.forms import LookupValue

runner = CliRunner()
BERLIN = ZoneInfo("Europe/Berlin")
ERIKA = LookupValue(9000012001, "Erika Muster", "erika.muster@example.com")


@pytest.fixture
def globex(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    (config.CONFIG_DIR / "config.json").write_text('{"domain": "globex.freshservice.com"}')
    config.reload()
    t = Globex()
    c = t.client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    return t, c


def fill(s):
    s.set("requester", ERIKA)
    s.set("subject", "Druckerserver tauschen")
    s.set("description", "Alter Server wird **ersetzt**.")
    s.set("planned_start_date", dt.datetime(2031, 3, 2, 18, 0, tzinfo=s.tz))
    s.set("planned_end_date", dt.datetime(2031, 3, 2, 20, 0, tzinfo=s.tz))


def test_change_form_without_workspaces_or_rules(globex):
    t, c = globex
    s = change_session(c)
    assert [it.label for it in s.items][:4] == ["Antragsteller", "Betreff", "Änderungstyp", "Umgebung"]
    assert any("business rules are not available (HTTP 404)" in n for n in s.load_notes)
    assert not any("/ws/" in p for p in t.paths())
    # the agent profile's time zone is used when the rules carry no user
    assert s.tz_source == "Freshservice user time zone (Berlin)" and str(s.tz) == "Europe/Berlin"
    assert s.view("subject").required and s.view("planned_start_date").required
    fill(s)
    plan = s.build_plan()
    assert plan.ok, plan.errors
    assert any("business rules are not available" in w for w in plan.review.warnings)
    assert plan.payload["planned_start_date"] == "2031-03-02T18:00:00+01:00"


def test_choice_descriptions_and_accent_free_filter(globex):
    _t, c = globex
    s = change_session(c)
    sugg, _ = s.suggestions("umgebung", "prod")
    assert [(x.label, x.detail) for x in sugg] == [("Produktion", "Live-System")]
    sugg, _ = s.suggestions("priority", "dring")
    assert [x.label for x in sugg] == ["Dringend"]
    assert s.effects("umgebung") == []  # no rules: nothing to explain, nothing breaks


def test_restricted_role_shows_dim_notes_instead_of_failing(globex):
    t, c = globex
    s = change_session(c)

    async def script(app, pilot):
        await focus(app, pilot, "group")
        await settle(app, pilot, 0.4)
        note = text_of(app.screen.rows["group"].editor.query_one(".combo-note"))
        assert "not available for your role (HTTP 403)" in note
        await focus(app, pilot, "agent")
        await type_text(pilot, "max")
        await settle(app, pilot, 0.5)
        note = text_of(app.screen.rows["agent"].editor.query_one(".combo-note"))
        assert "agent search is not available for your role (HTTP 403)" in note
        await focus(app, pilot, "requester")
        await type_text(pilot, "jörg")
        await settle(app, pilot, 0.5)
        combo = app.screen.rows["requester"].editor
        assert [o.label for o in combo._options] == ["Jörg Beispiel <joerg.beispiel@example.com>"]
        await pilot.press("enter")
        await settle(app, pilot)
        assert text_of(app.screen.query_one("#form-title")).startswith("New Change")

    run_app(s, script)
    assert s.requester.email == "joerg.beispiel@example.com"
    assert t.gets("/search/autocomplete/requesters")[-1] == {"q": "jörg", "qf": "details"}


def test_requester_search_forbidden_accepts_a_typed_address(globex, monkeypatch):
    t, c = globex
    real = t.get

    def no_people(p, q):
        import httpx

        if p == "/search/autocomplete/requesters":
            return httpx.Response(403, json={"message": "forbidden"})
        return real(p, q)

    monkeypatch.setattr(t, "get", no_people)
    s = change_session(c)
    sugg, note = s.suggestions("requester", "erika.muster@example.com")
    assert "not available for your role (HTTP 403)" in note
    assert sugg and sugg[0].label == "use erika.muster@example.com"
    s.set("requester", sugg[0].value)
    assert s.requester.email == "erika.muster@example.com"


def test_create_from_the_form_on_globex(globex, monkeypatch):
    t, _c = globex
    monkeypatch.setattr(flow, "interactive", lambda: True)
    monkeypatch.setattr(change_cli, "form_tty", lambda: True)

    async def script(app, pilot):
        fill(app.screen.session)
        app.screen.refresh_rows()
        await pilot.press("ctrl+s")
        await settle(app, pilot, 0.1)
        assert "business rules are not available" in text_of(app.screen.query_one("#rv-body"))
        await pilot.click("#rv-submit")
        await settle(app, pilot)

    monkeypatch.setattr(formui_app, "run_form", lambda s: drive(s, script))
    r = runner.invoke(cli.app, ["changes", "create"])
    assert r.exit_code == 0, r.output
    posts = [w for w in t.writes() if w[1] == "/api/_/changes"]
    assert len(posts) == 1 and posts[0][2]["email"] == "Erika Muster <erika.muster@example.com>"
    assert not any("/ws/" in p for p in t.paths())


def test_service_request_without_sections_rules_or_attachments(globex, tmp_path):
    t, c = globex
    s = sr_session(c, 501)
    assert [it.label for it in s.items] == ["Requester", "Begründung", "Modell", "Benötigt bis", "Angebot (.pdf)"]
    assert any("business rules are not available (HTTP 404)" in n for n in s.load_notes)
    (tmp_path / "angebot.pdf").write_bytes(b"%PDF")
    (tmp_path / "angebot.docx").write_bytes(b"doc")
    s.set("__requester__", ERIKA)
    s.set("begruendung", "Altgerät defekt")
    assert s.set("modell", "Entwickler 16 Zoll").notes == []
    s.set("benoetigt_bis", dt.date(2031, 4, 1))
    assert "allowed file types are .pdf" in s.set("angebot", str(tmp_path / "angebot.docx")).error
    s.set("angebot", str(tmp_path / "angebot.pdf"))
    plan = s.build_plan()
    assert plan.ok, plan.errors
    out = s.send(plan)
    assert out["created"]["display"] == "SR-777"
    assert [w[1] for w in t.writes()] == ["/api/_/attachments", "/api/_/catalog/service-items/501/place-request"]
    assert not any("/ws/" in p for p in t.paths())
