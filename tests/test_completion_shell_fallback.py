from __future__ import annotations

from typer.testing import CliRunner


def test_shell_from_env_detects_supported_shell(monkeypatch):
    import fsv.cli as cli

    monkeypatch.setenv("SHELL", "/opt/homebrew/bin/fish")

    assert cli._shell_from_env() == "fish"


def test_shell_from_env_maps_sh_to_bash(monkeypatch):
    import fsv.cli as cli

    monkeypatch.setenv("SHELL", "/bin/sh")

    assert cli._shell_from_env() == "bash"


def test_top_level_show_completion_uses_shell_env_fallback(monkeypatch):
    import fsv.cli as cli
    import typer._completion_shared as shared

    monkeypatch.setenv("SHELL", "/opt/homebrew/bin/fish")
    monkeypatch.setattr(shared, "_fsv_original_get_shell_name", lambda: None, raising=False)
    cli._patch_typer_shell_detection(force=True)

    result = CliRunner().invoke(cli.app, ["--show-completion"])

    assert result.exit_code == 0, result.output
    assert "complete_fish" in result.output
    assert "--no-files" in result.output
