import copy
import json
from types import SimpleNamespace
import pytest
from fsv import config, solutions as a
from fsv.reviewed_write import ReviewedWriteError


@pytest.fixture
def wire(monkeypatch, tmp_path):
    monkeypatch.setattr(config, "DOMAIN", "acme.example.com")
    monkeypatch.setenv("FSV_CACHE_DIR", str(tmp_path))
    state = {"articles": {}, "linked": [], "writes": [], "persist": True, "partial": False}
    def get(path, params=None):
        if path == "solutions/categories":
            return {"categories": [{"id": 71, "name": "Acme", "default_category": True, "workspace_id": 1}], "meta": {"count": 1, "page": None, "per_page": "300"}}
        if path == "solutions/folders":
            assert params["category_id"] == 71
            return {"folders": [{"id": 81, "name": "Drafts", "category_id": 71, "default_folder": True, "workspace_id": 1}], "meta": {"count": 1, "page": None, "per_page": "300"}}
        if path.startswith("solutions/articles/"):
            return {"article": copy.deepcopy(state["articles"][int(path.split("/")[-1])])}
        if path.endswith("/solutions"):
            return {"articles": copy.deepcopy(state["linked"]), "has_more": state["partial"]}
        return {"problem": {"id": 23, "subject": "Acme problem", "workspace_id": 2}}
    def request(method, path, **kw):
        body = json.loads(kw["content"])
        state["writes"].append((method, path, body))
        if method == "POST":
            row = {"id": 91, **body}
            if state["persist"]:
                state["articles"][91] = {**row, "description": "<div style='font:inherit'>" + body["description"] + "</div>"}
            return {"article": row}
        if state["persist"]:
            if body["type"] == "link":
                state["linked"].append(copy.deepcopy(state["articles"][91]))
            else:
                state["linked"].clear()
        return {}
    return SimpleNamespace(int_get=get, _request=request, _get_csrf=lambda: "synthetic", state=state)


@pytest.mark.parametrize("kind,type_id", [("workaround", 2), ("permanent", 1)])
def test_create_then_link_then_unlink(wire, kind, type_id):
    plan = a.create_plan(wire, "Acme article", "Synthetic guidance", kind)
    assert plan.payload["article_type"] == type_id and plan.payload["status"] == 1
    assert type(plan.payload["folder_id"]) is int
    assert plan.payload["folder_id"] == 81
    assert a.send_create(plan, wire, reviewed_hash=plan.hash)["verified"]
    link = a.link_plan(wire, "PRB-23", 91)
    assert link.payload == {"type": "link"}
    assert a.send_link(link, wire, reviewed_hash=link.hash)["verified"]
    unlink = a.link_plan(wire, 23, 91, unlink=True)
    assert a.send_link(unlink, wire, reviewed_hash=unlink.hash)["verified"]
    assert len(wire.state["writes"]) == 3


def test_echo_not_readback_and_replay_guard(wire):
    plan = a.create_plan(wire, "Acme article", "Guidance", "workaround")
    wire.state["persist"] = False
    assert not a.send_create(plan, wire, reviewed_hash=plan.hash)["verified"]
    with pytest.raises(ReviewedWriteError, match="outcome unknown"):
        a.send_create(plan, wire, reviewed_hash=plan.hash)
    assert len(wire.state["writes"]) == 1


def test_invalid_folder_and_type_fail_closed(wire):
    with pytest.raises(ReviewedWriteError, match="uniquely"):
        a.create_plan(wire, "Acme", "Guidance", "permanent", selected_folder="unknown")
    with pytest.raises(ValueError):
        a.create_plan(wire, "Acme", "Guidance", "publish")
    assert not wire.state["writes"]
