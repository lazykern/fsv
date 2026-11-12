"""Native planning text contracts and main-TUI multiline review."""
import asyncio
import copy
import json
from urllib.parse import parse_qs, urlsplit

import pytest
from textual.widgets import Input, TextArea
from typer.testing import CliRunner

from fsv import cli, config, journal, planning_text as planning
from fsv.errors import APIError
from fsv.resources import CHANGES, PROBLEMS
from fsv.reviewed_write import ReviewedWriteError
from fsv.tui.app import FsvApp
from fsv.tui.modals import ChoiceScreen, ConfirmScreen, OutcomeScreen, PlanningTextScreen
from test_tui_integration import offline  # noqa: F401


class Client:
    def __init__(self, module="changes"):
        self.module = module
        self.rows = []
        self.writes = []
        self.failure = None

    def int_get(self, path):
        return {"change_planning_fields" if self.module == "changes" else "problem_planning_fields": copy.deepcopy(self.rows)}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, path, **kw):
        payload = json.loads(kw["content"]) if "content" in kw else None
        self.writes.append((method, path, payload))
        if self.failure:
            raise self.failure
        key = (parse_qs(urlsplit(path).query)["id"][0] if method == "POST" else
               urlsplit(path).path.rsplit("/", 1)[1])
        self.rows = [] if method == "DELETE" else [
            {"name": key, "description": payload["description"], "attachments": []}]
        return {}


@pytest.fixture(autouse=True)
def setup(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(planning.schema, "load", lambda *a, **kw: {"fields": [
        {"name": "acme_plan", "label": "Rollout Plan", "field_type": "planning_field", "id": 901}]})


@pytest.mark.parametrize("module,section,key", [("changes", "Rollout Plan", "acme_plan"),
    ("problems", "Root Cause", "problem_cause"), ("problems", "Impact", "problem_impact"),
    ("problems", "Symptoms", "problem_symptom")])
def test_section_query_exact_wire_and_readback(module, section, key):
    c = Client(module)
    p = planning.build_plan(c, module, 123, section, "<div>Acme evidence</div>")
    assert p.method == "POST" and p.path.endswith("?id=" + key)
    assert p.payload == {"description": "<div>Acme evidence</div>"}
    assert planning.send(p, c, reviewed_hash=p.hash)["verified"]
    assert len(c.writes) == 1
    with pytest.raises(ReviewedWriteError, match="was sent"):
        planning.send(p, c, reviewed_hash=p.hash)


def test_attachments_unknown_identity_and_ambiguous_send_refused():
    c = Client()
    p = planning.build_plan(c, "changes", 123, "Rollout Plan", "Acme")
    c.failure = APIError(403, "uncertain")
    with pytest.raises(ReviewedWriteError, match="unknown"):
        planning.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="unknown"):
        planning.send(p, c, reviewed_hash=p.hash)
    assert len(c.writes) == 1
    c.rows = [{"name": "acme_plan", "description": "old", "attachments": [{"id": 999}]}]
    with pytest.raises(ReviewedWriteError, match="attachments"):
        planning.build_plan(c, "changes", 123, "Rollout Plan", "new")
    with pytest.raises(ReviewedWriteError, match="choose Root Cause"):
        planning.build_plan(Client("problems"), "problems", 123, "Workaround", "Acme")


def test_planning_cli_review_hash_then_send(monkeypatch):
    c = Client("problems")
    monkeypatch.setattr(cli, "_client", lambda: c)
    args = ["problems", "planning-text", "PRB-123", "--section", "Root Cause", "--description", "Acme evidence"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert result.exit_code == 0, result.output
    assert len(c.writes) == 1


@pytest.mark.parametrize("cancel", [True, False])
@pytest.mark.parametrize("native_empty", [True, False])
def test_tui_planning_picker_editor_review_and_outcome(offline, monkeypatch, cancel, native_empty):
    c = Client()
    if native_empty:
        c.rows = [{"name": "acme_plan", "content": "", "files": [], "empty": True}]
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: c)
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._selected = {"id": 9000001001, "display_id": 123, "_resource": CHANGES}
            await pilot.press("a")
            app.screen.query_one(Input).value = "native planning"
            await pilot.pause()
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ChoiceScreen)
            assert app.screen._title == "Native planning/analysis section"
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, PlanningTextScreen)
            app.screen.query_one(TextArea).text = "Acme\nsecond line"
            await pilot.press("ctrl+s")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen) and not c.writes
            assert "acme_plan" in app.screen._lines[0] and "plan_hash" in app.screen._lines[0]
            review = json.loads(app.screen._lines[0])
            assert review["before"] == "" and review["after"] == "Acme\nsecond line"
            assert review["operation"] == "create"
            await pilot.press("escape" if cancel else "y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            if not cancel:
                assert isinstance(app.screen, OutcomeScreen)
    asyncio.run(go())
    assert len(c.writes) == (0 if cancel else 1)


def test_tui_native_existing_content_preloads_and_reviews_edit(offline, monkeypatch):

    c = Client()
    c.rows = [{"name": "acme_plan", "content": "Acme original", "files": [], "empty": False}]
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: c)

    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            item = {"display_id": 123, "_resource": CHANGES}
            app._load_planning_editor(item, "acme_plan")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, PlanningTextScreen)
            assert app.screen.query_one(TextArea).text == "Acme original"
            app.screen.query_one(TextArea).text = "Acme replacement"
            await pilot.press("ctrl+s")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen)
            review = json.loads(app.screen._lines[0])
            assert review["operation"] == "edit" and review["method"] == "PUT"
            assert review["before"] == "Acme original"
            assert review["after"] == "Acme replacement"
            assert not c.writes
            await pilot.press("y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, OutcomeScreen)
            assert len(c.writes) == 1 and c.writes[0][0] == "PUT"

    asyncio.run(go())
