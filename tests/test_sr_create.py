"""``fsv sr create``: drafts, review, dry-run, plan hash, confirmation,
uploads after confirmation, requester rules, local attachment rules,
journal duplicates, freshness. Acme fixtures behind a mocked httpx
transport; nothing reaches a network and no real request is placed."""
from __future__ import annotations

import json
import textwrap
from pathlib import Path

import pytest
from typer.testing import CliRunner

import acme_catalog as A
import fsv.change_flow as flow
import fsv.cli_sr as cli_sr
from fsv import config, journal
from fsv.forms import decode_br_validation_excludes

runner = CliRunner()
PLACE = "/api/_/catalog/service-items/77/place-request"

DRAFT = textwrap.dedent("""\
    requester: jane.doe@example.com
    answers:
      Title: Dark mode
      Details: Please add dark mode.
      Needed by: 2030-12-03
      1. Expected benefit: More revenue (Total USD)
      1.1 Revenue target: "+5%"
      Priority: Low
      Overview deck (.pptx): deck.pptx
    attachments: [brief.pdf]
    """)

EXPECTED_CUSTOM = {
    "title_100": "Dark mode",
    "paragraph_101": "Please add dark mode.",
    "date_103": "2030-12-03",
    "goal": "More revenue  (Total USD)",  # raw value, double space kept
    "goal_1_kpi": "+5%",
    "priority": "Low - cosmetic, reports",
    "overview_deck": ["<upload 1: deck.pptx>"],
}


@pytest.fixture
def fake(monkeypatch):
    f = A.AcmeCatalog()
    c = f.client()
    monkeypatch.setattr(cli_sr, "_client", lambda: c)
    monkeypatch.setattr(flow, "interactive", lambda: False)
    monkeypatch.setattr(cli_sr.console, "width", 240)
    monkeypatch.setattr(cli_sr.err, "width", 240)
    return f


@pytest.fixture
def work(tmp_path, monkeypatch) -> Path:
    monkeypatch.chdir(tmp_path)
    (tmp_path / "deck.pptx").write_bytes(b"pptx-bytes")
    (tmp_path / "brief.pdf").write_bytes(b"pdf-bytes")
    (tmp_path / "sr.yaml").write_text(DRAFT)
    return tmp_path


def run(*args, input=None):
    r = runner.invoke(cli_sr.app, list(args), input=input)
    if r.exception and not isinstance(r.exception, SystemExit):
        raise r.exception
    r.flat = " ".join(r.output.split())
    return r


def dry(*args) -> dict:
    r = run("create", *args, "--dry-run", "-o", "json")
    assert r.exit_code == 0, r.output
    return json.loads(r.output)


def places(fake):
    return [w for w in fake.writes() if w[1] == PLACE]


# ── dry run ─────────────────────────────────────────────────────────────────
def test_dry_run_payload_matches_the_captured_shape(fake, work):
    out = dry("77", "--from", "sr.yaml")
    body = out["payload"]
    assert list(body) == ["email", "custom_fields", "br_validation_excludes", "attachments"]
    assert body["email"] == A.JANE  # bare address, not "Name <email>"
    assert body["custom_fields"] == EXPECTED_CUSTOM
    assert body["attachments"] == ["<upload 2: brief.pdf>"]
    token = body["br_validation_excludes"]
    assert token.count(".") == 1
    assert decode_br_validation_excludes(token)["items"] == {"4400001": {"optional": [], "disabled": [], "hidden": []}}
    assert len(out["plan_hash"]) == 64
    assert [u["name"] for u in out["uploads"]] == ["deck.pptx", "brief.pdf"]
    assert all(len(u["sha256"]) == 64 for u in out["uploads"])
    assert out["safety"] == "This sends a real service request to the catalog item's fulfilment team."
    assert "Priority: 'Low' matched the choice 'Low - cosmetic, reports'" in out["warnings"]
    assert dry("77", "--from", "sr.yaml")["plan_hash"] == out["plan_hash"]  # deterministic
    assert not fake.writes()


def test_dry_run_review_has_safety_and_requester_lines(fake, work):
    r = run("create", "77", "--from", "sr.yaml", "--dry-run")
    assert r.exit_code == 0, r.output
    assert "This sends a real service request to the catalog item's fulfilment team." in r.flat
    assert "Requester: Jane Doe (approvals route via Jane Doe’s manager)" in r.flat
    assert "deck.pptx 10 B sha256:" in r.flat and "brief.pdf 9 B" in r.flat
    assert "Subject (item template) Dark mode" in r.flat
    assert "plan_hash:" in r.flat and "--yes --expect-plan" in r.flat
    assert not fake.writes()


def test_plan_hash_changes_with_file_content(fake, work):
    h1 = dry("77", "--from", "sr.yaml")["plan_hash"]
    (work / "deck.pptx").write_bytes(b"other bytes")
    assert dry("77", "--from", "sr.yaml")["plan_hash"] != h1


def test_template_when_no_input_then_round_trip(fake, work):
    r = run("create", "77", "--dry-run")
    assert r.exit_code == 0
    t = r.output
    for text in ("# REQUIRED · email or exact name of the requester", "requester:", "  Title:",
                 "# choices: More revenue  (Total USD) | Lower cost | Other", "→ adds: 1.1 Revenue target",
                 "types .pptx · at most 1 file(s)", "Overview deck (.pptx): []", "attachments: []",
                 "# This sends a real service request"):
        assert text in t, text
    yaml_text = "\n".join(line for line in t.splitlines() if not line.startswith("fill this in"))
    filled = (yaml_text.replace("requester:\n", f"requester: {A.JANE}\n")
              .replace("  Title:\n", "  Title: Dark mode\n").replace("  Details:\n", "  Details: x\n")
              .replace("  Needed by:\n", "  Needed by: 2030-12-03\n")
              .replace("  1. Expected benefit:\n", "  1. Expected benefit: Other\n")
              .replace("  Priority:\n", "  Priority: Medium\n")
              .replace("Overview deck (.pptx): []", "Overview deck (.pptx): [deck.pptx]")
              .replace("attachments: []", "attachments: [brief.pdf]"))
    (work / "t.yaml").write_text(filled)
    out = dry("77", "--from", "t.yaml")
    assert out["payload"]["custom_fields"]["priority"] == "Medium - planned work"
    assert out["payload"]["custom_fields"]["goal"] == "Other"


# ── sending ─────────────────────────────────────────────────────────────────
def test_yes_expect_plan_uploads_then_places_with_ids(fake, work):
    h = dry("77", "--from", "sr.yaml")["plan_hash"]
    assert not fake.writes()
    r = run("create", "77", "--from", "sr.yaml", "--yes", "--expect-plan", h[:16])
    assert r.exit_code == 0, r.output
    w = fake.writes()
    assert [x[1] for x in w] == ["/api/_/attachments", "/api/_/attachments", PLACE]
    up1, up2, (_, _, body) = w
    assert up1[2]["content_file_name"] == "deck.pptx" and up1[2]["content"].startswith("data:")
    assert up2[2]["content_file_name"] == "brief.pdf"
    # captured shape: bare email, file field -> [attachment id], general ids, token present
    assert list(body) == ["email", "custom_fields", "br_validation_excludes", "attachments"]
    assert body["email"] == A.JANE
    assert body["custom_fields"] == {**EXPECTED_CUSTOM, "overview_deck": [9000005001]}
    assert body["attachments"] == [9000005002]
    assert isinstance(body["br_validation_excludes"], str) and body["br_validation_excludes"]
    assert "placed SR-123" in r.flat
    done = [x for x in journal.read() if x.get("kind") == "plan:create" and x.get("event") == "done"]
    assert done and done[-1]["plan"] == h and done[-1]["created_id"] == 9100000123
    assert done[-1]["target"] == "catalog-item/77"


def test_non_tty_without_yes_refuses(fake, work):
    r = run("create", "77", "--from", "sr.yaml")
    assert r.exit_code == 2
    assert "not a terminal" in r.flat and "--dry-run" in r.flat
    assert not fake.writes()


def test_expect_plan_mismatch_sends_nothing(fake, work):
    r = run("create", "77", "--from", "sr.yaml", "--yes", "--expect-plan", "0" * 16)
    assert r.exit_code == 4 and "plan changed" in r.flat
    assert not fake.writes()


def test_duplicate_plan_refused_then_allowed(fake, work):
    assert run("create", "77", "--from", "sr.yaml", "--yes").exit_code == 0
    n = len(fake.writes())
    r = run("create", "77", "--from", "sr.yaml", "--yes")
    assert r.exit_code == 4 and "already sent" in r.flat
    assert len(fake.writes()) == n
    assert run("create", "77", "--from", "sr.yaml", "--yes", "--allow-duplicate").exit_code == 0
    assert len(places(fake)) == 2


def test_upload_failure_places_nothing(fake, work):
    fake.fail_upload = True
    r = run("create", "77", "--from", "sr.yaml", "--yes")
    assert r.exit_code == 5 and "upload failed" in r.flat and "nothing was placed" in r.flat
    assert not places(fake)
    assert journal.read()[-1]["event"] == "failed"


def test_rejected_request_reports_uploaded_ids(fake, work):
    fake.fail_place = 422
    r = run("create", "77", "--from", "sr.yaml", "--yes")
    assert r.exit_code == 5
    assert "9000005001 (deck.pptx)" in r.flat and "9000005002 (brief.pdf)" in r.flat
    assert "nothing was placed" in r.flat
    # a rejected plan may be retried (not a duplicate)
    fake.fail_place = None
    assert run("create", "77", "--from", "sr.yaml", "--yes").exit_code == 0


def test_metadata_change_before_send_refuses_with_yes(fake, work):
    def renamed(data):
        for f in data["service_item"]["custom_fields"]:
            if f["name"] == "priority":
                f["choices"][2]["value"] = f["choices"][2]["label"] = "Low - cosmetic"
        return data

    fake.on_second_item_get = renamed
    r = run("create", "77", "--from", "sr.yaml", "--yes")
    assert r.exit_code == 4 and "plan changed" in r.flat and "Priority" in r.flat
    assert not fake.writes()


def test_unrelated_metadata_change_still_sends(fake, work):
    def touched(data):
        data["service_item"]["updated_at"] = "2030-02-01T00:00:00+09:00"
        return data

    fake.on_second_item_get = touched
    assert run("create", "77", "--from", "sr.yaml", "--yes").exit_code == 0
    assert len(places(fake)) == 1


# ── requester ───────────────────────────────────────────────────────────────
def test_no_requester_non_interactive_exits_2(fake, work):
    (work / "nr.yaml").write_text(DRAFT.replace("requester: jane.doe@example.com\n", ""))
    r = run("create", "77", "--from", "nr.yaml", "--dry-run")
    assert r.exit_code == 2 and "requester is not set" in r.flat
    r = run("create", "77", "--from", "nr.yaml", "--yes")
    assert r.exit_code == 2
    assert not fake.writes()
    assert not any("agents/me" in p for p in fake.paths())  # never the logged-in user


def test_requester_from_flag_and_config(fake, work):
    (work / "nr.yaml").write_text(DRAFT.replace("requester: jane.doe@example.com\n", ""))
    out = dry("77", "--from", "nr.yaml", "--requester", "Sam Lee")
    assert out["payload"]["email"] == "sam.lee@example.com"
    assert "Requester: Sam Lee (approvals route via Sam Lee’s manager)" in out["review"]["notify"]
    cfg = config.load_config()
    cfg.setdefault("defaults", {})["sr_requester"] = A.JANE
    config.save_config(cfg)
    assert dry("77", "--from", "nr.yaml")["payload"]["email"] == A.JANE


def test_requester_must_match_exactly(fake, work):
    (work / "nr.yaml").write_text(DRAFT.replace("requester: jane.doe@example.com\n", ""))
    r = run("create", "77", "--from", "nr.yaml", "--requester", "Sam", "--dry-run")
    assert r.exit_code == 2 and "no exact match" in r.flat


def test_interactive_asks_for_requester_then_confirms(fake, work, monkeypatch):
    monkeypatch.setattr(flow, "interactive", lambda: True)
    (work / "nr.yaml").write_text(DRAFT.replace("requester: jane.doe@example.com\n", ""))
    r = run("create", "77", "--from", "nr.yaml", input=f"{A.JANE}\ny\n")
    assert r.exit_code == 0, r.output
    assert "Requester (email" in r.output
    assert "This sends a real service request" in r.flat
    assert places(fake) and places(fake)[0][2]["email"] == A.JANE


def test_interactive_cancel_uploads_nothing(fake, work, monkeypatch):
    monkeypatch.setattr(flow, "interactive", lambda: True)
    r = run("create", "77", "--from", "sr.yaml", input="n\n")
    assert r.exit_code == 1 and "nothing was uploaded or sent" in r.flat
    assert not fake.writes()


def test_interactive_empty_requester_answer_exits_2(fake, work, monkeypatch):
    monkeypatch.setattr(flow, "interactive", lambda: True)
    (work / "nr.yaml").write_text(DRAFT.replace("requester: jane.doe@example.com\n", ""))
    r = run("create", "77", "--from", "nr.yaml", input="\n")
    assert r.exit_code == 2 and "no requester given" in r.flat
    assert not fake.writes()


# ── local validation ────────────────────────────────────────────────────────
@pytest.mark.parametrize("edit, message", [
    (lambda t: t.replace("Overview deck (.pptx): deck.pptx", "Overview deck (.pptx): brief.pdf"),
     "allowed file types are .pptx"),
    (lambda t: t.replace("Overview deck (.pptx): deck.pptx", "Overview deck (.pptx): [deck.pptx, deck.pptx]"),
     "accepts at most 1 file"),
    (lambda t: t.replace("attachments: [brief.pdf]", "attachments: []"),
     "this item requires at least one general attachment"),
    (lambda t: t.replace("attachments: [brief.pdf]", "attachments: [missing.pdf]"), "file not found"),
    (lambda t: t.replace("  1.1 Revenue target: \"+5%\"\n", ""), "1.1 Revenue target is required"),
    (lambda t: t.replace("Priority: Low", "Priority: Urgent"), "'Urgent' is not a valid choice"),
    (lambda t: t + "description_md: hi\n", "catalog items have no description field"),
])
def test_local_rules_block_the_plan(fake, work, edit, message):
    (work / "bad.yaml").write_text(edit(DRAFT))
    r = run("create", "77", "--from", "bad.yaml", "--dry-run")
    assert r.exit_code == 2
    assert message in r.flat
    r = run("create", "77", "--from", "bad.yaml", "--yes")
    assert r.exit_code == 2
    assert not fake.writes()


def test_item_without_attachments_and_rule_driven_field(fake, work):
    base = ["--requester", A.JANE, "--set", "Reason=remote work", "--set", "Access level=admin"]
    r = run("create", "78", *base, "--dry-run")
    assert r.exit_code == 2 and "Justification is required" in r.flat  # shown + mandated by rule
    out = dry("78", *base, "--set", "Justification=on call")
    assert out["payload"]["custom_fields"] == {"reason_200": "remote work", "access_level": "Admin",
                                               "justification": "on call"}
    assert out["payload"]["attachments"] == []
    r = run("create", "78", *base, "--set", "Justification=x", "--attach", "brief.pdf", "--dry-run")
    assert r.exit_code == 2 and "does not accept general attachments" in r.flat
    # whitespace-insensitive match keeps the raw value byte for byte
    out = dry("78", "--requester", A.JANE, "--set", "Reason=r", "--set", "Access level=read write")
    assert out["payload"]["custom_fields"]["access_level"] == "Read  write"
    tok = decode_br_validation_excludes(out["payload"]["br_validation_excludes"])
    assert tok["items"]["4400002"]["hidden"] == [f"justification_{A.ACCOUNT}"]


def test_set_paths_resolve_against_cwd(fake, work, tmp_path_factory):
    other = tmp_path_factory.mktemp("elsewhere")
    (other / "sr.yaml").write_text(DRAFT.replace("deck.pptx", "decks/deck.pptx").replace("[brief.pdf]", "[]"))
    (other / "decks").mkdir()
    (other / "decks" / "deck.pptx").write_bytes(b"x")
    out = dry("77", "--from", str(other / "sr.yaml"), "--attach", "brief.pdf")
    assert [u["name"] for u in out["uploads"]] == ["deck.pptx", "brief.pdf"]
    out = dry("77", "--from", str(other / "sr.yaml"), "--attach", "brief.pdf",
              "--set", "Overview deck (.pptx)=deck.pptx")  # relative to the cwd (work)
    assert out["uploads"][0]["size"] == len(b"pptx-bytes")


def test_non_tty_without_any_input_explains(fake, work):
    r = run("create", "77")
    assert r.exit_code == 2 and "--dry-run" in r.flat
    r = run("create", "77", "--edit")
    assert r.exit_code == 2 and "--edit needs a terminal" in r.flat


# ── editor face ─────────────────────────────────────────────────────────────
def test_edit_loop_reopens_until_complete_then_places(fake, work, monkeypatch):
    from fsv import editor

    monkeypatch.setattr(flow, "interactive", lambda: True)
    seen: list[str] = []

    def fake_edit(text, parse, **kw):
        seen.append(text)
        if len(seen) == 1:
            t = (text.replace("requester:\n", f"requester: {A.JANE}\n")
                 .replace("  Title:\n", "  Title: Dark mode\n").replace("  Details:\n", "  Details: d\n")
                 .replace("  Needed by:\n", "  Needed by: 2030-12-03\n")
                 .replace("  1. Expected benefit:\n", "  1. Expected benefit: Lower cost\n")
                 .replace("  Priority:\n", "  Priority: Low\n")
                 .replace("Overview deck (.pptx): []", "Overview deck (.pptx): [deck.pptx]")
                 .replace("attachments: []", "attachments: [brief.pdf]"))
        else:
            t = text.replace("  1.2 Cost target:\n", "  1.2 Cost target: -3%\n")
        return parse(t, Path(work))

    monkeypatch.setattr(editor, "edit_text", fake_edit)
    r = run("create", "77", "--edit", input="y\n")
    assert r.exit_code == 0, r.output
    assert len(seen) == 2
    assert "# ERROR: 1.2 Cost target is required" in seen[1] and "  1.2 Cost target:" in seen[1]
    body = places(fake)[0][2]
    assert body["custom_fields"]["goal"] == "Lower cost" and body["custom_fields"]["goal_2_kpi"] == "-3%"
    assert body["custom_fields"]["overview_deck"] == [9000005001]
