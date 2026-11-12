"""Populated lifecycle rendering and optional endpoint failures, entirely offline."""
from io import StringIO
from types import SimpleNamespace

import pytest
from rich.console import Console

from fsv import state_flow
from fsv.errors import APIError


@pytest.mark.parametrize("workspace,wrapped", [(None, True), ("", False), (17, True)])
def test_flow_uses_change_workspace_and_unwraps_response(workspace, wrapped, monkeypatch):
    calls = []
    flow = {"name": "Acme lifecycle", "state_list": []}
    def get(path, params=None):
        calls.append((path, params))
        return {"state_flow": flow} if wrapped else flow
    c = SimpleNamespace(int_get=get)
    monkeypatch.setattr(state_flow, "get_client", lambda: c)
    assert state_flow.get_state_flow(51, workspace_id=workspace) == flow
    assert calls == [("ws/17/change_management/51" if workspace else "change_management/51", {"include": "state_info"})]


@pytest.mark.parametrize("status", [403, 404, 429, 500])
def test_only_workspace_404_can_fall_back_to_plain_flow(status):
    calls = []
    def get(path, params=None):
        calls.append(path)
        if path.startswith("ws/"):
            raise APIError(status, "synthetic denial")
        return {"state_flow": {"name": "Acme fallback"}}
    c = SimpleNamespace(int_get=get)
    if status == 404:
        assert state_flow.get_state_flow(51, c, workspace_id=17) == {"name": "Acme fallback"}
        assert calls == ["ws/17/change_management/51", "change_management/51"]
    else:
        with pytest.raises(APIError) as exc:
            state_flow.get_state_flow(51, c, workspace_id=17)
        assert exc.value.status == status and calls == ["ws/17/change_management/51"]


@pytest.fixture
def flow_env(monkeypatch):
    flow = {"name": "Acme release", "state_list": [{"id": i, "value": f"Phase {i}"} for i in range(1, 8)]}
    calls = []
    approvals = [{"level_id": 1, "status": {"name": "approved"}, "member": {"name": "Alex Acme"}},
                 {"level_id": 2, "status": {"name": "rejected"}, "member_id": 72},
                 {"level_id": 3, "member_id": 73}]
    def get(path, params=None):
        calls.append((path, params))
        if path == "ws/17/change_management/51":
            return {"state_flow": flow}
        assert path == "changes/123/approvals"
        return {"approvals": approvals}
    monkeypatch.setattr(state_flow, "get_client", lambda: SimpleNamespace(int_get=get))
    change = {"id": 123, "human_display_id": "CHN-123", "state_flow_id": 51, "workspace_id": 17,
              "status": 4, "status_name": "Phase 4", "state_traversal": [1, 2, 3], "change_type": 2}
    return flow, change, calls


def capture(renderer, change, **kwargs):
    stream = StringIO()
    renderer(change, Console(file=stream, width=180, color_system=None), **kwargs)
    return stream.getvalue()


def test_full_flow_displays_current_visited_future_and_approval_statuses(flow_env):
    _, change, calls = flow_env
    out = capture(state_flow.render_flow, change)
    assert "CHN-123 flow" in out and "Acme release" in out
    assert "passed" in out and "current" in out and "future" in out
    assert "past: 3" in out and "ahead: 3" in out
    assert "✓ L1: Alex Acme — approved" in out
    assert "✗ L2: 72 — rejected" in out and "◷ L3: 73 — ?" in out
    assert calls == [("ws/17/change_management/51", {"include": "state_info"}), ("changes/123/approvals", None)]


@pytest.mark.parametrize("renderer", [state_flow.render_compact, state_flow.render_state_diagram])
@pytest.mark.parametrize("status,context", [(1, 1), (4, 1), (7, 1), (4, 10)])
def test_context_windows_clip_without_losing_current_state(flow_env, renderer, status, context):
    _, change, calls = flow_env
    change["status"] = status
    out = capture(renderer, change, context=context)
    lo, hi = max(1, status-context), min(7, status+context)
    for i in range(1, 8):
        assert (f"Phase {i}" in out) == (lo <= i <= hi)
    assert f"▶ Phase {status}" in out
    assert ("…" in out) == (lo > 1 or hi < 7)
    if renderer == state_flow.render_state_diagram:
        assert "43% complete" in out and "(3/7)" in out
        assert ("for full flow" in out) == (context < 7)
    else:
        assert "[3/7]" in out and "fsv changes state CHN-123" in out
    assert len(calls) == 1  # compact output never fetches approval details


@pytest.mark.parametrize("renderer", [state_flow.render_compact, state_flow.render_state_diagram])
def test_compact_renderers_escape_names_instead_of_interpreting_markup(flow_env, renderer):
    flow, change, _ = flow_env
    flow["state_list"][3]["value"] = "Review [Acme] → Δ"
    out = capture(renderer, change)
    assert "Review [Acme] → Δ" in out


@pytest.mark.parametrize("renderer", [state_flow.render_compact, state_flow.render_state_diagram])
@pytest.mark.parametrize("case", ["missing-flow", "empty-states", "unknown-current", "denied"])
def test_optional_diagrams_never_invent_state_when_data_is_unavailable(flow_env, monkeypatch, renderer, case):
    flow, change, calls = flow_env
    if case == "missing-flow":
        change.pop("state_flow_id")
    elif case == "empty-states":
        flow["state_list"] = []
    elif case == "unknown-current":
        change["status"] = 999
    else:
        def denied(*a, **k):
            raise APIError(500, "synthetic failure")
        monkeypatch.setattr(state_flow, "get_state_flow", denied)
    assert capture(renderer, change) == ""
    assert not any("approvals" in path for path, _ in calls)


@pytest.mark.parametrize("case,message", [("missing", "no state_flow_id"), ("empty", "empty state_list")])
def test_full_flow_explicitly_reports_absent_lifecycle(flow_env, case, message):
    flow, change, _ = flow_env
    if case == "missing":
        change.pop("state_flow_id")
    else:
        flow["state_list"] = []
    assert message in capture(state_flow.render_flow, change)


def test_approval_denial_is_optional_but_main_flow_denial_propagates(flow_env, monkeypatch):
    _, change, _ = flow_env
    def denied(path, params=None):
        raise APIError(403, "synthetic denial")
    assert state_flow._try_get_approvals(123, SimpleNamespace(int_get=denied)) == []
    monkeypatch.setattr(state_flow, "get_client", lambda: SimpleNamespace(int_get=denied))
    with pytest.raises(APIError) as exc:
        capture(state_flow.render_flow, change)
    assert exc.value.status == 403
