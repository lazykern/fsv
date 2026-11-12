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
