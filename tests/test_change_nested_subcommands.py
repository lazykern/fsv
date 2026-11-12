from __future__ import annotations

from typer.testing import CliRunner


runner = CliRunner()


def test_assets_new_ls_subcommand_calls_integrated_inventory(monkeypatch):
    import fsv.cli as cli
    from fsv import integrated_assets
    calls = []
    monkeypatch.setattr(cli, "_client", lambda: None)
    monkeypatch.setattr(integrated_assets, "inventory", lambda *a: calls.append(a) or [])
    monkeypatch.setattr(integrated_assets, "driver", lambda *a: integrated_assets.INTEGRATED)
    result = runner.invoke(cli.app, ["changes", "assets", "ls", "CHN-1", "--json"])
    assert result.exit_code == 0, result.output
    assert calls == [(None, "changes", "CHN-1", "devices")]


def test_assets_legacy_search_routes_to_integrated_inventory(monkeypatch):
    import fsv.cli as cli
    from fsv import integrated_assets
    calls = []
    monkeypatch.setattr(cli, "_client", lambda: None)
    monkeypatch.setattr(integrated_assets, "search", lambda *a: calls.append(a) or {})
    result = runner.invoke(cli.app, ["changes", "assets", "CHN-1", "--search", "AcmeApp", "--json"])
    assert result.exit_code == 0, result.output
    assert "deprecated" in result.output
    assert calls == [(None, "AcmeApp")]


def test_associations_new_add_subcommand_calls_resource(monkeypatch):
    import fsv.cli as cli

    calls = []
    monkeypatch.setattr(cli, "change_associations_resource", lambda *args, **kwargs: calls.append((args, kwargs)))

    result = runner.invoke(cli.app, ["changes", "associations", "add", "CHN-1", "SR-1", "--json"])

    assert result.exit_code == 0
    assert calls == [(("CHN-1", None, ["SR-1"], [], False, False, True), {})]


def test_associations_legacy_syntax_routes_to_hidden_legacy_command(monkeypatch):
    import fsv.cli as cli

    calls = []
    monkeypatch.setattr(cli, "change_associations_resource", lambda *args, **kwargs: calls.append((args, kwargs)))

    result = runner.invoke(cli.app, ["changes", "associations", "CHN-1", "--add", "SR-1", "--dry-run"])

    assert result.exit_code == 0
    assert "deprecated" in result.output
    assert calls == [(("CHN-1", None, ["SR-1"], [], True, False, False, cli.OutputFormat.table), {"pick": False})]


def test_task_update_retires_unproven_due_date(monkeypatch):
    import fsv.cli as cli
    import fsv.create as create
    calls = []
    monkeypatch.setattr(create, "update_task", lambda *a, **k: calls.append(a))
    result = runner.invoke(cli.app, ["changes", "tasks-update", "CHN-1", "17", "--due-date", "2026-12-01T10:00:00Z", "--yes"])
    assert result.exit_code == 2
    assert "not captured" in result.output
    assert not calls
