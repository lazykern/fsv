"""Captured conversation selector with independent delivery readback."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, ticket_conversation as conv
from test_ticket_properties_http import wire  # noqa: F401


def test_exact_source_forward_cli(wire, monkeypatch):
    monkeypatch.setattr(cli, "_client", lambda: wire.client)
    wire.conversation = {"id": 901, "body": "<div>Original Acme message</div>"}
    args = ["tickets", "forward", str(wire.cid), "Acme update", "--conversation", "901",
            "--to", "person@example.com", "--sender", "support@example.com", "--json"]
    runner = CliRunner()
    review = runner.invoke(cli.app, args + ["--dry-run"])
    assert review.exit_code == 0, review.output
    plan = json.loads(review.stdout)
    assert plan["payload"]["conversation_id"] == 901
    assert not [r for r in wire.requests if r.method == "POST"]
    sent = runner.invoke(cli.app, args + ["--yes", "--expect-plan", plan["plan_hash"]])
    assert sent.exit_code == 0, sent.output
    assert json.loads(sent.stdout)["verified"]
    posts = [r for r in wire.requests if r.method == "POST"]
    assert len(posts) == 1 and json.loads(posts[0].content) == plan["payload"]


def test_source_drift_blocks_forward(wire):
    wire.conversation = {"id": 901, "body": "<div>Original Acme message</div>"}
    p = conv.build_plan(wire.client, wire.cid, "Acme update", kind="forward", conversation_id=901,
                        sender="support@example.com", notify=("person@example.com",))
    wire.conversation["body"] = "Changed"
    with pytest.raises(conv.ConversationError, match="source changed"):
        conv.send(p, wire.client, reviewed_hash=p.hash, public_confirmed=True)
    assert not [r for r in wire.requests if r.method == "POST"]


def test_missing_source_blocks_forward(wire):
    with pytest.raises(conv.ConversationError, match="exact forward source"):
        conv.build_plan(wire.client, wire.cid, "Acme update", kind="forward", conversation_id=901,
                        sender="support@example.com", notify=("person@example.com",))
    assert not [r for r in wire.requests if r.method == "POST"]
