"""Synthetic readback normalization, including the real transport boundary."""
import copy
import json

import httpx
import pytest

from fsv import config, journal, ticket_time
from fsv.client import Client
from fsv.readback import same_duration, same_instant
from fsv.reviewed_write import ReviewedWriteError


@pytest.mark.parametrize("actual,expected,matched", [
    ("00:05:00", "00:05", True), ("125:05:00", "125:05", True),
    ("00:05:01", "00:05", False), ("00:06:00", "00:05", False),
    ("00:65:00", "01:05", False), (None, "00:05", False),
    (300, "00:05", False), ("00:05:00.0", "00:05", False),
])
def test_duration_semantics(actual, expected, matched):
    assert same_duration(actual, expected) is matched


@pytest.mark.parametrize("actual,matched", [
    ("2030-01-01T20:00:00Z", True), ("2030-01-02T03:00:00+07:00", True),
    ("2030-01-01T15:00:00-05:00", True), ("2030-01-01T20:00:00.000+00:00", True),
    ("2030-01-01T20:00:01Z", False), ("2030-01-02T03:00:00", False),
    ("2030-01-02", False), (None, False), (False, False), ("invalid", False),
])
def test_timestamp_semantics(actual, matched):
    assert same_instant(actual, "2030-01-02T03:00:00+07:00") is matched


@pytest.mark.parametrize("override,field,state", [
    ({}, None, None), ({"time_spent": "00:06:00"}, "time_spent", "mismatched"),
    ({"executed_at": "2030-01-01T20:00:01Z"}, "executed_at", "mismatched"),
    ({"executed_at": None}, "executed_at", "mismatched"),
    ({"billable": 0}, "billable", "mismatched"),
    ({"omit": "executed_at"}, "executed_at", "unavailable"),
])
def test_normalized_exact_id_transport_readback(monkeypatch, tmp_path, override, field, state):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    rows, writes = [], []

    def handle(request):
        if request.method == "GET":
            assert request.url.path.endswith("/tickets/123/time_entries")
            return httpx.Response(200, json={"time_entries": copy.deepcopy(rows), "has_more": False})
        body = json.loads(request.content)
        writes.append((request.method, body))
        row = {"id": 456, **body, "time_spent": "00:05:00", "executed_at": "2030-01-01T20:00:00Z"}
        row.update({k: v for k, v in override.items() if k != "omit"})
        if "omit" in override:
            row.pop(override["omit"])
        rows[:] = [row, {"id": 999, **body}]
        return httpx.Response(201, json={"time_entry": {"id": 456}})

    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handle))
    monkeypatch.setattr(c, "me", lambda: {"id": 901})
    monkeypatch.setattr(c, "_get_csrf", lambda: "synthetic")
    plan = ticket_time.build_plan(c, 123, action="create", spent="00:05", executed_at="2030-01-02T03:00:00+07:00")
    result = ticket_time.send(plan, c, reviewed_hash=plan.hash)
    assert result["entry_id"] == 456
    assert result["verified"] is (field is None)
    if field:
        assert result["fields"][field] == state
    assert len(writes) == 1
    assert writes[0][1]["time_spent"] == "00:05"
    assert writes[0][1]["executed_at"] == "2030-01-02T03:00:00+07:00"
    with pytest.raises(ReviewedWriteError, match="was sent"):
        ticket_time.send(plan, c, reviewed_hash=plan.hash)
    c.close()


def test_note_only_update_preserves_normalized_minutes(monkeypatch):
    class ReadClient:
        def me(self):
            return {"id": 901}

        def int_get(self, path, params=None):
            return {"time_entries": [{"id": 456, "agent_id": 901, "billable": False,
                    "timer_running": False, "time_spent": "125:05:00", "note": "Acme"}], "has_more": False}

    plan = ticket_time.build_plan(ReadClient(), 123, action="update", entry_id=456, note="Globex")
    assert plan.payload == {"time_spent": "125:05", "note": "Globex"}
