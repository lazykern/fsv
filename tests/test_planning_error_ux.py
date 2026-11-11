"""Local planning mistakes are clean usage failures, not fake HTTP failures."""
import pytest
from typer.testing import CliRunner

from fsv import cli, completion, create, planning_cli, planning_text
from fsv.errors import APIError, UsageError, exit_code_for
from fsv.resources import CHANGES, PROBLEMS
from test_planning_text import Client, setup  # noqa: F401


@pytest.mark.parametrize("section", ["Unknown", "Rollout Plann"])
def test_unknown_section_has_clean_choices_and_no_write(monkeypatch, section):
    client = Client()
    monkeypatch.setattr(cli, "_client", lambda: client)
    result = CliRunner().invoke(cli.app, ["changes", "planning-text", "CHN-123", "--section", section,
                                        "--description", "Acme", "--dry-run"])
    assert result.exit_code == 2, result.output
    assert "planning field not found" in result.output
    assert "choices: Rollout Plan" in result.output
    assert "Traceback" not in result.output and "HTTP 0" not in result.output
    assert not client.writes


def test_all_choices_listed_not_only_first_twelve():
    fields = [{"name": f"acme_{i}", "label": f"Acme section {i}", "field_type": "planning_field"}
              for i in range(15)]
    with pytest.raises(UsageError) as exc:
        create.resolve_planning_definition("Unknown", fields)
    assert "Acme section 14" in str(exc.value)
    assert exit_code_for(exc.value) == 2


def test_ambiguous_label_is_usage_error():
    fields = [{"name": name, "label": "Acme Plan", "field_type": "planning_field"}
              for name in ("acme_a", "acme_b")]
    with pytest.raises(UsageError, match="ambiguous planning field") as exc:
        create.resolve_planning_definition("Acme Plan", fields)
    assert exit_code_for(exc.value) == 2


@pytest.mark.parametrize("resource,prefix,expected", [
    (CHANGES, "Acme", "Acme Plan"), (PROBLEMS, "Ro", "Root Cause"),
    (PROBLEMS, "problem_impact", "Impact"),
])
def test_section_completion_uses_metadata_or_native_analysis(monkeypatch, resource, prefix, expected):
    monkeypatch.setattr(completion, "_fields_or_fallback", lambda res: [
        {"name": "acme_plan", "label": "Acme Plan", "field_type": "planning_field"}])
    assert expected in [value for value, _ in planning_cli.complete_section(resource)(prefix)]


@pytest.mark.parametrize("status,code", [(403, 5), (429, 5)])
def test_api_failure_is_clean_not_a_traceback(monkeypatch, status, code):
    client = Client()
    monkeypatch.setattr(cli, "_client", lambda: client)
    def denied(*args, **kwargs):
        raise APIError(status, "synthetic denial")
    monkeypatch.setattr(planning_text, "definitions", denied)
    result = CliRunner().invoke(cli.app, ["changes", "planning-text", "CHN-123", "--section", "Acme",
                                        "--description", "Acme", "--dry-run"])
    assert result.exit_code == code
    assert f"HTTP {status}" in result.output and "Traceback" not in result.output
    assert not client.writes


def test_runtime_local_guard_never_claims_http_zero():
    error = APIError(0, "attachment upload returned no id")
    assert "HTTP 0" not in str(error)
    assert exit_code_for(error) == 5


@pytest.mark.parametrize("helper", ["attachments", "planning"])
def test_duplicate_mode_is_local_usage_error(helper):
    class Client:
        def int_get(self, path, params=None):
            return {"change": {"attachments": []}, "change_planning_fields": []}

    with pytest.raises(UsageError) as exc:
        if helper == "attachments":
            create.attach_files_to_change(123, [], Client(), duplicate="unknown")
        else:
            create.update_planning_field(123, "acme_plan", description="Acme", c=Client(), duplicate="unknown")
    assert isinstance(exc.value, APIError) and isinstance(exc.value, ValueError)
    assert exit_code_for(exc.value) == 2 and "HTTP 0" not in str(exc.value)


def test_unknown_problem_section_also_lists_native_choices(monkeypatch):
    client = Client("problems")
    monkeypatch.setattr(cli, "_client", lambda: client)
    result = CliRunner().invoke(cli.app, ["problems", "planning-text", "PRB-123", "--section", "Unknown",
                                        "--description", "Acme", "--dry-run"])
    assert result.exit_code == 2
    assert "Root Cause, Impact or Symptoms" in result.output
    assert "Traceback" not in result.output and "HTTP 0" not in result.output
    assert not client.writes
