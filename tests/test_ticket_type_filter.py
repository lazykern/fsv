"""Ticket type list filters use the tenant's name form, never a numeric choice ID."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, filter_proof, query
from fsv.resources import TICKETS
from test_views_cli import _world

SCHEMA = {"fields": [{"name": "ticket_type", "label": "Type", "default_field": True,
                      "field_type": "default_ticket_type",
                      "choices": [{"id": 3, "value": "Incident"}, {"id": 4, "value": "Service Request"},
                                  {"id": 5, "value": "Retired", "deleted": True}]}]}


@pytest.mark.parametrize("expr,operator,value", [
    ("type=Service Request", "is_in", "Service Request"), ("Type=service request", "is_in", "Service Request"),
    ("ticket_type=4", "is_in", "Service Request"), ("type!=Incident", "is_not_in", "Incident")])
def test_shared_builder_sends_choice_name(expr, operator, value):
    payload = json.loads(query.build_query_hash_from_schema(None, TICKETS, SCHEMA, [expr]))
    assert payload == [{"condition": "ticket_type", "operator": operator, "type": "default", "value": [value]}]


@pytest.mark.parametrize("expr", ["type=Case", "type=5", "type=Retired", "type=9"])
def test_unknown_or_inactive_type_fails_closed(expr):
    with pytest.raises(ValueError, match="no unique runtime choice"):
        query.build_query_hash_from_schema(None, TICKETS, SCHEMA, [expr])


def test_duplicate_type_names_are_ambiguous():
    field = {**SCHEMA["fields"][0], "choices": [{"id": 3, "value": "Incident"}, {"id": 6, "value": "incident"}]}
    assert filter_proof.name_valued_choice(field, "Incident") == "Incident"  # same folded name
    field["choices"].append({"id": 7, "value": "Request"})
    with pytest.raises(ValueError):
        filter_proof.name_valued_choice(field, "Case")


@pytest.mark.parametrize("raw", ["Service Request", "7402"])
def test_cli_list_sends_name_form(monkeypatch, raw):
    world = _world(monkeypatch, "acme")
    result = CliRunner().invoke(cli.app, ["tickets", "ls", "--where", f"type={raw}", "--json"])
    assert result.exit_code == 0, result.output
    sent = json.loads(world.calls("/api/_/tickets")[-1]["query_hash"])
    assert sent == [{"condition": "ticket_type", "operator": "is_in", "type": "default", "value": ["Service Request"]}]
    assert not world.writes()


def test_cli_unknown_type_never_lists(monkeypatch):
    world = _world(monkeypatch, "acme")
    result = CliRunner().invoke(cli.app, ["tickets", "ls", "--where", "type=Case", "--json"])
    assert result.exit_code == 2, result.output
    assert not world.calls("/api/_/tickets")


@pytest.mark.parametrize("row,expected", [
    ({"type": "Service Request"}, True), ({"ticket_type": "Service Request"}, True),
    ({"type": "Incident"}, False), ({"type": "Service Request", "ticket_type": "Incident"}, None), ({}, None)])
def test_independent_proof_reads_native_type(row, expected):
    clause = [{"condition": "ticket_type", "operator": "is_in", "type": "default", "value": ["Service Request"]}]
    if expected is None:
        with pytest.raises(ValueError, match="unavailable"):
            filter_proof.matches(row, clause)
    else:
        assert filter_proof.matches(row, clause) is expected


def test_filtered_bulk_by_type_still_requires_independent_proof(monkeypatch):
    world = _world(monkeypatch, "acme")
    result = CliRunner().invoke(cli.app, ["tickets", "bulk", "spam", "--where", "type=Service Request", "--select-all",
                                          "--max-records", "50", "--confirm-destructive", "--dry-run", "--json"])
    assert result.exit_code == 2, result.output
    sent = json.loads(world.calls("/api/_/tickets")[-1]["query_hash"])
    assert sent[0]["value"] == ["Service Request"]
    assert not world.writes()
