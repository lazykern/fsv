"""Write-path safety: confirmation gate, clone hardening, exact lookups,
exit codes. All mocked; no network."""
from __future__ import annotations

import json
from datetime import datetime, timezone

import pytest
import typer
from typer.testing import CliRunner

import fsv.cli as cli
import fsv.create as create
from fsv import safety
from fsv.errors import APIError, SessionError, exit_code_for

runner = CliRunner()

SCHEMA = {
    "fields": [
        {"name": "status", "label": "Status", "field_type": "default_status", "default_field": True,
         "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"},
                     {"id": 3, "value": "Closed"}, {"id": 4, "value": "Resolved"}]},
        {"name": "priority", "label": "Priority", "field_type": "default_priority", "default_field": True,
         "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "High"}]},
    ]
}


class FakeClient:
    def __init__(self, current=None):
        self.current = current or {"id": 1, "status": 1, "agent_id": 11, "group_id": 22, "responder_id": 11}
        self.writes: list[tuple] = []

    def int_get(self, path, params=None):
        if path.startswith("changes/") and path.count("/") == 1:
            return {"change": dict(self.current)}
        if path.startswith("tickets/") and path.count("/") == 1:
            return {"ticket": dict(self.current)}
        return {}

    def int_put(self, path, body=None):
        self.writes.append(("PUT", path, body))
        return {"change": {"id": 1}, "ticket": {"id": 1}}

    def int_post(self, path, body=None):
        self.writes.append(("POST", path, body))
        if path == "changes":
            return {"change": {"id": 4242, "human_display_id": "CHN-4242"}}
        return {"note": {"id": 5}, "conversation": {"id": 6}}

    def autocomplete(self, kind, query, params=None):
        return []


@pytest.fixture
def fake(monkeypatch):
    c = FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr("fsv.schema.load", lambda res, client: SCHEMA)
    monkeypatch.setattr(create, "get_client", lambda: c)
    monkeypatch.setattr(create, "_get_change_schema_fields", lambda: SCHEMA["fields"])
    return c


# ── safety.write_risks (pure) ────────────────────────────────────────────────
def lab(v):
    return {1: "Open", 2: "Planning", 3: "Closed", 4: "Resolved"}.get(v, str(v))


def test_change_any_status_transition_is_risky_same_status_is_not():
    assert safety.write_risks("changes", {"status": 2}, current={"status": 1}, status_label=lab)
    assert not safety.write_risks("changes", {"status": 1}, current={"status": 1}, status_label=lab)
    assert safety.write_risks("changes", {"status": 2}, status_label=lab)  # unknown current


def test_ticket_only_terminal_status_is_risky():
    assert safety.write_risks("tickets", {"status": 4}, current={"status": 1}, status_label=lab)
    assert not safety.write_risks("tickets", {"status": 2}, current={"status": 1}, status_label=lab)


def test_reassignment_detected_only_when_changed():
    assert safety.write_risks("changes", {"group_id": 9}, current={"group_id": 22})
    assert not safety.write_risks("changes", {"group_id": 22}, current={"group_id": 22})
    assert safety.write_risks("tickets", {"responder_id": 3}, current={"responder_id": 11})


def test_task_status_and_owner():
    assert safety.write_risks("changes", {"status": 3}, kind="task")
    assert safety.write_risks("changes", {"owner_id": 3}, kind="task")
    assert not safety.write_risks("changes", {"title": "x"}, kind="task")


def test_notifying_notes_and_replies():
    assert safety.note_risks(True) and not safety.note_risks(False)
    assert safety.reply_risks() and safety.clone_risks(True) and not safety.clone_risks(False)


# ── CLI gate: quick flags, --set, --edit, tasks, notes ───────────────────────
def test_update_planning_refused_without_yes_then_allowed(fake):
    r = runner.invoke(cli.app, ["tickets", "update", "INC-1", "--status", "Closed"])
    assert r.exit_code == 2 and "--yes" in r.output and not fake.writes
    r = runner.invoke(cli.app, ["tickets", "update", "INC-1", "--status", "Closed", "--yes"])
    assert r.exit_code == 0 and fake.writes == [("PUT", "tickets/1", {"status": 3})]


def test_change_planning_status_needs_yes(fake):
    r = runner.invoke(cli.app, ["changes", "update", "CHN-1", "--status", "Planning"])
    assert r.exit_code == 2 and not fake.writes


def test_set_status_is_gated_like_status_flag(fake, monkeypatch):
    r = runner.invoke(cli.app, ["tickets", "update", "INC-1", "--set", "status=Closed"])
    assert r.exit_code == 2 and not fake.writes
    # changes --set runs the edit form's rules; the same gate applies
    import acme_change_form as A
    import fsv.change_flow as flow
    acme = A.FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: acme)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    r = runner.invoke(cli.app, ["changes", "update", "CHN-501", "--set", "status=Planning"])
    assert r.exit_code == 2 and "--yes" in r.output and not acme.writes()
    r = runner.invoke(cli.app, ["changes", "update", "CHN-501", "--set", "status=Planning", "--yes"])
    assert r.exit_code == 0 and acme.writes() == [("PUT", "changes/501", {"status": 2})]


def test_same_status_is_not_gated(fake):
    r = runner.invoke(cli.app, ["changes", "update", "CHN-1", "--status", "Open"])
    assert r.exit_code == 0 and fake.writes


def test_reassign_gated(fake):
    r = runner.invoke(cli.app, ["changes", "update", "CHN-1", "--group", "999"])
    assert r.exit_code == 2 and not fake.writes
    r = runner.invoke(cli.app, ["changes", "update", "CHN-1", "--group", "999", "--yes"])
    assert r.exit_code == 0


def test_dry_run_never_asks_and_reports_risks(fake):
    r = runner.invoke(cli.app, ["changes", "update", "CHN-1", "--status", "Planning", "--dry-run"])
    assert r.exit_code == 0 and not fake.writes
    out = json.loads(r.output)
    assert out["body"] == {"status": 2} and out["requires_confirmation"]


def test_update_edit_final_body_is_gated(monkeypatch, tmp_path):
    """--edit opens the YAML draft (edit-form rules); the review is the gate:
    nothing is sent until [y], and a reassignment shows as a risk."""
    import sys
    import acme_change_form as A
    import fsv.change_flow as flow
    acme = A.FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: acme)
    monkeypatch.setattr(flow, "interactive", lambda: True)
    editor = tmp_path / "ed.py"
    editor.write_text(
        "import sys, pathlib\n"
        "p = pathlib.Path(sys.argv[1])\n"
        "p.write_text(p.read_text().replace('  Group: Platform Ops', '  Group: 9000009002'))\n")
    monkeypatch.setenv("VISUAL", f"{sys.executable} {editor}")
    r = runner.invoke(cli.app, ["changes", "update", "CHN-501", "--edit"], input="n\n")
    assert r.exit_code == 1 and not acme.writes()
    assert "reassign group" in r.output
    r = runner.invoke(cli.app, ["changes", "update", "CHN-501", "--edit"], input="y\n")
    assert r.exit_code == 0, r.output
    assert acme.writes() == [("PUT", "changes/501", {"group_id": 9000009002})]


def test_tasks_update_flags_and_edit_gated(monkeypatch):
    monkeypatch.setattr(cli, "_task_field_defs", lambda: [{"name": "status", "choices": [
        {"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"}, {"id": 3, "value": "Completed"}]}])
    sent = []
    monkeypatch.setattr(create, "update_task", lambda cid, tid, body, c=None: sent.append(body) or {"id": tid})
    monkeypatch.setattr(create, "get_task_for_edit", lambda cid, tid, c=None: {"status": 1, "owner_id": 5, "title": "t"})
    r = runner.invoke(cli.app, ["changes", "tasks-update", "CHN-1", "7", "--status", "2"])
    assert r.exit_code == 2 and not sent
    r = runner.invoke(cli.app, ["changes", "tasks-update", "CHN-1", "7", "--status", "2", "--yes"])
    assert r.exit_code == 0 and sent == [{"status": 2}]
    sent.clear()
    monkeypatch.setattr(cli, "_edit_body", lambda data, hint, no_input=False: {**data, "owner_id": 6})
    r = runner.invoke(cli.app, ["changes", "tasks-update", "CHN-1", "7", "--edit"])
    assert r.exit_code == 2 and not sent
    r = runner.invoke(cli.app, ["changes", "tasks-update", "CHN-1", "7", "--edit", "--yes"])
    assert r.exit_code == 0 and sent


def test_public_note_and_reply_gated_private_note_is_not(fake):
    r = runner.invoke(cli.app, ["changes", "add-note", "CHN-1", "hi"])
    assert r.exit_code == 0 and fake.writes[-1][2]["private"] is True
    fake.writes.clear()
    r = runner.invoke(cli.app, ["changes", "add-note", "CHN-1", "hi", "--public"])
    assert r.exit_code == 2 and not fake.writes
    r = runner.invoke(cli.app, ["changes", "add-note", "CHN-1", "hi", "--public", "--yes"])
    assert r.exit_code == 0 and fake.writes
    fake.writes.clear()
    r = runner.invoke(cli.app, ["tickets", "reply", "INC-1", "hello"])
    assert r.exit_code == 2 and not fake.writes
    r = runner.invoke(cli.app, ["tickets", "reply", "INC-1", "hello", "--yes"])
    assert r.exit_code == 0 and fake.writes[-1][1] == "tickets/1/reply"


def test_note_and_reply_dry_run(fake):
    r = runner.invoke(cli.app, ["tickets", "reply", "INC-1", "hello", "--dry-run"])
    assert r.exit_code == 0 and not fake.writes and json.loads(r.output)["action"] == "reply"
    r = runner.invoke(cli.app, ["changes", "add-note", "CHN-1", "hi", "--public", "--dry-run"])
    assert r.exit_code == 0 and not fake.writes and json.loads(r.output)["body"]["private"] is False


def test_interactive_confirm_shows_plan_and_asks(monkeypatch):
    class TTY:
        def isatty(self):
            return True
    monkeypatch.setattr(cli.sys, "stdin", TTY())
    asked = []
    monkeypatch.setattr(typer, "confirm", lambda msg, default=False, abort=False: asked.append((msg, default, abort)))
    cli._confirm_writes(["change status -> Planning"], "CHN-1", yes=False)
    assert asked == [("Proceed?", False, True)]
    asked.clear()
    cli._confirm_writes(["x"], "CHN-1", yes=True)
    cli._confirm_writes([], "CHN-1", yes=False)
    assert not asked


def test_no_input_refuses_even_on_tty(monkeypatch):
    class TTY:
        def isatty(self):
            return True
    monkeypatch.setattr(cli.sys, "stdin", TTY())
    with pytest.raises(typer.Exit) as e:
        cli._confirm_writes(["x"], "CHN-1", yes=False, no_input=True)
    assert e.value.exit_code == 2


# ── clone hardening ──────────────────────────────────────────────────────────
class CloneClient:
    def __init__(self, fail_tasks=False):
        self.fail_tasks = fail_tasks
        self.posts = []

    def int_get(self, path, params=None):
        if path == "changes/1":
            return {"change": {"id": 1, "subject": "s", "status": 3}}
        if path.endswith("/tasks"):
            return {"tasks": [
                {"id": 1, "title": "Deploy", "owner_id": 77, "group_id": 5, "status": 3,
                 "due_date": "2001-01-01T00:00:00+00:00", "planned_end_date": "2999-01-01T00:00:00+00:00"},
                {"id": 2, "title": "Verify", "status": 1},
            ]}
        if path.endswith("planning-fields"):
            return {"change_planning_fields": [{"name": "f", "description": "x", "attachments": [{"id": 1}]}]}
        return {}

    def int_post(self, path, body):
        self.posts.append((path, body))
        if self.fail_tasks and path.endswith("/tasks"):
            raise APIError(500, "boom", "POST", path)
        if path == "changes":
            return {"change": {"id": 4242, "human_display_id": "CHN-4242"}}
        return {"id": 1}


def test_task_clone_body_drops_owner_and_past_dates_by_default():
    now = datetime(2026, 1, 1, tzinfo=timezone.utc)
    dropped: list[str] = []
    task = {"title": "Deploy", "owner_id": 77, "group_id": 5, "due_date": "2001-01-01T00:00:00Z",
            "planned_end_date": "2999-01-01T00:00:00Z", "status": 3}
    body = create._task_clone_body(task, now=now, dropped=dropped)
    assert "owner_id" not in body and "due_date" not in body
    assert body["group_id"] == 5 and body["planned_end_date"] and body["status"] == 1
    assert any("owner" in d for d in dropped) and any("due_date" in d for d in dropped)
    kept = create._task_clone_body(task, keep_owners=True, now=now)
    assert kept["owner_id"] == 77


def test_clone_tasks_default_and_keep_owners():
    c = CloneClient()
    create.clone_tasks(1, 2, c)
    assert all("owner_id" not in b["change_task"] for _, b in c.posts)
    c2 = CloneClient()
    create.clone_tasks(1, 2, c2, keep_owners=True)
    assert c2.posts[0][1]["change_task"]["owner_id"] == 77


def _clone_env(monkeypatch, client):
    monkeypatch.setattr(cli, "_client", lambda: client)
    monkeypatch.setattr(create, "get_client", lambda: client)
    monkeypatch.setattr(create, "_get_change_schema_fields", lambda: SCHEMA["fields"])
    monkeypatch.setattr(create, "get_change_assets", lambda cid, c=None: [])


def test_clone_prints_new_id_and_exits_nonzero_when_a_later_step_fails(monkeypatch):
    client = CloneClient(fail_tasks=True)
    _clone_env(monkeypatch, client)
    monkeypatch.setattr(create, "get_change_assets", lambda cid, c=None: [{"display_id": 1}, {"display_id": 2}])
    client.int_put = lambda path, body=None: {}
    r = runner.invoke(cli.app, ["changes", "clone", "CHN-1", "--no-edit", "--with-tasks", "--with-assets", "--yes"])
    assert "CHN-4242" in r.output
    assert r.exit_code == 5  # server rejected a step
    assert "do not rerun" in r.output.replace("\n", " ")
    assert "associated 2 asset" in r.output  # later steps still run


def test_clone_with_tasks_needs_yes_and_creates_nothing(monkeypatch):
    client = CloneClient()
    _clone_env(monkeypatch, client)
    r = runner.invoke(cli.app, ["changes", "clone", "CHN-1", "--no-edit", "--with-tasks"])
    assert r.exit_code == 2 and not client.posts


def test_clone_dry_run_lists_side_effects(monkeypatch):
    client = CloneClient()
    _clone_env(monkeypatch, client)
    monkeypatch.setattr(create, "get_change_assets", lambda cid, c=None: [{"display_id": 1}])
    r = runner.invoke(cli.app, ["changes", "clone", "CHN-1", "--dry-run", "--with-tasks", "--with-assets", "--with-planning"])
    assert r.exit_code == 0 and not client.posts
    text = " ".join(r.output.split())
    assert "2 task(s)" in text and "1 with an owner" in text and "1 asset" in text
    assert "1 planning field" in text and "workflows" in text


# ── exact lookups ────────────────────────────────────────────────────────────
GROUPS = [{"id": 1, "name": "Infrastructure Security Team"}, {"id": 2, "name": "Infra"}, {"id": 3, "name": "Infra Ops"}]


class GroupClient:
    def __init__(self, groups):
        self.groups = groups

    def int_get(self, path, params=None):
        return {"groups": self.groups}


def test_group_exact_match_wins_over_substrings():
    assert cli._resolve_group(GroupClient(GROUPS), "infra") == "2"
    assert cli._resolve_group(GroupClient(GROUPS), "INFRA") == "2"


def test_group_substring_only_is_an_error_even_if_single():
    with pytest.raises(typer.Exit) as e:
        cli._resolve_group(GroupClient([GROUPS[0]]), "Infra")
    assert e.value.exit_code == 2


def test_group_substring_multiple_lists_candidates(capsys):
    with pytest.raises(typer.Exit):
        cli._resolve_group(GroupClient(GROUPS[:1] + GROUPS[2:]), "Infra")
    out = " ".join((capsys.readouterr().err + capsys.readouterr().out).split())
    assert "Infrastructure Security Team" in out and "Infra Ops" in out


def test_exact_match_is_nfc_normalized():
    decomposed = "Cafe\u0301 Team"  # e + combining acute
    composed = "Caf\u00e9 Team"
    assert cli._resolve_group(GroupClient([{"id": 9, "name": composed}]), decomposed) == "9"


def test_agent_and_requester_exact_by_email_only_single_substring_rejected():
    class C:
        def autocomplete(self, kind, q, params=None):
            return [{"value": "Pat Example", "details": "pat@example.com", "email": "pat@example.com",
                     "user_id": 31, "id": 31}]
    assert cli._resolve_agent(C(), "PAT@example.com") == "31"
    with pytest.raises(typer.Exit):
        cli._resolve_agent(C(), "Pat")
    with pytest.raises(typer.Exit):
        cli._resolve_requester(C(), "Pat")


def test_interactive_lookup_lets_user_pick(monkeypatch):
    class TTY:
        def isatty(self):
            return True
    monkeypatch.setattr(cli.sys, "stdin", TTY())
    monkeypatch.setattr(typer, "prompt", lambda *a, **k: 2)
    assert cli._resolve_group(GroupClient(GROUPS[:1] + GROUPS[2:]), "Infra") == "3"


# ── exit codes ───────────────────────────────────────────────────────────────
@pytest.mark.parametrize("exc,code", [
    (SessionError("x"), 3),
    (APIError(409, "conflict"), 4),
    (APIError(422, {"errors": "Name has already been taken"}), 4),
    (APIError(400, "bad"), 5),
    (APIError(503, "down"), 5),
    (ValueError("bad"), 2),
    (KeyError("x"), 1),
])
def test_exit_code_mapping(exc, code):
    assert exit_code_for(exc) == code


def test_wrapped_api_error_keeps_cause_code():
    try:
        try:
            raise APIError(500, "x")
        except APIError as e:
            raise RuntimeError("update failed") from e
    except RuntimeError as wrapped:
        assert exit_code_for(wrapped) == 5


def test_cli_exit_codes_end_to_end(monkeypatch):
    def expired():
        raise SystemExit(3)
    monkeypatch.setattr(cli, "_client", expired)
    assert runner.invoke(cli.app, ["changes", "add-note", "CHN-1", "x"]).exit_code == 3
    assert runner.invoke(cli.app, ["changes", "update", "CHN-1"]).exit_code in (2, 3)


# ── asset CI-type suffix is discovered, not hard-coded ───────────────────────
def test_asset_display_discovers_ci_type_suffix():
    row = cli._asset_display({"config_item": {"display_id": 1, "name": "n", "ci_type_id": 77,
                                                "asset_state_77": "In Use", "serial_number_77": "SN1",
                                                "asset_state_88": "Other"}})
    assert row["state"] == "In Use" and row["serial"] == "SN1"
    row = cli._asset_display({"display_id": 2, "name": "m", "asset_state_5": "Stock", "serial_number_5": "S2"})
    assert row["state"] == "Stock" and row["serial"] == "S2"
    row = cli._asset_display({"display_id": 3, "name": "m"})
    assert row["state"] == "-" and row["serial"] == "-"
