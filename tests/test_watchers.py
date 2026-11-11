"""Captured self-watch paths, immutable reviews and ambiguity handling."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, watchers
from fsv.errors import APIError
from fsv.reviewed_write import ReviewedWriteError


class Client:
    def __init__(self):
        self.user = 901
        self.members = [902]
        self.writes = []
        self.failure = None
        self.read_failure = False

    def me(self):
        return {"id": self.user}

    def int_get(self, path):
        assert path.endswith("/watchers")
        if self.read_failure:
            raise APIError(403, "denied")
        return {"watcher_ids": list(self.members)}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kw):
        body = json.loads(kw["content"]) if "content" in kw else None
        self.writes.append((method, url, body))
        if self.failure:
            raise self.failure
        if url.endswith(("/watch", "/watchers")):
            self.members.append(self.user)
        else:
            self.members.remove(self.user)
        return {}


@pytest.fixture(autouse=True)
def setup(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")


@pytest.mark.parametrize("module", ["tickets", "changes"])
def test_self_watch_unwatch_preserves_other_members_and_exact_methods(module):
    c = Client()
    p = watchers.build_plan(c, module, 123, watch=True)
    assert p.payload == {"user_id": 901}
    assert watchers.send(p, c, reviewed_hash=p.hash)["verified"]
    q = watchers.build_plan(c, module, 123, watch=False)
    assert watchers.send(q, c, reviewed_hash=q.hash)["verified"]
    assert c.members == [902]
    assert c.writes[0][0] == "POST"
    assert c.writes[1][0] == ("PUT" if module == "tickets" else "DELETE")
    assert c.writes[1][2] == ({"user_id": 901} if module == "tickets" else None)
    if module == "changes":
        assert c.writes[1][1].endswith("/unwatch?user_id=901")


@pytest.mark.parametrize("status", [403, 429, 500])
def test_ambiguous_write_is_never_replayed(status):
    c = Client()
    p = watchers.build_plan(c, "tickets", 123, watch=True)
    c.failure = APIError(status, "uncertain")
    with pytest.raises(ReviewedWriteError, match="unknown"):
        watchers.send(p, c, reviewed_hash=p.hash)
    with pytest.raises(ReviewedWriteError, match="unknown"):
        watchers.send(p, c, reviewed_hash=p.hash)
    assert len(c.writes) == 1 and journal.read()[-1]["event"] == "started"


def test_stale_identity_partial_membership_and_noop(monkeypatch):
    c = Client()
    p = watchers.build_plan(c, "tickets", 123, watch=True)
    c.user = 903
    with pytest.raises(ReviewedWriteError, match="changed"):
        watchers.send(p, c, reviewed_hash=p.hash)
    c.user = 901
    monkeypatch.setattr(c, "int_get", lambda path: {"watcher_ids": [], "has_more": True})
    with pytest.raises(ReviewedWriteError, match="unavailable"):
        watchers.build_plan(c, "tickets", 123, watch=True)
    assert not c.writes


def test_readback_failure_and_journal_failure_do_not_resend(monkeypatch):
    c = Client()
    p = watchers.build_plan(c, "tickets", 123, watch=True)
    original = c._request
    def write(*a, **kw):
        result = original(*a, **kw)
        c.read_failure = True
        return result
    monkeypatch.setattr(c, "_request", write)
    result = watchers.send(p, c, reviewed_hash=p.hash)
    assert not result["verified"] and "do not resend" in result["warning"]
    assert len(c.writes) == 1


def test_noop_and_missing_journal(monkeypatch):
    c = Client()
    p = watchers.build_plan(c, "tickets", 123, watch=False)
    assert watchers.send(p, c, reviewed_hash=p.hash)["status"] == "already_in_requested_state"
    q = watchers.build_plan(c, "tickets", 123, watch=True)
    monkeypatch.setattr(journal, "record_plan", lambda *a, **kw: None)
    with pytest.raises(ReviewedWriteError, match="persist"):
        watchers.send(q, c, reviewed_hash=q.hash)
    assert not c.writes


def test_cli_review_hash_then_send(monkeypatch):
    c = Client()
    monkeypatch.setattr(cli, "_client", lambda: c)
    args = ["tickets", "watchers", "set", "INC-123", "--watch"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.output)["plan_hash"]
    assert not c.writes
    assert runner.invoke(cli.app, args + ["--yes"]).exit_code == 2
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", digest])
    assert sent.exit_code == 0, sent.output
    assert len(c.writes) == 1
