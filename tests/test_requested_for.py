"""The web's "Request for" user field on changes (invented Acme data)."""
from __future__ import annotations

import json

import pytest
from typer.testing import CliRunner

import acme_change_form as A
import fsv.change_flow as flow
import fsv.cli as cli
from fsv.forms import FormSpec, FormState, LookupValue, build_change_payload
from fsv.formmeta import ClientResolver, FormMeta
from fsv.formui.session import change_session
from test_changes_form_cli import DRAFT

runner = CliRunner()


@pytest.fixture
def fake(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    c = A.FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    return c


def test_request_for_is_a_known_default_field_after_the_requester():
    spec = FormSpec.from_form_fields(A.fields(), module="change")
    names = [f.api_name for f in spec.ordered()]
    assert names[:3] == ["requester", "requested_for", "subject"]
    f = spec.field("Request for")
    assert f.kind == "lookup_user" and f.lookup_entity == "user"
    assert not f.sent and f.synthetic and not f.required_base
    assert spec.resolve_name("requestedForId") == "requested_for"  # the web DOM name


def test_metadata_request_for_is_used_not_duplicated():
    data = A.fields()
    data["change_fields"].insert(1, A._f("requested_for", "Requested for", "default_requested_for", 1))
    spec = FormSpec.from_form_fields(data, module="change")
    hits = [f for f in spec.fields if f.field_type == "default_requested_for"]
    assert len(hits) == 1 and hits[0].label == "Requested for" and not hits[0].synthetic
    assert hits[0].kind == "lookup_user" and not hits[0].sent  # the key is still unverified


def test_answer_is_resolved_reviewed_as_not_sent_and_kept_off_the_wire():
    fake = A.FakeClient()
    meta = FormMeta("change", "new_form", A.fields(), A.rules(), 2, 0.0, "server")
    state = FormState(meta.spec, meta.engine(), mode="new", resolver=ClientResolver(fake))
    state.set("Request for", "jane.doerr@example.com")
    v = state.get("requested_for")
    assert isinstance(v, LookupValue) and v.email == "jane.doerr@example.com"
    assert any(label == "Request for" and text.endswith("(not sent)") for label, text in state.to_review())
    body = build_change_payload(state, "jane.doe@example.com", "Jane Doe", allow_incomplete=True)
    assert "requested_for" not in json.dumps(body)


def test_dry_run_review_marks_request_for_not_sent(fake, tmp_path):
    p = tmp_path / "change.yaml"
    p.write_text(DRAFT.replace("answers:\n", "answers:\n  Request for: jane.doerr@example.com\n", 1))
    r = runner.invoke(cli.app, ["changes", "create", "--from", str(p), "--dry-run", "-o", "json"])
    assert r.exit_code == 0, r.output
    out = json.loads(r.output)
    assert "requested_for" not in json.dumps(out["payload"])
    rows = {row["label"]: row["value"] for row in out["review"]["rows"]}
    assert "Jane Doerr" in rows["Request for"] and "not sent" in rows["Request for"]


def test_interactive_form_offers_request_for_as_a_user_lookup(fake):
    s = change_session(fake, requester=LookupValue(9000007001, "Jane Doe", "jane.doe@example.com"))
    it = s.by_key["requested_for"]
    assert it.widget == "lookup" and it.entity == "user"
    assert [x.key for x in s.items][:2] == ["requester", "requested_for"]
    assert not s.view("requested_for").required


def test_edit_record_requested_for_is_read_but_never_updated(fake):
    fake.change = A.record(requested_for_id=9000007001)
    r = runner.invoke(cli.app, ["changes", "update", "CHN-501", "--set", "Request for=jane.doerr@example.com",
                                "--set", "Subject=Billing: rotate all keys", "--dry-run"])
    assert r.exit_code == 0, r.output
    flat = " ".join(r.output.split())
    assert "Request for" in flat and "not sent" in flat
    r = runner.invoke(cli.app, ["changes", "update", "CHN-501", "--set", "Request for=jane.doerr@example.com",
                                "--set", "Subject=Billing: rotate all keys"])
    assert r.exit_code == 0, r.output
    assert fake.writes() == [("PUT", "changes/501", {"subject": "Billing: rotate all keys"})]
