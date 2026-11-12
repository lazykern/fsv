"""Change-task reminders and statuses come from runtime metadata, never from
the platform-default tables (shifted ids, renamed choices)."""
import json

import pytest
from typer.testing import CliRunner

from fsv import change_tasks as tasks, cli, config, journal

SHIFTED = [{"name": "status", "choices": [{"id": 7, "value": "Todo"}, {"id": 8, "value": "Doing"},
                                          {"id": 9, "value": "Done"}]},
           {"name": "notify_before", "choices": [{"id": 0, "value": "None"}, {"id": 600, "value": "10 Min"}]}]


@pytest.fixture
def shifted(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    monkeypatch.setattr(tasks, "runtime_fields", lambda: SHIFTED)
    monkeypatch.setattr(tasks, "resolve_status", lambda value, client=None: tasks.task_status.resolve(value, SHIFTED)[0])
    return SHIFTED


def run(*args):
    return CliRunner().invoke(cli.app, ["changes", "tasks-add", "CHN-77", "--title", "Globex task", "--dry-run", *args])


def test_defaults_are_the_runtime_first_choices_never_id_1(shifted):
    r = run()
    assert r.exit_code == 0, r.output
    payload = json.loads(r.stdout)["payload"]
    assert payload["status"] == 7 and payload["notify_before"] == 0
    assert journal.read() == []


def test_renamed_reminder_sends_runtime_seconds_and_static_label_is_refused(shifted):
    r = run("--notify-before", "10 Min", "--status", "Doing")
    assert r.exit_code == 0, r.output
    payload = json.loads(r.stdout)["payload"]
    assert payload == {"title": "Globex task", "status": 8, "notify_before": 600}
    r = run("--notify-before", "Never")
    assert r.exit_code == 2 and "runtime choices: None, 10 Min" in r.output
    r = run("--notify-before", "15 Minutes")
    assert r.exit_code == 2


def test_library_plan_with_metadata_ignores_the_static_table(shifted):
    plan = tasks.build_plan(77, "Globex task", status="Todo", status_id=7, notify_before="600", fields=SHIFTED)
    assert plan.notify_before == "10 Min" and plan.payload["notify_before"] == 600
    with pytest.raises(ValueError):
        tasks.build_plan(77, "Globex task", status_id=7, notify_before="30 Minutes", fields=SHIFTED)
    with pytest.raises(tasks.TaskError, match="reminder metadata unavailable"):
        tasks.build_plan(77, "Globex task", status_id=7, fields=[SHIFTED[0]])


def test_runtime_choice_helpers(shifted):
    assert tasks.status_choices(SHIFTED) == ["Todo", "Doing", "Done"]
    assert tasks.default_status(SHIFTED) == "Todo"
    assert tasks.reminder_choices(SHIFTED) == [(0, "None"), (600, "10 Min")]
    assert tasks.runtime_reminder_label(None, SHIFTED) == "None"
