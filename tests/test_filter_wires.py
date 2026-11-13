"""Captured list-filter wires: dates, numbers, multi-select, text refusal."""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli, filter_proof, query, service
from fsv.resources import CHANGES, PROBLEMS, TICKETS
from test_views_cli import _world

SCH = {"fields": [
    {"name": "status", "label": "Status", "field_type": "default_status", "default_field": True,
     "choices": [{"id": 2, "value": "Open"}]},
    {"name": "subject", "label": "Subject", "field_type": "default_subject", "default_field": True},
    {"name": "cf_notes", "label": "Notes", "field_type": "custom_paragraph"},
    {"name": "cf_score", "label": "Impact Score", "field_type": "custom_number", "workspace_id": 7},
    {"name": "msf_kind", "label": "Impact Kind", "field_type": "custom_multi_select", "workspace_id": 7,
     "choices": [{"id": 160, "value": "Outage"}, {"id": "161", "value": "Slow"}, {"id": "x", "value": "Bad"}]},
]}


def build(res, *where):
    return query.build_query_hash_from_schema(None, res, SCH, list(where))


@pytest.mark.parametrize("res", [TICKETS, CHANGES, PROBLEMS])
def test_day_and_between_use_inclusive_is_in_the_range(res):
    assert json.loads(build(res, "created_at=2030-01-05")) == [
        {"value": {"from": "2030-01-05", "to": "2030-01-05"}, "condition": "created_at",
         "operator": "is_in_the_range", "type": "default"}]
    assert json.loads(build(res, "created_at>=2030-01-01", "created_at<=2030-01-31"))[0]["value"] == {
        "from": "2030-01-01", "to": "2030-01-31"}


def test_strict_bounds_shift_one_whole_day_and_explain_it():
    explain: list = []
    item = query.date_condition("tickets", "created_at", ">", "2030-01-31", explain, "created_at")
    assert item["value"] == {"from": "2030-02-01"}
    assert "sent as from 2030-02-01" in explain[0]["value"]
    item = query.date_condition("tickets", "created_at", "<", "2030-03-01", [], "created_at")
    assert item["value"] == {"to": "2030-02-28"}


@pytest.mark.parametrize("where,msg", [
    (["created_at>=2030-01-01"], "needs both ends"),
    (["created_at<=2030-01-01"], "needs both ends"),
    (["created_at=2030-01-01T10:00:00"], "whole day"),
    (["created_at=2030-02-30"], "invalid date"),
    (["created_at!=2030-01-01"], "not supported"),
])
def test_one_sided_and_sub_day_dates_are_refused(where, msg):
    with pytest.raises(ValueError, match=msg):
        build(TICKETS, *where)


@pytest.mark.parametrize("raw,operator,token", [
    ("on:yesterday", "on", "yesterday"), ("last:one_week", "in_the_last", "one_week"),
    ("next:six months", "in_the_next", "six_months")])
def test_relative_dates_go_to_advanced_with_workspace(raw, operator, token):
    payload = json.loads(build(TICKETS, f"created_at={raw}"))
    assert payload == {"all": [
        {"value": token, "condition": "created_at", "operator": operator, "type": "default"},
        {"condition": "workspace_id", "operator": "is_in", "value": [{"id": 7}], "type": "default"}], "any": []}
    due = json.loads(build(CHANGES, "due_by=on:week"))
    assert due["all"][0]["condition"] == "due_by_date"
    assert due["all"][1] == {"condition": "workspace_id", "operator": "is", "value": 7, "type": "default"}


@pytest.mark.parametrize("where", ["created_at=on:yesterday", "created_at=last:one_week", "Impact Score=3",
                                   "Impact Kind=Outage", "updated_at>=2030-01-01"])
def test_problems_refuse_unproven_shapes(where):
    with pytest.raises(ValueError, match="nothing was sent"):
        build(PROBLEMS, where, "updated_at<=2030-02-01") if where.startswith("updated") else build(PROBLEMS, where)


@pytest.mark.parametrize("bad", ["on:fortnight", "last:yesterday", "on:5"])
def test_unknown_relative_tokens_are_refused(bad):
    with pytest.raises(ValueError, match="unknown"):
        build(TICKETS, f"created_at={bad}")


@pytest.mark.parametrize("op,operator", [("=", "is"), ("!=", "is_not"), (">=", "is_greater_than_or_equal"),
                                         ("<=", "is_less_than_or_equal")])
def test_number_operators_send_string_values(op, operator):
    payload = json.loads(build(CHANGES, f"Impact Score{op}3"))
    assert payload["all"][0] == {"value": "3", "condition": "cf_score", "operator": operator, "type": "custom_field"}


@pytest.mark.parametrize("expr", ["Impact Score>3", "Impact Score<3", "Impact Score=3.5", "Impact Score~=3"])
def test_number_strict_and_non_integer_are_refused(expr):
    with pytest.raises(ValueError):
        build(TICKETS, expr)


def test_multi_select_sends_integer_choice_id():
    assert json.loads(build(TICKETS, "Impact Kind=Outage"))["all"][0]["value"] == [160]
    assert json.loads(build(TICKETS, "Impact Kind!=Slow"))["all"][0] == {
        "value": [161], "condition": "msf_kind", "operator": "is_not_in", "type": "custom_field"}
    with pytest.raises(ValueError, match="integer choice id"):
        build(TICKETS, "Impact Kind=Bad")


@pytest.mark.parametrize("expr", ["Subject=Acme", "Notes=Globex", "description=x"])
def test_text_fields_fail_closed(expr):
    sch = {"fields": SCH["fields"] + [{"name": "description", "field_type": "default_description"}]}
    with pytest.raises(ValueError, match="no list filter exists"):
        query.build_query_hash_from_schema(None, TICKETS, sch, [expr])


def test_or_grouping_keeps_workspace_in_all():
    payload = query.serialize("tickets", [{"condition": "status", "operator": "is_in", "value": ["2"],
                                           "type": "default"}], True, 7)[0]
    assert json.loads(payload) == {"any": [{"condition": "status", "operator": "is_in", "value": ["2"],
                                            "type": "default"}],
                                   "all": [{"condition": "workspace_id", "operator": "is_in",
                                            "value": [{"id": 7}], "type": "default"}]}
    with pytest.raises(ValueError, match="no advanced filter"):
        query.serialize("problems", [{"condition": "status"}], True, 7)


def test_advanced_payload_uses_advanced_param():
    params = service._list_params(TICKETS, 1, 30, None, None, None, '{"all":[]}', True)
    assert params["advanced_query_hash"] == '{"all":[]}' and params["query_hash"] == ""


def test_number_predicates_are_verified_client_side():
    clause = [{"condition": "cf_score", "operator": "is_greater_than_or_equal", "value": "3", "type": "custom_field"}]
    assert filter_proof.matches({"custom_fields": {"cf_score": 4}}, clause)
    assert not filter_proof.matches({"custom_fields": {"cf_score": "2"}}, clause)
    assert filter_proof.contradictions([{"id": 9, "custom_fields": {"cf_score": 1}}], clause) == [9]
    with pytest.raises(ValueError):
        filter_proof.matches({"custom_fields": {"cf_score": None}}, clause)


@pytest.mark.parametrize("tenant", ["acme", "globex"])
@pytest.mark.parametrize("args", [["tickets", "ls", "--where", "Subject=Acme"],
                                  ["changes", "ls", "--where", "created_at>=2030-01-01"],
                                  ["tickets", "ls", "--where", "Created=on:fortnight"]])
def test_cli_refusals_send_no_list_request(monkeypatch, tenant, args):
    world = _world(monkeypatch, tenant)
    result = CliRunner().invoke(cli.app, args + ["--json"])
    assert result.exit_code == 2, result.output
    assert not world.calls(f"/api/_/{args[0]}") and not world.writes()


@pytest.mark.parametrize("tenant", ["acme", "globex"])
def test_cli_strict_date_shift_is_sent_and_noted(monkeypatch, tenant):
    world = _world(monkeypatch, tenant)
    result = CliRunner().invoke(cli.app, ["changes", "ls", "--where", "created_at>2030-01-31",
                                          "--where", "created_at<2030-03-01", "--json"])
    assert result.exit_code == 0, result.output
    sent = json.loads(world.calls("/api/_/changes")[-1]["query_hash"])
    assert sent == [{"value": {"from": "2030-02-01", "to": "2030-02-28"}, "condition": "created_at",
                     "operator": "is_in_the_range", "type": "default"}]


WS_TICKETS = {"condition": "workspace_id", "operator": "is_in", "value": [{"id": 7}], "type": "default"}


@pytest.mark.parametrize("field,advanced", [("due_by", "due_by_date"), ("fr_due_by", "fr_due_by_date"),
                                            ("closed_at", "closed_date"), ("resolved_at", "resolved_date")])
def test_date_ranges_without_a_basic_wire_use_the_advanced_condition_name(field, advanced):
    # A basic query_hash due_by range answers 206 + search_error (unfiltered rows).
    payload = json.loads(build(TICKETS, f"{field}>=2030-01-01", f"{field}<=2030-01-31"))
    assert payload == {"all": [
        {"value": {"from": "2030-01-01", "to": "2030-01-31"}, "condition": advanced,
         "operator": "is_in_the_range", "type": "default"}, WS_TICKETS], "any": []}


@pytest.mark.parametrize("field", ["created_at", "updated_at", "planned_start_date", "planned_end_date"])
def test_date_ranges_with_a_basic_name_stay_in_query_hash(field):
    assert json.loads(build(TICKETS, f"{field}=2030-01-05")) == [
        {"value": {"from": "2030-01-05", "to": "2030-01-05"}, "condition": field,
         "operator": "is_in_the_range", "type": "default"}]


def test_due_range_on_changes_is_advanced_and_ticket_only_dates_refuse_elsewhere():
    due = json.loads(build(CHANGES, "due_by=2030-01-05"))
    assert due["all"][0]["condition"] == "due_by_date"
    for res in (CHANGES, PROBLEMS):
        with pytest.raises(ValueError, match="nothing was sent"):
            build(res, "closed_at=2030-01-05")
    with pytest.raises(ValueError, match="nothing was sent"):
        build(CHANGES, "resolved_at=on:today")


def test_updated_relative_period_uses_advanced_updated_at():
    payload = json.loads(build(TICKETS, "updated_at=last:one_week"))
    assert payload["all"][0] == {"value": "one_week", "condition": "updated_at", "operator": "in_the_last",
                                 "type": "default"}


@pytest.mark.parametrize("label,advanced", [("Due By", "due_by_date"), ("Closed", "closed_date"),
                                            ("Resolved date", "resolved_date"), ("First response due", "fr_due_by_date")])
def test_cli_panel_labels_for_due_closed_resolved_ranges(label, advanced):
    explain: list = []
    items = [cli._resolve_where(None, TICKETS, SCH, f"{label}{op}{day}", explain)
             for op, day in ((">=", "2030-11-13"), ("<=", "2030-11-16"))]
    payload, adv, _ = query.serialize("tickets", query.merge_date_ranges(items), False, 7)
    assert adv and json.loads(payload)["all"][0] == {
        "value": {"from": "2030-11-13", "to": "2030-11-16"}, "condition": advanced,
        "operator": "is_in_the_range", "type": "default"}
    assert [x["condition"] for x in explain] == [advanced, advanced]
