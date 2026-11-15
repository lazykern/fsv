"""Plain `tickets reply` on the reviewed-write contract: plan hash in the
dry-run, optional --expect-plan, journaled single send, conversation readback.
Offline, synthetic tenant."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, journal, ticket_plain_reply as plain
from test_tui_integration import offline  # noqa: F401
from test_tui_send_transport import transport  # noqa: F401


@pytest.fixture
def run(transport, monkeypatch):
    c, state = transport
    monkeypatch.setattr(cli, "_client", lambda: c)
    def invoke(*extra):
        return CliRunner().invoke(cli.app, ["tickets", "reply", "INC-123", "Acme is working on it", *extra])
    return invoke, state


def doc(output):
    return json.loads(output[output.index("{"):output.rindex("}") + 1])


def test_plain_reply_dry_run_shows_plan_hash_and_keeps_the_preview(run):
    invoke, state = run
    result = invoke("--dry-run")
    assert result.exit_code == 0, result.output
    review = doc(result.output)
    assert review["dry_run"] and review["action"] == "reply" and review["body"] == {"body": "Acme is working on it"}
    assert len(review["plan_hash"]) == 64 and review["requires_confirmation"] and review["record_state"] == {"sends": 0}
    assert not state["writes"]
    assert doc(invoke("--dry-run").output)["plan_hash"] == review["plan_hash"]


def test_plain_reply_expect_plan_send_verifies_and_never_replays(run):
    invoke, state = run
    review = doc(invoke("--dry-run").output)
    assert invoke("--yes", "--expect-plan", "0" * 64).exit_code != 0 and not state["writes"]
    other = CliRunner().invoke(cli.app, ["tickets", "reply", "INC-123", "Globex text", "--yes",
                                         "--expect-plan", review["plan_hash"]])
    assert other.exit_code != 0 and not state["writes"]
    result = invoke("--yes", "--expect-plan", review["plan_hash"], "--json")
    assert result.exit_code == 0, result.output
    out = doc(result.output)
    assert out["verified"] is True and out["conversation_id"] == 501 and out["fields"] == {"body": "matched"}
    assert [json.loads(r.content) for r in state["writes"]] == [{"body": "Acme is working on it"}]
    assert state["writes"][0].url.path == "/api/_/tickets/123/reply"
    # the same reviewed hash again: refused, nothing sent
    again = invoke("--yes", "--expect-plan", review["plan_hash"])
    assert again.exit_code == 2 and "already sent" in again.output and len(state["writes"]) == 1
    # a fresh review of the same body is a new plan (an intended repeat)
    fresh = doc(invoke("--dry-run").output)
    assert fresh["plan_hash"] != review["plan_hash"] and fresh["record_state"] == {"sends": 1}


def test_plain_reply_without_expect_plan_still_sends_with_yes(run):
    invoke, state = run
    result = invoke("--yes")
    assert result.exit_code == 0, result.output
    assert "verified: true" in result.output and len(state["writes"]) == 1


@pytest.mark.parametrize("mode", ["disconnect", "500", "unverified"])
def test_plain_reply_ambiguous_or_unverified_is_not_resent(run, mode):
    invoke, state = run
    state["mode"] = mode
    first = invoke("--yes")
    assert first.exit_code != 0 and len(state["writes"]) == 1
    state["mode"] = "confirm"
    second = invoke("--yes")
    assert second.exit_code != 0 and len(state["writes"]) == 1
    rows = [r for r in journal.read() if r.get("kind") == f"plan:{plain.ACTION}"]
    assert rows[0]["event"] == "started"


def test_plain_reply_rejected_names_the_server_answer(run, monkeypatch):
    import httpx
    invoke, state = run
    state["mode"] = "confirm"
    from fsv.errors import APIError
    c = cli._client()
    def reject(*a, **k):
        state["writes"].append(object())
        raise APIError(400, {"description": "Validation failed"}, "POST", "tickets/123/reply")
    monkeypatch.setattr(c, "_request", reject)
    result = invoke("--yes")
    assert result.exit_code != 0 and "HTTP 400" in result.output and "no retry" in result.output
    assert httpx  # imported for parity with the transport fixture
