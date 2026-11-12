"""Synthetic create/edit/clear boundaries and single-attempt transport."""
import copy
import json
from dataclasses import replace

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, journal, planning_text as planning
from fsv.client import Client as HttpClient
from fsv.reviewed_write import ReviewedWriteError
from test_planning_text import Client, setup  # noqa: F401


@pytest.fixture(params=[("changes", "Rollout Plan", "acme_plan"),
                        ("problems", "Root Cause", "problem_cause")])
def target(request):
    return request.param


@pytest.mark.parametrize("shape", ["description", "content", "description_html", "description_text", "body"])
def test_known_empty_create_review_and_readback(target, shape):
    module, section, key = target
    c = Client(module)
    c.rows = [{"name": key, shape: "", "files": [], "empty": True}]
    p = planning.build_plan(c, module, 123, section, "Acme replacement")
    review = p.review()
    assert review["operation"] == "create"
    assert review["before"] == "" and review["after"] == "Acme replacement"
    assert review["method"] == "POST"
    assert review["payload"] == {"description": "Acme replacement"}
    assert replace(p, before="other baseline").hash != p.hash
    assert planning.send(p, c, reviewed_hash=p.hash)["verified"]
    assert len(c.writes) == 1


@pytest.mark.parametrize("shape", ["description", "content", "description_html", "description_text", "body"])
@pytest.mark.parametrize("stored", ["Acme original", " ", "<div><br></div>"])
def test_existing_content_uses_edit_without_post(target, shape, stored):
    module, section, key = target
    c = Client(module)
    c.rows = [{"name": key, shape: stored, "files": []}]
    before = copy.deepcopy(c.rows)
    plan = planning.build_plan(c, module, 123, section, "Acme replacement")
    assert plan.method == "PUT" and plan.path.endswith("/planning-fields/" + key)
    assert plan.review()["before"] == stored
    assert plan.review()["operation"] == "edit"
    assert c.rows == before and not c.writes and not journal.read()
    assert planning.send(plan, c, reviewed_hash=plan.hash)["verified"]
    assert c.writes[0][0] == "PUT"


@pytest.mark.parametrize("fields", [
    {"content": None}, {"content": 42}, {}, {"empty": True},
    {"description": "", "content": "Acme original"},
    {"description": "", "content": 42}, {"description": "", "content": None},
    {"content": "", "empty": False},
])
def test_unknown_or_conflicting_text_state_refused(target, fields):
    module, section, key = target
    c = Client(module)
    c.rows = [{"name": key, "files": [], **fields}]
    with pytest.raises(ReviewedWriteError):
        planning.build_plan(c, module, 123, section, "Acme replacement")
    assert not c.writes


@pytest.mark.parametrize("files", [{}, {"files": None}, {"attachments": [], "files": [{"id": 456}]}])
def test_files_must_all_be_explicitly_empty(files):
    c = Client()
    c.rows = [{"name": "acme_plan", "content": "", **files}]
    with pytest.raises(ReviewedWriteError, match="attachments"):
        planning.build_plan(c, "changes", 123, "Rollout Plan", "Acme")
    assert not c.writes


def test_get_race_to_existing_content_never_posts(target):
    module, section, key = target
    c = Client(module)
    p = planning.build_plan(c, module, 123, section, "Acme")
    c.rows = [{"name": key, "content": "Someone else's update", "files": []}]
    with pytest.raises(ReviewedWriteError, match="changed since review"):
        planning.send(p, c, reviewed_hash=p.hash)
    assert not c.writes and not journal.read()


@pytest.mark.parametrize("flags", [["--dry-run"], ["--yes", "--expect-plan", "0" * 64]])
def test_cli_existing_content_review_or_unreviewed_refusal(monkeypatch, target, flags):
    module, section, key = target
    c = Client(module)
    c.rows = [{"name": key, "content": "Acme original", "files": []}]
    monkeypatch.setattr(cli, "_client", lambda: c)
    result = CliRunner().invoke(cli.app, [module, "planning-text", "123", "--section", section,
                                       "--description", "Acme", *flags])
    if flags == ["--dry-run"]:
        assert result.exit_code == 0, result.output
        assert json.loads(result.output)["operation"] == "edit"
    else:
        assert result.exit_code == 2, result.output
        assert "full --expect-plan hash" in result.output
    assert "Traceback" not in result.output
    assert not c.writes


@pytest.mark.parametrize("failure", [403, 429, "timeout"])
def test_real_transport_single_attempt_and_duplicate_guard(monkeypatch, failure):
    seen = []

    def handle(request):
        seen.append(request)
        if request.method == "GET":
            return httpx.Response(200, json={"change_planning_fields": []})
        assert request.method == "POST"
        assert request.url.path == "/api/_/changes/123/planning-fields"
        assert request.url.query == b"id=acme_plan"
        assert json.loads(request.content) == {"description": "Acme"}
        if failure == "timeout":
            raise httpx.ReadTimeout("synthetic timeout", request=request)
        return httpx.Response(failure, json={"message": "synthetic rejection"})

    c = HttpClient(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    try:
        p = planning.build_plan(c, "changes", 123, "Rollout Plan", "Acme")
        for _ in range(2):
            with pytest.raises(ReviewedWriteError, match="unknown"):
                planning.send(p, c, reviewed_hash=p.hash)
        assert len([r for r in seen if r.method == "POST"]) == 1
    finally:
        c.close()
