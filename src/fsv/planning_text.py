"""Reviewed native planning text saves from captured section-query contracts."""
from __future__ import annotations

import json
from typing import Any
from urllib.parse import parse_qs, quote, urlsplit

from fsv import config, journal, schema
from fsv.create import match_planning_row, planning_definitions, planning_server_key, resolve_planning_definition
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


def build_plan(client: Any, module: str, target: str | int, section: str, description: str) -> WritePlan:
    if not isinstance(description, str) or not description.strip():
        raise ValueError("planning description must be nonempty text/HTML")
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
    if existing and ("attachments" not in existing or existing["attachments"]):
        raise ReviewedWriteError("planning attachments are present or unavailable; text-only preservation is not proven")
    return WritePlan(f"{module}_planning_text", cid, config.DOMAIN, "POST",
                     f"{module}/{cid}/planning-fields?id={quote(key, safe='')}",
                     json.dumps({"description": description}, sort_keys=True, ensure_ascii=False),
                     journal.plan_hash({"definitions": defs, "rows": rows}) or "")


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    module = plan.path.split("/")[0]
    section = parse_qs(urlsplit(plan.path).query)["id"][0]
    description = plan.payload["description"]

    def verify(response: Any) -> dict[str, Any]:
        defs = definitions(client, module)
        definition = next(d for d in defs if planning_server_key(d) == section)
        row = match_planning_row(definition, inventory(client, module, plan.target), defs)
        field = "unavailable" if row is None or "description" not in row else "matched" if row["description"] == description else "mismatched"
        return {"section": section, "description": field, "verified": field == "matched"}

    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: build_plan(client, module, plan.target, section, description), verify=verify)
