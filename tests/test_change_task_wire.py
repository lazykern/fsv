from __future__ import annotations

import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, create, journal
from fsv.client import APIError, Client


@pytest.fixture(params=["acme.example", "globex.example"])
def tenant(request):
    # Synthetic hosts; MockTransport never performs network I/O.
    return request.param


def task_client(monkeypatch, tenant, handler):
    monkeypatch.setattr(config, "API_INT", f"https://{tenant}/api/_")
    monkeypatch.setattr(journal, "record", lambda *args, **kwargs: None)
    c = Client.__new__(Client)
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "fake-token"
    c._fw_domain = None
    c._fw_session_id = None
    c._persist = False
    c._rl_rem = c._rl_tot = None
    c._probe_ok_until = 0
    monkeypatch.setattr(c, "_probe_session", lambda: True)
    return c


@pytest.mark.parametrize("method", ["POST", "PUT", "PATCH", "DELETE"])
@pytest.mark.parametrize("outcome", ["403", "disconnect"])
def test_internal_mutation_never_replayed_after_ambiguous_failure(monkeypatch, tenant, method, outcome):
    seen = []

    def handler(request):
        seen.append(request)
        if outcome == "disconnect":
            raise httpx.ReadError("synthetic disconnect", request=request)
        return httpx.Response(403, json={"error": "denied"})

    c = task_client(monkeypatch, tenant, handler)
    try:
        with pytest.raises((APIError, httpx.ReadError)):
            c._int_request(method, "changes/41/tasks", body={"title": "Synthetic"})
        assert len(seen) == 1
        assert seen[0].headers["X-CSRF-Token"] == "fake-token"
    finally:
        c._client.close()


@pytest.mark.parametrize("operation", ["update", "delete"])
def test_task_exact_wire_contract(monkeypatch, tenant, operation):
    seen = []

    def handler(request):
        seen.append(request)
        return httpx.Response(200, json={"task": {"status": 2}} if operation == "update" else {"tasks": []})

    c = task_client(monkeypatch, tenant, handler)
    try:
        if operation == "update":
            body = {"status": 2, "custom_fields": {"system": 9, "team_name": "ignored", "environment": None}}
            assert create.update_task(41, 73, body, c=c) == {"status": 2}
            assert body["custom_fields"]["team_name"] == "ignored"
            path = "/api/_/changes/41/tasks/73"
            expected = {"status": 2, "custom_fields": {"system": 9}}
        else:
            assert create.delete_task(41, 73, c=c) is None
            path = "/api/_/changes/41/tasks/update"
            expected = {"id": "73", "action": "delete"}
        assert len(seen) == 1
        req = seen[0]
        assert req.method == "PUT"
        assert str(req.url) == f"https://{tenant}{path}"
        assert req.headers["x-csrf-token"] == "fake-token"
        assert json.loads(req.content) == expected
    finally:
        c.close()


@pytest.mark.parametrize("operation", ["update", "delete"])
@pytest.mark.parametrize("failure", ["timeout", 403, 404, 503])
def test_task_write_never_retries_or_falls_back(monkeypatch, tenant, operation, failure):
    seen = []

    def handler(request):
        seen.append(request)
        if failure == "timeout":
            raise httpx.ReadTimeout("uncertain outcome", request=request)
        return httpx.Response(failure, json={"error": "rejected"})

    c = task_client(monkeypatch, tenant, handler)
    try:
        with pytest.raises(httpx.ReadTimeout if failure == "timeout" else APIError):
            if operation == "delete":
                create.delete_task(41, 73, c=c)
            else:
                create.update_task(41, 73, {"status": 2}, c=c)
        assert len(seen) == 1
        assert seen[0].method == "PUT"
        assert seen[0].url.path == (
            "/api/_/changes/41/tasks/update" if operation == "delete"
            else "/api/_/changes/41/tasks/73"
        )
    finally:
        c.close()


def test_delete_dry_run_and_confirmation(monkeypatch):
    sent = []

    class FakeClient:
        rows = [{"id": 73, "title": "Acme task"}]
        def int_get(self, path, params=None):
            return {"tasks": list(self.rows), "meta": {"count": len(self.rows)}}
        def _get_csrf(self):
            return "synthetic"
        def _request(self, method, path, **kwargs):
            body = json.loads(kwargs["content"])
            sent.append(("changes/41/tasks/update", body))
            self.rows = []
            return {}

    c = FakeClient()
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(cli, "_client", lambda: c)
    runner = CliRunner()
    args = ["changes", "tasks-delete", "CHN-41", "73"]
    result = runner.invoke(cli.app, args + ["--dry-run"])
    assert result.exit_code == 0 and not sent
    review = json.loads(result.output)
    assert review["payload"] == {"id": "73", "action": "delete"}
    assert review["plan_hash"]
    result = runner.invoke(cli.app, args)
    assert result.exit_code != 0 and not sent
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code != 0 and not sent
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", review["plan_hash"]])
    assert result.exit_code == 0 and sent == [("changes/41/tasks/update", {"id": "73", "action": "delete"})]
