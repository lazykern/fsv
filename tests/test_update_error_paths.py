"""A failed update never reports a bare failure: any non-2xx answer names
the server's HTTP status and its (sanitized) message, for changes, problems
and tickets update paths. Synthetic Acme data only."""
import json

import pytest
from typer.testing import CliRunner

import fsv.cli as cli
from fsv import config, journal, reviewed_write, ticket_properties as props
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError, WritePlan

from test_changes_form_cli import fake, run  # noqa: F401
from test_problem_write_fixes import SCHEMA
from test_ticket_properties import Client as TicketClient, metadata as ticket_metadata

HTML_503 = "<html><head><style>p{}</style></head><body><h1>Service Unavailable</h1><p>Try later</p></body></html>"


class _Fail:
    def __init__(self, exc):
        self.exc = exc
        self.sent = 0

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kw):
        self.sent += 1
        raise self.exc


def _plan():
    return WritePlan("acme_update", 7, config.DOMAIN, "PUT", "problems/7", json.dumps({"priority": 3}), "")


@pytest.fixture
def tenant(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")


@pytest.mark.parametrize("status,body,kind,message", [
    (422, {"description": "Validation failed", "errors": [{"field": "priority", "message": "Synthetic invalid"}]},
     "rejected", "priority: Synthetic invalid"),
    (500, {"message": "Synthetic server error"}, "outcome unknown", "Synthetic server error"),
    (503, HTML_503, "outcome unknown", "Service Unavailable Try later"),
    (429, {"message": "Synthetic rate limit"}, "outcome unknown", "Synthetic rate limit"),
    (403, "", "outcome unknown", ""),
])
def test_execute_names_status_and_message(tenant, status, body, kind, message):
    plan = _plan()
    client = _Fail(APIError(status, body, "PUT", "problems/7"))
    with pytest.raises(ReviewedWriteError) as caught:
        reviewed_write.execute(plan, client, reviewed_hash=plan.hash, rebuild=lambda: plan,
                               verify=lambda r: {"verified": True})
    text = str(caught.value)
    assert kind in text and f"HTTP {status}" in text and client.sent == 1
    assert "<" not in text and "style" not in text
    assert (message in text) if message else "empty response body" in text
    assert caught.value.http_status == status
    doc = caught.value.as_dict()
    assert doc["error"] == text and doc["http_status"] == status and message in doc["server_message"]


def test_execute_names_a_transport_failure(tenant):
    plan = _plan()
    with pytest.raises(ReviewedWriteError, match=r"outcome unknown \(ConnectError\)"):
        import httpx
        reviewed_write.execute(plan, _Fail(httpx.ConnectError("synthetic")), reviewed_hash=plan.hash,
                               rebuild=lambda: plan, verify=lambda r: {})


def _fail_change_puts(fake, exc):  # noqa: F811
    original = fake._request

    def failing(method, url, **kw):
        original(method, url, **kw)  # record the write, then answer with the failure
        raise exc
    fake._request = failing


@pytest.mark.parametrize("json_flag", [True, False])
def test_changes_update_failure_prints_status_and_body(fake, json_flag):  # noqa: F811
    _fail_change_puts(fake, APIError(503, HTML_503, "PUT", "changes/501"))
    args = ["changes", "update", "CHN-501", "--set", "Subject=Billing: rotate all keys", "--yes"]
    r = CliRunner().invoke(cli.app, args + (["--json"] if json_flag else []))
    assert r.exit_code == 5, r.output
    assert len(fake.writes()) == 1
    flat = " ".join(r.output.split())
    assert "HTTP 503" in flat and "Service Unavailable" in flat and "<h1>" not in flat
    if json_flag:
        doc = json.loads(r.stdout[r.stdout.index("{"):r.stdout.rindex("}") + 1])
        assert doc["verified"] is False and doc["http_status"] == 503
        assert doc["server_message"] == "Service Unavailable Try later" and len(doc["plan_hash"]) == 64


def test_changes_update_rejection_names_the_field(fake):  # noqa: F811
    _fail_change_puts(fake, APIError(422, {"description": "Validation failed", "errors": [
        {"field": "subject", "message": "Synthetic too long"}]}, "PUT", "changes/501"))
    r = CliRunner().invoke(cli.app, ["changes", "update", "CHN-501", "--set", "Subject=Billing: rotate all keys",
                                     "--yes", "--json"])
    assert r.exit_code == 5
    flat = " ".join(r.output.split())
    assert "HTTP 422" in flat and "subject: Synthetic too long" in flat


@pytest.mark.parametrize("status,body,expect", [
    (500, {"message": "Synthetic server error"}, "outcome unknown (HTTP 500: Synthetic server error)"),
    (422, {"errors": [{"field": "tags", "message": "Synthetic tag limit"}]}, "rejected (HTTP 422: tags: Synthetic tag limit)"),
    (502, "", "outcome unknown (HTTP 502 (empty response body))"),
])
def test_ticket_reviewed_update_names_status_and_message(tenant, monkeypatch, status, body, expect):
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: ticket_metadata())
    c = TicketClient()
    plan = props.build_plan(c, 123, tags=["Acme"])
    c.failure = APIError(status, body, "PUT", "tickets/123")
    with pytest.raises(props.PropertyError) as caught:
        props.send(plan, c, reviewed_hash=plan.hash)
    assert expect in str(caught.value) and len(c.writes) == 1


class _ProblemClient:
    def __init__(self, exc):
        self.exc = exc
        self.writes = []

    def int_get(self, path, params=None):
        return {"problem": {"id": 7, "deleted": False, "custom_fields": {"acme_resolve_by": None}}}

    def int_put(self, path, body=None):
        self.writes.append((path, body))
        raise self.exc


@pytest.mark.parametrize("status,body,expect", [
    (500, {"description": "Synthetic outage"}, "Synthetic outage"),
    (400, HTML_503, "Service Unavailable"),
])
def test_problem_update_failure_names_status_and_message(monkeypatch, status, body, expect):
    client = _ProblemClient(APIError(status, body, "PUT", "problems/7"))
    monkeypatch.setattr(cli, "_client", lambda: client)
    monkeypatch.setattr("fsv.schema.load", lambda *a, **k: SCHEMA)
    r = CliRunner().invoke(cli.app, ["problems", "update", "PRB-7", "--priority", "High", "--yes", "--json"])
    assert r.exit_code != 0 and len(client.writes) == 1
    flat = " ".join(r.output.split())
    assert f"HTTP {status}" in flat and expect in flat and "<h1>" not in flat
