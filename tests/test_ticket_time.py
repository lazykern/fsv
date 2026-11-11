"""Offline manual self-time CRUD; exact IDs only, no timers/billing."""
import copy
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, ticket_time as time
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError


class Client:
    def __init__(self):
        self.rows = []
        self.writes = []
        self.failure = None
        self.omit_id = False

    def me(self):
        return {"id": 901}

    def int_get(self, path, params=None):
        return {"time_entries": copy.deepcopy(self.rows), "has_more": False}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, path, **kw):
        body = json.loads(kw["content"]) if "content" in kw else None
        self.writes.append((method, path, body))
        if self.failure:
            raise self.failure
        if method == "POST":
            self.rows.append({"id": 456, **body})
        elif method == "PUT":
            self.rows[0].update(body)
        else:
            self.rows.clear()
        return {} if self.omit_id else {"time_entry": {"id": 456}}


@pytest.fixture(autouse=True)
def setup(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")


def create(c):
    return time.build_plan(c, "INC-123", action="create", spent="00:01",
                           executed_at="2030-01-02T10:00:00-05:00", note="Acme")


def test_exact_wire_crud_and_independent_readback():
    c = Client()
    p = create(c)
    assert p.payload == {"executed_at": "2030-01-02T10:00:00-05:00", "timer_running": False,
                         "billable": False, "time_spent": "00:01", "note": "Acme", "agent_id": 901}
    result = time.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] and result["entry_id"] == 456
    q = time.build_plan(c, 123, action="update", entry_id=456, note="Globex")
    assert q.payload == {"time_spent": "00:01", "note": "Globex"}
    assert time.send(q, c, reviewed_hash=q.hash)["verified"]
    d = time.build_plan(c, 123, action="delete", entry_id=456)
    assert d.payload is None and time.send(d, c, reviewed_hash=d.hash)["verified"]
    assert [w[0] for w in c.writes] == ["POST", "PUT", "DELETE"] and not c.rows


def test_unknown_response_id_never_infers_latest_or_resends():
    c = Client()
    c.omit_id = True
    p = create(c)
    result = time.send(p, c, reviewed_hash=p.hash)
    assert not result["verified"] and "do not resend" in result["warning"]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        time.send(p, c, reviewed_hash=p.hash)
    assert len(c.writes) == 1


@pytest.mark.parametrize("change", [{"agent_id": 902}, {"billable": True}, {"timer_running": True}])
def test_nonself_billable_or_timer_entry_refused(change):
    c = Client()
    c.rows = [{"id": 456, "agent_id": 901, "billable": False, "timer_running": False, "time_spent": "00:01", "note": "Acme", **change}]
    with pytest.raises(ReviewedWriteError, match="self-owned"):
        time.build_plan(c, 123, action="delete", entry_id=456)
    assert not c.writes


def test_stale_inventory_ambiguous_write_and_partial_read(monkeypatch):
    c = Client()
    p = create(c)
    c.rows.append({"id": 999})
    with pytest.raises(ReviewedWriteError, match="changed"):
        time.send(p, c, reviewed_hash=p.hash)
    c.rows.clear()
    c.failure = APIError(403, "uncertain")
    with pytest.raises(ReviewedWriteError, match="unknown"):
        time.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="unknown"):
        time.send(p, c, reviewed_hash=p.hash)
    assert len(c.writes) == 1
    monkeypatch.setattr(time, "collect_pages", lambda *a, **kw: ([], {"complete": False}, []))
    with pytest.raises(ReviewedWriteError, match="complete"):
        create(c)


@pytest.mark.parametrize("spent", ["1", "00:00", "00:60", "-1:00"])
def test_duration_validation(spent):
    with pytest.raises(ValueError, match="HH:MM"):
        time.build_plan(Client(), 123, action="create", spent=spent, executed_at="2030-01-02T10:00:00Z")


def test_cli_hash_bound_create(monkeypatch):
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    args = ["tickets", "time", "write", "INC-123", "create", "--spent", "00:01", "--executed-at", "2030-01-02T10:00:00Z", "--note", "Acme"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    result = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert result.exit_code == 0, result.output
    assert len(c.writes) == 1
