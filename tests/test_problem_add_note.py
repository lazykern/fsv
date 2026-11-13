"""Legacy problem add-note sends the captured body only (no visibility key)."""
import json

import httpx
import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import client as client_mod, config, journal, service
from fsv.resources import CHANGES, PROBLEMS


@pytest.fixture
def wire(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    posts = []

    def handler(req):
        body = json.loads(req.content)
        posts.append((req.url.path, body))
        if "private" in body and "/problems/" in req.url.path:
            # Real tenant answer to the old body.
            return httpx.Response(400, json={"description": "Validation failed", "errors": [
                {"field": "private", "message": "Unexpected/invalid field in request", "code": "invalid_field"}]})
        return httpx.Response(201, json={"note": {"id": 9000000077, "body": body["body"]}})
    c = client_mod.Client(cookies={})
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    return posts


def test_problem_note_body_is_exactly_the_captured_shape(wire):
    assert service.note_payload(PROBLEMS, "<div>Acme</div>") == {"body": "<div>Acme</div>"}
    assert service.note_payload(CHANGES, "x") == {"body": "x", "private": True}
    r = CliRunner().invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme evidence", "--yes"])
    assert r.exit_code == 0, r.output
    assert wire == [("/api/_/problems/12/notes", {"body": "Acme evidence"})]
    assert "not controllable" in r.output and "private" not in r.output


def test_problem_note_dry_run_states_visibility_and_public_is_refused(wire):
    r = CliRunner().invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme evidence", "--dry-run"])
    assert r.exit_code == 0, r.output
    doc = json.loads(r.stdout)
    assert doc["body"] == {"body": "Acme evidence"}
    assert "visibility is not controllable" in doc["warnings"][0]
    refused = CliRunner().invoke(cli.app, ["problems", "add-note", "PRB-12", "Acme", "--public", "--yes"])
    assert refused.exit_code == 2 and "not controllable" in refused.output
    assert not wire


def test_change_note_keeps_its_visibility_key(wire):
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-3", "Acme change note"])
    assert r.exit_code == 0, r.output
    assert wire == [("/api/_/changes/3/notes", {"body": "Acme change note", "private": True})]
