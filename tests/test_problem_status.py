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
         "solutions": [{"id": 701, "title": "Acme solution", "article_type": 1}], "writes": [], "fail": None, "mismatch": False, "partial": False}
    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "problem_form_fields":
                return httpx.Response(200, json={"problem_fields": fields})
            if path.endswith("business_rules"):
                if s.get("rules_status"):
                    return httpx.Response(s["rules_status"], json={"error": "synthetic"})
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
            if path == "problems/123/activities":
                return httpx.Response(200, json={"activities": copy.deepcopy(s.get("activities", []))})
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
    with pytest.raises(ReviewedWriteError, match=r"missing Root Cause, Impact, Symptoms$"):
        flow.build_plan(c, 123, "Closed")
    assert not s["writes"]


def test_missing_solution_warns_until_the_tenant_proves_the_rule(api):
    c, s, _, _ = api
    s["solutions"] = []
    p = flow.build_plan(c, 123, "Closed")
    (warning,) = p.review()["warnings"]
    assert "no linked workaround / solution" in warning and "2 of 62" in warning
    # the server rejects: that proves the rule for this tenant, and later preflights block
    s["fail"] = "closure"
    with pytest.raises(ReviewedWriteError, match="a workaround / solution must be linked"):
        flow.send(p, c, reviewed_hash=p.hash)
    assert flow.SOLUTION_RULE in flow.proven_closure_rules()
    with pytest.raises(ReviewedWriteError, match="missing linked workaround / solution"):
        flow.build_plan(c, 123, "Closed")


@pytest.mark.parametrize("articles", [[], [{"id": 701, "title": "Acme workaround", "article_type": 2}],
                                     [{"id": 801, "title": "Globex permanent solution", "article_type": 1}]])
def test_native_articles_envelope_and_empty_inventory_diagnostic(api, articles):
    c, s, _, _ = api
    s["solution_raw"] = {"articles": articles}
    runner = CliRunner()
    r = runner.invoke(cli.app, ["problems", "status", "PRB-123", "--to", "Closed", "--dry-run"])
    assert r.exit_code == 0, r.output
    p = flow.build_plan(c, 123, "Closed")
    assert flow.build_plan(c, 123, "Closed").hash == p.hash
    if not articles:  # a warning, not a block, until the tenant proves the rule
        assert "no linked workaround / solution" in json.loads(r.output)["warnings"][0]
        assert "inventory is unavailable" not in r.output
    assert not s["writes"]


def test_native_articles_pagination_stays_on_solutions_route(api):
    c, s, _, _ = api
    s["solution_pages"] = [{"articles": [{"id": 701, "title": "Acme workaround", "article_type": 2}], "has_more": True},
                           {"articles": [{"id": 801, "title": "Globex solution", "article_type": 1}], "has_more": False}]
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


def test_rules_500_degrades_to_warning_in_review_and_json(api):
    c, s, _, n = api
    s["rules_status"] = 500
    p = flow.build_plan(c, 123, "In Progress")
    assert any("rules unavailable" in w and "HTTP 500" in w for w in p.review()["warnings"])
    out = CliRunner().invoke(cli.app, ["problems", "status", "PRB-123", "--to", "In Progress", "--dry-run", "--json"])
    assert out.exit_code == 0, out.output
    assert "rules unavailable" in json.loads(out.output)["warnings"][0]
    assert not s["writes"]


@pytest.mark.parametrize("evidence", [True, False])
def test_workflow_rewrite_to_the_prior_status_uses_activity_evidence(api, evidence):
    import datetime as dt
    c, s, _fields, n = api
    plan = flow.build_plan(c, 123, "Resolve")
    s["mismatch"] = True  # a tenant workflow put the status straight back to Open
    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    if evidence:
        s["activities"] = [
            {"actor": {"id": 0, "name": "Problem Workflow", "is_agent_deleted": False},
             "content": "executed  <a>Acme status workflow</a>  workflow from  <b>Problem status changed</b>  event",
             "sub_contents": ["set status as <b>Open</b>", "<b>Workflow Ends</b>"], "created_at": now},
            {"actor": {"id": 9000001, "name": "Acme Agent", "is_agent": True},
             "content": "set status as  <b>Resolve</b>", "sub_contents": None, "created_at": now}]
    result = flow.send(plan, c, reviewed_hash=plan.hash)
    assert len(s["writes"]) == 1 and s["row"]["status"] == n
    assert result["verified"] is evidence
    assert ("rewritten server-side to Open" in result.get("verified_with_note", "")) is evidence



@pytest.mark.parametrize("case,verified", [
    ("closure-workflow-stamped-first", True),   # workflow row 1 s older than the closure row
    ("closure-same-second", True),
    ("workflow-label-differs-from-readback", False),
    ("workflow-minutes-older", False),
    ("no-own-closure-entry", False),
])
def test_closed_rewritten_by_workflow_is_verified_with_note(api, case, verified):
    """Real live shape: Closed (with planned dates filled) then a workflow
    put the status back to the pre-write monitoring status."""
    import datetime as dt
    c, s, fields, n = api
    fields[1]["choices"].append({"id": n + 4, "value": "Monitor"})
    s["row"]["status"] = n + 4  # the record was already in the monitoring status
    plan = flow.build_plan(c, 123, "Closed")
    s["mismatch"] = True  # the workflow rewrote Closed back to Monitor
    now = dt.datetime.now(dt.timezone.utc)
    stamp = lambda d: (now + dt.timedelta(seconds=d)).strftime("%Y-%m-%dT%H:%M:%SZ")
    wf_at = {"closure-workflow-stamped-first": -1, "workflow-minutes-older": -60}.get(case, 0)
    rewritten = "Open" if case == "workflow-label-differs-from-readback" else "Monitor"
    own = ("set priority as  <b>Low</b>" if case == "no-own-closure-entry" else
           "set status as  <b>Closed</b> , set planned end date as  <b>Thu, 2 Jan  4:30 PM</b> , "
           "set planned start date as  <b>Thu, 2 Jan  3:30 PM</b>")
    s["activities"] = [
        {"actor": {"id": 0, "name": "Problem Workflow", "is_agent_deleted": False},
         "content": "executed  <a>Acme status workflow</a>  workflow from  <b>Problem status changed</b>  event",
         "sub_contents": [f"set status as <b>{rewritten}</b>", "<b>Workflow Ends</b>"], "created_at": stamp(wf_at)},
        {"actor": {"id": 9000001, "name": "Acme Agent", "is_agent": True},
         "content": own, "sub_contents": None, "created_at": stamp(0)}]
    result = flow.send(plan, c, reviewed_hash=plan.hash)
    assert len(s["writes"]) == 1 and result["verified"] is verified
    assert result["readback"]["status"] == n + 4 and result["readback"]["planned_end_date"]
    if verified:
        assert "set to Closed by this write, then rewritten server-side to Monitor" in result["verified_with_note"]
        assert "warning" not in result
    else:
        assert "verified_with_note" not in result and "do not resend" in result["warning"]
    if case == "workflow-label-differs-from-readback":
        assert "readback status is Monitor" in result["readback_conflict"]



@pytest.mark.parametrize("articles,expected", [
    ([], "missing: this tenant may require"),
    ([{"id": 701, "title": "Acme workaround", "article_type": 2}], "linked (1)"),
])
def test_closed_dry_run_shows_closure_check_up_front(api, articles, expected):
    c, s, _, _ = api
    s["solution_raw"] = {"articles": articles}
    r = CliRunner().invoke(cli.app, ["problems", "status", "PRB-123", "--to", "Closed", "--dry-run"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.output)
    assert expected in doc["closure_check"]["linked_solution"]
    assert "filled" in doc["closure_check"]["analysis"] and not s["writes"]
    other = CliRunner().invoke(cli.app, ["problems", "status", "PRB-123", "--to", "Resolve", "--dry-run"])
    assert "closure_check" not in json.loads(other.output)


def test_remembered_solution_rule_blocks_the_dry_run_up_front(api):
    c, s, _, _ = api
    s["solution_raw"] = {"articles": []}
    flow.remember_closure_rules([flow.SOLUTION_RULE])
    r = CliRunner().invoke(cli.app, ["problems", "status", "PRB-123", "--to", "Closed", "--dry-run"])
    assert r.exit_code == 2
    assert "missing linked workaround / solution" in r.output and "rejected a closure" in r.output
    assert not s["writes"]
