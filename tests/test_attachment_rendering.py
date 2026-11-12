"""Attachment-only rich-text lines render once; JSON remains lossless."""
import asyncio
import copy
import datetime as dt
import json

import httpx
import pytest
from rich.text import Text
from typer.testing import CliRunner

import acme_views as W
from fsv import cli, pages, views as V
from fsv.resources import CHANGES


NAME = "Acme_plan_v1.xlsx"
OTHER = "Acme_checklist.txt"


def detail(text, files):
    p = V.PlanningEntry("go_live_checklist", "Acme planning", "", V.html_to_text(text), files)
    return V.Detail(CHANGES, {"id": 501}, {}, {}, dt.timezone.utc, [], planning=[p])


def plain(lines):
    return Text.from_markup("\n".join(lines)).plain


@pytest.mark.parametrize("body", [
    f"<div>Acme rollout</div><div>{NAME}</div>",
    f'<p>Acme rollout</p><p><a href="https://files.example.com/701">{NAME}</a></p>',
    f'<p>Acme rollout</p><ul><li><a href="https://files.example.com/701">{NAME}</a></li></ul>',
    f"Acme rollout\n{NAME}\n{NAME}",
    NAME,
])
def test_planning_plain_link_and_repeated_file_lines_render_once(body):
    d = detail(body, [NAME, OTHER])
    before = copy.deepcopy(d.to_json())
    rendered = plain(pages.main_lines(d))
    assert rendered.count(NAME) == 1 and "📎 " + NAME in rendered
    assert rendered.count(OTHER) == 1 and "📎 " + OTHER in rendered
    if "Acme rollout" in body:
        assert "Acme rollout" in rendered
    assert d.to_json() == before
    assert d.planning[0].text == V.html_to_text(body)


def test_prose_and_unmatched_files_are_never_removed():
    text = f"Review {NAME} before deploying.\nAcme_plan_v1.xlsx.backup\n{NAME}"
    rendered = plain(pages.main_lines(detail(text, [NAME, OTHER])))
    assert f"Review {NAME} before deploying." in rendered
    assert "Acme_plan_v1.xlsx.backup" in rendered
    assert rendered.count("📎 " + NAME) == 1 and rendered.count(OTHER) == 1
    assert pages._attachment_text(NAME, []) == NAME


@pytest.mark.parametrize("body", [f"<div>Acme evidence</div><div>{NAME}</div>",
                                  f'<a href="https://files.example.com/701">{NAME}</a>'])
def test_conversation_uses_same_dedup_without_changing_json(body):
    cv = {"id": 901, "body": body, "source": 2, "private": True, "user": {"name": "Acme Agent"},
          "attachments": [{"id": 701, "name": NAME}, {"id": 702, "name": OTHER}]}
    before = V.conversation_json(cv, dt.timezone.utc)
    rendered = plain(pages.conversation_lines(cv, dt.timezone.utc))
    assert rendered.count(NAME) == 1 and "📎 " + NAME in rendered
    assert rendered.count(OTHER) == 1
    assert V.conversation_json(cv, dt.timezone.utc) == before
    assert "(empty)" not in rendered


def test_main_description_attachments_share_safe_dedup():
    d = detail("", [])
    d.record.update(description=f"<div>Acme evidence</div><div>{NAME}</div>", attachments=[{"id": 701, "name": NAME, "size": 123}])
    before = copy.deepcopy(d.to_json())
    rendered = plain(pages.main_lines(d))
    assert rendered.count(NAME) == 1 and "📎 " + NAME in rendered
    assert "Acme evidence" in rendered and "123 B" in rendered
    assert d.to_json() == before


class FileWorld(W.World):
    def __call__(self, request):
        if request.url.path == "/api/_/changes/501/planning-fields":
            self.requests.append(request)
            return httpx.Response(200, json={"change_planning_fields": [{"name": "go_live_checklist",
                "description": f'<div>Acme rollout</div><div><a href="https://files.example.com/701">{NAME}</a></div>',
                "attachments": [{"id": 701, "name": NAME}, {"id": 702, "name": OTHER}]}]})
        return super().__call__(request)


def test_changes_get_deduplicates_but_json_keeps_source(monkeypatch):
    world = FileWorld("acme")
    client = world.client()
    monkeypatch.setattr(cli, "_client", lambda: client)
    runner = CliRunner()
    result = runner.invoke(cli.app, ["changes", "get", "CHN-501"])
    assert result.exit_code == 0, result.output
    assert result.output.count(NAME) == 1 and "📎 " + NAME in result.output
    assert result.output.count(OTHER) == 1
    result = runner.invoke(cli.app, ["changes", "get", "CHN-501", "--json"])
    assert result.exit_code == 0, result.output
    p = json.loads(result.output)["view"]["details"]["planning"][0]
    assert NAME in p["content"] and p["files"] == [NAME, OTHER]
    assert not world.writes()
    client.close()


def test_tui_change_detail_uses_the_same_deduplicated_renderer(monkeypatch):
    from fsv.tui import app as tui_app
    from fsv.tui.app import FsvApp
    from test_tui_views import settle, text_of
    world = FileWorld("acme")
    client = world.client()
    monkeypatch.setattr(tui_app, "get_client", lambda: client)
    monkeypatch.setattr(tui_app.config, "get_default_where", lambda entity: [])
    async def go():
        app = FsvApp()
        async with app.run_test(size=(170, 50)) as pilot:
            await settle(app, pilot)
            await pilot.press("g", "c")
            await settle(app, pilot, 8)
            rendered = text_of(app.query_one("#detail-content"))
            assert rendered.count(NAME) == 1 and "📎 " + NAME in rendered
            assert rendered.count(OTHER) == 1 and "Acme rollout" in rendered
    asyncio.run(go())
    assert not world.writes()
    client.close()
