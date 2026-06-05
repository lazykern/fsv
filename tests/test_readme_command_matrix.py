from __future__ import annotations

from pathlib import Path


README = Path(__file__).resolve().parents[1] / "README.md"


def _readme_command_matrix() -> dict[str, list[str]]:
    text = README.read_text()
    block = text.split("## Commands", 1)[1].split("```", 2)[1]
    rows: dict[str, list[str]] = {}
    for line in block.splitlines():
        line = line.strip()
        if not line.startswith("fsv "):
            continue
        line = line.split("#", 1)[0].rstrip()
        if "|" not in line:
            continue
        parts = line.split()
        rows[parts[1]] = [part.strip() for part in " ".join(parts[2:]).split("|") if part.strip()]
    return rows


def _registered_group_commands(group_name: str) -> list[str]:
    import fsv.cli as cli

    for group in cli.app.registered_groups:
        if group.name == group_name and group.typer_instance:
            return [cmd.name for cmd in group.typer_instance.registered_commands]
    raise AssertionError(f"group not found: {group_name}")


def test_readme_lists_ticket_commands_added_to_matrix():
    readme = set(_readme_command_matrix()["tickets"])
    actual = set(_registered_group_commands("tickets"))

    for name in ("approvals", "associations", "download", "conversations"):
        assert name in actual
        assert name in readme


def test_readme_lists_problem_download_command():
    readme = set(_readme_command_matrix()["problems"])
    actual = set(_registered_group_commands("problems"))

    assert "download" in actual
    assert "download" in readme
