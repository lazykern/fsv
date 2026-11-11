"""Ticket update previews retain the real or explicitly supplied ticket prefix."""
import json

import pytest
from typer.testing import CliRunner
from fsv import cli


@pytest.mark.parametrize("entered,human,expected", [
    ("SR-11", "SR-11", "SR-11"),
    ("11", "SR-11", "SR-11"),
    ("INC-11", "SR-11", "SR-11"),
    ("SR-11", None, "SR-11"),
    ("11", None, "tickets/11"),
])
def test_update_preview_uses_real_ticket_identity(monkeypatch, entered, human, expected):
    class Client:
        def int_get(self, path, params=None):
            return {"ticket": {"id": 11, "human_display_id": human}}
        def int_put(self, *args, **kwargs):
            pytest.fail("dry run wrote")
    monkeypatch.setattr(cli, "_client", lambda: Client())
    monkeypatch.setattr("fsv.schema.load", lambda *args: {"fields": [
        {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True}]})
    out = CliRunner().invoke(cli.app, ["tickets", "update", entered, "--set", "subject=Acme", "--dry-run", "--json"])
    assert out.exit_code == 0, out.output
    assert json.loads(out.output)["id"] == expected
