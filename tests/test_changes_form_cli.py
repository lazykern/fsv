"""CLI faces for changes on the form engine (Acme fixtures, mocked client).

Covers: --from/--set/--preset, the annotated YAML draft and its reopen loop,
--dry-run payload + plan hash, -o json, --yes/--expect-plan, non-TTY
refusal, journal duplicate refusal, uploads only after confirmation,
metadata freshness, update sending only the diff, hints, `fsv form`.
"""
from __future__ import annotations

import json
import os
import sys
import textwrap
from pathlib import Path

import pytest
from typer.testing import CliRunner

import acme_change_form as A
import fsv.change_flow as flow
import fsv.cli as cli
from fsv import journal
from fsv.forms import decode_br_validation_excludes

runner = CliRunner()
JANE = "jane.doe@example.com"

DRAFT = textwrap.dedent("""\
    requester: jane.doe@example.com
    answers:
      Subject: "Billing: rotate keys"
      Cloud Only: No
      Stage: Staging
      Staging Start: 2030-11-18 09:00
      Staging End: 2030-11-18 12:00
      Services: [Billing, Search]
      Planned Start Date: 2030-11-26 19:00
      Planned End Date: 2030-11-26 22:00
    description_md: |
      Rotate the **billing** keys.

      - step one
      - step two
    """)

EXPECTED_PAYLOAD = {
    "subject": "Billing: rotate keys",
    "description": "<p>Rotate the <strong>billing</strong> keys.</p><ul><li>step one</li><li>step two</li></ul>",
    "status": 1,
    "priority": 1,
    "change_type": 1,
    "risk": 1,
    "impact": 1,
    "planned_start_date": "2030-11-26T19:00:00+09:00",
    "planned_end_date": "2030-11-26T22:00:00+09:00",
    "custom_fields": {
        "cloud_only": "No",
        "stage": "Staging",
        "staging_start": "2030-11-18T09:00:00+09:00",
        "staging_end": "2030-11-18T12:00:00+09:00",
        "services": ["Billing", "Search"],
    },
    "email": "Jane Doe <jane.doe@example.com>",
}


@pytest.fixture
def fake(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    c = A.FakeClient()
    monkeypatch.setattr(cli, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    return c


@pytest.fixture
def draft(tmp_path) -> Path:
    p = tmp_path / "change.yaml"
    p.write_text(DRAFT)
    return p


class _Result:
    """CliRunner result with whitespace-normalised output (rich wraps lines)."""

    def __init__(self, r):
        self.exit_code = r.exit_code
        self.raw = r.output
        self.output = " ".join(r.output.split())


def run(*args, input=None):
    r = runner.invoke(cli.app, list(args), input=input)
    if r.exception and not isinstance(r.exception, SystemExit):
        raise r.exception
    return _Result(r)


def dry_json(*args) -> dict:
    r = run("changes", "create", *args, "--dry-run", "-o", "json")
    assert r.exit_code == 0, r.output
    return json.loads(r.raw)


# ── dry run ─────────────────────────────────────────────────────────────────
def test_dry_run_payload_snapshot_and_plan_hash(fake, draft):
    out = dry_json("--from", str(draft))
    assert set(out) >= {"plan_hash", "payload", "review", "warnings"}
    payload = dict(out["payload"])
    token = payload.pop("br_validation_excludes")
    assert payload == EXPECTED_PAYLOAD
    # engine-built token: staging dates were shown by a rule, legacy flag hidden
    decoded = decode_br_validation_excludes(token)
    assert decoded["hidden"] == [f"legacy_flag_{A.ACCOUNT}"]
    assert len(out["plan_hash"]) == 64
    # deterministic: same inputs, same plan
    assert dry_json("--from", str(draft))["plan_hash"] == out["plan_hash"]
    assert not fake.writes()


def test_dry_run_text_shows_review_payload_and_hash(fake, draft):
    r = run("changes", "create", "--from", str(draft), "--dry-run")
    assert r.exit_code == 0, r.output
    for text in ("Review: New Change", "Staging Start", "Status on create", "(form default)",
                 "Payload", '"cloud_only": "No"', "plan_hash:", "--expect-plan"):
        assert text in r.output
    assert not fake.writes()


def test_rules_singular_shape_with_conditions_is_accepted(fake, draft):
    plural = dry_json("--from", str(draft))["plan_hash"]
    fake.rules_shape = "singular"
    os.environ.pop("FSV_TZ", None)
    assert dry_json("--from", str(draft))["plan_hash"] == plural


def test_rules_listing_without_conditions_fails_loudly(fake, draft):
    fake.rules = {"business_rule": [{"id": 9000004199, "name": "x", "position": 1}]}
    r = run("changes", "create", "--from", str(draft), "--dry-run")
    assert r.exit_code != 0 and "cannot evaluate" in r.output


def test_set_and_legacy_flags_go_through_the_engine(fake):
    out = dry_json("--subject", "Patch DB", "--priority", "High", "--set", "Cloud Only=Yes",
                   "--set", "Stage=Live", "--set", "Planned Start Date=2030-11-26 19:00",
                   "--set", "Planned End Date=2030-11-26 20:00", "--description", "plain text",
                   "--requester", JANE)
    p = out["payload"]
    assert p["subject"] == "Patch DB" and p["priority"] == 3 and p["description"] == "<div>plain text</div>"
    assert p["custom_fields"] == {"cloud_only": "Yes", "stage": "Live"}
    # Cloud Only = Yes hid the staging dates
    assert "staging_start" not in json.dumps(p["custom_fields"])


def test_validation_errors_exit_2_and_name_the_fields(fake):
    r = run("changes", "create", "--set", "Subject=x", "--requester", JANE, "--dry-run")
    assert r.exit_code == 2
    assert "Cloud Only is required" in r.output and "Description is required" in r.output


def test_restricted_choice_is_refused(fake, draft):
    draft.write_text(DRAFT.replace("Stage: Staging", "Stage: Live"))
    r = run("changes", "create", "--from", str(draft), "--dry-run")
    assert r.exit_code == 2 and "not offered" in r.output


def test_yaml_no_stays_a_label_not_a_bool():
    from fsv import yamlio
    assert yamlio.loads("a: No\nb: 22:30\nc: 2030-01-01\nd: true") == {"a": "No", "b": "22:30", "c": "2030-01-01", "d": True}
    for v in ("No", "true", "a: b", "# x", "22:30", "Reports, Exports", "-5"):
        assert yamlio.loads(f"k: {yamlio.scalar(v)}") == {"k": v}
        assert yamlio.loads(f"k: {yamlio.flow_list([v])}") == {"k": [v]}


# ── requester ───────────────────────────────────────────────────────────────
def test_requester_is_never_the_logged_in_user_silently(fake, draft):
    draft.write_text(DRAFT.replace("requester: jane.doe@example.com\n", ""))
    r = run("changes", "create", "--from", str(draft), "--dry-run")
    assert r.exit_code == 2 and "requester is not set" in r.output and "defaults.requester" in r.output


def test_requester_from_config_default_and_exact_match(fake, draft):
    from fsv import config
    draft.write_text(DRAFT.replace("requester: jane.doe@example.com\n", ""))
    config.save_config({"domain": "acme.freshservice.com", "defaults": {"requester": JANE}})
    assert dry_json("--from", str(draft))["payload"]["email"] == f"Jane Doe <{JANE}>"
    # "Jane" matches two people and neither exactly: error, never a pick
    r = run("changes", "create", "--from", str(draft), "--requester", "Jane", "--dry-run")
    assert r.exit_code == 2 and "no exact match" in r.output


# ── presets and hints ───────────────────────────────────────────────────────
def test_preset_answers_are_replayed_and_set_wins(fake, draft):
    from fsv import config
    presets = config.CONFIG_DIR / "presets"
    presets.mkdir(parents=True)
    (presets / "acme-normal.yaml").write_text("answers:\n  Priority: High\n  Services: [Mail]\n")
    out = dry_json("--from", str(draft), "--preset", "acme-normal", "--set", "Services=Chat")
    p = out["payload"]
    assert p["priority"] == 3  # from the preset
    assert p["custom_fields"]["services"] == ["Chat"]  # --set beats file beats preset
    r = run("changes", "create", "--preset", "missing", "--dry-run")
    assert r.exit_code == 2 and "not found" in r.output


def test_hints_file_adds_will_notify_lines(fake, draft):
    from fsv import config
    (config.CONFIG_DIR / "hints.yaml").write_text(textwrap.dedent("""\
        changes:
          - when: {"Notify Ops Team": "Yes"}
            warn: creates a task for the Acme ops team
          - when: {"Cloud Only": "Yes"}
            warn: never shown here
        """))
    draft.write_text(DRAFT.replace("  Services:", "  Notify Ops Team: Yes\n  Services:"))
    review = dry_json("--from", str(draft))["review"]
    assert "creates a task for the Acme ops team" in review["notify"]
    assert "never shown here" not in review["notify"]


# ── confirmation, plan hash, journal ────────────────────────────────────────
def test_non_tty_refuses_without_yes(fake, draft):
    r = run("changes", "create", "--from", str(draft))
    assert r.exit_code == 2 and "--yes" in r.output and not fake.writes()


def test_expect_plan_mismatch_refuses_then_match_creates(fake, draft):
    plan = dry_json("--from", str(draft))["plan_hash"]
    r = run("changes", "create", "--from", str(draft), "--yes", "--expect-plan", "0" * 16)
    assert r.exit_code == 4 and "plan changed" in r.output and not fake.writes()
    r = run("changes", "create", "--from", str(draft), "--yes", "--expect-plan", plan[:12])
    assert r.exit_code == 0, r.output
    posts = [w for w in fake.writes() if w[1] == "changes"]
    assert len(posts) == 1 and posts[0][2]["subject"] == "Billing: rotate keys"
    assert "CHN-502" in r.output


def test_journal_refuses_same_plan_within_24h(fake, draft):
    assert run("changes", "create", "--from", str(draft), "--yes").exit_code == 0
    r = run("changes", "create", "--from", str(draft), "--yes")
    assert r.exit_code == 4 and "--allow-duplicate" in r.output
    assert len([w for w in fake.writes() if w[1] == "changes"]) == 1
    r = run("changes", "create", "--from", str(draft), "--yes", "--allow-duplicate")
    assert r.exit_code == 0
    events = [e["event"] for e in journal.read() if e.get("kind") == "plan:create"]
    assert events == ["started", "done", "started", "done"]


def test_started_without_outcome_counts_as_possible_duplicate(fake, draft):
    plan = dry_json("--from", str(draft))["plan_hash"]
    journal.record_plan("started", plan, action="create")
    r = run("changes", "create", "--from", str(draft), "--yes")
    assert r.exit_code == 4 and "no recorded outcome" in r.output


def test_server_rejection_is_not_a_duplicate(fake, draft):
    from fsv.errors import APIError
    real_post = fake.int_post

    def reject(path, body):
        if path == "changes":
            raise APIError(422, {"errors": [{"field": "subject", "message": "bad"}]}, "POST", path)
        return real_post(path, body)

    fake.int_post = reject
    r = run("changes", "create", "--from", str(draft), "--yes")
    assert r.exit_code == 5 and "subject: bad" in r.output
    fake.int_post = real_post
    assert run("changes", "create", "--from", str(draft), "--yes").exit_code == 0


def test_blocking_message_needs_ack(fake, draft):
    draft.write_text(DRAFT.replace("  Services:", "  Priority: Urgent\n  Services:"))
    r = run("changes", "create", "--from", str(draft), "--yes")
    assert r.exit_code == 2 and "--ack" in r.output and not fake.writes()
    assert run("changes", "create", "--from", str(draft), "--yes", "--ack").exit_code == 0


# ── attachments ─────────────────────────────────────────────────────────────
def _with_files(draft: Path) -> None:
    (draft.parent / "plan.pdf").write_bytes(b"%PDF-1.4 acme")
    (draft.parent / "checklist.txt").write_text("go")
    draft.write_text(DRAFT + "attachments: [plan.pdf]\nplanning:\n  Go-Live Checklist: [checklist.txt]\n")


def test_attachments_upload_only_after_confirmation(fake, draft):
    _with_files(draft)
    out = dry_json("--from", str(draft))
    assert [a["name"] for a in out["review"]["attachments"]] == ["plan.pdf", "checklist.txt"]
    assert all(len(a["sha256"]) == 64 for a in out["review"]["attachments"])
    assert not fake.writes()  # dry run: nothing uploaded
    assert run("changes", "create", "--from", str(draft)).exit_code == 2  # refused: nothing uploaded
    assert not fake.writes()
    r = run("changes", "create", "--from", str(draft), "--yes", "--expect-plan", out["plan_hash"])
    assert r.exit_code == 0, r.output
    order = [(m, p) for m, p, _ in fake.writes()]
    assert order[0] == ("POST", "changes")  # the change first, then files
    assert ("POST", "attachments") in order[1:]
    put = next(b for m, p, b in fake.writes() if m == "PUT" and p == "changes/900000502")
    assert put == {"attachments": [9000005001]}
    assert any(p.startswith("changes/900000502/planning-fields") for _, p in order)


def test_file_change_changes_the_plan(fake, draft):
    _with_files(draft)
    before = dry_json("--from", str(draft))["plan_hash"]
    (draft.parent / "plan.pdf").write_bytes(b"%PDF-1.4 acme v2")
    assert dry_json("--from", str(draft))["plan_hash"] != before


def test_interactive_cancel_uploads_nothing(fake, draft, monkeypatch):
    _with_files(draft)
    monkeypatch.setattr(flow, "interactive", lambda: True)
    r = run("changes", "create", "--from", str(draft), input="n\n")
    assert r.exit_code == 1 and "nothing was sent" in r.output and not fake.writes()


def test_partial_failure_prints_created_id(fake, draft):
    _with_files(draft)
    fake.fail_upload = True
    r = run("changes", "create", "--from", str(draft), "--yes")
    assert r.exit_code == 5
    assert "900000502" in r.output and "upload refused" in r.output
    done = [e for e in journal.read() if e.get("kind") == "plan:create" and e["event"] == "done"]
    assert done and done[-1]["created_id"] == 900000502


# ── metadata freshness ──────────────────────────────────────────────────────
def test_metadata_change_before_submit_refuses_with_yes(fake, draft):
    real_get = fake.int_get
    seen = {"n": 0}

    def get(path, params=None):
        data = real_get(path, params)
        if path.endswith("business_rules"):
            seen["n"] += 1
            if seen["n"] > 1:  # an admin hid the staging dates since the review
                data["business_rules"][0]["action_data"]["action"][1]["property"] = "hide"
        return data

    fake.int_get = get
    r = run("changes", "create", "--from", str(draft), "--yes")
    assert r.exit_code in (2, 4) and "metadata changed" in r.output
    assert not [w for w in fake.writes() if w[1] == "changes"]


def test_unrelated_metadata_change_keeps_the_plan(fake, draft):
    real_get = fake.int_get
    seen = {"n": 0}

    def get(path, params=None):
        data = real_get(path, params)
        if path == "change_form_fields":
            seen["n"] += 1
            if seen["n"] > 1:
                data["change_fields"][0]["updated_at"] = "2030-02-02T00:00:00Z"
        return data

    fake.int_get = get
    plan = dry_json("--from", str(draft))["plan_hash"]
    seen["n"] = 0
    r = run("changes", "create", "--from", str(draft), "--yes", "--expect-plan", plan)
    assert r.exit_code == 0, r.output


# ── editor face ─────────────────────────────────────────────────────────────
def test_template_is_annotated_in_web_order(fake):
    r = run("changes", "create", "--dry-run")
    assert r.exit_code == 0, r.output
    t = r.raw
    assert "# REQUIRED · dropdown · Only the cloud team performs this change" in t
    assert "# choices: Yes | No" in t
    assert "(14 total; fsv form fields changes --choices Services)" in t
    assert "Status" not in t  # never asked on create
    assert "Legacy Flag" not in t  # hidden by a rule
    # optional until Cloud Only = No mandates it
    assert "  # optional · date-time (YYYY-MM-DD HH:MM, optional +HH:MM)\n  Staging Start:" in t
    assert t.index("Subject:") < t.index("Change Type: Normal") < t.index("Cloud Only:")
    assert "description_md: |" in t and "attachments: []" in t


FAKE_EDITOR = textwrap.dedent("""\
    import sys, pathlib
    path = pathlib.Path(sys.argv[1])
    state = pathlib.Path(sys.argv[0]).with_suffix(".round")
    n = int(state.read_text()) if state.exists() else 0
    state.write_text(str(n + 1))
    text = path.read_text()
    log = pathlib.Path(sys.argv[0]).with_suffix(".log")
    log.write_text(log.read_text() + "\\n=====\\n" + text if log.exists() else text)
    if n == 0:
        text = text.replace("requester:", "requester: jane.doe@example.com", 1)
        text = text.replace("  Subject:", '  Subject: "Billing: rotate keys"', 1)
        text = text.replace("  Cloud Only:", "  Cloud Only: No", 1)
        text = text.replace("  Planned Start Date:", "  Planned Start Date: 2030-11-26 19:00", 1)
        text = text.replace("  Planned End Date:", "  Planned End Date: 2030-11-26 22:00", 1)
        text = text.replace("description_md: |\\n  \\n", "description_md: |\\n  Rotate keys.\\n", 1)
    else:
        assert text.startswith("# New change\\n# ERROR: ") or "# ERROR: " in text.splitlines()[1]
        text = text.replace("  Stage:", "  Stage: Staging", 1)
        text = text.replace("  Staging Start:", "  Staging Start: 2030-11-18 09:00", 1)
        text = text.replace("  Staging End:", "  Staging End: 2030-11-18 12:00", 1)
    path.write_text(text)
    """)


def test_edit_reopens_with_errors_until_complete(fake, tmp_path, monkeypatch):
    editor = tmp_path / "editor.py"
    editor.write_text(FAKE_EDITOR)
    monkeypatch.setenv("VISUAL", f"{sys.executable} {editor}")
    monkeypatch.setattr(flow, "interactive", lambda: True)
    r = run("changes", "create", "--edit", input="y\n")
    assert r.exit_code == 0, r.output
    assert (tmp_path / "editor.round").read_text() == "2"
    log = (tmp_path / "editor.log").read_text()
    second = log.split("=====")[1]
    # round 2 shows the errors at the top; Cloud Only = No made the dates required
    assert second.lstrip().startswith("# New change\n# ERROR: Staging Start is required")
    assert "# REQUIRED · date-time (YYYY-MM-DD HH:MM, optional +HH:MM)\n  Staging Start:" in second
    assert "# choices: Staging\n  Stage:" in second  # the rule limited the choices
    post = next(b for m, p, b in fake.writes() if p == "changes")
    assert post["custom_fields"]["staging_start"] == "2030-11-18T09:00:00+09:00"
    assert post["subject"] == "Billing: rotate keys"


def test_edit_needs_a_terminal(fake):
    r = run("changes", "create", "--edit")
    assert r.exit_code == 2 and "terminal" in r.output


def test_no_input_and_no_flags_errors(fake):
    r = run("changes", "create", "--no-input")
    assert r.exit_code == 2 and "--from" in r.output


def test_interactive_save_draft(fake, draft, tmp_path, monkeypatch):
    monkeypatch.setattr(flow, "interactive", lambda: True)
    out = tmp_path / "saved.yaml"
    r = run("changes", "create", "--from", str(draft), input=f"s\n{out}\n")
    assert r.exit_code == 0, r.output
    saved = out.read_text()
    assert "Cloud Only: No" in saved and "Status" not in saved and not fake.writes()
    # the saved draft is a valid --from file with the same plan
    assert dry_json("--from", str(out))["payload"]["custom_fields"] == EXPECTED_PAYLOAD["custom_fields"]


# ── update ──────────────────────────────────────────────────────────────────
def test_update_sends_only_changed_fields(fake):
    out = run("changes", "update", "CHN-501", "--set", "Subject=Billing: rotate all keys",
              "--set", "Services=Billing", "--dry-run")
    assert out.exit_code == 0, out.output
    assert "Billing: rotate keys" in out.output and "Billing: rotate all keys" in out.output  # old -> new
    r = run("changes", "update", "CHN-501", "--set", "Subject=Billing: rotate all keys",
            "--set", "Services=Billing")
    assert r.exit_code == 0, r.output
    assert fake.writes() == [("PUT", "changes/501", {"subject": "Billing: rotate all keys"})]
    rules_calls = [p for m, path, p in fake.calls if m == "GET" and path.endswith("business_rules")]
    assert rules_calls and all(dict(q)["form_type"] == "edit_form" for q in rules_calls)


def test_update_runs_edit_form_rules(fake):
    # edit-form rule: Stage = Live makes Downtime Window required
    r = run("changes", "update", "CHN-501", "--set", "Downtime Window=", "--yes")
    assert r.exit_code == 2 and "Downtime Window is required" in r.output and not fake.writes()


def test_update_status_and_custom_field_diff_shape(fake):
    r = run("changes", "update", "CHN-501", "--set", "Status=Planning", "--set", "Notify Ops Team=Yes")
    assert r.exit_code == 2 and "status" in r.output and not fake.writes()  # safety gate
    r = run("changes", "update", "CHN-501", "--set", "Status=Planning", "--set", "Notify Ops Team=Yes", "--yes")
    assert r.exit_code == 0, r.output
    assert fake.writes() == [("PUT", "changes/501", {"status": 2, "custom_fields": {"notify_ops": "Yes"}})]


def test_update_nothing_changed(fake):
    r = run("changes", "update", "CHN-501", "--set", "Subject=Billing: rotate keys")
    assert r.exit_code == 0 and "no changes" in r.output and not fake.writes()


# ── fsv form ────────────────────────────────────────────────────────────────
def test_form_explain_names_the_rule(fake):
    r = run("form", "explain", "changes", "--set", "Cloud Only=No", "-o", "json")
    assert r.exit_code == 0, r.output
    rows = {x["label"]: x for x in json.loads(r.raw)["rows"]}
    assert rows["Staging Start"]["required"] and rows["Staging Start"]["visible"]
    assert any("Cloud only: no" in w for w in rows["Staging Start"]["why"])
    r = run("form", "explain", "changes", "--set", "Cloud Only=Yes")
    assert r.exit_code == 0 and "Cloud only: yes" in r.output and "hidden" in r.output


def test_form_fields_and_choices(fake):
    r = run("form", "fields", "changes", "-o", "json")
    rows = json.loads(r.raw)
    assert [x["label"] for x in rows][:3] == ["Requester", "Subject", "Change Type"]
    r = run("form", "fields", "changes", "--choices", "Priority", "-o", "json")
    choices = {c["label"]: c["description"] for c in json.loads(r.raw)["choices"]}
    assert choices["Urgent"] == "Outage"


def test_form_dump_and_diff(fake, tmp_path):
    r = run("form", "dump", "changes")
    assert r.exit_code == 0
    snap = tmp_path / "snap.json"
    snap.write_text(r.raw)
    assert "robin.agent" not in r.output  # the user's profile is not in snapshots
    assert "no changes" in run("form", "diff", str(snap)).output
    fake.fields["change_fields"][3]["choices"].append({"id": 9000002103, "display_id": 9000003103,
                                                       "value": "Maybe", "label": "Maybe"})
    fake.rules["business_rules"].pop()
    out = json.loads(run("form", "diff", str(snap), "-o", "json").output)
    kinds = {(d["kind"], d["change"]) for d in out}
    assert ("field", "changed") in kinds and ("rule", "removed") in kinds


# ── small pieces ────────────────────────────────────────────────────────────
def test_markdown_is_converted_safely():
    from fsv.mdlite import to_html
    assert to_html("# T\n\nHi **b** `x<y>` [l](https://example.com)") == (
        '<h1>T</h1><p>Hi <strong>b</strong> <code>x&lt;y&gt;</code> <a href="https://example.com">l</a></p>')
    assert "<script>" not in to_html("<script>alert(1)</script>")
    assert "javascript" not in to_html("[x](javascript:alert(1))").split("href")[-1] or "href" not in to_html(
        "[x](javascript:alert(1))")
    assert to_html("1. a\n2. b") == "<ol><li>a</li><li>b</li></ol>"


def test_submit_change_rejects_user_supplied_excludes_token():
    import fsv.create as create
    with pytest.raises(ValueError, match="computed by fsv"):
        create.submit_change({"subject": "x", "br_validation_excludes": "eyJ.eyJ"}, c=object())
