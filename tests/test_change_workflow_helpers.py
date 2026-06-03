from __future__ import annotations

import pytest

from fsv import create


class WorkflowClient:
    def __init__(self):
        self.puts = []
        self.asset_response = {"cis": [{"id": 9}], "services": [], "softwares": []}

    def int_get(self, path, params=None):
        if path == "changes/1/associated-cis":
            return {"data": self.asset_response[params["association_type"]]}
        if path == "assets-to-associate":
            return {"assets": [{"display_id": 123, "name": "EDP"}], "meta": {"total_count": 1}, "params": params}
        if path == "changes/tickets/search":
            return {"tickets": [{"id": 565163, "human_display_id": "SR-565163"}], "params": params}
        if path == "changes/1/activities":
            return {"activities": [{"content": "created", "created_at": "2026-05-26T18:14:00Z"}]}
        raise AssertionError(path)

    def int_put(self, path, body=None):
        self.puts.append(("PUT", path, body))
        return {}

    def int_delete(self, path):
        self.puts.append(("DELETE", path, {}))
        return {}


def test_assets_resource_rejects_multiple_actions():
    import fsv.cli as cli

    with pytest.raises(cli.typer.Exit):
        cli.assets_resource("CHN-1", "edp", [123], [], 1, 30, True, False, False)


def test_change_assets_list_search_and_associate():
    c = WorkflowClient()

    assert create.get_change_assets(1, c=c) == [{"id": 9}]
    search = create.search_assets_for_change(1, "edp", c=c)
    assert search["assets"] == [{"display_id": 123, "name": "EDP"}]
    assert search["params"]["search_term"] == "edp"
    assert search["params"]["entity"] == "change"
    assert search["params"]["entity_id"] == 1

    create.associate_assets(1, [123], c=c)
    assert c.puts == [("PUT", "changes/1/assets/associate", {"item_ids": [123]})]


def test_delete_task():
    c = WorkflowClient()

    create.delete_task(1, 172149, c=c)

    assert c.puts == [("DELETE", "changes/1/tasks/172149", {})]


def test_dissociate_assets():
    c = WorkflowClient()
    c.asset_response = {
        "cis": [
            {"id": 16001574706, "config_item": {"display_id": 38679, "name": "OOS", "ci_type_name": "Application Portfolio"}},
            {"id": 16001572545, "config_item": {"display_id": 27914, "name": "EDP", "ci_type_name": "Application Portfolio"}},
        ],
        "services": [],
        "softwares": [],
    }

    create.dissociate_assets(1, [38679], c=c)

    assert c.puts == [("PUT", "changes/1/assets/detach", {"cmdb_request_id": 16001574706})]


def test_ticket_search_and_activities():
    c = WorkflowClient()

    tickets = create.search_change_tickets("sr-565163", c=c)
    assert tickets == [{"id": 565163, "human_display_id": "SR-565163"}]

    acts = create.get_change_activities(1, c=c)
    assert acts == [{"content": "created", "created_at": "2026-05-26T18:14:00Z"}]


class CloneClient:
    def int_get(self, path, params=None):
        assert path == "changes/1"
        return {
            "change": {
                "id": 1,
                "status": 2,
                "subject": "demo",
                "associated_cis_and_services_ids": {"x": 1},
            }
        }


def test_change_clone_data_strips_internal_associations():
    data = create.change_clone_data(1, c=CloneClient())

    assert data["status"] == 1
    assert "associated_cis_and_services_ids" not in data


class TaskUpdateClient:
    def __init__(self):
        self.puts = []

    def int_get(self, path, params=None):
        assert path == "changes/1/tasks/1"
        return {
            "task": {
                "id": 1,
                "status": 1,
                "title": "FSV task 1",
                "custom_fields": {},
                "human_display_id": "TSK-1",
            }
        }

    def int_put(self, path, body=None):
        self.puts.append((path, body))
        return {"task": body["change_task"]}


def test_update_task_does_not_hardcode_required_custom_fields():
    c = TaskUpdateClient()

    task = create.update_task(1, 1, {"status": 2}, c=c)

    assert task["status"] == 2
    assert c.puts == [
        ("changes/1/tasks/1", {"change_task": {"status": 2, "title": "FSV task 1"}})
    ]
