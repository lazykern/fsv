from __future__ import annotations

from typer.testing import CliRunner


def test_cache_clear_help_uses_views():
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["cache", "clear", "--help"])

    assert result.exit_code == 0
    assert "schema|views|groups|all" in result.output
    assert "schema|filters|groups|all" not in result.output


def test_cache_clear_rejects_invalid_target():
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["cache", "clear", "bogus"])

    assert result.exit_code == 1
    assert "choose schema|views|groups|all" in result.output


def test_cache_clear_accepts_legacy_filters_alias():
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["cache", "clear", "filters"])

    assert result.exit_code == 0
