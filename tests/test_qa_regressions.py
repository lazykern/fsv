"""Regression tests for the QA review findings (invented Acme data, no network)."""
from __future__ import annotations

from typing import Any

import acme_change_form as A
from fsv import views


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
