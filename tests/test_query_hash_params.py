from __future__ import annotations

from fsv.resources import CHANGES, PROBLEMS, TICKETS


def test_problem_where_uses_query_hash_only(monkeypatch):
    import fsv.cli as cli

    seen = {}

    class FakeClient:
        def int_get(self, path, params=None):
            seen["path"] = path
            seen["params"] = dict(params or {})
            return {"problems": []}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "_api", lambda fn: fn())
    monkeypatch.setattr("fsv.schema.load", lambda res, c: {"fields": []})
    monkeypatch.setattr(cli, "_build_query_hash", lambda c, res, raw, where, or_grouping: ("qh", [], []))
    monkeypatch.setattr(cli, "_emit_items", lambda *args, **kwargs: None)

    cli.list_resource(
        PROBLEMS,
        filter_name=None,
        where=["status=1"],
        debug=False,
        per_page=10,
        page=1,
        all_pages=False,

        format_="json",
        json_out=True,
    )

    assert seen["path"] == "problems"
    assert seen["params"]["query_hash"] == "qh"
    assert "advanced_query_hash" not in seen["params"]


def test_change_where_keeps_advanced_query_hash_blank(monkeypatch):
    import fsv.cli as cli

    seen = {}

    class FakeClient:
        def int_get(self, path, params=None):
            seen["params"] = dict(params or {})
            return {"changes": []}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "_api", lambda fn: fn())
    monkeypatch.setattr("fsv.schema.load", lambda res, c: {"fields": []})
    monkeypatch.setattr(cli, "_build_query_hash", lambda c, res, raw, where, or_grouping: ("qh", [], []))
    monkeypatch.setattr(cli, "_emit_items", lambda *args, **kwargs: None)

    cli.list_resource(
        CHANGES,
        filter_name=None,
        where=["status=1"],
        debug=False,
        per_page=10,
        page=1,
        all_pages=False,

        format_="json",
        json_out=True,
    )

    assert seen["params"]["query_hash"] == "qh"
    assert seen["params"]["advanced_query_hash"] == ""


def test_ticket_query_hash_supports_order_and_advanced_query_hash(monkeypatch):
    import fsv.cli as cli

    seen = {}

    class FakeClient:
        def int_get(self, path, params=None):
            seen["path"] = path
            seen["params"] = dict(params or {})
            return {"tickets": []}

    monkeypatch.setattr(cli, "_client", lambda: FakeClient())
    monkeypatch.setattr(cli, "_api", lambda fn: fn())
    monkeypatch.setattr("fsv.schema.load", lambda res, c: {"fields": []})
    monkeypatch.setattr(cli, "_build_query_hash", lambda c, res, raw, where, or_grouping: (raw, [], []))
    monkeypatch.setattr(cli, "_emit_items", lambda *args, **kwargs: None)

    cli.list_resource(
        TICKETS,
        filter_name="new_and_my_open",
        where=[],
        debug=False,
        per_page=100,
        page=1,
        all_pages=False,

        format_="json",
        json_out=True,
        raw_query_hash="[{\"condition\":\"requester_id\"}]",
        order_by="created_at",
        order_type=cli.SortOrder.desc,
    )

    assert seen["path"] == "tickets"
    assert seen["params"]["filter"] == "new_and_my_open"
    assert seen["params"]["query_hash"] == "[{\"condition\":\"requester_id\"}]"
    assert seen["params"]["advanced_query_hash"] == ""
    assert seen["params"]["order_by"] == "created_at"
    assert seen["params"]["order_type"] == "desc"
