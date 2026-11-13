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

    assert result.exit_code == 2
    assert "choose schema|views|groups|all" in result.output


def test_cache_clear_accepts_legacy_filters_alias():
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["cache", "clear", "filters"])

    assert result.exit_code == 0



def test_cache_clear_tolerates_a_directory_removed_by_a_parallel_process(tmp_path, monkeypatch):
    """Live race: another fsv process removed the schema dir mid-clear."""
    import shutil

    import fsv.cli as cli
    from fsv import config

    schema = tmp_path / "schema"
    (schema / "sub").mkdir(parents=True)
    (schema / "sub" / "a.json").write_text("{}")
    monkeypatch.setattr(config, "SCHEMA_DIR", schema)
    monkeypatch.setattr(config, "CONFIG_DIR", tmp_path / "config")
    monkeypatch.setattr(config, "groups_cache_candidates", lambda: [tmp_path / "groups.json"])
    (tmp_path / "groups.json").write_text("[]")
    real = shutil.rmtree

    def racing_rmtree(path, *args, **kwargs):
        real(path)  # the other process wins the race
        real(path, *args, **kwargs)  # our walk then meets a vanished tree
    monkeypatch.setattr(shutil, "rmtree", racing_rmtree)
    result = CliRunner().invoke(cli.app, ["cache", "clear", "all"])
    assert result.exit_code == 0, result.output
    assert result.exception is None
    assert not schema.exists() and not (tmp_path / "groups.json").exists()
    assert "schema cache cleared" in result.output


def test_cache_clear_still_reports_real_errors(tmp_path, monkeypatch):
    import shutil

    import fsv.cli as cli
    from fsv import config

    schema = tmp_path / "schema"
    schema.mkdir()
    monkeypatch.setattr(config, "SCHEMA_DIR", schema)

    def denied(path, *args, onexc=None, **kwargs):
        onexc(None, str(path), PermissionError(13, "denied"))
    monkeypatch.setattr(shutil, "rmtree", denied)
    result = CliRunner().invoke(cli.app, ["cache", "clear", "schema"])
    assert isinstance(result.exception, PermissionError)
