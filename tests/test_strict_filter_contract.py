"""Strict grammar and no broadened/unproven list-filter operators."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, query
from fsv.resources import CHANGES, PROBLEMS, TICKETS
from test_views_cli import _world


@pytest.mark.parametrize("expr", ["status==Open", "status=>Open", "status=<Open", "status<>Open", "status!==Open",
                                  "status~~=Open", "status!>Open", "status=", "=Open", "status='Open", "status=Open\npriority=High"])
def test_malformed_operator_is_never_repaired(expr):
    with pytest.raises(ValueError, match="invalid filter"):
        query._parse_where(expr)
    import typer
    with pytest.raises(typer.Exit):
        cli._parse_where(expr)


@pytest.mark.parametrize("expr,expected", [
    (" Subject = '=Acme' ", ("Subject", "=", "=Acme")),
    ('Subject="Acme >= Globex"', ("Subject", "=", "Acme >= Globex")),
    ("Subject=''", ("Subject", "=", "")),
    ("status!=Open", ("status", "!=", "Open")),
])
def test_quoted_literal_values_are_preserved(expr, expected):
    assert query._parse_where(expr) == expected
    assert cli._parse_where(expr) == expected


@pytest.mark.parametrize("op", ["", "==", "contains", "gte", "is_greater_than", None])
def test_unknown_internal_operator_has_no_equality_fallback(op):
    with pytest.raises(ValueError, match="no equality fallback"):
        query._where_operator(op)
    import typer
    with pytest.raises(typer.Exit):
        cli._where_operator(op)


@pytest.mark.parametrize("resource", [CHANGES, TICKETS, PROBLEMS])
@pytest.mark.parametrize("expr", ["created_at>2030-01-01", "created_at<2030-01-01", "Subject~=Acme", "Count>12", "status~=Open"])
def test_shared_builder_blocks_unproven_strict_and_contains(resource, expr):
    schema = {"fields": [{"name": "subject", "label": "Subject", "field_type": "default_text", "default_field": True},
                         {"name": "count", "label": "Count", "field_type": "custom_number"},
                         {"name": "status", "field_type": "default_status", "default_field": True}]}
    with pytest.raises(ValueError):
        query.build_query_hash_from_schema(None, resource, schema, [expr])


@pytest.mark.parametrize("tenant", ["acme", "globex"])
@pytest.mark.parametrize("expr", ["Created>2030-01-01", "created_at<2030-01-01", "Subject~=Acme", "status==Open", "Created<=Last 6 months"])
def test_cli_never_sends_broadened_list_request(monkeypatch, tenant, expr):
    world = _world(monkeypatch, tenant)
    result = CliRunner().invoke(cli.app, ["changes", "ls", "--where", expr, "--json"])
    assert result.exit_code == 2, result.output
    assert not world.calls("/api/_/changes")
    assert not world.writes()


@pytest.mark.parametrize("tenant", ["acme", "globex"])
def test_supported_ranges_and_period_tokens_use_real_list_query_hash(monkeypatch, tenant):
    world = _world(monkeypatch, tenant)
    result = CliRunner().invoke(cli.app, ["changes", "ls", "--where", "created_at>=2030-01-01", "--where", "created_at<=2030-02-01", "--json"])
    assert result.exit_code == 0, result.output
    items = json.loads(world.calls("/api/_/changes")[-1]["query_hash"])
    assert items == [{"value": {"from": "2030-01-01", "to": "2030-02-01"}, "condition": "created_at", "operator": "is_in_the_range", "type": "default"}]
    result = CliRunner().invoke(cli.app, ["changes", "ls", "--where", "Created=Last 6 months", "--json"])
    assert result.exit_code == 0, result.output
    items = json.loads(world.calls("/api/_/changes")[-1]["query_hash"])
    assert items == [{"value": "six_months", "condition": "created_at", "operator": "is_greater_than", "type": "default"}]
    assert not world.writes()

@pytest.mark.parametrize("resource", [CHANGES, TICKETS, PROBLEMS])
def test_subject_equality_has_no_guessed_choice_operator(resource):
    fields = {"fields": [{"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True}]}
    with pytest.raises(ValueError, match="no list filter exists"):
        query.build_query_hash_from_schema(None, resource, fields, ["Subject=Acme"])


def test_read_warns_when_fake_server_ignores_status_filter(monkeypatch):
    world = _world(monkeypatch, "acme")
    result = CliRunner().invoke(cli.app, ["changes", "ls", "--where", "status=Closed", "--json"])
    assert result.exit_code == 0, result.output
    assert "contradict the requested predicate" in result.stderr
    assert isinstance(json.loads(result.stdout), list)
    assert not world.writes()

