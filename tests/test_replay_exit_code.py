"""Every journal replay refusal (plan already sent, or an earlier send of it
has no recorded outcome) exits with one code, EXIT_REPLAY (2), distinct from
4 (sent but unverified). Walks the repeatable reviewed-write commands through
the real CLI, the real stale --expect-plan lookup and the real send guards
(synthetic tenant, builders stubbed, no HTTP)."""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any

import pytest
from typer.testing import CliRunner

from fsv import (cli, config, journal, problem_incidents, problem_properties, problem_status, ticket_assignment,
                 ticket_lifecycle)
from fsv.errors import EXIT_REPLAY
from test_changes_form_cli import fake  # noqa: F401

TENANT = "acme.example.com"


@dataclass
class FakePlan:
    hash: str
    action: str
    target: int = 7
    tenant: str = TENANT
    context_hash: str = "{}"
    payload: dict[str, Any] = field(default_factory=dict)
    intent_json: str = '{"ids": [7]}'
    label: str = "Open"
    prior_context: str = ""

    @property
    def write(self) -> "FakePlan":  # AssignmentPlan shape
        return self

    @property
    def prior_hash(self) -> str:  # LifecyclePlan shape
        return "f" * 64

    def review(self) -> dict[str, Any]:
        return {"action": self.action, "plan_hash": self.hash}


class NoHTTP:
    def __getattr__(self, name: str) -> Any:
        raise AssertionError(f"no HTTP expected ({name})")


# (argv, module whose build_plan is stubbed, journal action)
COMMANDS = [
    (["problems", "status", "PRB-7", "--to", "Open"], problem_status, "problem_status"),
    (["problems", "link-incident", "PRB-7", "INC-9"], problem_incidents, "problem_link_incident"),
    (["problems", "unlink-incident", "PRB-7", "INC-9"], problem_incidents, "problem_unlink_incident"),
    (["problems", "update", "PRB-7", "--planned-effort", "2h"], problem_properties, "problem_planning"),
    (["tickets", "status", "INC-7", "Pending"], ticket_lifecycle, "ticket_transition"),
    (["tickets", "resolve", "INC-7", "--note", "Acme fixed"], ticket_lifecycle, "ticket_transition"),
    (["tickets", "close", "INC-7"], ticket_lifecycle, "ticket_transition"),
    (["tickets", "reopen", "INC-7"], ticket_lifecycle, "ticket_transition"),
    (["tickets", "assign", "INC-7", "--group", "Acme Desk"], ticket_assignment, "ticket_assign"),
    (["tickets", "pick-up", "INC-7"], ticket_assignment, "ticket_pick_up"),
]


@pytest.fixture
def offline(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", TENANT)
    monkeypatch.setattr(config, "API_INT", f"https://{TENANT}/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal.jsonl")
    monkeypatch.setattr(cli, "_client", lambda: NoHTTP())
    for factory in ("client_factory", "_client_factory"):
        if hasattr(cli, factory):
            monkeypatch.setattr(cli, factory, lambda: NoHTTP())


def _stub(monkeypatch, module, plan: FakePlan) -> None:
    monkeypatch.setattr(module, "build_plan", lambda *a, **k: plan)


@pytest.mark.parametrize("argv,module,action", COMMANDS, ids=[" ".join(c[0][:2]) + f"-{i}" for i, c in enumerate(COMMANDS)])
@pytest.mark.parametrize("case", ["replay", "stale", "unresolved"])
def test_every_replay_refusal_exits_with_one_code(offline, monkeypatch, argv, module, action, case):
    sent, current = "a" * 64, "b" * 64
    if case == "replay":  # the very same reviewed hash again: the send guard refuses
        journal.record_plan("started", sent, action=action, target=7)
        journal.record_plan("done", sent, action=action, target=7)
        _stub(monkeypatch, module, FakePlan(sent, action))
        expected, marker = sent, "already sent"
    elif case == "stale":  # the record moved on; the old reviewed hash names the earlier send
        journal.record_plan("started", sent, action=action, target=7)
        journal.record_plan("done", sent, action=action, target=7)
        _stub(monkeypatch, module, FakePlan(current, action))
        expected, marker = sent, "already sent"
    else:  # an earlier send of this plan has no recorded outcome
        journal.record_plan("started", sent, action=action, target=7)
        _stub(monkeypatch, module, FakePlan(sent, action))
        expected, marker = sent, "unknown"
    r = CliRunner().invoke(cli.app, [*argv, "--yes", "--expect-plan", expected])
    assert r.exit_code == EXIT_REPLAY == 2, (argv, r.output)
    assert marker in r.output, r.output
    rows = [e for e in journal.read() if e.get("kind") == f"plan:{action}"]
    assert [e["event"] for e in rows][-1] in ("done", "started") and len(rows) <= 2  # nothing new sent


def test_replay_exit_differs_from_unverified():
    from fsv import errors
    assert errors.EXIT_REPLAY == errors.EXIT_USAGE and errors.EXIT_REPLAY != errors.EXIT_CONFLICT


def test_refusal_flag_does_not_leak_into_the_next_command(offline, monkeypatch):
    """A refusal in one invocation must not turn a later unrelated exit 4
    (sent, unverified) into 2."""
    journal.note_refusal("earlier")
    plan = FakePlan("c" * 64, "problem_status")
    _stub(monkeypatch, problem_status, plan)
    monkeypatch.setattr(problem_status, "send", lambda *a, **k: {"status": "submitted", "verified": False})
    r = CliRunner().invoke(cli.app, ["problems", "status", "PRB-7", "--to", "Open", "--yes", "--expect-plan", plan.hash])
    assert r.exit_code == 4, r.output
    assert json.loads(r.stdout[r.stdout.index("{"):])["verified"] is False


def test_changes_update_stale_replay_exits_2_not_4(fake):  # noqa: F811 - fixture from test_changes_form_cli
    from test_changes_form_cli import run
    hashes = []
    for priority in ("High", "Low"):
        review = json.loads(run("changes", "update", "CHN-501", "--priority", priority, "--dry-run", "--json").raw)
        sent = run("changes", "update", "CHN-501", "--priority", priority, "--yes", "--expect-plan", review["plan_hash"])
        assert sent.exit_code in (0, 4), sent.output
        hashes.append(review["plan_hash"])
    writes = len(fake.writes())
    again = run("changes", "update", "CHN-501", "--priority", "High", "--yes", "--expect-plan", hashes[0])
    assert again.exit_code == EXIT_REPLAY and "already sent" in again.output and len(fake.writes()) == writes
