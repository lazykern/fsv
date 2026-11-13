"""Single-ticket spam / unspam: exact wires (spam carries {}, unspam no body),
flag readback, confirmation for spam, single attempt."""
import json

import httpx
import pytest
from typer.testing import CliRunner

from fsv import cli, config, journal, ticket_spam
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_notes import mock_client


@pytest.fixture
def tenant(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setattr(config, "API_INT", "https://acme.example.com/api/_")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"spam": {32: False, 33: True}, "writes": [], "effect": True, "fail": None}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        tid = int(path.split("/")[1])
        if req.method == "GET":
            if tid not in s["spam"]:
                return httpx.Response(404, json={})
            return httpx.Response(200, json={"ticket": {"id": tid, "spam": s["spam"][tid], "deleted": False}})
        s["writes"].append((req.method, path, req.content))
        if s["fail"]:
            return httpx.Response(s["fail"], json={})
        if s["effect"]:
            s["spam"][tid] = path.endswith("/spam")
        return httpx.Response(204)

    c = mock_client(monkeypatch, handler)
    monkeypatch.setattr(cli, "_client", lambda: c)
    return c, s


def test_spam_and_unspam_wires_and_flag_readback(tenant):
    c, s = tenant
    p = ticket_spam.build_plan(c, "INC-32", spam=True)
    assert (p.method, p.path, p.payload) == ("PUT", "tickets/32/spam", {})
    with pytest.raises(ReviewedWriteError, match="confirm-destructive"):
        ticket_spam.send(p, c, reviewed_hash=p.hash)
    assert ticket_spam.send(p, c, reviewed_hash=p.hash, confirm_destructive=True)["verified"]
    u = ticket_spam.build_plan(c, 32, spam=False)
    assert (u.path, u.payload) == ("tickets/32/unspam", None)
    assert ticket_spam.send(u, c, reviewed_hash=u.hash)["verified"]
    assert [w[2] for w in s["writes"]] == [b"{}", b""]


def test_state_guards_unverified_and_no_resend(tenant):
    c, s = tenant
    with pytest.raises(ReviewedWriteError, match="already"):
        ticket_spam.build_plan(c, 33, spam=True)
    with pytest.raises(ReviewedWriteError, match="not marked"):
        ticket_spam.build_plan(c, 32, spam=False)
    with pytest.raises(ReviewedWriteError, match="not found"):
        ticket_spam.build_plan(c, 99, spam=True)
    s["effect"] = False
    p = ticket_spam.build_plan(c, 33, spam=False)
    assert ticket_spam.send(p, c, reviewed_hash=p.hash)["verified"] is False
    with pytest.raises(ReviewedWriteError, match="sent"):
        ticket_spam.send(p, c, reviewed_hash=p.hash)
    assert len(s["writes"]) == 1


def test_cli_gate(tenant):
    c, s = tenant
    run = CliRunner().invoke
    review = run(cli.app, ["tickets", "spam", "32", "--dry-run"])
    assert review.exit_code == 0, review.output
    digest = json.loads(review.stdout)["plan_hash"]
    assert run(cli.app, ["tickets", "spam", "32", "--yes", "--expect-plan", digest]).exit_code == 2
    assert not s["writes"]
    done = run(cli.app, ["tickets", "spam", "32", "--yes", "--expect-plan", digest, "--confirm-destructive"])
    assert done.exit_code == 0 and json.loads(done.stdout)["spam"] is True
