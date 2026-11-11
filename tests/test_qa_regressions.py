"""Regression tests for the QA review findings (invented Acme data, no network)."""
from __future__ import annotations

from typing import Any

import acme_change_form as A
from fsv import safety, views


class _RecordRules:
    """Edit-form rules that differ per record (the server filters by module_id)."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, Any]] = []

    def int_get(self, path: str, params: Any = None) -> Any:
        self.calls.append((path, dict(params or {})))
        if path == "ws/2/business_rules":
            r = A.rules()
            r["business_rules"] = r["business_rules"][:1]
            r["business_rules"][0]["name"] = f"rule for {params.get('module_id')}"
            return r
        return {}


def test_detail_rules_are_cached_per_record():
    c = _RecordRules()
    fields = A.fields()["change_fields"]
    r111, _ = views.load_edit_rules(c, "change", {"display_id": 111, "workspace_id": 2}, fields)
    r222, _ = views.load_edit_rules(c, "change", {"display_id": 222, "workspace_id": 2}, fields)
    assert r111["business_rules"][0]["name"] == "rule for 111"
    assert r222["business_rules"][0]["name"] == "rule for 222"
    assert [p["module_id"] for path, p in c.calls if path.endswith("business_rules")] == ["111", "222"]
    again, _ = views.load_edit_rules(c, "change", {"display_id": 111, "workspace_id": 2}, fields)
    assert again["business_rules"][0]["name"] == "rule for 111"
    assert len([1 for path, _ in c.calls if path.endswith("business_rules")]) == 2  # cached


# P1 #7: localized / custom terminal statuses must not bypass the confirmation gate
def test_localized_terminal_status_needs_confirmation():
    german = {2: "Offen", 3: "Wartend", 5: "Geschlossen", 12: "Erledigt (Kunde)"}
    lab = lambda v: german.get(v, str(v))  # noqa: E731
    assert safety.write_risks("tickets", {"status": 5}, current={"status": 2}, status_label=lab)
    unknown = safety.write_risks("tickets", {"status": 12}, current={"status": 2}, status_label=lab)
    assert unknown and "confirmation" in unknown[0]
    assert safety.write_risks("problems", {"status": 3}, current={"status": 1}, status_label=lambda v: "Cerrado")
    english = {2: "Open", 3: "Pending", 4: "Resolved"}
    assert safety.write_risks("tickets", {"status": 3}, current={"status": 2}, status_label=english.get)
    assert safety.write_risks("tickets", {"status": 99}, current={"status": 2},
                              status_label=lambda _: "Pending final closure")
    assert safety.write_risks("tickets", {"status": 4}, current={"status": 2}, status_label=english.get)


# P2 #9: --all-conversations never claims a partial fetch is complete
class _Convs:
    def __init__(self, total: int, fail_page: int | None = None) -> None:
        self.total, self.fail_page = total, fail_page

    def int_get(self, path: str, params=None):
        from fsv.errors import APIError

        page = int((params or {}).get("page") or 1)
        if page == self.fail_page:
            raise APIError(403, "denied", "GET", path)
        start = (page - 1) * 30
        rows = [{"id": i, "created_at": f"2030-01-01T00:{i:02d}:00Z", "body": f"m{i}"}
                for i in range(start, min(start + 30, self.total))]
        return {"conversations": rows, "meta": {"count": self.total, "has_more": start + 30 < self.total}}


def test_all_conversations_keeps_a_partial_fetch_and_says_so():
    rows, meta, miss = views.load_conversations(_Convs(52, fail_page=2), 5, all_pages=True)
    assert len(rows) == 30 and meta["has_more"] is True and meta["count"] == 52
    assert miss is not None and miss.status == 403 and "after the first 30" in miss.note


def test_all_conversations_page_cap_is_not_complete(monkeypatch):
    monkeypatch.setattr(views, "MAX_CONVERSATION_PAGES", 2)
    rows, meta, miss = views.load_conversations(_Convs(100), 5, all_pages=True)
    assert len(rows) == 60 and meta["has_more"] is True
    assert miss is not None and "after page 2" in miss.note


def test_all_conversations_complete_fetch():
    rows, meta, miss = views.load_conversations(_Convs(52), 5, all_pages=True)
    assert len(rows) == 52 and meta["has_more"] is False and miss is None


# P2 #14: the state-flow command follows the record's workspace
def test_state_flow_uses_the_record_workspace_and_falls_back():
    from fsv import state_flow
    from fsv.errors import APIError

    class C:
        def __init__(self, ws_ok: bool) -> None:
            self.ws_ok, self.paths = ws_ok, []

        def int_get(self, path, params=None):
            self.paths.append(path)
            if path.startswith("ws/") and not self.ws_ok:
                raise APIError(404, "no", "GET", path)
            return {"state_flow": {"name": "Acme flow", "state_list": []}}

    c = C(True)
    assert state_flow.get_state_flow(8001, c, 3)["name"] == "Acme flow"
    assert c.paths == ["ws/3/change_management/8001"]
    c = C(False)
    state_flow.get_state_flow(8001, c, 3)
    assert c.paths == ["ws/3/change_management/8001", "change_management/8001"]
    c = C(True)
    state_flow.get_state_flow(8001, c, None)
    assert c.paths == ["change_management/8001"]


# P2 #15: a malformed date-only value is shown as stored
def test_fmt_when_keeps_an_impossible_date():
    assert views.fmt_when("2031-02-30") == "2031-02-30"
    assert views.fmt_when("2031-02-28").startswith("Fri, 28 Feb 2031")


# P2 #11 (list part): page > 1 without a total never prints an impossible range
def test_list_header_without_a_total_on_a_later_page():
    from fsv.resources import CHANGES

    def page(n: int, has_next: bool, total=None) -> str:
        return views.ListPage(CHANGES, [{}] * n, [], [], None, 2, 30, total, has_next, "created_at", "desc",
                              "Created Date").header()

    assert "31-33 of 33 ·" in page(3, False)
    assert "31-60 of 60+ ·" in page(30, True)
    assert "31-60 of 75 ·" in page(30, True, 75)
