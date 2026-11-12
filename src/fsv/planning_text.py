"""Reviewed native planning text saves from captured section-query contracts."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any
from urllib.parse import parse_qs, quote, unquote, urlsplit

from fsv import config, journal, schema
from fsv.reviewed_snapshot import stable_snapshot
from fsv.create import match_planning_row, planning_definitions, planning_server_key, resolve_planning_definition
from fsv.readback import same_text
from fsv.resources import CHANGES, PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id

# Native semantic section keys, not tenant-specific numeric choice IDs.
PROBLEM_SECTIONS = {"root cause": "problem_cause", "impact": "problem_impact", "symptoms": "problem_symptom"}


def definitions(client: Any, module: str) -> list[dict[str, Any]]:
    if module == "changes":
        return planning_definitions(schema.load(CHANGES, client, force=True)["fields"])
    if module == "problems":
        return [{"name": key, "label": label.title()} for label, key in PROBLEM_SECTIONS.items()]
    raise ReviewedWriteError("planning wire is proven only for changes and the three native problem analysis sections")


def inventory(client: Any, module: str, target: int) -> list[dict[str, Any]]:
    raw = client.int_get(f"{module}/{target}/planning-fields")
    key = "change_planning_fields" if module == "changes" else "problem_planning_fields"
    if (not isinstance(raw, dict) or not isinstance(raw.get(key), list) or
            raw.get("has_more") or raw.get("has_next") or raw.get("next_page")):
        raise ReviewedWriteError("complete planning-field inventory is unavailable")
    rows = raw[key]
    if any(not isinstance(row, dict) or not (row.get("name") or row.get("label")) for row in rows):
        raise ReviewedWriteError("planning rows have unavailable identities")
    if raw.get("total") is not None and raw["total"] != len(rows):
        raise ReviewedWriteError("planning-field inventory is incomplete")
    return rows


def section_content(row: dict[str, Any] | None) -> str | None:
    """Read native section text in both edit and detail response shapes."""
    if row is not None:
        for key in ("description", "content", "description_html", "description_text", "body"):
            if isinstance(row.get(key), str):
                return row[key]
    return None


@dataclass(frozen=True)
class PlanningTextPlan(WritePlan):
    before: str

    @property
    def hash(self) -> str:
        return journal.plan_hash({"wire_plan": super().hash, "before": stable_snapshot(self.before)}) or ""

    def review(self) -> dict[str, Any]:
        return {**super().review(), "operation": "clear" if self.method == "DELETE" else
                "edit" if self.method == "PUT" else "create", "before": self.before,
                "after": "" if self.method == "DELETE" else self.payload["description"]}


def editable_content(row: dict[str, Any] | None) -> str:
    """Require a readable text baseline and an explicitly empty file inventory."""
    if row is None:
        return ""
    inventories = [row[k] for k in ("attachments", "files") if k in row]
    if not inventories or any(value != [] for value in inventories):
        raise ReviewedWriteError("planning attachments are present or unavailable; text-only preservation is not proven")
    content = section_content(row)
    if content is None or any(k in row and not isinstance(row[k], str)
                              for k in ("description", "content", "description_html", "description_text", "body")):
        raise ReviewedWriteError("planning text state is unavailable; no write attempted")
    values = [row[k] for k in ("description", "content", "description_html", "description_text", "body")
              if isinstance(row.get(k), str)]
    # HTML/plain aliases may differ in markup, but must describe the same text
    # and agree about literal emptiness (never strip markup to choose a wire).
    if (any(bool(value) != bool(content) or not same_text(value, content) for value in values) or
            ("empty" in row and (not isinstance(row["empty"], bool) or row["empty"] == bool(content)))):
        raise ReviewedWriteError("planning text state is contradictory; no write attempted")
    return content


def build_plan(client: Any, module: str, target: str | int, section: str,
               description: str | None = None, *, clear: bool = False) -> PlanningTextPlan:
    if clear:
        if description is not None:
            raise ValueError("--clear and --description are mutually exclusive")
    elif not isinstance(description, str) or not description.strip():
        raise ValueError("planning description must be nonempty text/HTML; use --clear to delete")
    defs = definitions(client, module)
    cid = positive_id(parse_id(str(target), CHANGES if module == "changes" else PROBLEMS))
    if module == "changes":
        definition = resolve_planning_definition(section, defs)
    else:
        hits = [d for d in defs if section.casefold() in {d["label"].casefold(), d["name"]}]
        if len(hits) != 1:
            raise ReviewedWriteError("choose Root Cause, Impact or Symptoms; other analysis wires are not proven")
        definition = hits[0]
    key = planning_server_key(definition)
    rows = inventory(client, module, cid)
    existing = match_planning_row(definition, rows, defs)
    before = editable_content(existing)
    if existing is not None and not before and existing.get("id") is not None:
        raise ReviewedWriteError("stored empty planning row has unknown create/edit state; no write attempted")
    if clear and not before:
        raise ReviewedWriteError("planning section is already empty; no write attempted")
    method = "DELETE" if clear else "PUT" if before else "POST"
    base = f"{module}/{cid}/planning-fields"
    path = f"{base}?id={quote(key, safe='')}" if method == "POST" else f"{base}/{quote(key, safe='')}"
    return PlanningTextPlan(f"{module}_planning_text", cid, config.DOMAIN, method, path,
                            json.dumps(None if clear else {"description": description},
                                       sort_keys=True, ensure_ascii=False),
                            journal.snapshot_hash({"definitions": defs, "rows": rows}) or "", before)


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    module = plan.path.split("/")[0]
    section = (parse_qs(urlsplit(plan.path).query)["id"][0] if plan.method == "POST" else
               unquote(urlsplit(plan.path).path.rsplit("/", 1)[1]))
    clear = plan.method == "DELETE"
    description = None if clear else plan.payload["description"]

    def verify(response: Any) -> dict[str, Any]:
        defs = definitions(client, module)
        definition = next(d for d in defs if planning_server_key(d) == section)
        row = match_planning_row(definition, inventory(client, module, plan.target), defs)
        content = section_content(row)
        try:
            checked = editable_content(row)
        except ReviewedWriteError:
            checked = None
        if clear:
            # The accepted delete removes the row; detail views may synthesize
            # a known-empty placeholder. Never normalize HTML to prove deletion.
            matched = checked == "" and (row is None or row.get("id") is None)
            field = "matched" if matched else "unavailable" if checked is None else "mismatched"
        else:
            field = ("unavailable" if content is None or checked is None else
                     "matched" if same_text(content, description) else "mismatched")
        return {"section": section, "description": field, "verified": field == "matched"}

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, module, plan.target, section, description, clear=clear),
                   verify=verify)
