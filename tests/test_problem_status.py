"""Problem lifecycle uses real envelopes and runtime enums, entirely offline."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, formmeta, journal, problem_status as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture(params=[41, 81])
def api(request, monkeypatch, tmp_path):
    n = request.param
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(config, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.setattr(formmeta, "configured_workspace", lambda: 17)
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    fields = [{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1,
               "required_for_closure": True},
              {"id": 502, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
               "choices": [{"id": n+i, "value": label} for i, label in enumerate(["Open", "In Progress", "Resolve", "Closed"])]}]
    s = {"row": {"id": 123, "subject": "Acme problem", "status": n, "planned_start_date": None, "planned_end_date": None},
         "analysis": [{"id": i+1, "name": key, "description": "<div>Acme evidence</div>"} for i, key in enumerate(flow.planning_text.PROBLEM_SECTIONS.values())],
         "solutions": [{"id": 701, "title": "Acme solution"}], "writes": [], "fail": None, "mismatch": False, "partial": False}
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "problem_form_fields":
                return httpx.Response(200, json={"problem_fields": fields})
            if path.endswith("business_rules"):
                return httpx.Response(200, json={"business_rules": []})
            if path.endswith("planning-fields"):
                return httpx.Response(200, json={"problem_planning_fields": copy.deepcopy(s["analysis"])})
            if path.endswith("solutions"):
                s.setdefault("solution_reads", []).append(dict(req.url.params))
                if s.get("solution_error"):
                    return httpx.Response(s["solution_error"], json={"error": "unavailable"})
                if "solution_raw" in s:
                    return httpx.Response(200, json=copy.deepcopy(s["solution_raw"]))
                if s.get("solution_pages"):
                    page = int(req.url.params.get("page", 1))
                    return httpx.Response(200, json=copy.deepcopy(s["solution_pages"][page - 1]))
                return httpx.Response(200, json={"articles": copy.deepcopy(s["solutions"]), "has_more": s["partial"]})
            assert path == "problems/123"
            return httpx.Response(200, json={"problem": copy.deepcopy(s["row"])})
        body = json.loads(req.content)
        s["writes"].append((req.method, path, body))
        if s["fail"] == "disconnect":
            raise httpx.ReadError("disconnect", request=req)
        if s["fail"] == "closure":
            return httpx.Response(400, json={"errors": [{"code": "violated_closure_rules", "description": {"check_analysis_completed": "Analysis required", "check_workaround_or_solution_added": "Solution required"}}]})
        if s["fail"]:
            return httpx.Response(s["fail"], json={"error": "failed"})
        if not s["mismatch"]:
            s["row"].update(body)
        s["row"].update(planned_start_date="2030-01-02T10:00:00Z", planned_end_date="2030-01-02T11:00:00Z")
        # The independent GET, not this deliberately incomplete response, proves state.
        return httpx.Response(200, json={"problem": {"id": 123}})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s, fields, n
    c.close()


@pytest.mark.parametrize("label", ["In Progress", "Resolve", "Closed", "Open"])
def test_status_transitions_and_side_effect_review(api, label):
    c, s, _, n = api
    p = flow.build_plan(c, "PRB-123", label)
    assert "auto-fill" in p.review()["side_effects"]
    r = flow.send(p, c, reviewed_hash=p.hash)
    assert r["verified"] and r["readback"]["planned_start_date"]
    assert s["writes"] == [("PUT", "problems/123", {"status": n+["Open", "In Progress", "Resolve", "Closed"].index(label)})]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)


def test_closure_missing_sections_and_solution(api):
    c, s, _, _ = api
    s["analysis"] = []
    s["solutions"] = []
    with pytest.raises(ReviewedWriteError, match="Root Cause, Impact, Symptoms, linked workaround / solution"):
        flow.build_plan(c, 123, "Closed")
    assert not s["writes"]


@pytest.mark.parametrize("articles", [[], [{"id": 701, "title": "Acme workaround", "article_type": 2}],
                                     [{"id": 801, "title": "Globex permanent solution", "article_type": 1}]])
def test_native_articles_envelope_and_empty_inventory_diagnostic(api, articles):
    c, s, _, _ = api
    s["solution_raw"] = {"articles": articles}
    runner = CliRunner()
    r = runner.invoke(cli.app, ["problems", "status", "PRB-123", "--to", "Closed", "--dry-run"])
    if articles:
        assert r.exit_code == 0, r.output
        p = flow.build_plan(c, 123, "Closed")
        assert flow.build_plan(c, 123, "Closed").hash == p.hash
    else:
        assert r.exit_code == 2, r.output
        assert "violated_closure_rules: missing linked workaround / solution" in r.output
        assert "inventory is unavailable" not in r.output
    assert not s["writes"]


def test_native_articles_pagination_stays_on_solutions_route(api):
    c, s, _, _ = api
    s["solution_pages"] = [{"articles": [{"id": 701, "title": "Acme workaround"}], "has_more": True},
                           {"articles": [{"id": 801, "title": "Globex solution"}], "has_more": False}]
    context = flow.closure_context(c, 123)
    assert [r["id"] for r in context["solutions"]] == [701, 801]
    assert [r["page"] for r in s["solution_reads"]] == ["1", "2"]
    assert not s["writes"]


@pytest.mark.parametrize("raw", [{}, {"articles": None}, {"articles": {}},
                                 {"solutions": []}, {"articles": [], "meta": {"count": 1}},
                                 {"articles": [], "has_more": True}])
def test_unknown_or_incomplete_solutions_never_means_empty(api, raw):
    c, s, _, _ = api
    s["solution_raw"] = raw
    with pytest.raises(ReviewedWriteError, match="complete problem solutions inventory is unavailable"):
        flow.build_plan(c, 123, "Closed")
    assert not s["writes"]


@pytest.mark.parametrize("status", [403, 429, 500])
def test_solutions_read_errors_fail_closed(api, status):
    c, s, _, _ = api
    s["solution_error"] = status
    with pytest.raises(ReviewedWriteError, match="complete problem solutions inventory is unavailable"):
        flow.build_plan(c, 123, "Closed")
    assert not s["writes"]


def test_closure_drift_and_required_field(api):
    c, s, _, _ = api
    p = flow.build_plan(c, 123, "Closed")
    s["analysis"][0]["description"] = "<div> </div>"
    with pytest.raises(ReviewedWriteError, match="Root Cause"):
        flow.send(p, c, reviewed_hash=p.hash)
    s["analysis"][0]["description"] = "Acme"
    s["row"]["subject"] = ""
    with pytest.raises(ReviewedWriteError, match="Subject"):
        flow.build_plan(c, 123, "Closed")
    assert not s["writes"]


@pytest.mark.parametrize("failure", [403, 429, 500, "disconnect"])
def test_ambiguity_never_replayed(api, failure):
    c, s, _, _ = api
    p = flow.build_plan(c, 123, "Resolve")
    s["fail"] = failure
    with pytest.raises(ReviewedWriteError, match="unknown"):
        flow.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


def test_server_closure_rules_are_explained(api):
    c, s, _, _ = api
    p = flow.build_plan(c, 123, "Closed")
    s["fail"] = "closure"
    with pytest.raises(ReviewedWriteError, match="violated_closure_rules.*Root Cause.*solution"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert [r["event"] for r in journal.read() if r.get("kind") == "plan:problem_status"] == ["started", "failed"]


def test_cli_and_mismatched_independent_get(api):
    _, s, _, _ = api
    args = ["problems", "status", "PRB-123", "--to", "Resolve"]
    runner = CliRunner()
    r = runner.invoke(cli.app, args+["--dry-run", "--json"])
    assert r.exit_code == 0, r.output
    hash_ = json.loads(r.output)["plan_hash"]
    assert runner.invoke(cli.app, args+["--yes", "--expect-plan", hash_[:16]]).exit_code == 2
    s["mismatch"] = True
    r = runner.invoke(cli.app, args+["--yes", "--expect-plan", hash_, "--json"])
    assert r.exit_code == 4, r.output
    assert not json.loads(r.output)["verified"] and len(s["writes"]) == 1
