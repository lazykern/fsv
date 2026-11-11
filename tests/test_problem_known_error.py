"""Known-error mark exact wire, journal ambiguity and TUI consent."""
import asyncio
import json

import pytest
from textual.widgets import Input
from typer.testing import CliRunner

from fsv import cli, config, journal, problem_known_error as known
from fsv.errors import APIError
from fsv.resources import PROBLEMS
from fsv.reviewed_write import ReviewedWriteError
from fsv.tui.app import FsvApp
from fsv.tui.modals import ConfirmScreen, OutcomeScreen
from test_tui_integration import offline  # noqa: F401


class Client:
    def __init__(self):
        self.row = {"id": 123, "known_error": False, "subject": "Acme problem"}
        self.writes = []
        self.failure: Exception | None = None

    def int_get(self, path):
        return {"problem": dict(self.row)}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kwargs):
        payload = json.loads(kwargs["content"])
        self.writes.append((method, url, payload))
        if self.failure:
            raise self.failure
        self.row.update(payload)
        return {}


@pytest.fixture(autouse=True)
def setup(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")


def test_exact_mark_and_noop():
    c = Client()
    p = known.build_plan(c, "PRB-123")
    assert p.payload == {"known_error": True} and p.method == "PUT"
    assert known.send(p, c, reviewed_hash=p.hash)["verified"]
    q = known.build_plan(c, 123)
    assert known.send(q, c, reviewed_hash=q.hash)["status"] == "already_in_requested_state"
    assert len(c.writes) == 1


def test_ambiguous_failure_and_stale_review():
    c = Client()
    p = known.build_plan(c, 123)
    c.row["subject"] = "Globex"
    with pytest.raises(ReviewedWriteError, match="changed"):
        known.send(p, c, reviewed_hash=p.hash)
    q = known.build_plan(c, 123)
    c.failure = APIError(403, "uncertain")
    for _ in range(2):
        with pytest.raises(ReviewedWriteError, match="unknown"):
            known.send(q, c, reviewed_hash=q.hash)
    assert len(c.writes) == 1 and journal.read()[-1]["event"] == "started"


def test_cli_hash_required(monkeypatch):
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    args = ["problems", "mark-known-error", "PRB-123"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert result.exit_code == 0, result.output
    assert len(c.writes) == 1


@pytest.mark.parametrize("cancel", [True, False])
def test_tui_known_error_review_and_cancel(offline, monkeypatch, cancel):
    c = Client()
    monkeypatch.setattr("fsv.tui.app.get_client", lambda: c)
    async def go():
        app = FsvApp()
        async with app.run_test(size=(120, 40)) as pilot:
            app._selected = {"id": 9000001001, "display_id": 123, "_resource": PROBLEMS}
            await pilot.press("a")
            app.screen.query_one(Input).value = "known error"
            await pilot.pause()
            await pilot.press("enter")
            await app.workers.wait_for_complete()
            await pilot.pause()
            assert isinstance(app.screen, ConfirmScreen) and not c.writes
            assert '"known_error": true' in app.screen._lines[0]
            await pilot.press("escape" if cancel else "y")
            await app.workers.wait_for_complete()
            await pilot.pause()
            if not cancel:
                assert isinstance(app.screen, OutcomeScreen)
    asyncio.run(go())
    assert len(c.writes) == (0 if cancel else 1)
