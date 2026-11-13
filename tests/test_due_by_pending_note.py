"""Due By list filters say that Pending tickets carry no due date (offline)."""
import pytest
from typer.testing import CliRunner

from fsv import cli
from test_views_cli import _world

NOTE = "Pending tickets have no Due By"


@pytest.mark.parametrize("tenant", ["acme", "globex"])
def test_due_by_filter_without_status_prints_one_stderr_note(monkeypatch, tenant):
    world = _world(monkeypatch, tenant)
    r = CliRunner().invoke(cli.app, ["tickets", "ls", "--where", "due_by>=2030-01-01",
                                     "--where", "due_by<=2030-02-01", "--json"])
    assert r.exit_code == 0, r.output
    assert r.stderr.count(NOTE) == 1 and NOTE not in r.stdout
    assert not world.writes()


@pytest.mark.parametrize("args", [
    ["tickets", "ls", "--where", "due_by>=2030-01-01", "--where", "due_by<=2030-02-01", "--where", "status=Open"],
    ["tickets", "ls", "--where", "created_at>=2030-01-01", "--where", "created_at<=2030-02-01"],
    ["changes", "ls", "--where", "created_at>=2030-01-01", "--where", "created_at<=2030-02-01"],
])
def test_no_note_with_status_filter_other_fields_or_other_modules(monkeypatch, args):
    _world(monkeypatch, "acme")
    r = CliRunner().invoke(cli.app, args + ["--json"])
    assert r.exit_code == 0, r.output
    assert NOTE not in r.stderr


def test_due_by_predicate_detection():
    assert cli._due_by_without_status([{"condition": "due_by_date"}])
    assert not cli._due_by_without_status([{"condition": "due_by"}, {"condition": "status"}])
    assert not cli._due_by_without_status([{"condition": "fr_due_by"}])
    assert not cli._due_by_without_status(None)
