"""A rejected reply/forward with an explicit sender names the server's
answer and what the sender must be. Offline, synthetic tenant."""
import pytest

from fsv import ticket_conversation as conv
from fsv.errors import APIError
from test_tui_integration import offline  # noqa: F401
from test_tui_send_transport import transport  # noqa: F401


def test_sender_rejection_prints_sanitized_server_body_and_mailbox_hint(transport, monkeypatch):
    c, state = transport
    plan = conv.build_plan(c, 123, "Acme update", sender="someone@example.org")
    def reject(*a, **k):
        state["writes"].append(object())
        raise APIError(400, {"errors": [{"field": "from_email", "message": "<b>is invalid</b>"}]}, "POST", "x")
    monkeypatch.setattr(c, "_request", reject)
    with pytest.raises(conv.ConversationError) as exc:
        conv.send(plan, c, reviewed_hash=plan.hash, public_confirmed=True)
    text = str(exc.value)
    assert "HTTP 400" in text and "from_email: is invalid" in text and "<b>" not in text
    assert "sender must be a configured support mailbox" in text and "someone@example.org" in text
    assert len(state["writes"]) == 1


def test_rejection_without_sender_or_body_says_so():
    text = conv.rejection_message(APIError(422, None, "POST", "x"), {"body": "x"})
    assert "HTTP 422" in text and "empty response body" in text and "sender" not in text
