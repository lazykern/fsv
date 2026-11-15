"""`tickets update` is reviewed by default (synthetic tenant): plan hash with a
record-state token, one journaled send, independent readback (`verified`),
replay refusals exit 2; `--no-review` is the deprecated body-only path."""
import json

import pytest
from typer.testing import CliRunner

import acme_change_form as a
from fsv import cli, config, journal, ticket_properties as props
from fsv.errors import APIError
from fsv.formmeta import FormMeta

SCHEMA = {"fields": [
    {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": True,
     "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]},
]}


def metadata():
    fields = [a._f("subject", "Subject", "default_subject", 1),
              a._f("priority", "Priority", "default_priority", 2,
                   choices=[{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}])]
    for ident, field in enumerate(fields, 101):  # stable ids: the hash binds the metadata
        field["id"] = ident
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 1, 0, "fixture", 11)


class Client:
    def __init__(self):
        self.row = {"id": 11, "human_display_id": "SR-11", "subject": "Acme laptop", "priority": 1,
                    "updated_at": "2030-01-01T00:00:00Z"}
        self.writes = []
        self.failure = None
        self.echo = True
        self.bump = True
        self.version = 0

    def int_get(self, path, params=None):
        assert path == "tickets/11"
        return {"ticket": dict(self.row)}

    def _get_csrf(self):
        return "example-token"

    def _request(self, method, path, **kw):
        body = json.loads(kw["content"])
        self.writes.append((method, path.rsplit("/api/_/", 1)[-1], body))
        if self.failure:
            raise self.failure
        if self.echo:
            self._apply(body)
        return {"ticket": self.row}

    def int_put(self, path, body):  # the deprecated body-only path
        self.writes.append(("PUT", path, body))
        self._apply(body)
        return {"ticket": self.row}

    def _apply(self, body):
        self.version += 1
        self.row.update({k: v for k, v in body.items() if k != "br_validation_excludes"})
        if self.bump:
            self.row["updated_at"] = f"2030-01-01T00:00:{self.version:02d}Z"


@pytest.fixture
def c(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: metadata())
    monkeypatch.setattr("fsv.schema.load", lambda *args: SCHEMA)
    client = Client()
    monkeypatch.setattr(cli, "_client", lambda: client)
    return client


def run(*args):
    return CliRunner().invoke(cli.app, ["tickets", "update", *args])


def review(*args):
    r = run(*args, "--dry-run")
    assert r.exit_code == 0, r.output
    return json.loads(r.stdout)


@pytest.mark.parametrize("target", ["INC-11", "SR-11", "11"])
def test_default_dry_run_shows_plan_hash_and_record_state_without_reviewed(c, target):
    plan = review(target, "--priority", "High")
    assert plan["action"] == "ticket_properties" and len(plan["plan_hash"]) == 64
    assert plan["record_state"] == {"sends": 0, "server_version": "2030-01-01T00:00:00Z"}
    assert plan["payload"]["priority"] == 3 and not c.writes
    # --reviewed is a no-op alias of the default.
    assert review(target, "--priority", "High", "--reviewed")["plan_hash"] == plan["plan_hash"]


def test_send_verifies_and_replay_exits_2(c):
    digest = review("SR-11", "--priority", "High")["plan_hash"]
    sent = run("SR-11", "--priority", "High", "--yes", "--expect-plan", digest, "--json")
    assert sent.exit_code == 0, sent.output
    result = json.loads(sent.stdout)
    assert result["verified"] is True and result["plan_hash"] == digest and result["fields"]["priority"] == "matched"
    assert [w[:2] for w in c.writes] == [("PUT", "tickets/11")]
    replay = run("SR-11", "--priority", "High", "--yes", "--expect-plan", digest)
    assert replay.exit_code == 2 and "already sent" in replay.output and len(c.writes) == 1
    kinds = [e.get("event") for e in journal.read() if e.get("kind") == "plan:ticket_properties"]
    assert kinds == ["started", "done"]


def test_yes_without_the_full_hash_sends_nothing(c):
    digest = review("INC-11", "--priority", "High")["plan_hash"]
    assert run("INC-11", "--priority", "High", "--yes").exit_code == 2
    assert run("INC-11", "--priority", "High", "--yes", "--expect-plan", digest[:16]).exit_code == 2
    assert not c.writes


def test_ab_alternation_four_times_plans_distinct_hashes(c):
    hashes = []
    for value in ["High", "Low", "High", "Low"]:
        digest = review("INC-11", "--priority", value)["plan_hash"]
        sent = run("INC-11", "--priority", value, "--yes", "--expect-plan", digest)
        assert sent.exit_code == 0, sent.output
        hashes.append(digest)
    assert len(set(hashes)) == 4 and [w[2]["priority"] for w in c.writes] == [3, 1, 3, 1]


def test_same_values_repeat_with_unchanged_server_version_still_plans_fresh_hash(c):
    """Even when the server keeps updated_at, the verified-send count moves on."""
    c.bump = False
    first = review("INC-11", "--priority", "High")
    assert run("INC-11", "--priority", "High", "--yes", "--expect-plan", first["plan_hash"]).exit_code == 0
    second = review("INC-11", "--priority", "High")
    assert second["record_state"]["sends"] == 1 and second["plan_hash"] != first["plan_hash"]


def test_ambiguous_outcome_is_not_replayable(c):
    digest = review("INC-11", "--priority", "High")["plan_hash"]
    c.failure = APIError(503, "synthetic outage")
    first = run("INC-11", "--priority", "High", "--yes", "--expect-plan", digest)
    assert first.exit_code == 2 and "outcome unknown" in first.output
    c.failure = None
    # The record did not move and nothing was verified: a fresh review plans
    # the same hash, and sending it is refused.
    again = review("INC-11", "--priority", "High")
    assert again["plan_hash"] == digest and again["record_state"]["sends"] == 0
    second = run("INC-11", "--priority", "High", "--yes", "--expect-plan", digest)
    assert second.exit_code == 2 and "unknown" in second.output and len(c.writes) == 1


def test_unverified_readback_exits_4_and_does_not_count_as_a_send(c):
    c.echo = False
    digest = review("INC-11", "--priority", "High")["plan_hash"]
    sent = run("INC-11", "--priority", "High", "--yes", "--expect-plan", digest, "--json")
    assert sent.exit_code == 4 and json.loads(sent.stdout)["verified"] is False
    assert review("INC-11", "--priority", "High")["plan_hash"] == digest  # no fresh plan after an unverified send


def test_no_review_prints_one_notice_and_sends_body_only(c):
    dry = run("--no-review", "SR-11", "--priority", "High", "--dry-run", "--json")
    assert dry.exit_code == 0, dry.output
    assert json.loads(dry.stdout)["body"] == {"priority": 3}  # stdout stays pure JSON
    notice = [line for line in dry.stderr.splitlines() if line.strip()]
    assert len(notice) == 1 and "--no-review is deprecated" in notice[0] and "next release" in notice[0]
    sent = run("--no-review", "SR-11", "--priority", "High", "--yes")
    assert sent.exit_code == 0, sent.output
    assert "--no-review is deprecated" in sent.stderr
    assert c.writes == [("PUT", "tickets/11", {"priority": 3})]
    assert not [e for e in journal.read() if str(e.get("kind", "")).startswith("plan:")]


def test_no_review_refuses_reviewed_only_options(c):
    for extra in (["--ack"], ["--tag", "acme"], ["--planned-start", "2030-01-02T09:00:00Z"]):
        r = run("--no-review", "INC-11", "--priority", "High", *extra, "--dry-run")
        assert r.exit_code == 2 and "drop --no-review" in r.output
    assert not c.writes
