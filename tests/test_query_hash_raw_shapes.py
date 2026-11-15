"""Raw ``--query-hash`` values: scalar and list shapes the list UI really sends.

A scalar ``value`` with ``is_in`` once crashed with a traceback while merging
same-condition items; it must be a clean refusal (exit 2, nothing sent), and
the shapes the basic filter panel sends (list ids, scalar presets, ranges)
must pass through unchanged.
"""
import json

import pytest
from typer.testing import CliRunner

from fsv import cli
from test_views_cli import _world

PANEL_SHAPES = [
    # status list + created preset (minutes as a string scalar)
    [{"value": ["2"], "condition": "status", "operator": "is_in", "type": "default"},
     {"value": "240", "condition": "created_at", "operator": "is_greater_than", "type": "default"}],
    # inclusive custom range object
    [{"value": {"from": "2030-01-01T00:00:00-04:00", "to": "2030-02-01T00:00:00-05:00"},
      "condition": "created_at", "operator": "is_greater_than", "type": "default"}],
    # unassigned agent checkbox (string ids in a list)
    [{"value": ["-1"], "condition": "responder_id", "operator": "is_in", "type": "default"}],
    # integer list value
    [{"condition": "status", "operator": "is_in", "type": "default", "value": [2]}],
]


@pytest.mark.parametrize("module,path", [("tickets", "/api/_/tickets"), ("changes", "/api/_/changes")])
@pytest.mark.parametrize("shape", PANEL_SHAPES, ids=["status+preset", "range", "agent", "int-list"])
def test_panel_shapes_pass_through_unchanged(monkeypatch, module, path, shape):
    world = _world(monkeypatch, "acme")
    r = CliRunner().invoke(cli.app, [module, "ls", "--json", "--query-hash", json.dumps(shape)])
    assert r.exit_code == 0, r.output
    assert "Traceback" not in r.output
    assert json.loads(world.calls(path)[-1]["query_hash"]) == shape
    assert not world.writes()


@pytest.mark.parametrize("module,path", [("tickets", "/api/_/tickets"), ("changes", "/api/_/changes")])
@pytest.mark.parametrize("value", [2, "2", {"id": 2}])
def test_scalar_value_with_list_operator_is_a_clean_refusal(monkeypatch, module, path, value):
    world = _world(monkeypatch, "acme")
    raw = json.dumps([{"condition": "status", "operator": "is_in", "type": "default", "value": value}])
    r = CliRunner().invoke(cli.app, [module, "ls", "--json", "--query-hash", raw])
    assert r.exit_code == 2, r.output
    assert "Traceback" not in r.output and "TypeError" not in r.output
    assert "takes a JSON list value" in r.output and "nothing was sent" in r.output
    assert not world.calls(path) and not world.writes()


def test_repeated_condition_merges_lists_and_keeps_scalars(monkeypatch):
    world = _world(monkeypatch, "acme")
    raw = json.dumps([
        {"condition": "status", "operator": "is_in", "type": "default", "value": [2]},
        {"condition": "status", "operator": "is_in", "type": "default", "value": [3]},
        {"condition": "known_error", "operator": "is", "type": "default", "value": "1"},
    ])
    r = CliRunner().invoke(cli.app, ["tickets", "ls", "--json", "--query-hash", raw])
    assert r.exit_code == 0, r.output
    sent = json.loads(world.calls("/api/_/tickets")[-1]["query_hash"])
    assert {"condition": "known_error", "operator": "is", "type": "default", "value": "1"} in sent
    status = [i for i in sent if i["condition"] == "status"]
    assert len(status) == 1 and sorted(status[0]["value"]) == [2, 3]
