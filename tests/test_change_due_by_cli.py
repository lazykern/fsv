"""changes update --due-by as it behaves today (characterization, offline).

There is no tickets update --due-by, and the change path is not on the
reviewed contract yet: the dry-run prints the body without a plan hash and a
send is one direct PATCH. These tests pin that behaviour so a later move to
the reviewed contract (plan hash, due_by readback, local timestamp refusal)
is a deliberate, visible change."""
import json

from typer.testing import CliRunner

from fsv import cli, config, create

WHEN = "2030-06-09T18:00:00+07:00"


class Fake:
    def __init__(self):
        self.writes = []

    def int_patch(self, path, body):
        self.writes.append(("PATCH", path, body))
        return {"change": {"id": 123, **body}}


def run(*args):
    return CliRunner().invoke(cli.app, ["changes", "update", "CHN-123", *args])


def setup(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    c = Fake()
    monkeypatch.setattr(create, "get_client", lambda: c)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return c


def test_dry_run_prints_the_body_and_sends_nothing(monkeypatch):
    c = setup(monkeypatch)
    r = run("--due-by", WHEN, "--dry-run")
    assert r.exit_code == 0, r.output
    out = json.loads(r.stdout)
    assert out["action"] == "set_due_by" and out["body"] == {"due_by": WHEN}
    assert "plan_hash" not in out  # not on the reviewed contract (yet)
    assert c.writes == []


def test_send_is_one_patch_with_only_due_by(monkeypatch):
    c = setup(monkeypatch)
    r = run("--due-by", WHEN, "--yes")
    assert r.exit_code == 0, r.output
    assert c.writes == [("PATCH", "changes/123", {"due_by": WHEN})]


def test_due_by_does_not_combine_with_other_update_modes(monkeypatch):
    c = setup(monkeypatch)
    r = run("--due-by", WHEN, "--planning", "Rollout Plan", "--description", "x", "--dry-run")
    assert r.exit_code != 0 and "choose one update mode" in r.output
    assert c.writes == []
