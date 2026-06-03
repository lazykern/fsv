from __future__ import annotations

import pytest
import typer

from fsv.resources import CHANGES, PROBLEMS, TICKETS


def test_search_dsl_to_where_and():
    import fsv.cli as cli

    where, or_grouping = cli._search_dsl_to_where("priority:3 AND status:2")

    assert where == ["priority=3", "status=2"]
    assert or_grouping is False


def test_search_dsl_to_where_or_and_quotes():
    import fsv.cli as cli

    where, or_grouping = cli._search_dsl_to_where("subject:'demo change' OR status:1")

    assert where == ["subject=demo change", "status=1"]
    assert or_grouping is True


def test_search_dsl_to_where_rejects_mixed_joins():
    import fsv.cli as cli

    with pytest.raises(typer.Exit):
        cli._search_dsl_to_where("status:1 AND priority:2 OR impact:1")


def test_change_filter_delegates_to_internal_list(monkeypatch):
    import fsv.cli as cli

    calls = []

    def fake_list_resource(*args, **kwargs):
        calls.append((args, kwargs))

    monkeypatch.setattr(cli, "list_resource", fake_list_resource)

    cli.filter_resource(CHANGES, "priority:3 AND status:1", 20, 1, "json", True)

    assert calls == [
        (
            (CHANGES,),
            {
                "per_page": 20,
                "page": 1,
                "all_pages": False,
                "filter_name": None,
                "where": ["priority=3", "status=1"],
                "debug": False,
                "v2": False,
                "format_": "json",
                "json_out": True,
                "or_grouping": False,
                "pager": True,
                "n_pages": None,
            },
        )
    ]


def test_problem_filter_delegates_to_internal_list(monkeypatch):
    import fsv.cli as cli

    calls = []

    def fake_list_resource(*args, **kwargs):
        calls.append((args, kwargs))

    monkeypatch.setattr(cli, "list_resource", fake_list_resource)

    cli.filter_resource(PROBLEMS, "status:1", 10, 2, "table", False)

    assert calls[0][0] == (PROBLEMS,)
    assert calls[0][1]["where"] == ["status=1"]
    assert calls[0][1]["page"] == 2


def test_ticket_filter_keeps_v2_filter(monkeypatch):
    import fsv.cli as cli

    calls = []

    class FakeClient:
        def v2_get(self, path, params=None):
            calls.append((path, params))
            return {"tickets": [], "total": 0}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli.schema_mod, "load", lambda res, c: {"fields": []})
    monkeypatch.setattr(cli, "_emit_items", lambda *args, **kwargs: None)

    cli.filter_resource(TICKETS, "status:2", 30, 1, "json", True)

    assert calls == [
        ("tickets/filter", {"query": '"status:2"', "per_page": 30, "page": 1})
    ]


def test_search_resource_passes_sort_to_fulltext_search(monkeypatch):
    import fsv.cli as cli

    calls = []
    emitted = []

    class FakeClient:
        def fulltext_search(self, entity, term, page=1, sort=None):
            calls.append((entity, term, page, sort))
            return {"results": [{"id": 1, "display_id": "CHN-1", "subject": "demo"}], "total_entries": 1}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "emit_json", lambda data: emitted.append(data))

    cli.search_resource(CHANGES, "EDP", 2, True, cli.SearchSort.modified)

    assert calls == [("changes", "EDP", 2, "modified")]
    assert emitted == [[{"id": 1, "display_id": "CHN-1", "subject": "demo"}]]


def test_global_search_passes_sort_to_fulltext_search(monkeypatch):
    import fsv.cli as cli

    calls = []
    emitted = []

    class FakeClient:
        def fulltext_search(self, entity, term, page=1, sort=None):
            calls.append((entity, term, page, sort))
            return {
                "results": [
                    {
                        "result_type": "itil_change",
                        "itil_module_display_id": "CHN-1",
                        "subject": "demo",
                        "itil_module_status": "Open",
                        "itil_module_group": "Platform",
                    }
                ]
            }

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "emit_json", lambda data: emitted.append(data))

    cli.global_search("EDP", 3, "json", True, cli.SearchSort.created)

    assert calls == [("all", "EDP", 3, "created")]
    assert emitted[0][0]["itil_module_display_id"] == "CHN-1"
