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
    assert unknown and "cannot tell" in unknown[0]
    assert safety.write_risks("problems", {"status": 3}, current={"status": 1}, status_label=lambda v: "Cerrado")
    english = {2: "Open", 3: "Pending", 4: "Resolved"}
    assert not safety.write_risks("tickets", {"status": 3}, current={"status": 2}, status_label=english.get)
    assert safety.write_risks("tickets", {"status": 4}, current={"status": 2}, status_label=english.get)
