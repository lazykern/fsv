"""Record-state tokens: legitimate repeats of an identical payload plan fresh
hashes, while true replays and ambiguous or unverified outcomes still refuse.
Offline, synthetic tenant."""
import json

import pytest
from typer.testing import CliRunner

from fsv import change_flow, change_form_update, cli, journal, problem_notes, ticket_notes
from fsv.formui import change_session
from fsv.reviewed_write import ReviewedWriteError
from test_tui_integration import offline  # noqa: F401
from test_tui_send_transport import transport  # noqa: F401


def change_plan(c, subject):
    session = change_session(c, mode="edit", target=123)
    assert not session.set("subject", subject).error
    result = session.build_plan()
    assert result.ok and result.hash
    return result


def test_change_update_ab_alternation_six_sends(transport):
    c, state = transport
    hashes = []
    for i in range(6):
        source = change_plan(c, "Acme A" if i % 2 == 0 else "Acme B")
        hashes.append(source.hash)
        result = change_form_update.send(source, c, reviewed_hash=source.hash)
        assert result["verified"], result
        assert source.state_token == {"sends": i, "server_version": None}
    assert len(state["writes"]) == 6 and state["change"]["subject"] == "Acme B"
    # #2/#4/#6 send the same payload against the same business state
    assert len(set(hashes)) == 6


def test_change_update_plan_binds_server_updated_at(transport):
    c, state = transport
    state["change"]["updated_at"] = "2030-01-01T00:00:00Z"
    first = change_plan(c, "Acme A")
    state["change"]["updated_at"] = "2030-01-01T00:05:00Z"
    second = change_plan(c, "Acme A")
    assert first.legacy_hash == second.legacy_hash and first.record_hash == second.record_hash
    assert first.hash != second.hash and second.to_json()["record_state"]["server_version"] == "2030-01-01T00:05:00Z"


def test_change_update_true_replay_refused(transport):
    c, state = transport
    source = change_plan(c, "Acme A")
    assert change_form_update.send(source, c, reviewed_hash=source.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="already sent at"):
        change_form_update.send(source, c, reviewed_hash=source.hash)
    # The record is put back without a server version bump: a new review of
    # the same values is a fresh plan, and the old hash names the earlier send.
    state["change"]["subject"] = "Acme original"
    again = change_plan(c, "Acme A")
    assert again.hash != source.hash
    with pytest.raises(change_flow.FlowExit, match="already sent") as exc:
        change_flow.expect(again, source.hash)
    assert again.hash in str(exc.value)
    assert len(state["writes"]) == 1


@pytest.mark.parametrize("mode", ["disconnect", "500", "unverified"])
def test_change_update_ambiguous_or_unverified_refused_on_fresh_review(transport, mode):
    c, state = transport
    state["mode"] = mode
    source = change_plan(c, "Acme A")
    if mode == "unverified":
        assert not change_form_update.send(source, c, reviewed_hash=source.hash)["verified"]
    else:
        with pytest.raises(ReviewedWriteError, match="unknown"):
            change_form_update.send(source, c, reviewed_hash=source.hash)
    state["mode"] = "confirm"
    again = change_plan(c, "Acme A")
    assert again.hash == source.hash  # unchanged record, nothing verified
    with pytest.raises(ReviewedWriteError, match="already sent" if mode == "unverified" else "unknown"):
        change_form_update.send(again, c, reviewed_hash=again.hash)
    assert len(state["writes"]) == 1


def test_change_update_unresolved_pre_token_plan_still_refuses(transport):
    c, state = transport
    source = change_plan(c, "Acme A")
    journal.record_plan("started", source.prior_hash, action="change_form_update", target=123)
    with pytest.raises(ReviewedWriteError, match="unknown"):
        change_form_update.send(source, c, reviewed_hash=source.hash)
    assert not state["writes"]


def test_ticket_note_identical_repeat_and_replay(transport):
    c, state = transport
    first = ticket_notes.build_plan(123, "Acme check")
    assert first.sends == 0
    assert ticket_notes.send(first, c, reviewed_hash=first.hash)["verified"]
    with pytest.raises(ticket_notes.NoteError, match="was sent"):
        ticket_notes.send(first, c, reviewed_hash=first.hash)
    second = ticket_notes.build_plan(123, "Acme check")
    assert second.hash != first.hash
    assert ticket_notes.send(second, c, reviewed_hash=second.hash)["verified"]
    assert len(state["writes"]) == 2


def test_ticket_note_unverified_is_not_resent_by_a_fresh_review(transport):
    c, state = transport
    state["mode"] = "unverified"
    first = ticket_notes.build_plan(123, "Acme check")
    assert not ticket_notes.send(first, c, reviewed_hash=first.hash)["verified"]
    state["mode"] = "confirm"
    again = ticket_notes.build_plan(123, "Acme check")
    assert again.hash == first.hash
    with pytest.raises(ticket_notes.NoteError, match="was sent"):
        ticket_notes.send(again, c, reviewed_hash=again.hash)
    assert len(state["writes"]) == 1


def test_problem_note_identical_repeat_and_replay(transport):
    c, state = transport
    first = problem_notes.build_plan(123, "Acme evidence")
    assert problem_notes.review(first)["record_state"] == {"sends": 0}
    assert problem_notes.send(first, c, reviewed_hash=first.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="already sent"):
        problem_notes.send(first, c, reviewed_hash=first.hash)
    second = problem_notes.build_plan(123, "Acme evidence")
    assert second.hash != first.hash and problem_notes.review(second)["record_state"] == {"sends": 1}
    assert problem_notes.send(second, c, reviewed_hash=second.hash)["verified"]
    assert len(state["writes"]) == 2
    assert "already sent" in journal.stale_plan_refusal(first.hash, (problem_notes.ACTION,))
    assert journal.stale_plan_refusal("0" * 64, (problem_notes.ACTION,)) is None


def test_change_task_update_ab_alternation(transport):
    from fsv import change_task_update
    c, state = transport
    state["tasks"] = [{"id": 501, "title": "Acme original", "status": 1}]
    hashes = []
    for i in range(4):
        plan = change_task_update.build_plan(c, 123, 501, {"status": "Completed" if i % 2 == 0 else "Open"})
        hashes.append(plan.hash)
        assert change_task_update.send(plan, c, reviewed_hash=plan.hash)["verified"]
    assert len(state["writes"]) == 4 and len(set(hashes)) == 4


def test_problem_update_dry_run_shows_record_state(monkeypatch):
    from fsv import config, service
    from test_problem_write_fixes import SCHEMA, Client
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    client = Client()
    state = {"problem": {"id": 7, "deleted": False, "subject": "Acme outage", "updated_at": "2030-01-01T00:00:00Z"}}
    client.int_get = lambda path, params=None: (
        {"problem_fields": SCHEMA["fields"]} if "fields" in path else {"problem": dict(state["problem"])})
    monkeypatch.setattr(service, "update_item", lambda *a, **k: pytest.fail("dry-run sent"))
    monkeypatch.setattr(cli, "_client", lambda: client)
    monkeypatch.setattr("fsv.schema.load", lambda *a, **k: {**SCHEMA, "source": "server"})
    run = lambda: CliRunner().invoke(cli.app, ["problems", "update", "--no-review", "PRB-7", "--priority", "High", "--dry-run", "--json"])
    out = run().output
    doc = json.loads(out[out.index("{"):])
    assert doc["record_state"] == {"sends": 0, "server_version": "2030-01-01T00:00:00Z"}
    state["problem"]["updated_at"] = "2030-01-01T00:01:00Z"  # same business state, newer server version
    out = run().output
    assert json.loads(out[out.index("{"):])["plan_hash"] != doc["plan_hash"]
