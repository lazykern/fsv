"""The ticket list ignores approval_status (206 + meta.search_error, unfiltered
rows): fsv refuses it for tickets, keeps it for changes, and never shows rows
the server marked as a failed search."""
import json

import pytest

from fsv import query, service
from fsv.resources import CHANGES, TICKETS


class Fake:
    def __init__(self, response):
        self.response, self.calls = response, []

    def int_get(self, path, params=None):
        self.calls.append((path, params))
        return self.response


@pytest.mark.parametrize("where", ["approval_status=1", "approval_status!=0"])
def test_ticket_where_approval_status_is_refused_before_any_list_request(monkeypatch, where):
    from fsv import schema
    monkeypatch.setattr(schema, "load", lambda *a, **k: {"fields": [{"name": "approval_status", "label": "Approval Status",
                                                                     "field_type": "default_dropdown", "choices": []}]})
    c = Fake({})
    with pytest.raises(ValueError, match="not a tickets list filter"):
        query.build_query_hash(c, TICKETS, [where])
    assert c.calls == []


@pytest.mark.parametrize("raw", ['[{"condition":"approval_status","operator":"is_in","value":["1"],"type":"default"}]',
                                 '[{"any":[{"condition":"approval_status","operator":"is","value":[1],"type":"default"}]}]'])
def test_ticket_raw_query_hash_with_approval_status_is_refused(raw):
    with pytest.raises(ValueError, match="not a tickets list filter"):
        query.build_query_hash(Fake({}), TICKETS, [], raw=raw)


def test_change_approval_status_filter_is_kept(monkeypatch):
    from fsv import schema
    monkeypatch.setattr(schema, "load", lambda *a, **k: {"fields": []})
    qh, items, _ = query.build_query_hash(Fake({}), CHANGES, ["approval_status=0"])
    assert json.loads(qh) == [{"value": ["0"], "condition": "approval_status", "operator": "is_in", "type": "default"}]
    with pytest.raises(ValueError, match="numeric"):
        query.build_query_hash(Fake({}), CHANGES, ["approval_status=requested"])


@pytest.mark.parametrize("fn", [service.list_page, service.list_items])
def test_search_error_rows_are_never_shown(fn):
    c = Fake({"tickets": [{"id": 1}], "meta": {"search_error": True}})
    with pytest.raises(ValueError, match="search_error"):
        fn(TICKETS, client=c, query_hash='[{"condition":"status","operator":"is_in","value":["2"],"type":"default"}]')
    # without a filter there is nothing to have ignored
    assert fn(TICKETS, client=Fake({"tickets": [{"id": 1}], "meta": {"search_error": True}}))[0]



class Recorder:
    def __init__(self):
        self.calls = []

    def int_get(self, path, params=None):
        self.calls.append((path, params))
        return {}

    def __getattr__(self, name):  # any other client use is a test failure
        raise AssertionError(f"unexpected client call {name}")


def _cli(monkeypatch, *args):
    from typer.testing import CliRunner
    from fsv import cli, schema
    rec = Recorder()
    monkeypatch.setattr(cli, "_client", lambda: rec)
    monkeypatch.setattr(schema, "load", lambda *a, **k: {"fields": [{"name": "status", "label": "Status"}]})
    return CliRunner().invoke(cli.app, list(args)), rec


@pytest.mark.parametrize("value", ["0", "1", "3"])
def test_cli_change_where_approval_status_uses_the_proven_wire(monkeypatch, value):
    res, rec = _cli(monkeypatch, "changes", "ls", "--where", f"approval_status={value}", "--debug")
    assert res.exit_code == 0, res.output
    assert "approval_status" in res.output and "is_in" in res.output and "field not found" not in res.output
    assert rec.calls == []


def test_cli_ticket_where_approval_status_names_the_real_reason(monkeypatch):
    res, rec = _cli(monkeypatch, "tickets", "ls", "--where", "approval_status=1")
    assert res.exit_code == 2 and "not a tickets list filter" in " ".join(res.output.split())
    assert "field not found" not in res.output and rec.calls == []


@pytest.mark.parametrize("raw,needle", [
    ('[{"condition":"status","operator":"is_in","value":2,"type":"default"}]', "takes a JSON list value"),
    ('[{"condition":"status","operator":"is_not_in","value":"2","type":"default"}]', "takes a JSON list value"),
    ('[2]', "must be a JSON object"),
    ('[{"any":[{"condition":"status","operator":"is_in","value":2}]}]', "takes a JSON list value"),
    ('[{"condition":"approval_status","operator":"is_in","value":["1"],"type":"default"}]', "not a tickets list"),
])
def test_cli_raw_query_hash_bad_shapes_refuse_cleanly(monkeypatch, raw, needle):
    res, rec = _cli(monkeypatch, "tickets", "ls", "--query-hash", raw)
    assert res.exit_code == 2, res.output
    assert needle in " ".join(res.output.split()) and "Traceback" not in res.output
    assert not isinstance(res.exception, TypeError) and rec.calls == []


@pytest.mark.parametrize("raw", ['[{"condition":"status","operator":"is_in","value":2,"type":"default"}]', '[2]'])
def test_shared_raw_query_hash_loader_refuses_scalars(raw):
    with pytest.raises(ValueError, match="nothing was sent"):
        query.build_query_hash(Fake({}), TICKETS, [], raw=raw)
