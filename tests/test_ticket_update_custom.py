"""Ticket update separates metadata-declared custom fields from core keys."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli

SCHEMA = {"fields": [
    {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True},
    {"name": "acme_code", "label": "Account code", "field_type": "custom_text", "default_field": False},
    {"name": "acme_type", "label": "Request type", "field_type": "custom_dropdown", "default_field": False,
     "choices": [{"id": 9000001001, "value": "Standard"}]},
]}


class Client:
    def __init__(self):
        self.writes = []

    def int_get(self, path, params=None):
        return {"ticket": {"id": 11, "subject": "Before", "custom_fields": {}}}

    def int_put(self, path, body=None):
        self.writes.append((path, body))
        return {"ticket": {"id": 11}}


@pytest.mark.parametrize("prefix", ["INC", "SR"])
def test_ticket_custom_fields_envelope_dry_run_and_send(monkeypatch, prefix):
    client = Client()
    monkeypatch.setattr(cli, "_client", lambda: client)
    monkeypatch.setattr("fsv.schema.load", lambda *args: SCHEMA)
    args = ["tickets", "update", f"{prefix}-11", "--set", "Subject=After", "--set", "Account code=A1",
            "--set", "Request type=Standard"]
    dry = CliRunner().invoke(cli.app, [*args, "--dry-run", "--json"])
    assert dry.exit_code == 0, dry.output
    expected = {"subject": "After", "custom_fields": {"acme_code": "A1", "acme_type": "Standard"}}
    assert json.loads(dry.output)["body"] == expected
    assert client.writes == []
    send = CliRunner().invoke(cli.app, [*args, "--yes", "--json"])
    assert send.exit_code == 0, send.output
    assert client.writes == [("tickets/11", expected)]
