"""Problem analysis uses the same native inventory as planning-text readback."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli
from fsv.errors import APIError


@pytest.mark.parametrize("output", ["json", "table"])
@pytest.mark.parametrize("shape", ["content", "description"])
def test_problem_get_exposes_all_analysis_sections(monkeypatch, output, shape):
    calls = []

    class Client:
        def int_get(self, path, params=None):
            calls.append(path)
            if path == "problems/123":
                return {"problem": {"id": 123, "display_id": 123, "subject": "Acme incident"}}
            if path == "problems/123/planning-fields":
                return {"problem_planning_fields": [
                    {"name": "problem_cause", shape: "<div>Acme cause &amp; evidence</div>", "files": []},
                    {"name": "problem_impact", shape: "Acme impact", "files": [{"name": "evidence.txt"}]},
                    {"name": "problem_symptom", shape: "Acme symptoms", "files": []},
                ]}
            raise APIError(404, "unavailable")

    monkeypatch.setattr(cli, "_client", Client)
    monkeypatch.setattr("fsv.schema.load", lambda *a, **kw: {"fields": []})
    result = CliRunner().invoke(cli.app, ["problems", "get", "PRB-123", "--output", output])
    assert result.exit_code == 0, result.output
    assert "problems/123/planning-fields" in calls
    for text in ("Root Cause", "Impact", "Symptoms", "Acme cause & evidence", "Acme impact", "Acme symptoms", "evidence.txt"):
        assert text in result.output
    if output == "json":
        sections = json.loads(result.output)["view"]["details"]["planning"]
        assert [p["name"] for p in sections] == ["problem_cause", "problem_impact", "problem_symptom"]


@pytest.mark.parametrize("payload", [{}, {"problem_planning_fields": None}, {"problem_planning_fields": [42]}])
def test_unavailable_analysis_is_not_reported_as_empty(payload):
    from fsv.views import load_planning

    class Client:
        def int_get(self, path, params=None):
            return payload

    rows, missing = load_planning(Client(), {"id": 123}, {}, module="problems")
    assert rows is None and missing is not None
