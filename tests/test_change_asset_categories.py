from __future__ import annotations

import fsv.cli as cli


class _HTMLResponse:
    def __init__(self, text: str):
        self.text = text


class _HTMLClient:
    def __init__(self, html: str):
        self.html = html
        self.calls: list[tuple[str, dict[str, str], bool]] = []
        self._client = self

    def get(self, url, headers=None, follow_redirects=True):
        self.calls.append((url, headers or {}, follow_redirects))
        return _HTMLResponse(self.html)


class _StubClient:
    pass


def test_fetch_asset_categories_parses_cmdb_select(monkeypatch):
    monkeypatch.setattr(cli, "_ASSET_CATEGORY_CACHE", None)
    monkeypatch.setattr(cli.config, "DOMAIN", "fresh.example")
    client = _HTMLClient(
        """
        <html><body>
          <select name="ci_type_id" id="ci_type_id">
            <option value="">  All assets  </option>
            <option value="16000791980">Application Portfolio</option>
            <option value="16000510422"><span>Batch</span></option>
            <option value="16000791980">Application Portfolio</option>
          </select>
        </body></html>
        """
    )

    items = cli._fetch_asset_categories(client)

    assert items == [
        {"name": "All assets", "filter": "all_assets", "ci_type_id": ""},
        {"name": "Application Portfolio", "filter": "16000791980", "ci_type_id": "16000791980"},
        {"name": "Batch", "filter": "16000510422", "ci_type_id": "16000510422"},
    ]
    assert client.calls == [
        (
            "https://fresh.example/cmdb/items",
            {"Accept": "text/html,application/xhtml+xml"},
            True,
        )
    ]


def test_resolve_asset_category_supports_all_alias_and_unique_partial(monkeypatch):
    monkeypatch.setattr(
        cli,
        "_fetch_asset_categories",
        lambda c=None: [
            {"name": "All assets", "filter": "all_assets", "ci_type_id": ""},
            {"name": "Application Portfolio", "filter": "16000791980", "ci_type_id": "16000791980"},
            {"name": "Batch", "filter": "16000510422", "ci_type_id": "16000510422"},
        ],
    )

    assert cli._resolve_asset_category("all") == {"name": "All assets", "filter": "all_assets", "ci_type_id": ""}
    assert cli._resolve_asset_category("application port") == {
        "name": "Application Portfolio",
        "filter": "16000791980",
        "ci_type_id": "16000791980",
    }


def test_assets_resource_search_filters_json_payload_by_category(monkeypatch):
    emitted = []
    monkeypatch.setattr(cli, "_client", lambda: _StubClient())
    monkeypatch.setattr(
        cli,
        "_resolve_asset_category",
        lambda value, c=None: {"name": "Batch", "filter": "16000510422", "ci_type_id": "16000510422"},
    )
    monkeypatch.setattr(
        cli,
        "search_assets_for_change",
        lambda change_id, search, page=1, per_page=30, c=None: {
            "assets": [
                {"display_id": 37451, "name": "EDP_TO_CBO_GROUP2", "ci_type_name": "Batch", "location_name": "--"},
                {"display_id": 38679, "name": "OOS (Object OS)", "ci_type_name": "Application Portfolio", "location_name": "--"},
            ],
            "meta": {"total_count": 2},
        },
    )
    monkeypatch.setattr(cli, "emit_json", emitted.append)

    cli.assets_resource("CHN-1", "EDP", [], [], 1, 30, False, False, True, "json", category_name="Batch")

    assert emitted == [
        {
            "assets": [
                {"display_id": 37451, "name": "EDP_TO_CBO_GROUP2", "ci_type_name": "Batch", "location_name": "--"}
            ],
            "meta": {"total_count": 2, "filtered_count": 1, "category": "Batch"},
        }
    ]


def test_assets_resource_pick_prompts_category_before_search(monkeypatch):
    emitted = []
    prompts = []
    selections = []
    answers = iter(["Application Portfolio", "OOS"])

    monkeypatch.setattr(cli, "_client", lambda: _StubClient())
    monkeypatch.setattr(cli, "_ASSET_CATEGORY_CACHE", None)
    monkeypatch.setattr(cli, "_no_input", lambda no_input=False: False)
    monkeypatch.setattr(cli.sys, "stdin", type("_TTY", (), {"isatty": lambda self: True})())
    monkeypatch.setattr(
        cli,
        "_fetch_asset_categories",
        lambda c=None: [
            {"name": "All assets", "filter": "all_assets", "ci_type_id": ""},
            {"name": "Application Portfolio", "filter": "16000791980", "ci_type_id": "16000791980"},
        ],
    )
    monkeypatch.setattr(
        cli,
        "_prompt_text",
        lambda title, text, default="": (prompts.append((title, text, default)) or next(answers)),
    )
    monkeypatch.setattr(
        cli,
        "_prompt_multi_select",
        lambda title, text, values: (selections.append((title, text, values)) or [values[0][0]]),
    )
    monkeypatch.setattr(
        cli,
        "search_assets_for_change",
        lambda change_id, search, page=1, per_page=50, c=None: {
            "assets": [
                {"display_id": 38679, "name": "OOS (Object OS)", "ci_type_name": "Application Portfolio", "location_name": "--"},
                {"display_id": 37451, "name": "EDP_TO_CBO_GROUP2", "ci_type_name": "Batch", "location_name": "--"},
            ]
        },
    )
    monkeypatch.setattr(cli, "emit_json", emitted.append)

    cli.assets_resource("CHN-1", None, [], [], 1, 30, True, False, False, "json", pick=True)

    assert prompts[0] == ("Asset category", "Category for asset search. Leave empty for All assets.", "All assets")
    assert prompts[1][0] == "Associate assets"
    assert "Category: Application Portfolio" in prompts[1][1]
    assert selections == [
        (
            "Associate assets",
            "Select asset(s) for CHN-1",
            [("38679", "[38679] OOS (Object OS) · Application Portfolio · --")],
        )
    ]
    assert emitted == [{"action": "associate_assets", "change_id": 1, "asset_ids": [38679]}]
