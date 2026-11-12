"""Synthetic default-tenant change-task metadata (status + reminder choices)."""


def default_task_fields() -> list[dict]:
    return [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "In Progress"},
                                           {"id": 3, "value": "Completed"}, {"id": 4, "value": "Blocked"}]},
            {"name": "notify_before", "choices": [{"id": 0, "value": "Never"}, {"id": 900, "value": "15 Minutes"},
                                                  {"id": 1800, "value": "30 Minutes"}, {"id": 2700, "value": "45 Minutes"},
                                                  {"id": 3600, "value": "1 Hour"}, {"id": 7200, "value": "2 Hours"}]}]
