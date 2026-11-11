"""Choice descriptions from help text, one problem per field, consistent
no-payload message, review labels."""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import pytest  # noqa: E402

import acme_catalog as A  # noqa: E402
import fsv.change_flow as flow  # noqa: E402
import fsv.cli_sr as cli_sr  # noqa: E402
from fsv.forms.spec import Choice, _fields_from_form_field, split_choice_help  # noqa: E402
from test_sr_create import DRAFT, fake, run, work  # noqa: E402,F401


def _ch(*labels: str) -> tuple[Choice, ...]:
    return tuple(Choice(value=x.lower(), label=x) for x in labels)


def test_longest_label_wins_and_unmatched_stays_help() -> None:
    help_ = "Very High - Cannot rollback  ||\n High - Rollback is slow || low-  can wait || General note"
    out, rest = split_choice_help(_ch("High", "Very High", "Low", "Medium"), help_)
    d = {c.label: c.description for c in out}
    assert d == {"High": "Rollback is slow", "Very High": "Cannot rollback", "Low": "can wait", "Medium": ""}
    assert rest == "General note"


def test_no_separator_leaves_helptext_alone() -> None:
    out, rest = split_choice_help(_ch("A"), "plain help - with dash")
    assert out[0].description == "" and rest == "plain help - with dash"


def test_boolean_hint_never_becomes_text_and_explicit_description() -> None:
    raw = {"name": "risk", "label": "Risk", "field_type": "custom_dropdown", "hint": True, "helptext": True,
           "choices": [{"value": "a", "label": "A", "description": "  long\n meaning "}]}
    f = _fields_from_form_field(raw, [])[0]
    assert f.helptext == "" and f.hint is True
    assert f.choices[0].description == "long meaning"


def test_wrong_file_type_reports_only_the_type_error(fake, work):
    (work / "bad.yaml").write_text(DRAFT.replace("Overview deck (.pptx): deck.pptx", "Overview deck (.pptx): brief.pdf"))
    r = run("create", "77", "--from", "bad.yaml", "--dry-run", "-o", "json")
    errs = json.loads(r.stdout)["errors"] if r.stdout.lstrip().startswith("{") else []
    deck = [e for e in errs if "Overview deck" in e]
    assert len(deck) == 1 and "allowed file types are .pptx" in deck[0]


def test_blocked_plan_prints_no_payload_and_other_attachments_label(fake, work):
    (work / "bad.yaml").write_text(DRAFT.replace("Priority: Low", "Priority: Urgent"))
    r = run("create", "77", "--from", "bad.yaml", "--dry-run")
    assert r.exit_code == 2
    assert "No payload yet — fix the problems above" in r.flat
    assert "Payload (POST" not in r.flat
    assert "Other attachments:" in r.flat


def test_label_with_double_space_matches() -> None:
    out, _ = split_choice_help((Choice("More revenue  (USD)", "More revenue  (USD)"),), "More revenue (USD) - Grows billings || x - y")
    assert out[0].description == "Grows billings"


def test_catalog_item_field_descriptions_from_helptext() -> None:
    from fsv.forms.spec import _field_from_sr
    c = {"id": "f", "name": "goal", "type": 1003, "label": "Goal", "hint": True,
         "helptext": "Low - cheap || Very High - costly || note",
         "choices": [{"id": 1, "value": "Low", "label": "Low"}, {"id": 2, "value": "Very High", "label": "Very High"}]}
    f, _ = _field_from_sr(c, None, [])
    assert [x.description for x in f.choices] == ["cheap", "costly"] and f.helptext == "note"
