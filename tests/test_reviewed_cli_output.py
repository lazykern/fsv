"""Output aliases are accepted consistently without weakening review gates."""
import json
from types import SimpleNamespace

import pytest
from typer.testing import CliRunner

from fsv import (cli, watchers, ticket_time, planning_text, problem_known_error,
                 bulk_priority, ticket_conversation)


CASES = [
    (["tickets", "watchers", "set", "INC-123", "--watch"], watchers),
    (["changes", "watchers", "set", "CHN-123", "--watch"], watchers),
    (["tickets", "time", "write", "INC-123", "create", "--spent", "00:05",
      "--executed-at", "2030-01-02T03:00:00+07:00"], ticket_time),
    (["changes", "planning-text", "CHN-123", "--section", "Rollout Plan", "--description", "Acme"], planning_text),
    (["problems", "planning-text", "PRB-123", "--section", "Impact", "--description", "Acme"], planning_text),
    (["problems", "mark-known-error", "PRB-123"], problem_known_error),
    (["tickets", "bulk-priority", "INC-123", "--priority", "High"], bulk_priority),
    (["tickets", "forward", "INC-123", "Acme", "--to", "reader@example.com", "--sender", "support@example.com"], ticket_conversation),
]


@pytest.mark.parametrize("args,module", CASES)
@pytest.mark.parametrize("flags", [[], ["--json"], ["--output", "json"], ["-o", "json"], ["--output=json", "--json"]])
def test_review_and_send_output_aliases(monkeypatch, args, module, flags):
    calls = []
    monkeypatch.setattr(cli, "_client", lambda: object())
    plan = SimpleNamespace(hash="a" * 64, review=lambda: {"plan_hash": "a" * 64})
    monkeypatch.setattr(module, "build_plan", lambda *a, **k: plan)
    monkeypatch.setattr(module, "send", lambda *a, **k: calls.append(k) or {"status": "submitted", "verified": True})
    monkeypatch.setattr(ticket_conversation, "recipients", lambda *a: None)
    runner = CliRunner()
    review = runner.invoke(cli.app, args + flags + ["--dry-run"])
    assert review.exit_code == 0, review.output
    assert json.loads(review.output) == {"status": "dry_run", "plan_hash": plan.hash,
                                         "tenant": cli.config.DOMAIN, "config_dir": str(cli.config.CONFIG_DIR)}
    assert not calls
    refused = runner.invoke(cli.app, args + flags + ["--yes"])
    assert refused.exit_code == 2 and not calls
    sent = runner.invoke(cli.app, args + flags + ["--yes", "--expect-plan", plan.hash])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.output)["verified"] and len(calls) == 1
    assert calls[0]["reviewed_hash"] == plan.hash


@pytest.mark.parametrize("args,module", CASES)
def test_unsupported_format_refused_before_client(monkeypatch, args, module):
    def forbidden():
        pytest.fail("invalid output must not create a client")
    monkeypatch.setattr(cli, "_client", forbidden)
    result = CliRunner().invoke(cli.app, args + ["--output", "yaml", "--dry-run"])
    assert result.exit_code == 2


@pytest.mark.parametrize("args,module,key,value", [
    (["tickets", "watchers", "list", "INC-123"], watchers, "watcher_ids", [901]),
    (["changes", "watchers", "list", "CHN-123"], watchers, "watcher_ids", [901]),
    (["tickets", "time", "list", "INC-123"], ticket_time, None, [{"id": 456}]),
])
@pytest.mark.parametrize("flags", [["--json"], ["--output", "json"], ["-o", "json"]])
def test_read_output_aliases(monkeypatch, args, module, key, value, flags):
    monkeypatch.setattr(cli, "_client", lambda: object())
    monkeypatch.setattr(module, "inventory", lambda *a, **k: value)
    result = CliRunner().invoke(cli.app, args + flags)
    assert result.exit_code == 0, result.output
    assert json.loads(result.output) == ({key: value} if key else value)
