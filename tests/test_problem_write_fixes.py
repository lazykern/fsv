"""Problem write fixes: custom-field envelope, deleted records, workflow
status rewrites and schema freshness (synthetic Acme metadata)."""
from __future__ import annotations

import datetime as dt
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, problem_status, schema
from fsv.errors import APIError
from fsv.resources import PROBLEMS

SCHEMA = {"fields": [
    {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": True,
     "choices": [{"id": 1, "value": "Low"}, {"id": 3, "value": "High"}]},
    {"name": "acme_resolve_by", "label": "Resolve By", "field_type": "custom_dropdown", "default_field": False,
     "choices": [{"id": 9000002001, "value": "Change Management"}, {"id": 9000002002, "value": "Cancel"}]},
]}


class Client:
    def __init__(self, deleted=False, status=405):
        self.writes = []
        self.deleted = deleted
        self.status = status

    def int_get(self, path, params=None):
        return {"problem": {"id": 7, "deleted": self.deleted, "custom_fields": {"acme_resolve_by": None}}}

    def int_put(self, path, body=None):
        self.writes.append((path, body))
        if self.deleted:
            raise APIError(self.status, {"message": "Method Not Allowed"}, "PUT", path)
        return {"problem": {"id": 7}}


def _invoke(monkeypatch, client, *args):
    monkeypatch.setattr(cli, "_client", lambda: client)
    monkeypatch.setattr("fsv.schema.load", lambda *a, **k: SCHEMA)
    return CliRunner().invoke(cli.app, ["problems", "update", "PRB-7", *args])


def test_problem_custom_fields_are_nested_and_priority_uses_metadata(monkeypatch):
    client = Client()
    args = ["--priority", "High", "--set", "Resolve By=Cancel"]
    dry = _invoke(monkeypatch, client, *args, "--dry-run", "--json")
    assert dry.exit_code == 0, dry.output
    expected = {"priority": 3, "custom_fields": {"acme_resolve_by": "Cancel"}}
    assert json.loads(dry.output)["body"] == expected
    sent = _invoke(monkeypatch, client, *args, "--yes", "--json")
    assert sent.exit_code == 0, sent.output
    assert client.writes == [("problems/7", expected)]


def test_deleted_problem_update_is_refused_before_sending(monkeypatch):
    client = Client(deleted=True)
    res = _invoke(monkeypatch, client, "--priority", "High", "--yes", "--json")
    assert res.exit_code != 0
    assert "deleted" in res.output and not client.writes


def test_problem_405_is_explained():
    exc = APIError(405, {"message": "Method Not Allowed"}, "PUT", "problems/7")
    assert "not editable" in problem_status.rejection(exc)


@pytest.mark.parametrize(("before", "after", "sent", "verified", "noted"), [
    (1, 5, 5, True, False),   # exact
    (1, 7, 4, False, False),  # anything but the requested status: activity decides
    (4, 4, 5, False, False),  # unchanged: not verified (activity decides)
    (1, None, 5, False, False),
])
def test_problem_status_readback(before, after, sent, verified, noted):
    ok, note = problem_status.status_readback(before, after, sent)
    assert ok is verified and (note is not None) is noted


class ActivityClient:
    def __init__(self, rows):
        self.rows = rows

    def int_get(self, path, params=None):
        assert path == "problems/7/activities"
        return {"activities": self.rows}


LABELS = {"1": "Open", "5": "In Progress", "7": "Monitor", "8": "Resolve", "9": "Closed"}


def _native_activity(write_at, workflow_at, requested="Resolve", rewritten="Monitor"):
    # Real envelope: agent row then a workflow row whose status is in sub_contents.
    return [
        {"actor": {"id": 0, "name": "Problem Workflow", "is_agent_deleted": False},
         "content": "executed  <a>Acme status workflow</a>  workflow from  <b>Problem status changed (any to any)</b>  event",
         "sub_contents": [f"set status as <b>{rewritten}</b>", "<b>Workflow Ends</b>"], "created_at": workflow_at},
        {"actor": {"id": 9000001, "name": "Acme Agent", "is_agent": True},
         "content": f"set status as  <b>{requested}</b> , set planned end date as  <b>Thu, 2 Jan  4:30 PM</b>",
         "sub_contents": None, "created_at": write_at},
    ]


def test_workflow_rewrite_back_to_prior_status_uses_native_activity_rows():
    since = dt.datetime(2030, 1, 1, tzinfo=dt.timezone.utc)
    rows = _native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z")
    note, conflict = problem_status.workflow_rewrite(rows, since, LABELS, 8, 7)
    assert note and "Resolve" in note and "Monitor (status 7)" in note and "Problem Workflow" in note and not conflict
    # No entry for this write (requested Closed): the readback stays unverified.
    assert problem_status.workflow_rewrite(rows, since, LABELS, 9, 7) == (None, None)
    # The workflow set another status than the readback: a conflict, not verified.
    note, conflict = problem_status.workflow_rewrite(rows, since, LABELS, 8, 1)
    assert note is None and "readback status is Open" in conflict
    # Workflow entry older than the write is not evidence.
    stale = _native_activity("2030-01-01T00:02:00Z", "2030-01-01T00:01:00Z")
    assert problem_status.workflow_rewrite(stale, since, LABELS, 8, 7) == (None, None)
    # Entries before the write window do not count.
    old = _native_activity("2029-12-31T00:01:00Z", "2029-12-31T00:01:01Z")
    assert problem_status.workflow_rewrite(old, since, LABELS, 8, 7) == (None, None)
    # A workflow row that did not set a status is not a rewrite.
    noset = _native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z")
    noset[0]["sub_contents"] = ["send email to <b>requester</b>", "<b>Workflow Ends</b>"]
    assert problem_status.workflow_rewrite(noset, since, LABELS, 8, 7) == (None, None)


def test_workflow_row_is_detected_structurally_not_by_wording():
    since = dt.datetime(2030, 1, 1, tzinfo=dt.timezone.utc)
    rows = _native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z")
    # An agent row that merely mentions a workflow is not a workflow row.
    rows[0]["actor"] = {"id": 9000002, "name": "Acme Agent Two", "is_agent": True}
    assert problem_status.workflow_rewrite(rows, since, LABELS, 8, 7) == (None, None)
    # A system actor row with sub_contents is one, whatever its content says.
    rows = _native_activity("2030-01-01T00:01:00Z", "2030-01-01T00:01:01Z")
    rows[0]["content"] = "ran automation"
    assert problem_status.workflow_rewrite(rows, since, LABELS, 8, 7)[0]
    # Labels map to ids through the plan's choices; an unknown label is unproven.
    assert problem_status.workflow_rewrite(rows, since, {"8": "Resolve"}, 8, 7)[0] is None
    # Two ids sharing a label are ambiguous.
    assert problem_status.workflow_rewrite(rows, since, {**LABELS, "10": "Monitor"}, 8, 7)[0] is None


def test_real_fixture_activity_shape_parses():
    # tests/fixtures/problem_activity.json is a private capture kept out of
    # git; when present, its rows must parse into agent and workflow entries.
    import pathlib
    path = pathlib.Path(__file__).parent / "fixtures" / "problem_activity.json"
    if not path.exists():
        pytest.skip("private activity capture not present")
    rows = problem_status._activity_rows(json.loads(path.read_text()))
    assert any(r.workflow and r.statuses for r in rows) and any(not r.workflow and r.statuses for r in rows)


class FieldsClient:
    def __init__(self, fields):
        self.fields = fields

    def int_get(self, path, params=None):
        return {PROBLEMS.form_fields_key: self.fields}


def test_schema_freshness_warns_on_new_server_choice():
    cached = {**SCHEMA, "source": "cache"}
    fresh = json.loads(json.dumps(SCHEMA["fields"]))
    assert schema.freshness_warning(PROBLEMS, FieldsClient(fresh), cached) is None
    fresh[1]["choices"].append({"id": 9000002003, "value": "Irrelevant"})
    warning = schema.freshness_warning(PROBLEMS, FieldsClient(fresh), cached)
    assert warning and "acme_resolve_by" in warning and "--refresh" in warning
    # a freshly fetched schema is not checked again
    assert schema.freshness_warning(PROBLEMS, FieldsClient(fresh), {**SCHEMA, "source": "server"}) is None


def test_dry_run_reports_stale_schema(monkeypatch):
    client = Client()
    fresh = json.loads(json.dumps(SCHEMA["fields"]))
    fresh[1]["choices"].append({"id": 9000002003, "value": "Irrelevant"})
    client.int_get = lambda path, params=None: (
        {PROBLEMS.form_fields_key: fresh} if path == PROBLEMS.form_fields_path
        else {"problem": {"id": 7, "deleted": False}})
    monkeypatch.setattr(cli, "_client", lambda: client)
    monkeypatch.setattr("fsv.schema.load", lambda *a, **k: {**SCHEMA, "source": "cache"})
    res = CliRunner().invoke(cli.app, ["problems", "update", "PRB-7", "--priority", "High", "--dry-run", "--json"])
    assert res.exit_code == 0, res.output
    doc = json.loads(res.output[res.output.index("{"):])
    assert any("--refresh" in w for w in doc["warnings"])


def test_problem_update_dry_run_prints_a_plan_hash_bound_on_send(monkeypatch):
    from fsv import config, service
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    client = Client()
    state = {"problem": {"id": 7, "deleted": False, "subject": "Acme outage"}}
    client.int_get = lambda path, params=None: (
        {PROBLEMS.form_fields_key: SCHEMA["fields"]} if path == PROBLEMS.form_fields_path
        else {"problem": dict(state["problem"])})
    sent = []
    monkeypatch.setattr(service, "update_item", lambda res, cid, body, client=None: sent.append(body) or
                        {"id": cid, **body})
    monkeypatch.setattr(cli, "_client", lambda: client)
    monkeypatch.setattr("fsv.schema.load", lambda *a, **k: {**SCHEMA, "source": "server"})
    args = ["problems", "update", "PRB-7", "--priority", "High"]
    run = lambda *extra: CliRunner().invoke(cli.app, [*args, *extra])
    res = run("--dry-run", "--json")
    assert res.exit_code == 0, res.output
    doc = json.loads(res.output[res.output.index("{"):])
    plan = doc["plan_hash"]
    assert len(plan) == 64 and doc["body"] and not sent
    again = run("--dry-run").output
    assert json.loads(again[again.index("{"):])["plan_hash"] == plan
    assert run("--yes", "--expect-plan", "0" * 64, "--json").exit_code != 0 and not sent
    state["problem"]["subject"] = "Globex outage"  # changed after review
    assert run("--yes", "--expect-plan", plan, "--json").exit_code != 0 and not sent
    state["problem"]["subject"] = "Acme outage"
    res = run("--yes", "--expect-plan", plan, "--json")
    assert res.exit_code == 0, res.output
    assert len(sent) == 1
