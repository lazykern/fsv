"""Problem planned dates and effort: metadata-gated reviewed PUT of only the
dirty keys (synthetic tenant)."""
import copy
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, formmeta, journal, problem_properties as flow
from fsv.client import Client
from fsv.reviewed_write import ReviewedWriteError

PLANNING = [{"id": 521, "name": "planned_start_date", "label": "Planned Start Date", "field_type": "default_planned_start_date", "position": 3},
            {"id": 522, "name": "planned_end_date", "label": "Planned End Date", "field_type": "default_planned_end_date", "position": 4},
            {"id": 523, "name": "planned_effort", "label": "Planned Effort", "field_type": "default_planned_effort", "position": 5}]


@pytest.fixture
def api(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(config, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.setattr(formmeta, "configured_workspace", lambda: 17)
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    fields = [{"id": 501, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1},
              {"id": 502, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
               "choices": [{"id": 31, "value": "Open"}]}, *copy.deepcopy(PLANNING)]
    s = {"row": {"id": 123, "subject": "Acme problem", "status": 31, "planned_start_date": None,
                 "planned_end_date": None, "planned_effort": None}, "writes": [], "fields": fields, "rules": 200}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "problem_form_fields":
                return httpx.Response(200, json={"problem_fields": s["fields"]})
            if path.endswith("business_rules"):
                return httpx.Response(s["rules"], json={"business_rules": []} if s["rules"] == 200 else {"e": 1})
            assert path == "problems/123"
            return httpx.Response(200, json={"problem": copy.deepcopy(s["row"])})
        body = json.loads(req.content)
        s["writes"].append((req.method, path, body))
        s["row"].update(body)
        if "planned_effort" in body:
            s["row"]["planned_effort"] = body["planned_effort"].replace(" 0m", "")  # server drops zero units
        return httpx.Response(200, json={})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    monkeypatch.setattr(cli, "_client", lambda: c)
    yield c, s
    c.close()


def test_dates_and_effort_send_only_dirty_keys_and_verify(api):
    c, s = api
    p = flow.build_plan(c, "PRB-123", planned_start="2026-12-05T09:00:00+07:00", planned_effort="2h")
    assert p.payload == {"planned_start_date": "2026-12-05T09:00:00+07:00", "planned_effort": "2h 0m"}
    assert p.method == "PUT" and p.path == "problems/123"
    result = flow.send(p, c, reviewed_hash=p.hash)
    assert result["verified"] is True
    assert s["writes"] == [("PUT", "problems/123", p.payload)]
    with pytest.raises(ReviewedWriteError, match="was sent"):
        flow.send(p, c, reviewed_hash=p.hash)


def test_cli_problem_update_is_not_ticket_only(api):
    c, s = api
    r = CliRunner().invoke(cli.app, ["problems", "update", "PRB-123", "--planned-start", "2026-12-05T09:00:00Z",
                                     "--planned-end", "2026-12-06T09:00:00Z", "--planned-effort", "01:30", "--dry-run"])
    assert r.exit_code == 0, r.output
    review = json.loads(r.output)
    assert review["payload"] == {"planned_start_date": "2026-12-05T09:00:00+00:00",
                                 "planned_end_date": "2026-12-06T09:00:00+00:00", "planned_effort": "1h 30m"}
    assert not s["writes"]
    r = CliRunner().invoke(cli.app, ["tickets", "update", "INC-1", "--planned-effort", "1h", "--dry-run"])
    assert r.exit_code != 0 and "only for problems" in r.output


def test_field_missing_from_metadata_or_bad_input_refuses(api):
    c, s = api
    s["fields"] = [f for f in s["fields"] if f["name"] != "planned_effort"]
    with pytest.raises(ReviewedWriteError, match="no planned_effort field"):
        flow.build_plan(c, 123, planned_effort="1h")
    with pytest.raises(ValueError, match="explicit offset"):
        flow.build_plan(c, 123, planned_start="2026-12-05 09:00")
    with pytest.raises(ValueError, match="precede"):
        flow.build_plan(c, 123, planned_start="2026-12-06T09:00:00Z", planned_end="2026-12-05T09:00:00Z")
    assert not s["writes"]


def test_rules_500_warns_and_still_reviews(api):
    c, s = api
    s["rules"] = 500
    p = flow.build_plan(c, 123, planned_end="2026-12-06T09:00:00Z")
    assert "rules unavailable" in p.review()["warnings"][0]
