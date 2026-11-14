"""Offline caller/transport contracts for non-transactional change clones.

These legacy writes have no restart guard or independent readback. Assert the
created identity and partial outcome, never treat a returned echo as proof.
"""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, create
from fsv.client import Client


def TENANT_FIELDS():
    return {"tenant": config.DOMAIN, "config_dir": str(config.CONFIG_DIR)}


@pytest.fixture
def clone_wire(monkeypatch):
    state = {"writes": [], "reads": [], "fail": None, "status": 500, "missing_id": False}

    def handle(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            state["reads"].append(path)
            rows = {
                "changes/123": {"change": {"id": 123, "subject": "Acme maintenance", "status": 47,
                    "attachments": [{"id": 11}], "state_flow_id": 27}},
                "changes/123/tasks": {"tasks": [{"id": 501, "title": "Verify Acme", "owner_id": 71, "status": 3}]},
                "changes/123/assets": {"associated_assets": [{"id": 801, "config_item": {"display_id": 701}}]},
                "changes/123/planning-fields": {"change_planning_fields": [{"name": "test_plan", "description": "Acme checks", "attachments": []}]},
            }
            assert path in rows, path
            return httpx.Response(200, json=rows[path])
        body = json.loads(req.content)
        state["writes"].append((req.method, path, body))
        kind = "base" if path == "changes" else "tasks" if path.endswith("/tasks") else "assets" if path.endswith("/associate") else "planning"
        if kind == state["fail"]:
            return httpx.Response(state["status"], json={"description": "synthetic denial"})
        if kind == "base":
            return httpx.Response(200, json={"change": {} if state["missing_id"] else {"id": 456, "human_display_id": "CHN-456"}})
        return httpx.Response(200, json={"task": {"id": 901}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic-token")
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    monkeypatch.setattr(create, "get_client", lambda: c)
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(create, "_get_change_schema_fields", lambda: [])
    yield state
    c.close()


ARGS = ["changes", "clone", "CHN-123", "--no-edit", "--with-tasks", "--with-assets", "--with-planning", "--yes", "--json"]


@pytest.mark.parametrize("failed", ["tasks", "assets", "planning"])
@pytest.mark.parametrize("status", [403, 409, 429, 500])
def test_partial_clone_preserves_new_identity_and_attempts_each_step_once(clone_wire, failed, status):
    clone_wire.update(fail=failed, status=status)
    result = CliRunner().invoke(cli.app, ARGS)
    assert result.exit_code == (4 if status == 409 else 5), result.output
    out = json.loads(result.stdout)
    assert out["id"] == 456 and out["human_display_id"] == "CHN-456"
    assert set(out["fsv_clone_steps"]) == {"tasks", "assets", "planning"}
    for step, outcome in out["fsv_clone_steps"].items():
        assert outcome.startswith("failed:") if step == failed else outcome == "ok"
    assert "do not rerun clone" in " ".join(result.stderr.split())
    writes = clone_wire["writes"]
    assert [(m, p) for m, p, _ in writes] == [
        ("POST", "changes"), ("POST", "changes/456/tasks"),
        ("PUT", "changes/456/assets/associate"), ("POST", "changes/456/planning-fields")]
    assert writes[0][2] == {"subject": "Acme maintenance", "status": 1}
    assert writes[1][2] == {"change_task": {"title": "Verify Acme", "status": 1}}
    assert writes[2][2] == {"item_ids": [701]}
    assert writes[3][2] == {"description": "Acme checks"}


def test_clone_dry_run_counts_side_effects_without_creating(clone_wire):
    result = CliRunner().invoke(cli.app, [*ARGS, "--dry-run", "--keep-task-owners"])
    assert result.exit_code == 0, result.output
    assert json.loads(result.stdout) == {"subject": "Acme maintenance", "status": 1, **TENANT_FIELDS()}
    assert "owners/past dates kept" in result.stderr
    assert "1 asset(s)" in result.stderr and "1 planning field(s)" in result.stderr
    assert "0 attachment(s)" in result.stderr
    assert clone_wire["writes"] == []


def test_clone_missing_returned_id_does_not_guess_side_effect_target(clone_wire):
    clone_wire["missing_id"] = True
    result = CliRunner().invoke(cli.app, ARGS)
    assert result.exit_code == 0, result.output
    assert json.loads(result.stdout) == TENANT_FIELDS()
    assert "response had no id" in result.stderr
    assert len(clone_wire["writes"]) == 1
    assert clone_wire["reads"] == ["changes/123"]


def test_clone_confirmation_failure_prevents_even_base_create(clone_wire):
    args = [a for a in ARGS if a != "--yes"] + ["--no-input"]
    result = CliRunner().invoke(cli.app, args)
    assert result.exit_code == 2 and "--yes" in result.output
    assert clone_wire["writes"] == []
