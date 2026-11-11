from __future__ import annotations

from pathlib import Path

from typer.testing import CliRunner


README = Path(__file__).resolve().parents[1] / "README.md"


def _documented_group_help(group_name: str) -> str:
    """The README points to module help instead of duplicating a command matrix."""
    import fsv.cli as cli

    assert f"fsv {group_name} --help" in README.read_text()
    result = CliRunner().invoke(cli.app, [group_name, "--help"], color=False)
    assert result.exit_code == 0, result.output
    return result.output


def test_readme_ticket_help_exposes_detail_commands():
    help_text = _documented_group_help("tickets")
    for name in ("approvals", "associations", "download", "conversations", "create"):
        assert name in help_text
    text = README.read_text()
    assert "fsv tickets create" in text and "docs/incidents.md" in text
    assert "Live incident creation has not yet been tested" in text


def test_readme_problem_help_exposes_download():
    assert "download" in _documented_group_help("problems")
