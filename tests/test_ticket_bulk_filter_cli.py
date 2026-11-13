"""CLI-level filtered bulk safety through the real list-filter builder."""
import pytest
from typer.testing import CliRunner

from fsv import cli
from test_views_cli import _world


@pytest.mark.parametrize("tenant", ["acme", "globex"])
@pytest.mark.parametrize("action,extra", [("status", ["--status", "Open"]), ("spam", ["--confirm-destructive"]),
                                          ("delete", ["--confirm-destructive"])])
@pytest.mark.parametrize("send", [["--dry-run"], ["--yes", "--expect-plan", "0" * 64]])
def test_free_text_subject_filter_fails_closed_before_listing(monkeypatch, tenant, action, extra, send):
    world = _world(monkeypatch, tenant)
    result = CliRunner().invoke(cli.app, ["tickets", "bulk", action, "--where", "Subject=Acme rollout",
                                          "--select-all", "--max-records", "5", *extra, *send, "--json"])
    assert result.exit_code == 2, result.output
    assert "no list filter exists" in result.output
    assert not world.calls("/api/_/tickets")
    assert not world.writes()


@pytest.mark.parametrize("bound", [[], ["--max-records", "0"], ["--max-records", "51"]])
def test_filtered_bulk_requires_an_explicit_bound(monkeypatch, bound):
    world = _world(monkeypatch, "acme")
    result = CliRunner().invoke(cli.app, ["tickets", "bulk", "spam", "--where", "status=Open", "--select-all",
                                          *bound, "--confirm-destructive", "--dry-run", "--json"])
    assert result.exit_code == 2, result.output
    assert "--max-records" in result.output
    assert not world.calls("/api/_/tickets")
    assert not world.writes()


@pytest.mark.parametrize("tenant,label", [("acme", "Closed"), ("globex", "Gelöst")])
def test_world_server_ignoring_choice_filter_refuses_whole_plan(monkeypatch, tenant, label):
    world = _world(monkeypatch, tenant)
    result = CliRunner().invoke(cli.app, ["tickets", "bulk", "spam", "--where", f"status={label}", "--select-all",
                                          "--max-records", "50", "--confirm-destructive", "--dry-run", "--json"])
    assert result.exit_code == 2, result.output
    assert "contradicts the predicate" in result.output
    assert world.calls("/api/_/tickets")
    assert not world.writes()
