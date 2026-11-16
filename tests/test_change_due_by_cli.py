"""changes update --due-by on the reviewed contract (offline): ISO-8601 with
an offset checked before planning, plan hash with record state, --yes needs
the full --expect-plan, one journaled PATCH of only due_by, instant readback.
There is no tickets update --due-by (no reviewed ticket due-by wire)."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal

WHEN = "2030-06-09T18:00:00+07:00"


class Fake:
    def __init__(self, store_utc=True):
        self.writes = []
        self.row = {"id": 123, "due_by": "2030-06-01T10:00:00+07:00", "updated_at": "2030-01-01T00:00:00Z",
                    "manual_dueby": False}
        self.store_utc = store_utc
        self.drop = False

    def me(self):
        return {"id": 9, "time_zone": "Asia/Bangkok"}

    def int_get(self, path, params=None):
        assert path == "changes/123", path
        return {"change": dict(self.row)}

    def _get_csrf(self):
        return "synthetic"

    def _request(self, method, url, **kw):
        body = json.loads(kw["content"])
        self.writes.append((method, url.removeprefix(config.API_INT + "/"), body))
        if not self.drop:
            from datetime import datetime, timezone
            when = datetime.fromisoformat(body["due_by"].replace("Z", "+00:00"))
            # the server stores the instant in another rendering
            self.row.update(due_by=when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
                            if self.store_utc else body["due_by"], manual_dueby=True,
                            updated_at="2030-01-01T00:05:00Z")
        return {"change": dict(self.row)}


def run(*args):
    return CliRunner().invoke(cli.app, ["changes", "update", "CHN-123", *args])


@pytest.fixture
def c(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    fake = Fake()
    monkeypatch.setattr(cli, "_client", lambda: fake)
    return fake


def review(*args):
    r = run("--due-by", *args, "--dry-run")
    assert r.exit_code == 0, r.output
    return json.loads(r.stdout)


def test_dry_run_prints_plan_hash_and_sends_nothing(c):
    out = review(WHEN)
    assert out["action"] == "change_due_by" and out["method"] == "PATCH" and out["path"] == "/api/_/changes/123"
    assert out["payload"] == {"due_by": WHEN} and out["plan_hash"] and out["record_state"]["sends"] == 0
    assert out["due_by"]["utc"] == "2030-06-09T11:00:00+00:00"
    assert c.writes == []


@pytest.mark.parametrize("bad", ["not-a-date", "2030-13-40T10:00:00+07:00", "2030-06-09", "2030-06-09T18:00:00"])
def test_invalid_or_naive_timestamp_refused_at_dry_run(c, bad):
    r = run("--due-by", bad, "--dry-run")
    assert r.exit_code == 2 and c.writes == []
    if bad.endswith("18:00:00"):
        assert "no UTC offset" in r.output and "Asia/Bangkok" in r.output


def test_send_needs_full_hash_then_one_patch_and_instant_readback(c):
    digest = review(WHEN)["plan_hash"]
    assert run("--due-by", WHEN, "--yes").exit_code == 2 and c.writes == []
    assert run("--due-by", WHEN, "--yes", "--expect-plan", digest[:12]).exit_code == 2 and c.writes == []
    sent = run("--due-by", WHEN, "--yes", "--expect-plan", digest)
    assert sent.exit_code == 0, sent.output
    result = json.loads(sent.stdout)
    assert result["verified"] is True and result["fields"] == {"due_by": "matched"}
    assert result["due_by"] == "2030-06-09T11:00:00Z" and result["manual_dueby"] is True
    assert c.writes == [("PATCH", "changes/123", {"due_by": WHEN})]
    replay = run("--due-by", WHEN, "--yes", "--expect-plan", digest)
    assert replay.exit_code == 2 and len(c.writes) == 1


def test_same_instant_is_nothing_to_change(c):
    r = run("--due-by", "2030-06-01T03:00:00Z", "--dry-run")
    assert r.exit_code == 0 and "nothing to change" in r.output and "plan_hash" not in r.output
    assert run("--due-by", "2030-06-01T03:00:00Z", "--yes", "--expect-plan", "0" * 64).exit_code == 2


def test_unapplied_due_by_is_unverified_exit_4(c):
    c.drop = True
    digest = review(WHEN)["plan_hash"]
    sent = run("--due-by", WHEN, "--yes", "--expect-plan", digest)
    assert sent.exit_code == 4 and json.loads(sent.stdout)["fields"]["due_by"] == "mismatched"


def test_due_by_does_not_combine_with_other_update_modes(c):
    r = run("--due-by", WHEN, "--planning", "Rollout Plan", "--description", "x", "--dry-run")
    assert r.exit_code != 0 and "choose one update mode" in r.output
    assert c.writes == []


def test_tickets_have_no_due_by_option(c):
    r = CliRunner().invoke(cli.app, ["tickets", "update", "INC-5", "--due-by", WHEN])
    assert r.exit_code == 2
