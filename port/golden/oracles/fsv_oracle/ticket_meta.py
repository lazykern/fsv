"""Ticket edit-form fixture shared by the conversation/properties oracles."""


def f(id_, name, label, field_type, position, **kw):
    default = field_type.startswith("default_")
    out = {"id": id_, "name": name, "label": label, "description": "", "field_type": field_type,
           "required": False, "required_for_closure": False, "position": position,
           "default_field": default, "choices": [], "nested_fields": [],
           "placeholder": "", "helptext": "", "hint": False,
           "content": None, "displayed_to_agents": True, "data_source": 0,
           "lookup_config": None, "workspace_id": 1 if default else 2,
           "updated_at": "2030-01-02T03:04:05Z"}
    out.update(kw)
    return out


def change_fields():
    return {"change_fields": [
        f(11, "requester", "Requester", "default_requester", 1),
        f(12, "group", "Group", "default_group", 2),
        f(13, "agent", "Agent", "default_agent", 3),
        f(14, "status", "Status", "default_status", 4, choices=[{"id": 1, "value": "Open"}, {"id": 2, "value": "Planning"}]),
        f(15, "description", "Description", "default_description", 5),
        f(16, "cf_note", "Dup", "custom_text", 6),
        f(17, "cf_other", "Dup", "custom_text", 7),
        f(18, "cf_env", "Environment", "custom_dropdown", 8, choices=[{"id": 1, "value": "Prod"}, {"id": 2, "value": "Test"}]),
        f(19, "subject", "Subject", "default_subject", 9, required=True),
    ]}


def ticket_fields(status_id=3):
    return {"ticket_fields": [
        f(1, "subject", "Subject", "default_subject", 1),
        f(2, "status", "Status", "default_status", 2, choices=[{"id": 2, "value": "Open"}, {"id": status_id, "value": "Pending"}]),
        f(3, "planned_start_date", "Start", "default_planned_start_date", 3),
        f(4, "planned_end_date", "End", "default_planned_end_date", 4),
    ]}
