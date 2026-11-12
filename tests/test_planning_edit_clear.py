"""Synthetic accepted edit/delete contracts; never contact a tenant."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, planning_text as planning
from fsv.client import Client as HttpClient
from fsv.reviewed_write import ReviewedWriteError
from test_planning_text import Client, setup  # noqa: F401


@pytest.fixture(params=[("changes", "Rollout Plan", "acme_plan"),
                        ("problems", "Root Cause", "problem_cause")])
def target(request):
    return request.param


def populated(c, key):
    c.rows = [{"id": 987, "name": key, "description": "<div>Acme original</div>",
               "description_text": "Acme original", "attachments": []}]


@pytest.mark.parametrize("clear", [False, True])
def test_exact_wire_review_and_independent_get(target, clear):
    module, section, key = target
    c = Client(module)
    populated(c, key)
    p = planning.build_plan(c, module, 123, section, None if clear else "Acme new", clear=clear)
    review = p.review()
    assert review["before"] == "<div>Acme original</div>"
    assert review["after"] == ("" if clear else "Acme new")
    assert review["operation"] == ("clear" if clear else "edit")
    assert p.method == ("DELETE" if clear else "PUT")
    assert p.path == f"{module}/123/planning-fields/{key}"
    assert p.payload == (None if clear else {"description": "Acme new"})
    assert planning.send(p, c, reviewed_hash=None)["status"] == "cancelled"
    assert not c.writes
    assert planning.send(p, c, reviewed_hash=p.hash)["verified"]
    assert len(c.writes) == 1
    with pytest.raises(ReviewedWriteError, match="was sent"):
        planning.send(p, c, reviewed_hash=p.hash)


@pytest.mark.parametrize("clear", [False, True])
@pytest.mark.parametrize("race", ["text", "files", "removed"])
def test_review_race_blocks_all_mutations(target, clear, race):
    module, section, key = target
    c = Client(module)
    populated(c, key)
    p = planning.build_plan(c, module, 123, section, None if clear else "Acme new", clear=clear)
    if race == "removed":
        c.rows = []
    elif race == "files":
        c.rows[0]["attachments"] = [{"id": 321}]
    else:
        c.rows[0]["description"] = "Changed by someone else"
        c.rows[0]["description_text"] = "Changed by someone else"
    with pytest.raises(ReviewedWriteError):
        planning.send(p, c, reviewed_hash=p.hash)
    assert not c.writes


@pytest.mark.parametrize("after,expected", [
    ([], True),
    ([{"name": "acme_plan", "content": "", "files": [], "empty": True}], True),
    ([{"name": "acme_plan", "content": "<div><br></div>", "files": []}], False),
    ([{"name": "acme_plan", "content": "", "files": None}], False),
    ([{"name": "acme_plan", "content": "", "files": [], "empty": False}], False),
    ([{"name": "acme_plan", "content": "", "files": [], "id": 987}], False),
    ([{"name": "acme_plan", "files": []}], False),
])
def test_delete_response_never_substitutes_for_readback(after, expected):
    class ReadbackClient(Client):
        def _request(self, *a, **kw):
            super()._request(*a, **kw)
            self.rows = copy.deepcopy(after)
            return {"success": True}
    c = ReadbackClient()
    populated(c, "acme_plan")
    p = planning.build_plan(c, "changes", 123, "Rollout Plan", clear=True)
    result = planning.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] is expected
    if not expected:
        assert "do not resend" in result["warning"]


@pytest.mark.parametrize("clear", [False, True])
@pytest.mark.parametrize("failure", [403, 429, "timeout"])
def test_real_transport_edit_and_clear_single_attempt(monkeypatch, target, clear, failure):
    module, section, key = target
    seen = []
    def handle(request):
        seen.append(request)
        if request.method == "GET":
            return httpx.Response(200, json={
                "change_planning_fields" if module == "changes" else "problem_planning_fields": [
                    {"id": 987, "name": key, "description": "Acme original", "attachments": []}]})
        assert request.method == ("DELETE" if clear else "PUT")
        assert request.url.path == f"/api/_/{module}/123/planning-fields/{key}"
        assert request.url.query == b""
        assert request.content == (b"" if clear else b'{"description": "Acme new"}')
        if failure == "timeout":
            raise httpx.ReadTimeout("synthetic timeout", request=request)
        return httpx.Response(failure, json={"message": "synthetic rejection"})
    c = HttpClient(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    try:
        p = planning.build_plan(c, module, 123, section, None if clear else "Acme new", clear=clear)
        for _ in range(2):
            with pytest.raises(ReviewedWriteError, match="unknown"):
                planning.send(p, c, reviewed_hash=p.hash)
        assert len([r for r in seen if r.method != "GET"]) == 1
    finally:
        c.close()


@pytest.mark.parametrize("clear", [False, True])
def test_cli_edit_clear_review_hash_and_send(monkeypatch, target, clear):
    module, section, key = target
    c = Client(module)
    populated(c, key)
    monkeypatch.setattr(cli, "_client", lambda: c)
    args = [module, "planning-text", "123", "--section", section]
    args += ["--clear"] if clear else ["--description", "Acme new"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    assert not c.writes
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["verified"]
    assert len(c.writes) == 1


@pytest.mark.parametrize("flags", [[], ["--clear", "--description", "Acme"], ["--description", ""]])
def test_cli_missing_or_conflicting_operation_refused(monkeypatch, flags):
    c = Client()
    populated(c, "acme_plan")
    monkeypatch.setattr(cli, "_client", lambda: c)
    result = CliRunner().invoke(cli.app, ["changes", "planning-text", "123", "--section", "Rollout Plan",
                                        "--dry-run", *flags])
    assert result.exit_code == 2 and "Traceback" not in result.output
    assert not c.writes


@pytest.mark.parametrize("rows", [[], [{"name": "acme_plan", "description": "", "attachments": []}],
                                  [{"id": 987, "name": "acme_plan", "description": "", "attachments": []}]])
def test_clear_empty_or_unknown_stored_state_is_not_a_delete(rows):
    c = Client()
    c.rows = rows
    with pytest.raises(ReviewedWriteError):
        planning.build_plan(c, "changes", 123, "Rollout Plan", clear=True)
    assert not c.writes


def test_empty_stored_row_does_not_guess_create():
    c = Client()
    c.rows = [{"id": 987, "name": "acme_plan", "description": "", "attachments": []}]
    with pytest.raises(ReviewedWriteError, match="unknown create/edit state"):
        planning.build_plan(c, "changes", 123, "Rollout Plan", "Acme")


@pytest.mark.parametrize("clear", [False, True])
def test_real_transport_accepted_envelope_then_fresh_get(monkeypatch, target, clear):
    module, section, key = target
    rows = [{"id": 987, "name": key, "description": "Acme original", "attachments": []}]
    methods = []
    def handle(request):
        methods.append(request.method)
        envelope = "change_planning_fields" if module == "changes" else "problem_planning_fields"
        if request.method == "GET":
            return httpx.Response(200, json={envelope: copy.deepcopy(rows)})
        assert request.method == ("DELETE" if clear else "PUT")
        if clear:
            assert request.content == b""
            rows.clear()
            return httpx.Response(204)
        rows[0]["description"] = "<div>Acme new</div>"
        return httpx.Response(200, json={envelope.removesuffix("s"): copy.deepcopy(rows[0])})
    c = HttpClient(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
    try:
        p = planning.build_plan(c, module, 123, section, None if clear else "Acme new", clear=clear)
        assert planning.send(p, c, reviewed_hash=p.hash)["verified"]
        assert methods == ["GET", "GET", "DELETE" if clear else "PUT", "GET"]
    finally:
        c.close()


@pytest.mark.parametrize("clear", [False, True])
def test_conflicting_nonempty_aliases_fail_closed(clear):
    c = Client()
    populated(c, "acme_plan")
    c.rows[0]["description_text"] = "Conflicting original"
    with pytest.raises(ReviewedWriteError, match="contradictory"):
        planning.build_plan(c, "changes", 123, "Rollout Plan", None if clear else "Acme new", clear=clear)
    assert not c.writes
