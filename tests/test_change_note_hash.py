"""`changes add-note`: the dry run shows a plan hash bound to the tenant, the
target and the exact POST body (visibility included); --expect-plan refuses
any difference and sends nothing."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli


@pytest.fixture
def posts(monkeypatch):
    sent = []

    class Client:
        def int_post(self, path, payload):
            sent.append((path, payload))
            return {"note": {"id": 4401, "body": payload["body"]}}

    monkeypatch.setattr(cli, "_client", lambda: Client())
    return sent


def _review(*extra):
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-3", "Acme change note", *extra, "--dry-run"])
    assert r.exit_code == 0, r.output
    return json.loads(r.output)


def test_dry_run_shows_hash_and_matching_send_posts_once(posts):
    doc = _review()
    assert doc["body"] == {"body": "Acme change note", "private": True} and len(doc["plan_hash"]) == 64
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-3", "Acme change note",
                                     "--expect-plan", doc["plan_hash"]])
    assert r.exit_code == 0, r.output
    assert posts == [("changes/3/notes", {"body": "Acme change note", "private": True})]


def test_hash_binds_body_visibility_target_and_tenant(posts, monkeypatch):
    from fsv import config
    base = _review()["plan_hash"]
    assert _review()["plan_hash"] == base
    assert _review("--public")["plan_hash"] != base
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-4", "Acme change note", "--dry-run"])
    assert json.loads(r.output)["plan_hash"] != base
    monkeypatch.setattr(config, "DOMAIN", "globex.freshservice.com")
    assert _review()["plan_hash"] != base
    assert not posts


@pytest.mark.parametrize("args,hash_of", [
    (["Acme change note (edited)"], "base"),
    (["Acme change note", "--public", "--yes"], "base"),
    (["Acme change note"], "prefix"),
    (["Acme change note"], "garbage"),
])
def test_mismatch_refuses_with_exit_2_and_sends_nothing(posts, args, hash_of):
    base = _review()["plan_hash"]
    h = {"base": base, "prefix": base[:16], "garbage": "0" * 64}[hash_of]
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-3", *args, "--expect-plan", h])
    assert r.exit_code == 2, r.output
    assert "does not match the reviewed plan" in " ".join(r.output.split()) and not posts


def test_without_expect_plan_the_legacy_behaviour_is_unchanged(posts):
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-3", "private words"])
    assert r.exit_code == 0 and posts[-1][1] == {"body": "private words", "private": True}
    r = CliRunner().invoke(cli.app, ["changes", "add-note", "CHN-3", "public words", "--public", "--no-input"])
    assert r.exit_code == 2 and len(posts) == 1  # a public note still needs --yes


def test_help_documents_the_hash():
    out = " ".join(CliRunner().invoke(cli.app, ["changes", "add-note", "-h"], terminal_width=200).output.split())
    assert "--expect-plan" in out and "unhashed" not in out
