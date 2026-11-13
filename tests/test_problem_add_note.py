"""Problem add-note is a reviewed write: plan hash bound send, ambiguity
journal, independent readback by listing the problem's notes, `verified` in
the result. The captured body is `{body}` only (no visibility key)."""
import json

import httpx
import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import client as client_mod, config, journal, problem_notes, service
from fsv.resources import CHANGES, PROBLEMS


def _setup(monkeypatch, tmp_path, *, stored_body=None, list_ids=None, post_status=201, post_json=None):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    state = {"posts": [], "gets": [], "notes": []}

    def handler(req):
        if req.method == "GET":
            state["gets"].append(req.url.path)
            return httpx.Response(200, json={"notes": state["notes"], "meta": {"total_count": len(state["notes"])}})
        body = json.loads(req.content)
        state["posts"].append((req.url.path, body))
        if "private" in body and "/problems/" in req.url.path:
            return httpx.Response(400, json={"description": "Validation failed", "errors": [
                {"field": "private", "message": "Unexpected/invalid field in request", "code": "invalid_field"}]})
        if post_status >= 400:
            return httpx.Response(post_status, json=post_json or {"description": "Synthetic failure"})
        state["notes"].append({"id": 9000000077, "body": stored_body if stored_body is not None
                               else f"<div>{body['body']}</div>", "user_id": 7})
        for extra in list_ids or ():
            state["notes"].append({"id": extra, "body": "<div>older</div>"})
        return httpx.Response(201, json={"note": {"id": 9000000077, "body": body["body"]}})

    c = client_mod.Client(cookies={})
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    return state


def _review(runner, *args):
    r = runner.invoke(cli.app, ["problems", "add-note", "PRB-12", *args, "--dry-run"])
    assert r.exit_code == 0, r.output
    return json.loads(r.stdout)


def test_dry_run_is_a_hash_bound_review_of_the_captured_body(monkeypatch, tmp_path):
    state = _setup(monkeypatch, tmp_path)
    doc = _review(CliRunner(), "Acme evidence")
    assert doc["dry_run"] is True and doc["status"] == "dry_run"
    assert doc["method"] == "POST" and doc["path"] == "/api/_/problems/12/notes"
    assert doc["payload"] == doc["body"] == {"body": "Acme evidence"}
    assert len(doc["plan_hash"]) == 64
    assert "visibility is not controllable" in doc["warnings"][0]
    assert service.note_payload(PROBLEMS, "<div>Acme</div>") == {"body": "<div>Acme</div>"}
    assert service.note_payload(CHANGES, "x") == {"body": "x", "private": True}
    assert state["posts"] == [] and not (tmp_path / "journal").exists()


def test_reviewed_send_posts_once_and_verifies_by_listing_notes(monkeypatch, tmp_path):
    state = _setup(monkeypatch, tmp_path, list_ids=[9000000001])
    runner = CliRunner()
    h = _review(runner, "Acme evidence")["plan_hash"]
    r = runner.invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme evidence", "--yes", "--expect-plan", h, "--json"])
    assert r.exit_code == 0, r.output
    res = json.loads(r.stdout)
    assert state["posts"] == [("/api/_/problems/12/notes", {"body": "Acme evidence"})]
    assert state["gets"] == ["/api/_/problems/12/notes"]
    assert res["verified"] is True and res["plan_hash"] == h and str(res["note_id"]) == "9000000077"
    assert res["fields"] == {"body": "matched"} and res["readback_complete"] is True
    events = [e for e in journal.read() if e.get("kind") == "plan:problem_note"]
    assert [e["event"] for e in events] == ["started", "done"]
    # The exact plan is never replayed.
    again = runner.invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme evidence", "--yes", "--expect-plan", h])
    assert again.exit_code == 2 and len(state["posts"]) == 1


def test_human_output_reports_verified(monkeypatch, tmp_path):
    _setup(monkeypatch, tmp_path)
    runner = CliRunner()
    h = _review(runner, "Acme evidence")["plan_hash"]
    r = runner.invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme evidence", "--yes", "--expect-plan", h])
    assert r.exit_code == 0, r.output
    assert "added note 9000000077" in r.stdout and "verified: true" in r.stdout
    assert "not controllable" in r.stdout


def test_readback_body_mismatch_is_not_verified(monkeypatch, tmp_path):
    _setup(monkeypatch, tmp_path, stored_body="<div>Globex text</div>")
    runner = CliRunner()
    h = _review(runner, "Acme evidence")["plan_hash"]
    r = runner.invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme evidence", "--yes", "--expect-plan", h, "--json"])
    res = json.loads(r.stdout)
    assert res["verified"] is False and res["fields"] == {"body": "mismatched"}
    assert "do not resend" in res["warning"]


def test_note_missing_from_listing_is_not_verified(monkeypatch, tmp_path):
    state = _setup(monkeypatch, tmp_path)
    plan = problem_notes.build_plan("PRB-12", "Acme evidence")
    orig = state["notes"]

    class Hide(list):
        def append(self, item):
            pass
    state["notes"] = Hide()
    res = problem_notes.send(plan, cli._client(), reviewed_hash=plan.hash)
    assert res["verified"] is False and res["identity_verified"] is False
    assert orig == []


@pytest.mark.parametrize("args,msg", [
    (["--yes"], "--expect-plan"),
    (["--yes", "--expect-plan", "0" * 64], "does not match"),
    (["--no-input"], "no-input"),
    (["--public", "--yes"], "not controllable"),
])
def test_refusals_send_nothing(monkeypatch, tmp_path, args, msg):
    state = _setup(monkeypatch, tmp_path)
    r = CliRunner().invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme", *args])
    assert r.exit_code == 2 and msg in r.output
    assert state["posts"] == []


def test_server_rejection_names_status_and_message(monkeypatch, tmp_path):
    state = _setup(monkeypatch, tmp_path, post_status=422,
                   post_json={"description": "Validation failed",
                              "errors": [{"field": "body", "message": "Synthetic body too long"}]})
    runner = CliRunner()
    h = _review(runner, "Acme")["plan_hash"]
    r = runner.invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme", "--yes", "--expect-plan", h])
    assert r.exit_code == 2 and "HTTP 422" in r.output and "Synthetic body too long" in r.output
    assert len(state["posts"]) == 1


def test_change_note_keeps_its_legacy_visibility_key(monkeypatch, tmp_path):
    state = _setup(monkeypatch, tmp_path)
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-3", "Acme change note"])
    assert r.exit_code == 0, r.output
    assert state["posts"] == [("/api/_/changes/3/notes", {"body": "Acme change note", "private": True})]
