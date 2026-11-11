"""Draft beneficiary support stays scoped to catalog requests."""
import json

import pytest
from typer.testing import CliRunner

import fsv.change_flow as flow
import fsv.cli_sr as cli
from fsv import catalog
from fsv.drafts import draft_from_data
from test_catalog_safety import meta
from test_sr_recipients import ACTOR, Client


@pytest.mark.parametrize("override", [False, True])
def test_draft_beneficiary_cli(monkeypatch, tmp_path, override):
    monkeypatch.setattr(cli, "_client", lambda: Client())
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr(catalog, "load_item_meta", lambda *a, **kw: meta())
    path = tmp_path / "request.yaml"
    path.write_text("requester: actor@example.com\nrequested_for: target@example.com\nanswers:\n  Answer: ok\n")
    args = ["create", "77", "--from", str(path), "--dry-run", "--output", "json"]
    if override:
        args += ["--requested-for", ACTOR.email]
    result = CliRunner().invoke(cli.app, args)
    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["payload"]["requested_for"] == (ACTOR.email if override else "target@example.com")


def test_sr_template_roundtrip_and_other_drafts_reject():
    data = {"requested_for": "target@example.com"}
    with pytest.raises(ValueError, match="unknown key"):
        draft_from_data(data)
    with pytest.raises(ValueError, match="email or exact name"):
        draft_from_data({"requested_for": 12}, allow_requested_for=True)
    draft = cli.parse_sr_template("requested_for: target@example.com\n")
    assert draft.requested_for == "target@example.com"
    assert draft.merged(cli.parse_sr_template("answers: {}\n")).requested_for == draft.requested_for
