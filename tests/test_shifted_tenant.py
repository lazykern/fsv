"""A 'shifted' synthetic tenant: every tenant-configurable id is moved away
from the platform defaults (statuses 21-23, task statuses 7-9, reminders
600/0 with renamed labels) and the tenant has two workspaces (5 and 9) whose
form fields differ. Touched modules must take every value from metadata."""
from __future__ import annotations

import copy
import json

import httpx
import pytest

from fsv import change_tasks, config, formmeta, journal, problem_status
from fsv.client import Client
from fsv.forms import FormSpec, RulesEngine

# Built-in problem statuses keep their platform internal ids (1 Open, 3 Closed)
# under renamed/translated labels; custom statuses get tenant-specific ids.
PROBLEM_STATUSES = [{"id": 1, "value": "Neu"}, {"id": 21, "value": "Prüfung"}, {"id": 22, "value": "Wip"},
                    {"id": 3, "value": "Erledigt"}]
# Ticket statuses: renamed built-ins (2-5) plus shifted custom statuses.
TICKET_STATUSES = [{"id": 2, "value": "Offen"}, {"id": 3, "value": "Wartend"}, {"id": 12, "value": "Abgebrochen"},
                   {"id": 4, "value": "Gelöst"}, {"id": 5, "value": "Erledigt"}, {"id": 13, "value": "In Arbeit"}]
# A metadata shape without the built-in internal ids: every meaning is ambiguous.
UNPROVABLE_STATUSES = [{"id": 10, "value": "New"}, {"id": 11, "value": "Wip"}, {"id": 12, "value": "Fixed"},
                       {"id": 13, "value": "Done"}]
TASK_FIELDS = [{"name": "status", "choices": [{"id": 7, "value": "Todo"}, {"id": 8, "value": "Doing"},
                                              {"id": 9, "value": "Done"}]},
               {"name": "notify_before", "choices": [{"id": 0, "value": "None"}, {"id": 600, "value": "10 Min"}]}]
DEFAULT_LITERALS = {1, 2, 3, 4, 5, 900, 1800, 2700, 3600, 7200}


def problem_fields(workspace: int) -> list[dict]:
    extra = "ws5_only" if workspace == 5 else "ws9_only"
    return [{"id": 601, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 1,
             "workspace_id": workspace},
            {"id": 602, "name": "status", "label": "Status", "field_type": "default_status", "position": 2,
             "choices": copy.deepcopy(PROBLEM_STATUSES), "workspace_id": workspace},
            {"id": 603, "name": extra, "label": extra, "field_type": "custom_text", "position": 3,
             "workspace_id": workspace, "default_field": False}]


@pytest.fixture
def shifted(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    monkeypatch.setattr(config, "API_INT", "https://globex.example.com/api/_")
    monkeypatch.setattr(config, "CACHE_DIR", tmp_path / "cache")
    monkeypatch.setattr(config, "CONFIG_FILE", tmp_path / "config.json")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    s = {"workspace": 5, "field_reads": 0, "writes": [],
         "row": {"id": 77, "subject": "Globex problem", "status": 21, "planned_start_date": None, "planned_end_date": None}}

    def handler(req):
        path = req.url.path.removeprefix("/api/_/")
        if req.method == "GET":
            if path == "problem_form_fields":
                s["field_reads"] += 1
                return httpx.Response(200, json={"problem_fields": problem_fields(s["workspace"])})
            if path.endswith("business_rules"):
                s.setdefault("rule_paths", []).append(path)
                return httpx.Response(200, json={"business_rules": []})
            if path == "problems/77":
                return httpx.Response(200, json={"problem": copy.deepcopy(s["row"])})
            raise AssertionError(path)
        s["writes"].append(json.loads(req.content))
        s["row"].update(s["writes"][-1])
        return httpx.Response(200, json={})
    c = Client(cookies={})
    c._client.close()
    c._client = httpx.Client(transport=httpx.MockTransport(handler))
    c._csrf = "synthetic"
    yield c, s
    c.close()


def use_workspace(ws: int) -> None:
    config.save_config({"domain": "globex.example.com", "forms": {"workspace_id": ws}})


def test_problem_status_sends_the_shifted_runtime_id(shifted):
    c, s = shifted
    use_workspace(5)
    plan = problem_status.build_plan(c, "PRB-77", "Wip")
    assert plan.payload == {"status": 22}
    result = problem_status.send(plan, c, reviewed_hash=plan.hash)
    assert result["verified"] and s["writes"] == [{"status": 22}]


def test_two_workspaces_never_share_cached_form_fields(shifted):
    c, s = shifted
    use_workspace(5)
    assert formmeta.load_meta(c, "problem", "new").spec.find("ws5_only") is not None
    s["workspace"] = 9
    use_workspace(9)
    meta9 = formmeta.load_meta(c, "problem", "new")
    assert meta9.spec.find("ws9_only") is not None and meta9.spec.find("ws5_only") is None
    assert s["field_reads"] == 2
    use_workspace(5)
    formmeta.load_meta(c, "problem", "new")  # ws5 cache still valid: no third read
    assert s["field_reads"] == 2


def test_change_task_payload_has_no_default_tenant_literals(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    label = change_tasks.runtime_reminder_label("10 Min", TASK_FIELDS)
    status = change_tasks.default_status(TASK_FIELDS)
    status_id = change_tasks.task_status.resolve(status, TASK_FIELDS)[0]
    plan = change_tasks.build_plan(77, "Globex task", status=status, status_id=status_id,
                                   notify_before=label, fields=TASK_FIELDS)
    assert plan.payload["status"] == 7 and plan.payload["notify_before"] == 600
    assert not {plan.payload["status"], plan.payload["notify_before"]} & DEFAULT_LITERALS


def test_current_entity_rule_reads_a_shifted_status_id():
    spec = FormSpec.from_form_fields({"problem_fields": problem_fields(5)}, module="problem")
    rule = {"id": 9001, "name": "Done locks subject", "module_type": "problem", "state": "active",
            "rule_type": "client_side", "form_type": "edit_form", "user_type": "both", "is_reversible": False,
            "condition_data": {"all": [{"name": "status", "operator": "is", "value": ["23"],
                                        "evaluate_on": "current_entity"}]},
            "action_data": {"action": [{"name": ["subject"], "property": "disable"}]}}
    eng = RulesEngine(spec, {"business_rules": [rule]}, module="problem")
    assert "subject" in eng.evaluate({"status": 21}, "edit", base={"status": 23, "subject": "x"}).disabled
    assert "subject" not in eng.evaluate({"status": 23}, "edit", base={"status": 3, "subject": "x"}).disabled


# --- status semantics: internal ids, never English labels --------------------

def ticket_meta(statuses=TICKET_STATUSES):
    from fsv.formmeta import FormMeta
    fields = [{"id": 9000007101, "name": "status", "label": "Zustand", "field_type": "default_status", "position": 1,
               "choices": copy.deepcopy(statuses)},
              {"id": 9000007102, "name": "summary", "label": "Lösung", "field_type": "custom_text", "position": 2,
               "required_for_closure": True}]
    return FormMeta("ticket", "edit_form", {"ticket_fields": fields}, {"business_rules": []}, 9, 0, "fixture", 123)


class TicketClient:
    def __init__(self):
        self.row = {"id": 123, "status": 2, "group_id": None, "responder_id": 9000001234, "workspace_id": 9,
                    "resolution_notes": None, "custom_fields": {"summary": "Globex fix"}}

    def int_get(self, path):
        return {"ticket": copy.deepcopy(self.row)}


@pytest.fixture
def globex_tickets(monkeypatch, tmp_path):
    from fsv import ticket_lifecycle, ticket_merge, ticket_conversation
    monkeypatch.setattr(config, "DOMAIN", "globex.example.com")
    monkeypatch.setattr(journal, "journal_path", lambda: tmp_path / "journal")
    holder = {"statuses": TICKET_STATUSES}
    for mod in (ticket_lifecycle, ticket_merge, ticket_conversation):
        monkeypatch.setattr(mod, "load_meta", lambda *a, **k: ticket_meta(holder["statuses"]))
    return holder


def test_renamed_builtin_ticket_statuses_keep_their_meaning(globex_tickets):
    from fsv import ticket_lifecycle as flow
    from fsv.status_semantics import CLOSED, OPEN, RESOLVED
    c = TicketClient()
    resolved = flow.build_plan(c, 123, RESOLVED, note="Globex fix")
    assert resolved.payload["status"] == 4 and resolved.status == "Gelöst" and resolved.note_html
    c.row["resolution_notes"] = {"notes_html": "<p>Globex fix</p>"}
    assert flow.build_plan(c, 123, CLOSED).payload["status"] == 5
    assert flow.build_plan(c, 123, OPEN).payload["status"] == 2
    # a custom status is not closing: no resolution note or closure field is demanded
    c.row["custom_fields"]["summary"] = None
    assert flow.build_plan(c, 123, "Abgebrochen").payload["status"] == 12
    with pytest.raises(Exception, match="Lösung"):
        flow.build_plan(c, 123, "Erledigt")  # the translated Closed still enforces closure fields


def test_ticket_statuses_without_builtin_ids_are_refused(globex_tickets):
    from fsv import ticket_lifecycle as flow
    from fsv.reviewed_write import ReviewedWriteError
    from fsv.status_semantics import RESOLVED
    globex_tickets["statuses"] = UNPROVABLE_STATUSES
    with pytest.raises(ReviewedWriteError, match="not identifiable"):
        flow.build_plan(TicketClient(), 123, RESOLVED, note="Globex fix")
    with pytest.raises(ReviewedWriteError, match="cannot tell"):
        flow.build_plan(TicketClient(), 123, "Fixed")  # an English-looking label proves nothing


def test_merge_and_conversation_resolve_builtin_ids_under_translated_labels(globex_tickets):
    from fsv import ticket_conversation as conv, ticket_merge
    assert ticket_merge._closed(None, 123)[0] == 5
    choice, role = conv._status_choice(None, 123, "Wartend")
    assert (choice.id, role) == (3, "pending")
    with pytest.raises(conv.ConversationError, match="not a supported status"):
        conv._status_choice(None, 123, "Abgebrochen")
    globex_tickets["statuses"] = UNPROVABLE_STATUSES
    with pytest.raises(Exception, match="Closed status"):
        ticket_merge._closed(None, 123)


def test_safety_and_problem_semantics_use_ids_not_words():
    from fsv import safety, status_semantics as sem
    assert safety.status_closes("tickets", 4, TICKET_STATUSES) is True
    assert safety.status_closes("tickets", 12, TICKET_STATUSES) is False
    assert safety.status_closes("tickets", 12, UNPROVABLE_STATUSES) is None
    assert safety.status_closes("changes", 4, TICKET_STATUSES) is None
    assert sem.closes(PROBLEM_STATUSES, "problem", 3) is True
    assert sem.closes(PROBLEM_STATUSES, "problem", 22) is False
    assert sem.closes([{"id": 21, "value": "Closed"}], "problem", 21) is None


def test_problem_closed_label_translated_still_runs_closure_checks(shifted):
    c, s = shifted
    use_workspace(5)
    with pytest.raises(Exception) as exc:
        problem_status.build_plan(c, "PRB-77", "Erledigt")
    assert "closes the problem" not in str(exc.value)  # closing was recognised by id 3


# --- ticket kind and display-id prefix from the record's own fields ---------

def test_ticket_kind_comes_from_the_records_own_prefix_and_type(monkeypatch, tmp_path):
    from fsv import ticket_kind as tk
    from fsv.resources import TICKETS, parse_id
    from fsv.reviewed_write import ReviewedWriteError
    monkeypatch.setattr(config, "CONFIG_FILE", tmp_path / "config.json")
    fault = {"id": 77, "display_id": 77, "type": "Fault", "human_display_id": "IT-77"}
    assert tk.ticket_kind(fault) is None  # renamed type and a custom prefix prove nothing yet
    with pytest.raises(ReviewedWriteError, match="ids.incident_prefix"):
        tk.require_kind(fault, tk.INCIDENT, "caused relationship")
    with pytest.raises(ValueError, match="prefix IT"):
        parse_id("IT-77", TICKETS)
    config.save_config({"ids": {"incident_prefix": "IT", "service_request_prefix": "RQ"}})
    assert tk.ticket_kind(fault) == tk.INCIDENT and parse_id("it-77", TICKETS) == 77
    assert tk.ticket_kind({"display_id": 78, "type": "Anfrage", "human_display_id": "RQ-78"}) == tk.SERVICE_REQUEST
    # translated built-in type with the platform prefix, and SR proof from requested items
    assert tk.ticket_kind({"display_id": 5, "type": "Störung", "human_display_id": "INC-5"}) == tk.INCIDENT
    assert tk.ticket_kind({"display_id": 6, "type": "Anfrage", "has_requested_items": True}) == tk.SERVICE_REQUEST
    # conflicting evidence is never resolved by preference
    assert tk.ticket_kind({"display_id": 7, "type": "Service Request", "human_display_id": "INC-7"}) is None
    # a prefix that does not match the record's own number is not evidence
    assert tk.ticket_kind({"display_id": 8, "type": "Fault", "human_display_id": "IT-9"}) is None


# --- display labels from runtime choices; static maps only as fallback ------

SHIFTED_SCHEMA = {"fields": [
    {"name": "priority", "choices": [{"id": 1, "value": "P4"}, {"id": 2, "value": "P3"}, {"id": 3, "value": "P2"},
                                     {"id": 4, "value": "P1"}]},
    {"name": "impact", "choices": [{"id": 1, "value": "Gering"}, {"id": 3, "value": "Hoch"}]},
    {"name": "source", "label": "Source", "choices": [{"id": 2, "value": "portal_key"}, {"id": 20, "value": "Slack"},
                                                      {"id": 5, "value": "feedback_widget_key"}]},
    {"name": "status", "choices": copy.deepcopy(TICKET_STATUSES)}]}


def test_priority_impact_labels_come_from_runtime_choices():
    from fsv import render, schema, views
    from fsv.resources import TICKETS
    assert schema.display_label("priority", 4, SHIFTED_SCHEMA) == "P1"
    assert schema.display_label("impact", 3, SHIFTED_SCHEMA) == "Hoch"
    assert schema.display_label("impact", 2, SHIFTED_SCHEMA) == "2"  # never relabelled by the static map
    assert schema.display_label("priority", 4, {"fields": []}) == "Urgent"  # documented fallback
    assert views.priority_name({"priority": 4}, SHIFTED_SCHEMA) == "P1"
    row = render.list_rows([{"id": 7, "display_id": 7, "human_display_id": "IT-7", "status": 4, "priority": 1,
                             "subject": "Globex"}], TICKETS, SHIFTED_SCHEMA)[0]
    assert row["priority"] == "P4" and row["id"] == "IT-7" and row["status"] == "Gelöst"


def test_source_labels_translate_raw_i18n_keys_and_filter_by_them():
    from fsv import query, views
    assert views.source_name({"source": 2}, SHIFTED_SCHEMA) == "Portal"
    assert views.source_name({"source": 5}, SHIFTED_SCHEMA) == "Feedback Widget"
    assert views.source_name({"source": 20}, SHIFTED_SCHEMA) == "Slack"
    assert views.source_name({"source": 11}, SHIFTED_SCHEMA) == "11"  # custom id: no static guess
    source = SHIFTED_SCHEMA["fields"][2]
    assert query._choice_value(source, "Portal", "default") == 2
    assert query._choice_value(source, "portal_key", "default") == 2


# --- per-record workspace and the default-form assumption ------------------

def test_record_workspace_beats_the_global_override_and_is_reviewed(shifted):
    c, s = shifted
    use_workspace(5)  # global override; the record carries workspace 9
    s["row"]["workspace_id"] = 9
    plan = problem_status.build_plan(c, "PRB-77", "Wip")
    assert s["rule_paths"] and all(p.startswith("ws/9/") for p in s["rule_paths"])
    notes = plan.review().get("warnings") or []
    assert any("record is in workspace 9" in n for n in notes)
    assert any("forms.workspace_id=5 does not apply" in n for n in notes)
    # a record without its own workspace still follows the override
    s["row"].pop("workspace_id")
    s["rule_paths"].clear()
    problem_status.build_plan(c, "PRB-77", "Wip")
    assert all(p.startswith("ws/5/") for p in s["rule_paths"])


def test_form_spanning_workspaces_makes_the_default_form_explicit(monkeypatch):
    from fsv.formmeta import FormMeta
    monkeypatch.setattr(formmeta, "configured_workspace", lambda: None)
    fields = problem_fields(5) + [dict(problem_fields(9)[2], id=604)]
    meta = FormMeta("problem", "edit_form", {"problem_fields": fields}, {"business_rules": []}, 5, 0, "fixture", 77)
    assert meta.field_workspaces == [5, 9]
    assert any("more than one form exists" in n for n in meta.form_notes)
    single = FormMeta("problem", "edit_form", {"problem_fields": problem_fields(5)}, {"business_rules": []}, 5, 0,
                      "fixture", 77, 5)
    assert single.form_notes == []


# --- catalog lookup fields fail closed, by name ------------------------------

def test_required_catalog_lookup_on_a_second_workspace_item_is_refused_by_name():
    from fsv import catalog
    from fsv.drafts import Draft
    from fsv.forms import FormState
    item = {"id": 4400909, "display_id": 909, "name": "Gerät anfordern", "workspace_id": 9, "custom_fields": [
        {"id": 9000000031, "name": "geraet", "label": "Gerät", "type": 502, "position": 1, "required": True,
         "account_id": 9000000004, "field_options": {"link": "/lookup_choices"}}]}
    meta = catalog.ItemMeta(item, {"business_rules": []}, 9, 0)
    state = FormState(meta.spec, meta.engine(), mode="new")
    errors = catalog.unsupported_lookup_errors(state, Draft())
    assert len(errors) == 1 and errors[0].startswith("Gerät (geraet): catalog lookup field (type 502)")
    assert "Nothing was sent" in errors[0]
