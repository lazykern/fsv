"""Accurate refusal wording for status labels, inventory pages and --set labels."""
from types import SimpleNamespace

import pytest

from fsv import config, integrated_assets, task_status, ticket_properties as props
from fsv.reviewed_write import ReviewedWriteError
from test_ticket_properties import Client, rich_metadata

FIELDS = [{"name": "status", "choices": [{"id": 1, "value": "Open"}, {"id": 2, "value": "Change Requested"},
                                         {"id": 3, "value": "Closed"}]}]


def test_problem_status_label_error_names_problem_status_and_options():
    with pytest.raises(ValueError) as exc:
        task_status.resolve("Resolved", FIELDS, kind="problem")
    text = str(exc.value)
    assert "problem-status" in text and "task-status" not in text and "Open, Change Requested, Closed" in text
    with pytest.raises(ValueError, match="task-status ID or label"):
        task_status.resolve("Resolved", FIELDS)  # task callers keep their wording
    assert task_status.resolve("closed", FIELDS, kind="problem") == (3, "Closed")


def _inventory(answer):
    calls = []

    def request(method, url, params=None, **_):
        assert method == "GET"
        calls.append(params)
        return answer(params["page"])
    return SimpleNamespace(_request=request, calls=calls)


def test_out_of_range_inventory_page_says_no_results(monkeypatch):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    native = lambda page: ({"page": 1, "error": None, "total_rows": 0, "data": []} if page == 1 else
                           {"page": 1, "error": "Requested page is out of range.", "total_rows": 0, "data": []})
    c = _inventory(native)
    assert integrated_assets.search(c, page=1)["complete"] is True
    with pytest.raises(ValueError, match="no results on page 2: the devices inventory has 0 row"):
        integrated_assets.search(c, page=2)
    # An error that is not explained by the totals stays "unavailable".
    broken = _inventory(lambda page: {"page": page, "error": "boom", "total_rows": 500, "data": []})
    with pytest.raises(ReviewedWriteError, match="integrated inventory unavailable"):
        integrated_assets.search(broken, page=2)
    with pytest.raises(ReviewedWriteError, match="integrated inventory unavailable"):
        integrated_assets.search(_inventory(lambda page: {"page": 1, "error": "x", "total_rows": 0, "data": []}),
                                 page=1)


def test_partial_set_label_names_the_full_labels(monkeypatch):
    monkeypatch.setattr(props, "load_meta", lambda *args, **kwargs: rich_metadata())
    c = Client()
    with pytest.raises(props.PropertyError) as exc:
        props.build_plan(c, "SR-123", answers={"Cost": "new"})
    text = str(exc.value)
    assert "did not match a ticket field label exactly" in text and "Cost Code" in text
    assert "could not evaluate" not in text and not c.writes
    with pytest.raises(props.PropertyError, match="tickets fields"):
        props.build_plan(c, "SR-123", answers={"Globex nothing": "x"})
