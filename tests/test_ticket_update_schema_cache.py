"""tickets update reviews through the form cache: inside the 15-minute TTL a
dry-run sends no ticket_form_fields GET; a refusal a newer form could lift
refetches once; the send's own rebuild always refetches. Synthetic tenant."""
import copy
import json

import pytest
from typer.testing import CliRunner

import acme_change_form as a
import fsv.cli as cli
from fsv import config, journal, ticket_properties as props


def _fields(extra_choice=False):
    choices = [{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}]
    if extra_choice:
        choices.append({"id": 3, "value": "Urgent Acme"})
    return {"ticket_fields": [a._f("subject", "Subject", "default_subject", 1),
                              a._f("priority", "Priority", "default_priority", 2, choices=choices)]}


class Client:
    def __init__(self):
        self.row = {"id": 123, "subject": "Acme", "priority": 1, "tags": ["old"],
                    "updated_at": "2030-01-01T00:00:00Z", "workspace_id": 1}
        self.fields = _fields()
        self.gets, self.writes = [], []

    def int_get(self, path, params=None):
        self.gets.append(path)
        if path == "ticket_form_fields":
            return copy.deepcopy(self.fields)
        if path.startswith("tickets/"):
            return {"ticket": dict(self.row)}
        if "business_rules" in path:
            return {"business_rules": []}
        return {}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, path, **kw):
        body = json.loads(kw["content"])
        self.writes.append((method, path, body))
        self.row.update({k: v for k, v in body.items() if k != "br_validation_excludes"})
        self.row["updated_at"] = "2030-01-01T00:05:00Z"
        return {"ticket": self.row}


@pytest.fixture
def c(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    client = Client()
    monkeypatch.setattr(cli, "_client", lambda: client)
    return client


def run(*args):
    return CliRunner().invoke(cli.app, ["tickets", "update", "INC-123", "--no-input", *args])


def test_dry_run_inside_ttl_sends_no_form_fields_get(c):
    first = run("--tag", "new", "--dry-run")
    assert first.exit_code == 0, first.output
    assert c.gets.count("ticket_form_fields") == 1
    second = run("--tag", "new", "--dry-run")
    assert second.exit_code == 0, second.output
    assert c.gets.count("ticket_form_fields") == 1  # served from the form cache
    assert json.loads(first.stdout)["plan_hash"] == json.loads(second.stdout)["plan_hash"]


def test_send_rebuild_refetches_and_verifies(c):
    digest = json.loads(run("--tag", "new", "--dry-run").stdout)["plan_hash"]
    before = c.gets.count("ticket_form_fields")
    sent = run("--tag", "new", "--yes", "--expect-plan", digest)
    assert sent.exit_code == 0, sent.output
    assert c.gets.count("ticket_form_fields") == before + 1  # the write's own fresh rebuild
    assert len(c.writes) == 1


def test_form_changed_since_review_is_refused_at_send(c):
    digest = json.loads(run("--priority", "Medium", "--dry-run").stdout)["plan_hash"]
    c.fields = _fields(extra_choice=True)  # an admin edited the form after the review
    sent = run("--priority", "Medium", "--yes", "--expect-plan", digest)
    assert sent.exit_code == 2 and "changed since review" in sent.output and not c.writes


def test_value_not_in_the_cached_form_refetches_once(c):
    assert run("--priority", "Medium", "--dry-run").exit_code == 0
    c.fields = _fields(extra_choice=True)  # new choice added inside the TTL
    n = c.gets.count("ticket_form_fields")
    r = run("--priority", "Urgent Acme", "--dry-run")
    assert r.exit_code == 0, r.output
    assert "refetching the ticket form once" in r.output
    assert c.gets.count("ticket_form_fields") == n + 1
    # a value no form offers is refused after the single refetch (no loop)
    n = c.gets.count("ticket_form_fields")
    bad = run("--priority", "Nope", "--dry-run")
    assert bad.exit_code == 2 and c.gets.count("ticket_form_fields") == n + 1 and not c.writes


def test_build_plan_fresh_bypasses_the_cache(c):
    props.build_plan(c, 123, tags=["x"])
    n = c.gets.count("ticket_form_fields")
    props.build_plan(c, 123, tags=["x"])
    assert c.gets.count("ticket_form_fields") == n
    props.build_plan(c, 123, tags=["x"], fresh=True)
    assert c.gets.count("ticket_form_fields") == n + 1
