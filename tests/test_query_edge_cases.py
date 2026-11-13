from __future__ import annotations

import json

import pytest

from fsv import query


def _where(expr):
    explain: list = []
    return query._resolve_where(None, None, {"fields": []}, expr, explain)


def test_date_not_equal_is_rejected_with_clear_error():
    with pytest.raises(ValueError, match="'!=' is not supported"):
        _where("created_at!=2026-01-01")


def test_date_bounds_are_inclusive_and_merge():
    assert _where("created_at>=2026-01-01")["value"] == {"from": "2026-01-01"}
    assert _where("created_at<=2026-02-01")["value"] == {"to": "2026-02-01"}
    # No strict operator exists: a whole-day shift, still one half of a range.
    assert _where("created_at>2026-01-01")["value"] == {"from": "2026-01-02"}
    assert _where("created_at<2026-01-01")["value"] == {"to": "2025-12-31"}
    assert _where("created_at=2026-01-01")["value"] == {"from": "2026-01-01", "to": "2026-01-01"}


@pytest.mark.parametrize("op", ["!=", "~="])
def test_cli_date_aliases_refuse_unproven_operators(op):
    import typer
    from fsv import cli
    from fsv.resources import CHANGES
    with pytest.raises(typer.Exit):
        cli._resolve_where(None, CHANGES, {"fields": []}, f"Created{op}2026-01-01", [])


def test_isdigit_is_ascii_only():
    assert query._isdigit("123")
    assert not query._isdigit("\u0661\u0662")  # Arabic-Indic digits
    assert not query._isdigit("\u00b2")  # superscript two
    assert not query._isdigit("")
    assert _where("group_id=12")["value"] == [12]
    assert _where("group_id=\u0661\u0662")["value"] == ["\u0661\u0662"]


def test_raw_query_hash_decoded_at_most_once():
    literal = json.dumps([{"condition": "subject", "operator": "is_in", "value": ["50%25 done"], "type": "default"}])
    assert query._load_query_hash(literal)[0]["value"] == ["50%25 done"]  # already JSON: untouched
    from urllib.parse import quote
    assert query._load_query_hash(quote(literal))[0]["value"] == ["50%25 done"]  # one decode only
    with pytest.raises(ValueError):
        query._load_query_hash("not json")
