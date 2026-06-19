from __future__ import annotations

from typer.testing import CliRunner


runner = CliRunner()


def test_assets_new_ls_subcommand_calls_assets_resource(monkeypatch):
    import fsv.cli as cli

    calls = []
    monkeypatch.setattr(cli, "assets_resource", lambda *args, **kwargs: calls.append((args, kwargs)))

    result = runner.invoke(cli.app, ["changes", "assets", "ls", "CHN-1", "--json"])

    assert result.exit_code == 0
    assert calls == [
        (
            ("CHN-1", None, [], [], 1, 30, False, False, True, cli.OutputFormat.table),
            {"category_name": None},
        )
    ]


def test_assets_legacy_syntax_routes_to_hidden_legacy_command(monkeypatch):
    import fsv.cli as cli

    calls = []
    monkeypatch.setattr(cli, "assets_resource", lambda *args, **kwargs: calls.append((args, kwargs)))

    result = runner.invoke(cli.app, ["changes", "assets", "CHN-1", "--search", "EDP", "--json"])

    assert result.exit_code == 0
    assert "deprecated" in result.output
    assert calls == [
        (
            ("CHN-1", "EDP", [], [], 1, 30, False, False, True, cli.OutputFormat.table),
            {"pick": False, "category_name": None, "list_categories": False},
        )
    ]


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


def test_task_update_uses_command_local_import(monkeypatch):
    import fsv.cli as cli
    import fsv.create as create

    calls = []

    def fake_update_task(*args, **kwargs):
        calls.append((args, kwargs))
        return {"id": args[1], "due_date": "2026-07-21T19:00:00+07:00"}

    monkeypatch.setattr(create, "update_task", fake_update_task)

    result = runner.invoke(
        cli.app,
        ["changes", "tasks-update", "CHN-1", "172231", "--due-date", "2026-07-21T19:00:00+07:00", "--json"],
    )

    assert result.exit_code == 0
    assert calls == [((1, 172231, {"due_date": "2026-07-21T19:00:00+07:00"}), {})]
