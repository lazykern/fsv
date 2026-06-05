from __future__ import annotations

from typer.testing import CliRunner

from fsv.resources import CHANGES


def test_views_command(monkeypatch):
    import fsv.cli as cli

    calls = []
    monkeypatch.setattr(cli, "views_resource", lambda res: calls.append(res.name))

    result = CliRunner().invoke(cli.app, ["changes", "views"])

    assert result.exit_code == 0
    assert calls == ["changes"]


def test_filters_and_filter_commands_are_removed():
    import fsv.cli as cli

    runner = CliRunner()
    filters = runner.invoke(cli.app, ["changes", "filters"])
    filter_ = runner.invoke(cli.app, ["changes", "filter", "status:Open"])

    assert filters.exit_code != 0
    assert filter_.exit_code != 0


def test_resource_help_shows_views_not_filter():
    import fsv.cli as cli

    result = CliRunner().invoke(cli.app, ["changes", "--help"])

    assert result.exit_code == 0
    assert "views" in result.output
    assert "filter" not in result.output.split("views", 1)[0]


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
