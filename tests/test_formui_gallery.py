"""SVG screenshots of the form screens (a smoke test of every screen).

Saved to ``$FSV_FORMUI_GALLERY`` when set, else to the test's temp dir."""
from __future__ import annotations

import os
from pathlib import Path

import pytest

import acme_catalog as AC
from formui_drive import JANE, fill_change, focus, run_app, settle, type_text
from formui_tenants import AcmeChanges, Globex
from fsv import config
from fsv.formui.session import change_session, sr_session


@pytest.fixture
def out(tmp_path, monkeypatch) -> Path:
    monkeypatch.delenv("FSV_TZ", raising=False)
    d = Path(os.environ.get("FSV_FORMUI_GALLERY") or tmp_path / "gallery")
    d.mkdir(parents=True, exist_ok=True)
    return d


def shot(app, out: Path, name: str) -> None:
    app.save_screenshot(filename=f"{name}.svg", path=str(out))
    assert (out / f"{name}.svg").stat().st_size > 1000


def test_gallery_change_form(out):
    c = AcmeChanges().client()

    async def script(app, pilot):
        shot(app, out, "01-new-change-empty")
        await focus(app, pilot, "cloud_only")
        await type_text(pilot, "no")
        await pilot.press("enter")
        await settle(app, pilot)
        shot(app, out, "02-rule-reveal-cloud-only-no")
        await focus(app, pilot, "services")
        await type_text(pilot, "s")
        await settle(app, pilot)
        shot(app, out, "03-dropdown-type-to-filter")
        await pilot.press("escape")
        await focus(app, pilot, "requester")
        await type_text(pilot, "jane")
        await settle(app, pilot, 0.5)
        shot(app, out, "04-requester-search")
        await pilot.press("escape")
        await focus(app, pilot, "planned_start_date")
        await pilot.press("f2")
        await settle(app, pilot)
        shot(app, out, "05-date-picker")
        await pilot.press("escape")
        await settle(app, pilot)
        s = app.screen.session
        fill_change(s)
        app.screen.refresh_rows()
        await pilot.press("ctrl+s")
        await settle(app, pilot, 0.1)
        shot(app, out, "06-review")

    run_app(change_session(c), script, size=(120, 40))


def test_gallery_edit_sr_globex_light(out, monkeypatch):
    t = AcmeChanges()
    c = t.client()

    async def edit(app, pilot):
        await focus(app, pilot, "priority")
        await pilot.press(*["backspace"] * 6)
        await type_text(pilot, "high")
        await pilot.press("enter")
        await settle(app, pilot)
        shot(app, out, "07-edit-form-prefilled")

    run_app(change_session(c, mode="edit", target=900000501), edit, size=(120, 40))

    cat = AC.AcmeCatalog().client()

    async def sr(app, pilot):
        await focus(app, pilot, "goal")
        await type_text(pilot, "lower")
        await pilot.press("enter")
        await settle(app, pilot)
        shot(app, out, "08-sr-form-section-reveal")

    run_app(sr_session(cat, 77), sr, size=(120, 40))

    (config.CONFIG_DIR / "config.json").write_text('{"domain": "globex.freshservice.com"}')
    config.reload()
    g = Globex().client()

    async def globex(app, pilot):
        await focus(app, pilot, "group")
        await settle(app, pilot, 0.4)
        shot(app, out, "09-globex-80-columns-no-rules-403")

    run_app(change_session(g), globex, size=(80, 30))

    async def light(app, pilot):
        app.theme = "textual-light"
        s = app.screen.session
        s.set("requester", JANE)
        app.screen.refresh_rows()
        await focus(app, pilot, "priority")
        await settle(app, pilot)
        shot(app, out, "10-light-theme")

    run_app(change_session(t.client()), light, size=(120, 40))
