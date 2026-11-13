"""Form-update freshness, payload binding and old ambiguity guards, offline."""
import copy

import pytest

from fsv import change_form_update, config, journal
from fsv.formui import change_session
from fsv.reviewed_write import ReviewedWriteError
from test_tui_integration import offline  # noqa: F401
from test_tui_send_transport import transport  # noqa: F401


def plan(c):
    session = change_session(c, mode="edit", target=123)
    assert not session.set("subject", "Acme revised").error
    result = session.build_plan()
    assert result.ok
    return result


@pytest.mark.parametrize("mode", ["payload", "record", "metadata", "tenant", "old-started", "old-done"])
def test_change_form_preflight_and_legacy_hash_block_without_write(transport, monkeypatch, mode):
    c, state = transport
    source = plan(c)
    if mode == "payload":
        source.payload["subject"] = "Unreviewed text"
    elif mode == "record":
        state["change"]["subject"] = "Changed concurrently"
    elif mode == "metadata":
        fresh = copy.deepcopy(source.meta)
        fresh.form_fields["change_fields"][0]["label"] = "Globex subject"
        monkeypatch.setattr(change_form_update, "load_meta", lambda *args, **kwargs: fresh)
    elif mode == "tenant":
        monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    else:
        journal.record_plan(mode.removeprefix("old-"), source.legacy_hash, action="update", target=123)
    with pytest.raises(ReviewedWriteError):
        change_form_update.send(source, c, reviewed_hash=source.hash)
    assert not state["writes"]


@pytest.mark.parametrize("mode", ["disconnect", "unverified"])
def test_change_form_same_plan_never_replayed(transport, mode):
    c, state = transport
    source = plan(c)
    state["mode"] = mode
    if mode == "disconnect":
        with pytest.raises(ReviewedWriteError, match="unknown"):
            change_form_update.send(source, c, reviewed_hash=source.hash)
    else:
        assert not change_form_update.send(source, c, reviewed_hash=source.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="unknown" if mode == "disconnect" else "already sent at"):
        change_form_update.send(source, c, reviewed_hash=source.hash)
    assert len(state["writes"]) == 1


def test_task_delete_requires_preservation_of_other_ids(transport):
    from fsv import change_task_delete
    c, state = transport
    state["tasks"] = [{"id": 501, "title": "Acme target"}, {"id": 502, "title": "Globex retained"}]
    source = change_task_delete.build_plan(c, 123, 501)
    # The fake server clears all tasks: target absence alone is insufficient.
    result = change_task_delete.send(source, c, reviewed_hash=source.hash)
    assert not result["verified"] and len(state["writes"]) == 1
    with pytest.raises(ReviewedWriteError):
        change_task_delete.send(source, c, reviewed_hash=source.hash)
    assert len(state["writes"]) == 1


def test_unreviewed_combined_form_steps_are_blocked(transport):
    c, state = transport
    source = plan(c)
    # Refuse before local bytes or any upload; never fall back to the legacy
    # transaction sender when a property plan includes more than one write.
    source.files = {"attachments": ["acme.txt"]}
    with pytest.raises(ReviewedWriteError, match="Combined"):
        change_form_update.send(source, c, reviewed_hash=source.hash)
    assert not state["writes"]
