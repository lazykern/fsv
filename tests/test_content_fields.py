"""Content-only form blocks (static notes / guides) are never inputs.

Invented Acme data; no network."""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import acme_change_form as A
import fsv.change_flow as flow
import fsv.cli as cli
from fsv import formmeta
from fsv.forms import FormSpec, FormState, LookupValue, ValueParseError
from fsv.formui.session import change_session
from test_changes_form_cli import DRAFT

runner = CliRunner()
GUIDE = "<p>Read the <b>Acme change guide</b> before you start.</p>"


def _fields() -> dict:
    f = A.fields()
    rows = f["change_fields"]
    rows.insert(0, A._f("untitled", "Guide", "custom_static_rich_text", 0, content=GUIDE))
    rows.insert(5, A._f("cloud_note", "Cloud note", "nested_content", 4,
                        field_options={"content": "<div>Cloud changes need a buddy.</div>"}))
    return f


@pytest.fixture
def fake(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    c = A.FakeClient()
    c.fields = _fields()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    return c


def test_content_types_are_notices_not_inputs():
    spec = FormSpec.from_form_fields(_fields(), module="change")
    guide, note = spec.by_name["untitled"], spec.by_name["cloud_note"]
    assert guide.kind == note.kind == "notice"
    assert not guide.is_input and "Acme change guide" in guide.content
    assert "buddy" in note.content
    assert not [w for w in spec.warnings if "untitled" in w or "cloud_note" in w]
    assert "untitled" not in [f.api_name for f in spec.inputs()]
    assert [f.api_name for f in spec.ordered()][0] == "untitled"  # still on the form, in place


def test_notice_cannot_be_answered_required_or_sent():
    spec = FormSpec.from_form_fields(_fields(), module="change")
    meta = formmeta.FormMeta("change", "new_form", _fields(), A.rules(), 2, 0.0, "server")
    state = FormState(spec, meta.engine(), mode="new")
    assert "untitled" in state.evaluated.visible  # shown like the web shows it
    assert "untitled" not in state.evaluated.required
    with pytest.raises(ValueParseError):
        state.set("Guide", "anything")
    assert "untitled" not in state.payload_values()
    assert all(label not in ("Guide", "Cloud note") for label, _ in state.to_review())


def test_form_fields_and_explain_leave_content_blocks_out(fake):
    r = runner.invoke(cli.app, ["form", "fields", "changes", "-o", "json"])
    assert r.exit_code == 0, r.output
    names = [row["name"] for row in json.loads(r.output)]
    assert "untitled" not in names and "cloud_note" not in names and "subject" in names
    r = runner.invoke(cli.app, ["form", "fields", "changes"])
    flat = " ".join(r.output.split())
    assert "information block(s) on the form, not inputs" in flat and "Acme change guide" in flat
    r = runner.invoke(cli.app, ["form", "explain", "changes", "--all", "-o", "json"])
    assert r.exit_code == 0, r.output
    rows = json.loads(r.output)["rows"]
    assert {"untitled", "cloud_note"}.isdisjoint(x["field"] for x in rows)


def test_dry_run_review_and_payload_skip_content_blocks(fake, tmp_path):
    p = tmp_path / "change.yaml"
    p.write_text(DRAFT)
    r = runner.invoke(cli.app, ["changes", "create", "--from", str(p), "--dry-run", "-o", "json"])
    assert r.exit_code == 0, r.output
    out = json.loads(r.output)
    body = json.dumps(out["payload"])
    assert "untitled" not in body and "cloud_note" not in body
    assert "Guide" not in json.dumps(out["review"]) and "Cloud note" not in json.dumps(out["review"])


def test_interactive_form_shows_the_block_read_only_in_place(fake):
    s = change_session(fake, requester=LookupValue(9000007001, "Jane Doe", "jane.doe@example.com"))
    keys = [it.key for it in s.items]
    assert keys[0] == "untitled"
    guide = s.by_key["untitled"]
    assert guide.widget == "notice"
    v = s.view("untitled")
    assert v.visible and not v.required
    s.show_all_errors = True
    assert "untitled" not in s.missing_keys()
    assert not any(p.startswith("Guide") for p in s.problems())
