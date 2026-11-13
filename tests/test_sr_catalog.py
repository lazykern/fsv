"""``fsv sr catalog`` / ``fsv sr fields`` against the Acme catalog (mocked
httpx transport behind a real client; no network)."""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import acme_catalog as A
import fsv.change_flow as flow
import fsv.cli_sr as cli_sr
from fsv import catalog as cat

runner = CliRunner()


@pytest.fixture
def fake(monkeypatch):
    f = A.AcmeCatalog()
    c = f.client()
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr(cli_sr.console, "width", 240)  # no table wrapping in assertions
    return f


def run(*args, input=None):
    r = runner.invoke(cli_sr.app, list(args), input=input)
    if r.exception and not isinstance(r.exception, SystemExit):
        raise r.exception
    r.flat = " ".join(r.output.split())
    return r


def test_catalog_lists_every_page_with_name_description_and_id(fake):
    r = run("catalog", "-o", "json")
    assert r.exit_code == 0, r.output
    rows = json.loads(r.output)
    assert {x["display_id"] for x in rows} == {77, 78, 79, 80}  # page 2 fetched too
    fr = next(x for x in rows if x["display_id"] == 77)
    assert fr["name"] == "Feature Request" and fr["short_description"] == "Ask for a product change"
    assert fr["category"] == "Engineering"
    pages = [r.url.params.get("page") for r in fake.requests if r.url.path == "/api/_/catalog/service-items"]
    assert pages == ["1", "2"]
    assert not fake.writes()


def test_catalog_table_and_category_filter_includes_subcategories(fake):
    r = run("catalog", "--category", "engineering")
    assert r.exit_code == 0
    assert "Feature Request" in r.flat and "Build Agent" in r.flat  # sub-category of Engineering
    assert "VPN Access" not in r.flat and "Laptop" not in r.flat
    r = run("catalog", "--category", "7002", "-o", "json")
    assert [x["name"] for x in json.loads(r.output)] == ["VPN Access"]


def test_catalog_search_and_unknown_category(fake):
    r = run("catalog", "--search", "replacement LAPTOP", "-o", "json")
    assert [x["display_id"] for x in json.loads(r.output)] == [79]
    r = run("catalog", "--category", "Plumbing")
    assert r.exit_code == 2
    assert "no category matches 'Plumbing'" in r.flat and "Engineering" in r.flat


def test_catalog_popular_and_categories_only(fake):
    r = run("catalog", "--popular", "-o", "json")
    assert [x["display_id"] for x in json.loads(r.output)] == [78]
    assert any(r.url.params.get("filter") == "frequently_requested" for r in fake.requests)
    r = run("catalog", "--categories", "-o", "json")
    assert [c["name"] for c in json.loads(r.output)][:2] == ["Engineering", "Engineering Tools"]


def test_fields_shows_the_form_like_the_web(fake):
    r = run("fields", "77")
    assert r.exit_code == 0, r.output
    out = r.flat
    for text in ("Feature Request", "Attachments: required: at least one general file", "Attach the overview deck.",
                 "Title", "e.g. Short name", "1. Expected benefit", "reveals: 1.1 Revenue target",
                 "1.1 Revenue target", "(when shown)", "Low - cosmetic, reports", "types: .pptx", "at most 1 file"):
        assert text in out, text


def test_fields_json_has_dependents_choices_and_file_rules(fake):
    r = run("fields", "Feature Request", "-o", "json")
    doc = json.loads(r.output)
    assert doc["item"]["display_id"] == 77
    assert doc["item"]["attachments"] == {"allowed": True, "mandatory": True, "effective_mandatory": True,
                                          "rule": "required: at least one general file"}
    by = {f["name"]: f for f in doc["fields"]}
    assert [f["name"] for f in doc["fields"]][:6] == ["notice", "title_100", "paragraph_101", "paragraph_102",
                                                      "date_103", "goal"]
    goal = by["goal"]
    assert goal["required"] is True
    assert [c["reveals"] for c in goal["choices"]] == [["1.1 Revenue target"], ["1.2 Cost target"], []]
    kid = by["goal_1_kpi"]
    assert kid["visible_at_load"] is False and kid["required"] is True
    assert kid["shown_when"] == {"field": "1. Expected benefit", "choices": ["More revenue  (Total USD)"]}
    deck = by["overview_deck"]
    assert deck["kind"] == "attachment" and deck["allowed_file_types"] == [".pptx"] and deck["max_files"] == 1
    assert by["notice"]["content"] == "Attach the overview deck."


def test_fields_reports_rule_hidden_fields(fake):
    doc = json.loads(run("fields", "78", "-o", "json").output)
    why = {f["name"]: f for f in doc["fields"]}["justification"]
    assert why["visible_at_load"] is False and why["hidden_by_rule"] == "Admin needs a justification"
    assert doc["item"]["attachments"]["rule"] == "not accepted"
    assert doc["rules"] == 1


def test_item_lookup_by_name_part_and_ambiguity(fake):
    assert json.loads(run("fields", "vpn", "-o", "json").output)["item"]["display_id"] == 78
    r = run("fields", "a")  # matches several items
    assert r.exit_code == 2 and "catalog items match 'a'" in r.flat and "Use the item id" in r.flat
    r = run("fields", "Submarine")
    assert r.exit_code == 2 and "no catalog item matches" in r.flat
    r = run("fields", "99")
    assert r.exit_code == 5  # server 404


def test_rules_are_fetched_like_the_web_item_page(fake):
    run("fields", "77", "-o", "json")
    rq = next(r for r in fake.requests if r.url.path.endswith("/business_rules"))
    assert rq.url.path == f"/api/_/ws/{A.WS}/business_rules"
    params = dict(rq.url.params)
    assert params["module_type"] == "service_request" and params["item_id"] == "4400001"
    assert params["cf_name_prefix"] == "customField."


def test_resolve_category_and_filter_units():
    cats = A.CATEGORIES["service_categories"]
    assert [c["id"] for c in cat.resolve_category(cats, "Engineering")] == [7001, 7004]
    with pytest.raises(ValueError, match="several categories"):
        cat.resolve_category(cats, "e")
    got = cat.filter_items(A.ITEMS, search="ci build", categories=cats)
    assert [g["display_id"] for g in got] == [80]


def test_catalog_page_limit_is_reported_not_silently_cut(fake, monkeypatch):
    import functools
    monkeypatch.setattr(cat, "list_items", functools.partial(cat.list_items, max_pages=1))
    r = runner.invoke(cli_sr.app, ["catalog", "-o", "json"])
    assert r.exit_code == 0, r.output
    body = json.loads(r.stdout)
    assert body["truncated"] is True and body["pages_read"] == 1 and "more items exist" in body["notice"]
    assert {x["display_id"] for x in body["items"]} < {77, 78, 79, 80}
    assert "catalog list truncated" in " ".join(r.stderr.split())
    table = runner.invoke(cli_sr.app, ["catalog"])
    assert "catalog list truncated" in " ".join(table.stderr.split())
    with pytest.raises(ValueError, match="catalog list truncated"):
        cat.find_item(fake.client(), "no such item anywhere")


def test_form_fields_explain_dump_diff_for_an_sr_item(fake, monkeypatch, tmp_path):
    import fsv.cli as cli
    c = fake.client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    fields = runner.invoke(cli.app, ["form", "fields", "sr", "--item", "78", "-o", "json"])
    assert fields.exit_code == 0, fields.output
    rows = {r["name"]: r for r in json.loads(fields.stdout)}
    assert rows["access_level"]["required_static"] and rows["access_level"]["choices"] == 3
    choices = runner.invoke(cli.app, ["form", "fields", "sr", "--item", "78", "--choices", "Access level", "-o", "json"])
    assert [ch["label"] for ch in json.loads(choices.stdout)["choices"]] == ["Read only", "Read  write", "Admin"]
    explain = runner.invoke(cli.app, ["form", "explain", "sr", "--item", "78", "--set", "Access level=Admin",
                                      "-o", "json"])
    assert explain.exit_code == 0, explain.output
    why = {r["field"]: r for r in json.loads(explain.stdout)["rows"]}
    assert why["justification"]["visible"] and why["justification"]["why"]  # shown by the item rule
    dump = runner.invoke(cli.app, ["form", "dump", "sr", "--item", "78"])
    snap = json.loads(dump.stdout)
    assert snap["module"] == "service_request" and snap["item"] == 78
    assert {f["name"] for f in snap["fields"]} >= {"reason_200", "access_level", "justification"}
    path = tmp_path / "snap.json"
    path.write_text(dump.stdout)
    same = runner.invoke(cli.app, ["form", "diff", str(path), "-o", "json"])
    assert same.exit_code == 0 and json.loads(same.stdout) == []
    snap["fields"] = [f for f in snap["fields"] if f["name"] != "justification"]
    path.write_text(json.dumps(snap))
    changed = json.loads(runner.invoke(cli.app, ["form", "diff", str(path), "-o", "json"]).stdout)
    assert [(d["change"], d["name"]) for d in changed] == [("added", "justification")]
    assert runner.invoke(cli.app, ["form", "fields", "sr"]).exit_code == 2  # --item is required
    assert runner.invoke(cli.app, ["form", "explain", "sr", "--item", "78", "--id", "SR-1"]).exit_code == 2
    assert not fake.writes()
