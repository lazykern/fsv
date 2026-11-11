"""Synthetic Freshservice form metadata for the form-engine tests.

Everything here is invented (acme domain, made-up field names, labels and
ids) but mirrors the shapes of the real endpoints:

* ``GET /api/_/change_form_fields``      -> ``change_fields()``
* ``GET /api/_/ws/2/business_rules``     -> ``change_rules()``
* ``GET /api/_/catalog/service-items/<id>`` -> ``service_item()``
* ``GET /api/v2/ticket_form_fields``     -> ``ticket_fields()``
"""
from __future__ import annotations

import copy
import itertools
import json
from typing import Any

_ID = itertools.count(7000)


def _f(name: str, label: str, field_type: str, position: int | None, **kw: Any) -> dict[str, Any]:
    out = {
        "id": kw.pop("id", next(_ID)),
        "name": name,
        "label": label,
        "description": kw.pop("description", ""),
        "field_type": field_type,
        "required": kw.pop("required", False),
        "required_for_closure": False,
        "position": position,
        "default_field": field_type.startswith("default_"),
        "choices": kw.pop("choices", []),
        "nested_fields": kw.pop("nested_fields", []),
        "placeholder": kw.pop("placeholder", ""),
        "helptext": kw.pop("helptext", ""),
        "hint": kw.pop("hint", False),
        "content": kw.pop("content", None),
        "displayed_to_agents": kw.pop("displayed_to_agents", True),
        "data_source": kw.pop("data_source", 0),
        "lookup_config": kw.pop("lookup_config", None),
    }
    out.update(kw)
    return out


def _cc(*pairs: tuple[int, int, str]) -> list[dict[str, Any]]:
    """custom choices: (id, display_id, value)."""
    return [{"id": i, "display_id": d, "value": v, "label": v} for i, d, v in pairs]


def change_fields() -> dict[str, Any]:
    fields = [
        _f("guide", "Guide", "custom_content", 1, content="&lt;p&gt;Read the &lt;b&gt;change guide&lt;/b&gt; first.&lt;/p&gt;"),
        _f("requester", "Requester", "default_requester", 2, required=True, hint=True,
           helptext="e.g. jane.doe@acme.example", placeholder="Requester's Email"),
        _f("department", "Department", "default_department", 3,
           choices=[{"id": 501, "value": "Engineering"}, {"id": 502, "value": "Finance"}]),
        _f("team", "Team", "custom_dropdown", 4, required=True,
           choices=_cc((1101, 11, "Platform"), (1102, 12, "Payments"), (1103, 13, "Labs"))),
        _f("group", "Group", "default_group", 5, required=True),
        _f("agent", "Agent", "default_agent", 6, required=True),
        _f("change_type", "Change Type", "default_change_type", 7, required=True,
           choices=[{"id": 1, "value": "Normal", "visible": True}, {"id": 2, "value": "Standard", "visible": True},
                    {"id": 3, "value": "Major", "visible": False}, {"id": 4, "value": "Emergency", "visible": True}]),
        _f("services", "Services", "custom_multi_select_dropdown", 8, required=True,
           choices=_cc((1201, 21, "Billing"), (1202, 22, "Search"), (1203, 23, "Reports, Exports"))),
        _f("cloud_only", "Cloud Only", "custom_dropdown", 9, required=True, hint=True,
           helptext="Only the cloud team performs this change",
           choices=_cc((1301, 901, "Yes"), (1302, 902, "No"))),
        _f("stage", "Stage", "custom_dropdown", 10, required=True,
           choices=_cc((1401, 801, "Live"), (1402, 802, "Staging"))),
        _f("rollout_tool", "Rollout Tool", "custom_dropdown", 11,
           choices=_cc((1501, 31, "Pipeline"), (1502, 32, "Script"), (1503, 33, "Manual"))),
        _f("reviewers", "Reviewers", "custom_multi_lookup", 12, data_source=5,
           lookup_config={"data_source": 5, "link": "/lookup_choices?field_name=reviewers_900000777&module=change"}),
        _f("staging_start", "Staging Start", "custom_date", 13),
        _f("staging_end", "Staging End", "custom_date", 14),
        _f("subject", "Subject", "default_subject", 15, required=True, placeholder="Service: objective"),
        _f("description", "Description", "default_description", 16, required=True, hint=True,
           helptext="What changes and why"),
        _f("emergency_reason", "Emergency Reason", "custom_dropdown", 17,
           choices=_cc((1601, 41, "Outage"), (1602, 42, "Security fix"))),
        _f("needs_review", "Needs Security Review", "custom_checkbox", 18, required=True),
        _f("review_scope", "Review Scope", "custom_multi_select_dropdown", 19,
           choices=_cc((1701, 51, "Code"), (1702, 52, "Infra"))),
        _f("planned_start_date", "Planned Start Date", "default_planned_start_date", 20, required=True),
        _f("planned_end_date", "Planned End Date", "default_planned_end_date", 21, required=True),
        _f("downtime", "Downtime", "custom_dropdown", 22, required=True,
           choices=_cc((1801, 61, "Yes"), (1802, 62, "No"))),
        _f("downtime_window", "Downtime Window", "custom_text", 23, placeholder="22:00 - 22:30"),
        _f("signoff_date", "Sign-off Date", "custom_dropdown", 24,
           choices=_cc((1901, 700, "N/A"), (1902, 701, "01/12/2030"), (1903, 702, "08/12/2030"))),
        _f("estimated_hours", "Estimated Hours", "custom_number", 25),
        _f("impact", "Impact", "default_impact", 26, required=True, hint=True,
           helptext="High - Customer facing   ||   \nMedium - Internal tools   ||   \nLow - Sandbox only",
           choices=[{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"}]),
        _f("priority", "Priority", "default_priority", 27, required=True,
           helptext="High - Revenue at risk || Medium - Planned work || Low - Nice to have",
           choices=[{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"},
                    {"id": 4, "value": "Urgent"}]),
        _f("risk", "Risk", "default_risk", 28, required=True,
           choices=[{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}, {"id": 3, "value": "High"},
                    {"id": 4, "value": "Very High"}]),
        _f("category", "Category", "default_category", 29,
           nested_fields=[{"id": 9101, "name": "sub_category", "label": "Sub-Category", "level": 2},
                          {"id": 9102, "name": "item_category", "label": "Item", "level": 3}],
           choices=[{"id": 2101, "display_id": 1, "value": "Hardware", "nested_options": [
                        {"id": 2102, "display_id": 2, "value": "Laptop", "nested_options": [
                            {"id": 2103, "display_id": 3, "value": "Battery", "nested_options": []}]},
                        {"id": 2104, "display_id": 4, "value": "Monitor", "nested_options": []}]},
                    {"id": 2105, "display_id": 5, "value": "Software", "nested_options": []}]),
        _f("status", "Status", "default_status", 30, required=True,
           choices=[{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}, {"id": 3, "value": "Approval"}]),
        _f("legacy_flag", "Legacy Flag", "custom_checkbox", 31),
        _f("change_window", "Maintenance Window", "default_change_window", 32),
        _f("rollback_doc", "Rollback Document", "default_backout_plan", None, helptext="Rollback steps"),
        _f("extra_notes", "Extra Notes", "planning_field", None),
    ]
    return {"change_fields": fields}


def _rule(rid: int, name: str, cond: Any, actions: list[dict[str, Any]], *, rev: bool = True, form: str = "both",
          user: str = "both", validation: bool = False, **kw: Any) -> dict[str, Any]:
    out = {
        "id": rid,
        "name": name,
        "workspace_id": 2,
        "module_type": kw.pop("module_type", "change"),
        "item_id": kw.pop("item_id", None),
        "deleted": kw.pop("deleted", False),
        "user_type": user,
        "form_type": form,
        "state": kw.pop("state", "active"),
        "rule_type": kw.pop("rule_type", "client_side"),
        "description": "",
        "condition_data": cond,
        "action_data": {"action": actions},
        "is_reversible": rev,
        "is_validation": validation,
    }
    out.update(kw)
    return out


def _c(name: str, op: str, value: Any = None) -> dict[str, Any]:
    out = {"name": name, "operator": op, "evaluate_on": "change"}
    if value is not None:
        out["value"] = value
    return out


def _a(prop: str, names: Any, values: Any = None) -> dict[str, Any]:
    out: dict[str, Any] = {"name": names, "property": prop}
    if values is not None:
        out["values"] = values
    return out


CF = "customFields."


def change_rules() -> dict[str, Any]:
    rules = [
        _rule(1, "Not labs: optional extras", {"all": [_c(CF + "team", "is_not", "Labs")]},
              [_a("unmandate", [CF + "cloud_only", CF + "stage", CF + "needs_review", CF + "ghost_field"])]),
        _rule(2, "Live stage", {"all": [_c(CF + "stage", "is", "Live")]},
              [_a("hide", [CF + "staging_start", CF + "staging_end"]),
               _a("unmandate", [CF + "staging_start", CF + "staging_end"])], rev=False),
        _rule(3, "Staging stage", {"all": [_c(CF + "stage", "is", "Staging")]},
              [_a("show", [CF + "staging_start", CF + "staging_end"]),
               _a("mandate", [CF + "staging_start", CF + "staging_end"])], rev=False),
        _rule(4, "Cloud only yes", {"all": [_c(CF + "cloud_only", "is", "Yes")]},
              [_a("populate", [CF + "stage"], [801]),
               _a("hide", [CF + "staging_start", CF + "staging_end", CF + "reviewers", CF + "rollout_tool"]),
               _a("unmandate", [CF + "staging_start", CF + "staging_end", CF + "rollout_tool"])]),
        _rule(5, "Cloud only no", {"all": [_c(CF + "cloud_only", "is", "No")]},
              [_a("populate", [CF + "stage"], [802]),
               _a("show", [CF + "staging_start", CF + "staging_end", CF + "reviewers", CF + "rollout_tool"]),
               _a("mandate", [CF + "staging_start", CF + "staging_end", CF + "reviewers", CF + "rollout_tool"])]),
        _rule(6, "Automated rollout", {"all": [_c(CF + "rollout_tool", "includes", ["Pipeline", "Script"])]},
              [_a("unmandate", [CF + "needs_review"])], rev=False),
        _rule(7, "Manual rollout", {"all": [_c(CF + "rollout_tool", "is", "Manual")]},
              [_a("mandate", [CF + "needs_review"])], rev=False),
        _rule(8, "Hide legacy flag", {"all": None}, [_a("hide", [CF + "legacy_flag"])]),
        _rule(9, "Normal needs sign-off", {"all": [_c("change_type", "is", "1")]},
              [_a("mandate", [CF + "signoff_date"]), _a("hide", [CF + "emergency_reason", "category"])]),
        _rule(10, "Emergency", {"all": [_c("change_type", "is", "4")]},
              [_a("mandate", [CF + "emergency_reason"]), _a("populate", [CF + "signoff_date"], [700]),
               _a("populate", [], [999])]),
        _rule(11, "No review: hide scope", {"all": [_c(CF + "needs_review", "not_selected")]},
              [_a("hide", [CF + "review_scope"])]),
        _rule(12, "Review: scope required", {"all": [_c(CF + "needs_review", "selected")]},
              [_a("mandate", [CF + "review_scope"])]),
        _rule(13, "Urgent priority", {"all": [_c("priority", "is", "4")]},
              [_a("message", ["Urgent changes need the duty manager's approval"])], validation=True),
        _rule(14, "Labs team", {"all": [_c(CF + "team", "is", "Labs")]},
              [_a("hide", [CF + "downtime"]), _a("disable", ["department_id"])], user="agent"),
        _rule(15, "Mandatory unless labs (new only)", {"all": [_c(CF + "team", "does_not_include", ["Labs"])]},
              [_a("mandate", [CF + "cloud_only", CF + "stage"])], form="new_form", user="agent"),
        _rule(16, "Downtime yes", {"all": [_c(CF + "downtime", "is", "Yes")]},
              [_a("mandate", [CF + "downtime_window"])], rev=False),
        _rule(17, "Downtime no", {"all": [_c(CF + "downtime", "is", "No")]},
              [_a("hide", [CF + "downtime_window"])], rev=False),
        _rule(18, "Payments heads-up",
              {"any": [{"all": [_c(CF + "team", "is", "Payments"), _c("priority", "is", "3")]},
                       _c("change_type", "is", "Emergency")]},
              [_a("message", ["Tell the payments on-call before you start"])], rev=False),
        _rule(19, "Edit: lock subject", {"all": None}, [_a("disable", ["subject"])], form="edit_form"),
        _rule(20, "Requester portal only", {"all": None}, [_a("hide", ["subject"])], user="requester"),
        _rule(21, "Inactive", {"all": None}, [_a("hide", ["subject"])], state="inactive"),
        _rule(22, "Server side", {"all": None}, [_a("hide", ["subject"])], rule_type="server_side"),
        _rule(23, "Deleted", {"all": None}, [_a("hide", ["subject"])], deleted=True),
        _rule(24, "Other module", {"all": None}, [_a("hide", ["subject"])], module_type="ticket"),
    ]
    # An exact duplicate entry (same id, same content) as the API sometimes returns.
    rules.insert(9, copy.deepcopy(rules[8]))
    return {"business_rules": rules, "user": {"first_name": "Test", "last_name": "Agent", "time_zone": "Bangkok"}}


def service_item() -> dict[str, Any]:
    parent_id = "f-goal"
    kid_a, kid_b = "f-goal-rev", "f-goal-cost"
    return {
        "service_item": {
            "id": 4400001,
            "display_id": 77,
            "name": "Feature Request",
            "short_description": "Ask for a product change",
            "category_name": "Engineering",
            "description": "<p>Use this form for feature work.</p>",
            "allow_attachments": True,
            "allow_quantity": False,
            "attachment_mandatory": True,
            "configs": {"attachment_mandatory": True, "subject": "{{custom_field.cf_title_100}}",
                        "auto_gen_document": False},
            "custom_fields": [
                {"id": "f-notice", "name": "notice", "type": 1024, "position": 1, "required": False,
                 "visible": True, "label": "&lt;p&gt;Attach the &lt;b&gt;overview deck&lt;/b&gt;.&lt;/p&gt;",
                 "field_options": {"content_name": "Notice"}, "choices": [], "fields": []},
                {"id": "f-title", "name": "title_100", "type": 1001, "position": 2, "required": True,
                 "visible": True, "label": "Title", "placeholder": "Short name", "field_options": {},
                 "choices": [], "fields": []},
                {"id": "f-details", "name": "paragraph_101", "type": 1008, "position": 3, "required": True,
                 "visible": True, "label": "Details", "field_options": {}, "choices": [], "fields": []},
                {"id": "f-sys", "name": "paragraph_102", "type": 1008, "position": 4, "required": False,
                 "visible": True, "label": "Affected system (if known)", "field_options": {}, "choices": [],
                 "fields": []},
                {"id": "f-date", "name": "date_103", "type": 1007, "position": 5, "required": True,
                 "visible": True, "label": "Needed by", "field_options": {"date_only": "true"}, "choices": [],
                 "fields": []},
                {"id": parent_id, "name": "goal", "type": 1003, "position": 6, "required": True, "visible": True,
                 "label": "1. Expected benefit",
                 "field_options": {"section_info": json.dumps([
                     {"sectionFields": [kid_a], "sectionChoices": ["More revenue  (Total USD)"]},
                     {"sectionFields": [kid_b], "sectionChoices": ["Lower cost"]}])},
                 "choices": [
                     {"id": "c-rev", "value": "More revenue  (Total USD)", "label": "More revenue  (Total USD)",
                      "position": 1, "dependent_ids": {"field": [kid_a]}},
                     {"id": "c-cost", "value": "Lower cost", "label": "Lower cost", "position": 2,
                      "dependent_ids": {"field": [kid_b]}},
                     {"id": "c-none", "value": "Other", "label": "Other", "position": 3, "dependent_ids": {}}],
                 "fields": [
                     {"id": kid_a, "name": "goal_1_kpi", "type": 1001, "position": 1, "required": True,
                      "visible": True, "parent_id": parent_id, "label": "1.1 Revenue target",
                      "field_options": {}, "choices": [], "fields": []},
                     {"id": kid_b, "name": "goal_2_kpi", "type": 1001, "position": 2, "required": True,
                      "visible": True, "parent_id": parent_id, "label": "1.2 Cost target",
                      "field_options": {}, "choices": [], "fields": []}]},
                {"id": "f-prio", "name": "priority", "type": 1003, "position": 7, "required": True,
                 "visible": True, "label": "Priority", "field_options": {},
                 "choices": [
                     {"id": "p1", "value": "Critical - blocks customers", "label": "Critical - blocks customers",
                      "position": 1},
                     {"id": "p4", "value": "Low - cosmetic, reports", "label": "Low - cosmetic, reports",
                      "position": 2}],
                 "fields": []},
                {"id": "f-deck", "name": "overview_deck", "type": 1014, "position": 8, "required": True,
                 "visible": True, "label": "Overview deck (.pptx)",
                 "placeholder": "&lt;span class=\"btn-link\"&gt;Upload files&lt;/span&gt; or drop here",
                 "field_options": {"allowed_file_types": "[\".pptx\"]", "max_attachment_limit": "1"},
                 "choices": [], "fields": []},
                {"id": "f-gone", "name": "old_field", "type": 1001, "position": 9, "required": True,
                 "deleted": True, "visible": True, "label": "Old", "field_options": {}, "choices": [],
                 "fields": []},
            ],
        }
    }


def ticket_fields() -> dict[str, Any]:
    """v2-shaped ticket fields with a dynamic section and a nested custom field."""
    return {
        "ticket_fields": [
            {"id": 301, "name": "requester", "label": "Requester", "field_type": "default_requester",
             "position": 1, "required_for_agents": True, "default_field": True, "choices": [],
             "nested_fields": [], "sections": [], "belongs_to_section": False},
            {"id": 302, "name": "subject", "label": "Subject", "field_type": "default_subject", "position": 2,
             "required_for_agents": True, "default_field": True, "choices": [], "nested_fields": [],
             "sections": [], "belongs_to_section": False},
            {"id": 303, "name": "issue", "label": "Issue", "field_type": "nested_field", "position": 3,
             "required_for_agents": False, "default_field": False,
             "choices": [{"id": 1, "value": "Login", "nested_options": [
                 {"id": 2, "value": "VPN", "nested_options": [{"id": 3, "value": "Timeout", "nested_options": []}]}]},
                 {"id": 4, "value": "Printer", "nested_options": []}],
             "nested_fields": [{"name": "issue_system", "id": 3031, "label": "System", "level": 2},
                               {"name": "issue_detail", "id": 3032, "label": "Detail", "level": 3}],
             "sections": [], "belongs_to_section": False},
            {"id": 304, "name": "ticket_type", "label": "Type", "field_type": "default_ticket_type",
             "position": 4, "required_for_agents": False, "default_field": True,
             "choices": [{"id": 10, "value": "Incident"}, {"id": 11, "value": "Major Incident"}],
             "nested_fields": [], "belongs_to_section": False,
             "sections": [{"id": 1, "label": "Major", "choice_ids": [11], "ticket_field_ids": [309]}]},
            {"id": 305, "name": "priority", "label": "Priority", "field_type": "default_priority", "position": 5,
             "required_for_agents": False, "default_field": True,
             "choices": [{"id": 1, "value": "Low"}, {"id": 2, "value": "Medium"}], "nested_fields": [],
             "sections": [], "belongs_to_section": False},
            {"id": 306, "name": "status", "label": "Status", "field_type": "default_status", "position": 6,
             "required_for_agents": True, "default_field": True,
             "choices": [{"id": 2, "value": "Open"}, {"id": 3, "value": "Pending"}], "nested_fields": [],
             "sections": [], "belongs_to_section": False},
            {"id": 307, "name": "source", "label": "Source", "field_type": "default_source", "position": 7,
             "required_for_agents": False, "default_field": True,
             "choices": [{"id": 3, "value": "Phone"}, {"id": 1, "value": "Email"}], "nested_fields": [],
             "sections": [], "belongs_to_section": False},
            {"id": 308, "name": "group", "label": "Group", "field_type": "default_group", "position": 8,
             "required_for_agents": False, "default_field": True, "choices": [], "nested_fields": [],
             "sections": [], "belongs_to_section": False},
            {"id": 309, "name": "customers_hit", "label": "Customers affected", "field_type": "custom_number",
             "position": 9, "required_for_agents": True, "default_field": False, "choices": [],
             "nested_fields": [], "sections": [], "belongs_to_section": True},
            {"id": 310, "name": "due_on", "label": "Due on", "field_type": "custom_date", "date_only": True,
             "position": 10, "required_for_agents": False, "default_field": False, "choices": [],
             "nested_fields": [], "sections": [], "belongs_to_section": False},
            {"id": 311, "name": "description", "label": "Description", "field_type": "default_description",
             "position": 11, "required_for_agents": False, "default_field": True, "choices": [],
             "nested_fields": [], "sections": [], "belongs_to_section": False},
            {"id": 312, "name": "department", "label": "Department", "field_type": "default_department",
             "position": 12, "required_for_agents": False, "default_field": True,
             "choices": [{"id": 501, "value": "Engineering"}], "nested_fields": [], "sections": [],
             "belongs_to_section": False},
            {"id": 313, "name": "mystery", "label": "Mystery", "field_type": "custom_hologram", "position": 13,
             "required_for_agents": False, "default_field": False, "choices": [], "nested_fields": [],
             "sections": [], "belongs_to_section": False},
        ]
    }


# ---------------------------------------------------------------------------
# A small "Acme ops" form exercising one rule semantic per rule.
# Ids come from an obviously fake range (900000xxxx); the account id is
# 900000777 (custom fields are known to the server as <name>_900000777).

ACCOUNT = "900000777"
GOLD_DID = 9000003001  # Gold's display id == Bronze's choice id (display id must win)


def _ch(i: int, did: int, value: str) -> dict[str, Any]:
    return {"id": i, "display_id": did, "value": value, "label": value}


def ops_fields() -> dict[str, Any]:
    fid = itertools.count(9000001001)
    F = lambda *a, **k: _f(*a, id=next(fid), **k)  # noqa: E731
    return {"change_fields": [
        F("tier", "Tier", "custom_dropdown", 1,
          choices=[_ch(9000002001, GOLD_DID, "Gold"), _ch(9000002002, 9000003002, "Silver"),
                   _ch(GOLD_DID, 9000003003, "Bronze")]),
        F("region", "Region", "custom_dropdown", 2, required=True,
          choices=[_ch(9000002011, 9000003011, "EU"), _ch(9000002012, 9000003012, "US")]),
        F("notes", "Notes", "custom_text", 3),
        F("region_notes", "Region Notes", "custom_text", 4),
        F("tags", "Tags", "custom_multi_select_dropdown", 5,
          choices=[_ch(9000002021, 9000003021, "A"), _ch(9000002022, 9000003022, "B"),
                   _ch(9000002023, 9000003023, "C")]),
        F("vip", "VIP", "custom_checkbox", 6),
        F("code", "Code", "custom_text", 7, required=True),
        F("owner", "Owner", "custom_lookup", 8, data_source=5,
          lookup_config={"data_source": 5, "link": f"/lookup_choices?field_name=owner_{ACCOUNT}&module=change"}),
        F("meeting", "Review Meeting", "custom_dropdown", 9,
          choices=[_ch(9000002031, 9000003031, "N/A"), _ch(9000002032, 9000003032, "01/12/2030"),
                   _ch(9000002033, 9000003033, "08/12/2030")]),
        F("when", "When", "custom_date", 10),
        F("area", "Area", "nested_field", 11,
          nested_fields=[{"id": 9000001901, "name": "area_team", "label": "Team", "level": 2},
                         {"id": 9000001902, "name": "area_unit", "label": "Unit", "level": 3}],
          choices=[{"id": 9000002041, "value": "Ops", "nested_options": [
                       {"id": 9000002042, "value": "Infra", "nested_options": [
                           {"id": 9000002043, "value": "Racks", "nested_options": []}]},
                       {"id": 9000002044, "value": "Other", "nested_options": []}]},
                   {"id": 9000002045, "value": "Dev", "nested_options": [
                       {"id": 9000002046, "value": "Other", "nested_options": [
                           {"id": 9000002047, "value": "Misc", "nested_options": []}]}]}]),
        F("priority", "Priority", "default_priority", 12, required=True,
          choices=[{"id": 1, "value": "Low"}, {"id": 4, "value": "Urgent"}]),
        F("subject", "Subject", "default_subject", 13, required=True),
    ]}


def ops_rule(rid: int, name: str, cond: Any, actions: list[dict[str, Any]], **kw: Any) -> dict[str, Any]:
    kw.setdefault("rev", False)
    return _rule(rid, name, cond, actions, **kw)


def ops_rules() -> dict[str, Any]:
    rules = [
        ops_rule(9000004001, "Gold tier", {"all": [_c(CF + "tier", "is", "Gold")]},
                 [_a("populate", [CF + "region"], [GOLD_DID + 10]),  # display id of EU
                  _a("show", [CF + "region_notes"])]),
        ops_rule(9000004002, "EU region", {"all": [_c(CF + "region", "is", "EU")]}, [_a("hide", [CF + "notes"])]),
        ops_rule(9000004003, "Always hide region notes", {"all": None}, [_a("hide", [CF + "region_notes"])]),
        ops_rule(9000004004, "Tags A or B", {"all": [_c(CF + "tags", "includes", ["A", "B"])]},
                 [_a("mandate", [CF + "notes"])]),
        ops_rule(9000004005, "Tier not Silver", {"all": [_c(CF + "tier", "does_not_include", ["Silver"])]},
                 [_a("unmandate", [CF + "code"])], rev=True),
        ops_rule(9000004006, "VIP", {"all": [_c(CF + "vip", "selected")]},
                 [_a("populate", [CF + "meeting"], [9000003031]), _a("disable", [CF + "meeting"])]),
        ops_rule(9000004007, "Urgent", {"all": [_c("priority", "is", "4")]},
                 [_a("message", ["Call the duty lead first"])], validation=True),
        ops_rule(9000004008, "Empty lists", {"any": None},
                 [_a("hide", []), _a("mandate", []), _a("populate", [], [123])]),
        ops_rule(9000004009, "Edit: lock subject", {"all": None}, [_a("disable", ["subject"])], form="edit_form"),
        ops_rule(9000004010, "Other workspace", {"all": None}, [_a("hide", ["subject"])], workspace_id=3),
        ops_rule(9000004011, "Other item", {"all": None}, [_a("hide", ["subject"])], item_id=9000009999),
        ops_rule(9000004012, "Portal only", {"all": None}, [_a("hide", ["subject"])], user="requester"),
        ops_rule(9000004013, "Ops staff",
                 {"all": [{"name": "department_ids", "value": ["9000005001"], "operator": "includes",
                           "field_type": "multiselect_choicelist", "evaluate_on": "current_user"}]},
                 [_a("hide", [CF + "when"])]),
        ops_rule(9000004014, "Silver shows notes", {"all": [_c(CF + "tier", "is", "Silver")]},
                 [_a("show", [CF + "notes"])]),
        ops_rule(9000004015, "Infra team needs unit", {"all": [_c(CF + "area_team", "is", "Infra")]},
                 [_a("mandate", [CF + "area_unit"])]),
        ops_rule(9000004016, "Ghost", {"all": None}, [_a("hide", [CF + "removed_field"])]),
    ]
    user = {"first_name": "Robin", "last_name": "Ops", "email": "robin.ops@example.com",
            "department_ids": [9000005002], "time_zone": "Tokyo"}
    return {"business_rules": rules, "user": user}
