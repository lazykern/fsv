"""Gallery fixes: crashes, abort, JSON hygiene, clean HTTP errors, conflicts,
restricted choices in review/draft, names in edit drafts, attach flow."""
from __future__ import annotations

import io
import json
from types import SimpleNamespace

import pytest
import typer
from typer.testing import CliRunner

import acme_change_form as A
import fsv.change_flow as flow
import fsv.cli as cli
import fsv.create as create
from fsv import render
from fsv.errors import APIError
from fsv.forms.rules import Conflict, Decision

runner = CliRunner()
JANE = "jane.doe@example.com"
DATES = "  Planned Start Date: 2030-11-26 19:00\n  Planned End Date: 2030-11-26 22:00\n"
DESC = "description_md: |\n  Rotate.\n"


@pytest.fixture
def fake(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    c = A.FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(create, "get_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr("fsv.session.session_age_hours", lambda: 1.0)
    monkeypatch.setattr("fsv.state_flow.get_client", lambda: c)
    return c


def run(*args, input=None):
    r = runner.invoke(cli.app, list(args), input=input)
    if r.exception and not isinstance(r.exception, SystemExit):
        raise r.exception
    return r


# -- crashes ---------------------------------------------------------------
def test_changes_state_and_approvals_do_not_crash(fake):
    r = run("changes", "state", "CHN-501")  # change has no state flow: a message, not a NameError
    assert r.exit_code == 0 and "no state_flow_id" in r.output
    r = run("changes", "approvals", "CHN-501")
    assert r.exit_code == 0


def test_declined_confirmation_says_aborted():
    with pytest.raises(typer.Exit) as e:
        cli._err_exc(typer.Abort())
    assert e.value.exit_code == 1


def test_declined_confirm_prompt_is_not_a_blank_error(fake, monkeypatch):
    def decline(*a, **k):
        raise typer.Abort()
    monkeypatch.setattr(cli, "_confirm_writes", decline)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    r = run("changes", "clone", "CHN-501", "--no-edit", "--with-tasks")
    assert r.exit_code == 1
    assert "Aborted" in r.output and "error: \n" not in r.output


def test_clone_with_tasks_refuses_before_opening_the_editor(fake, monkeypatch):
    opened = []
    monkeypatch.setattr(cli, "_edit_body", lambda *a, **k: opened.append(1) or {})
    monkeypatch.setattr(cli, "change_clone_data", lambda cid: {"subject": "x"}, raising=False)
    r = run("changes", "clone", "CHN-501", "--with-tasks", "--no-input")
    assert r.exit_code == 2 and not opened and "--yes" in r.output


# -- JSON / display --------------------------------------------------------
def test_emit_json_drops_internal_keys(capsys):
    render.emit_json([{"id": 1, "_resource": object(), "_search_result": True, "x": {"_agent": "a", "y": 1}}])
    out = json.loads(capsys.readouterr().out)
    assert out == [{"id": 1, "x": {"y": 1}}]


def test_value_formatting():
    assert render.fmt_value(["a", "b"]) == "a, b"
    assert render.fmt_value(True) == "Yes"
    assert render.fmt_dt("2030-11-26T12:00:00Z").startswith("2030-11-26 ")
    assert "T" not in render.fmt_dt("2030-11-26T12:00:00Z")
    assert render.fmt_dt(None) == "-" and render.fmt_dt("garbage") == "garbage"


# -- HTTP errors -----------------------------------------------------------
def test_http_error_is_one_clean_line():
    e = APIError(503, {"message": "Service Unavailable", "errors": []}, "GET",
                 "/api/_/changes?query_hash%5B%5D=%7B%7D&page=1")
    assert str(e) == "HTTP 503 listing changes: Service Unavailable"
    assert "{" not in str(e) and "query_hash" not in str(e)
    assert "query_hash" in e.detail()


@pytest.mark.parametrize("body,expect", [
    ({"description": "Validation failed", "errors": [{"field": "subject", "message": "can't be blank"}]},
     "Validation failed (subject: can't be blank)"),
    ("<html><body><h1>Bad gateway</h1></body></html>", "Bad gateway"),
    ({"error": "nope"}, "nope"),
    (None, ""),
])
def test_server_message_shapes(body, expect):
    from fsv.errors import server_message
    assert server_message(body) == expect


def test_cli_prints_clean_error_and_verbose_detail(monkeypatch):
    class Boom:
        def int_get(self, *a, **k):
            raise APIError(503, {"message": "down"}, "GET", "/api/_/changes?page=1")
    monkeypatch.setattr(cli, "_client", lambda: Boom())
    monkeypatch.setattr("fsv.schema.load", lambda *a, **k: {"fields": []})
    r = run("changes", "ls")
    assert r.exit_code == 5
    assert "HTTP 503 listing changes: down" in r.output and "{'" not in r.output
    r = run("-v", "changes", "ls")
    assert "GET /api/_/changes?page=1" in r.output


# -- conflicts -------------------------------------------------------------
def _d(rid, action, matched=True):
    return Decision(rid, f"R{rid}", action, matched)


def test_real_conflicts_need_two_currently_matching_rules():
    stale = Conflict("f", "visible", (_d(1, "show"), _d(2, "hide")), _d(2, "hide"))
    ev = SimpleNamespace(conflicts=[stale], matched=[2])  # rule 1 matched earlier, not now
    assert flow.real_conflicts(ev) == []
    ev = SimpleNamespace(conflicts=[stale], matched=[1, 2])
    (c,) = flow.real_conflicts(ev)
    assert c.winner.rule_id == 2


def test_yes_no_rule_pair_is_not_reported(fake, tmp_path):
    p = tmp_path / "c.yaml"
    p.write_text(f"requester: {JANE}\nanswers:\n  Subject: s\n  Cloud Only: Yes\n")
    r = run("changes", "create", "--from", str(p), "--dry-run", "-o", "json")
    review = json.loads(r.output)["review"]
    assert not any("conflict" in n for n in review["notes"])
    assert not any("wins" in n for n in review["notes"])


# -- restricted choices ----------------------------------------------------
def test_review_and_draft_name_the_only_option(fake, tmp_path):
    p = tmp_path / "c.yaml"
    p.write_text(f"requester: {JANE}\nanswers:\n  Subject: s\n  Cloud Only: No\n  Stage: Staging\n")
    out = json.loads(run("changes", "create", "--from", str(p), "--dry-run", "-o", "json").output)
    notes = " | ".join(out["review"]["notes"])
    assert "choices limited to Staging by rule" in notes
    rows = {r["label"]: r["value"] for r in out["review"]["rows"]}
    assert "only option allowed by rule" in rows["Stage"]
    q = tmp_path / "d.yaml"
    q.write_text(f"requester: {JANE}\nanswers:\n  Subject: s\n  Cloud Only: No\n")
    text = run("changes", "create", "--from", str(q), "--dry-run").output
    assert "Staging" in text


def test_draft_prefills_single_allowed_choice(fake):
    meta = flow.load_meta(fake, "change", "new", fresh=True)
    draft = flow.Draft(answers={"Cloud Only": "No"})
    plan = flow.build_create_plan(meta, draft, resolver=flow.default_resolver(fake), requester=None)
    text = flow.template_for(plan, draft, title="t")
    assert "only option allowed by rule" in text
    assert "# choices: Staging\n  Stage: Staging" in text


# -- edit drafts show names ------------------------------------------------
def test_edit_draft_shows_group_name_not_id(fake):
    meta = flow.load_meta(fake, "change", "edit", fresh=True, module_id=501)
    record = A.record()
    plan = flow.build_update_plan(meta, flow.Draft(), record, 501, resolver=flow.default_resolver(fake))
    text = flow.template_for(plan, flow.Draft(), title="t")
    assert "Group: Platform Ops" in text
    assert "9000009001" not in text


# -- attach flow -----------------------------------------------------------
def _draft_with_file(tmp_path):
    (tmp_path / "plan.pdf").write_bytes(b"%PDF-1.4 acme")
    p = tmp_path / "c.yaml"
    p.write_text(f"requester: {JANE}\nanswers:\n  Subject: s\n  Cloud Only: Yes\n" + DATES + DESC + "attachments: [plan.pdf]\n")
    return p


def test_attach_flow_default_is_after_create(fake, tmp_path):
    p = _draft_with_file(tmp_path)
    assert run("changes", "create", "--from", str(p), "--yes").exit_code == 0
    order = [(m, path) for m, path, _ in fake.writes()]
    assert order[0] == ("POST", "changes")
    assert ("PUT", "changes/900000502") in order


def test_attach_flow_before_create_sends_ids_in_the_body(fake, tmp_path):
    p = _draft_with_file(tmp_path)
    r = run("changes", "create", "--from", str(p), "--yes", "--attach-flow", "before-create")
    assert r.exit_code == 0, r.output
    writes = fake.writes()
    assert writes[0][:2] == ("POST", "attachments")
    post = next(b for m, path, b in writes if path == "changes")
    assert post["attachments"] == [9000005001]
    assert not any(m == "PUT" for m, _, _ in writes)


def test_attach_flow_from_config_and_validation(fake, tmp_path, monkeypatch):
    monkeypatch.setattr("fsv.config.load_config", lambda: {"changes": {"attach_flow": "before-create"}})
    assert flow.resolve_attach_flow(None) == "before-create"
    assert flow.resolve_attach_flow("after-create") == "after-create"
    with pytest.raises(flow.FlowExit):
        flow.resolve_attach_flow("sideways")


def test_dry_run_with_problems_has_no_null_payload(fake, tmp_path):
    p = tmp_path / "c.yaml"
    p.write_text(f"requester: {JANE}\nanswers:\n  Subject: s\n  Stage: Nowhere\n")
    r = run("changes", "create", "--from", str(p), "--dry-run")
    assert r.exit_code == 2
    assert "null" not in r.output.split("Payload")[-1] if "Payload" in r.output else True


def test_bad_choice_is_not_also_reported_required(fake, tmp_path):
    p = tmp_path / "c.yaml"
    p.write_text(f"requester: {JANE}\nanswers:\n  Subject: s\n  Cloud Only: Maybe\n")
    out = json.loads(run("changes", "create", "--from", str(p), "--dry-run", "-o", "json").output)
    errs = [e for e in out["errors"] if "Cloud Only" in e]
    assert len(errs) == 1 and "required" not in errs[0]
