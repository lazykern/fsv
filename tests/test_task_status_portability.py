"""Independent synthetic tenants: IDs and labels are tenant data."""
from types import SimpleNamespace
import pytest
from fsv import task_status, change_tasks, cli

@pytest.mark.parametrize("labels,ids", [
    (["Open", "In Progress", "Completed", "Blocked"], [71, 82, 93, 104]),
    (["Neu", "In Arbeit", "Fertig", "Gesperrt"], [11, 22, 33, 44]),
])
def test_runtime_resolution(monkeypatch, labels, ids):
    fields = [{"name": "status", "choices": [{"id": i, "value": v} for i, v in zip(ids, labels)]}]
    calls = []
    c = SimpleNamespace(int_get=lambda *a: calls.append(a) or {"change_task_fields": fields})
    monkeypatch.setattr(cli, "_task_field_defs", lambda: fields)
    for label, ident in zip(labels, ids):
        assert change_tasks.resolve_status(label, c) == ident
        assert cli._task_status_value(label) == ident
        assert cli._task_status_value(str(ident)) == ident
        assert task_status.display(ident, fields) == label
        assert change_tasks.build_plan(123, "Acme review", status=label, status_id=ident).payload["status"] == ident
    assert len(calls) == len(labels)
    with pytest.raises(ValueError):
        cli._task_status_value("done")
    with pytest.raises(ValueError):
        cli._task_status_value("999")

@pytest.mark.parametrize("choices", [[], [{"id": 1, "value": "Open", "deleted": True}],
    [{"id": 1, "value": "Open"}, {"id": 2, "value": "open"}],
    [{"id": 1, "value": "Open"}, {"id": 1, "value": "Other"}]])
def test_bad_choices_fail_closed(choices):
    with pytest.raises(ValueError):
        task_status.resolve("Open", [{"name": "status", "choices": choices}])

def test_no_default_id_inference():
    with pytest.raises(ValueError, match="runtime metadata"):
        change_tasks.build_plan(123, "Acme review")
    assert task_status.display(3) == "3 (unknown)"
