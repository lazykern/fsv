"""Form UI building blocks without a screen: fuzzy filter, typed dates,
file checks, path completion, and the session's live rules."""
from __future__ import annotations

import datetime as dt
from zoneinfo import ZoneInfo

import pytest

import acme_change_form as A
from fsv.formui import session as S
from fsv.formui.fuzzy import fold, fuzzy_filter
from fsv.formui.when import parse_when, preview
from fsv.formui.widgets import complete_path
from fsv.forms import LookupValue

BKK = ZoneInfo("Asia/Bangkok")
PROFILE = "Freshservice user time zone (Bangkok)"


@pytest.fixture
def acme(monkeypatch):
    monkeypatch.delenv("FSV_TZ", raising=False)
    c = A.FakeClient()
    return c, S.change_session(c)


# ── fuzzy ────────────────────────────────────────────────────────────────
def test_fuzzy_ranks_prefix_then_word_then_substring_then_description_then_letters():
    items = [("Staging", ""), ("Live", "production traffic"), ("Pre-stage", ""), ("Mostly stable", ""),
             ("Sandbox", "")]
    assert [items[i][0] for i in fuzzy_filter("sta", items)] == ["Staging", "Pre-stage", "Mostly stable"]
    assert [items[i][0] for i in fuzzy_filter("traffic", items)] == ["Live"]
    assert [items[i][0] for i in fuzzy_filter("sbx", items)] == ["Sandbox"]
    assert fuzzy_filter("", items) == [0, 1, 2, 3, 4]


def test_fuzzy_is_case_and_accent_insensitive():
    assert fold("  ÄNDERUNG  Typ ") == "anderung typ"
    assert fuzzy_filter("anderung", [("Änderungstyp", ""), ("Priorität", "")]) == [0]
    assert fuzzy_filter("PRIO", [("Änderungstyp", ""), ("Priorität", "")]) == [1]


# ── dates ────────────────────────────────────────────────────────────────
@pytest.mark.parametrize("text", ["2026-11-26 19:00", "2026-11-26T19:00", "26 Nov 2026 19:00", "Nov 26 2026 7pm",
                                  "thu 26 nov 2026 7:00pm", "2026/11/26 19:00"])
def test_datetime_inputs_and_preview(text):
    p = parse_when(text, tz=BKK, tz_source=PROFILE)
    assert p.ok, p.error
    assert p.value == dt.datetime(2026, 11, 26, 19, 0, tzinfo=BKK)
    assert p.preview == "Thu 26 Nov 2026 19:00 (+07:00 from your Freshservice profile tz)"


def test_relative_dates_and_typed_offset():
    today = dt.date(2026, 11, 11)  # a Wednesday
    assert parse_when("tomorrow 9am", tz=BKK, today=today).value == dt.datetime(2026, 11, 12, 9, 0, tzinfo=BKK)
    assert parse_when("+2d 10:00", tz=BKK, today=today).value.date() == dt.date(2026, 11, 13)
    assert parse_when("fri 22:00", tz=BKK, today=today).value.date() == dt.date(2026, 11, 13)
    p = parse_when("2026-11-26 19:00+09:00", tz=BKK, tz_source=PROFILE)
    assert p.value.utcoffset() == dt.timedelta(hours=9) and "as typed" in p.preview


def test_date_errors_are_explained():
    assert "add a time" in parse_when("2026-11-26", tz=BKK).error
    assert "not a real date" in parse_when("2026-02-30 10:00", tz=BKK).error
    assert "not a time of day" in parse_when("2026-11-26 25:00", tz=BKK).error
    assert "date only" in parse_when("2026-11-26 10:00", tz=BKK, date_only=True).error
    d = parse_when("26 Nov 2026", tz=BKK, date_only=True)
    assert d.value == dt.date(2026, 11, 26) and d.preview == "Thu 26 Nov 2026"
    assert parse_when("", tz=BKK).value is None and parse_when("", tz=BKK).ok
    assert preview(dt.datetime(2026, 1, 2, 3, 4, tzinfo=dt.timezone.utc), "system time zone").endswith(
        "(+00:00 from this computer's tz (no Freshservice profile tz))")


# ── files ────────────────────────────────────────────────────────────────
def test_check_files_types_count_and_missing(tmp_path):
    deck, pdf, empty = tmp_path / "deck.pptx", tmp_path / "brief.pdf", tmp_path / "empty.pptx"
    deck.write_bytes(b"deck")
    pdf.write_bytes(b"pdf")
    empty.write_bytes(b"")
    paths, e = S.check_files(f"{deck}", allowed=(".pptx",), max_files=1)
    assert paths == [str(deck)] and e == ""
    assert "allowed file types are .pptx" in S.check_files(str(pdf), allowed=(".pptx",))[1]
    assert "at most 1 file" in S.check_files(f"{deck}; {deck}", max_files=1)[1]
    assert "not found" in S.check_files(str(tmp_path / "nope.pptx"))[1]
    assert "empty" in S.check_files(str(empty))[1]
    big = tmp_path / "big.bin"
    big.write_bytes(b"x" * 2_000_001)
    assert "over the 2 MB limit" in S.check_files(str(big), max_bytes=2_000_000)[1]


def test_path_completion(tmp_path):
    (tmp_path / "rollout-plan.xlsx").write_bytes(b"x")
    (tmp_path / "rollback.md").write_bytes(b"x")
    (tmp_path / "evidence").mkdir()
    base = str(tmp_path)
    assert complete_path(f"{base}/rol") == f"{base}/roll"
    assert complete_path(f"{base}/rollo") == f"{base}/rollout-plan.xlsx"
    assert complete_path(f"a.pdf; {base}/ev") == f"a.pdf; {base}/evidence/"
    assert complete_path(f"{base}/zzz") is None


# ── session: live rules ──────────────────────────────────────────────────
def test_answer_reveals_and_notes(acme):
    _c, s = acme
    s.set("cloud_only", "Yes")
    assert not s.view("staging_start").visible
    out = s.set("cloud_only", "No")
    assert "+ Staging Start (required), Staging End (required) appeared because Cloud Only = No" in out.notes
    assert s.view("staging_start").visible and s.view("staging_start").required


def test_restricted_dropdown_offers_only_allowed_and_preselects(acme):
    _c, s = acme
    out = s.set("cloud_only", "Yes")
    sugg, _ = s.suggestions("stage", "")
    assert [x.label for x in sugg] == ["Live"]
    assert s.state.get("stage") == "Live"
    assert any("Stage = Live (the only choice rule Cloud only: yes allows)" == n for n in out.notes)
    assert "choices limited by rule Cloud only: yes" in s.view("stage").note
    assert "is not offered now" in s.set("stage", "Staging").error


def test_effects_explain_what_each_choice_does(acme):
    _c, s = acme
    eff = s.effects("cloud_only")
    assert any(e.startswith("Picking No will") and "limit Stage to Staging" in e for e in eff)
    assert any(e.startswith("Picking Yes will hide Staging Start, Staging End") for e in eff)
    assert any("show the message" in e for e in s.effects("priority"))


def test_problems_list_required_and_requester(acme):
    _c, s = acme
    probs = s.problems()
    assert probs[0] == "Requester is required" and "Subject is required" in probs
    s.set("requester", LookupValue(9000007001, "Jane Doe", "jane.doe@example.com"))
    assert "Requester is required" not in s.problems()
    assert s.view("subject").error == ""  # not touched yet
    s.show_all_errors = True
    assert s.view("subject").error == "required"


def test_bad_typed_value_is_an_inline_error(acme):
    _c, s = acme
    out = s.set("services", ["Billing", "Nope"])
    assert "not a valid choice" in out.error and "not a valid choice" in s.view("services").error
    assert any(p.startswith("Services:") for p in s.problems())


def test_effects_preview_limit_is_announced(acme, monkeypatch):
    import fsv.formui.session as session_mod
    _c, s = acme
    monkeypatch.setattr(session_mod, "MAX_EFFECT_CHOICES", 1)
    eff = s.effects("cloud_only")
    assert eff and "not previewed" in eff[-1] and "first 1 were checked" in eff[-1]
