"""Draft knowledge articles and reviewed problem solution links."""
from __future__ import annotations
import html
import json
from dataclasses import dataclass
from typing import Any
from fsv import config, journal
from fsv.problem_workflows import inventory, record
from fsv.readback import submitted_value_matches
from fsv.render import strip_html
from fsv.resources import PROBLEMS, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.views import collect_pages
from fsv.watchers import positive_id

# Accepted application enum values; never tenant folder/category identities.
ARTICLE_TYPES = {"permanent": 1, "workaround": 2}


@dataclass(frozen=True)
class SolutionPlan(WritePlan):
    inputs_json: str


def metadata(client: Any, path: str, key: str, params: dict[str, Any] | None = None) -> list[dict[str, Any]]:
    rows, meta, missing = collect_pages(client, path, key, {"per_page": 300, "page": 1, **(params or {})}, what=key)
    if missing or not meta.get("complete"):
        raise ReviewedWriteError("complete knowledge " + key + " metadata unavailable")
    ids = [positive_id(r.get("id")) for r in rows]
    if len(set(ids)) != len(ids):
        raise ReviewedWriteError("duplicate knowledge identities")
    return sorted([r for r in rows if not r.get("deleted")], key=lambda r: positive_id(r["id"]))


def folder(client: Any, category: str | None, selected: str | None) -> tuple[dict[str, Any], dict[str, Any]]:
    categories = metadata(client, "solutions/categories", "categories")
    def choose(rows, value, default):
        matches = [r for r in rows if r.get(default) is True] if value is None else [r for r in rows if str(r["id"]) == value or str(r.get("name", "")).casefold() == value.casefold()]
        if len(matches) != 1:
            raise ReviewedWriteError("category/folder must resolve uniquely from runtime metadata; specify --category and --folder")
        return matches[0]
    cat = choose(categories, category, "default_category")
    folders = metadata(client, "solutions/folders", "folders", {"category_id": positive_id(cat["id"])})
    dest = choose(folders, selected, "default_folder")
    if positive_id(dest.get("category_id")) != positive_id(cat["id"]):
        raise ReviewedWriteError("folder category identity conflicts")
    return cat, dest


def article(client: Any, ident: int) -> dict[str, Any]:
    raw = client.int_get(f"solutions/articles/{ident}")
    row = raw.get("article") if isinstance(raw, dict) else None
    if not isinstance(row, dict) or positive_id(row.get("id")) != ident or row.get("deleted"):
        raise ReviewedWriteError("exact active article unavailable")
    return row


def create_plan(client: Any, title: str, description: str, kind: str, *, category: str | None = None, selected_folder: str | None = None) -> SolutionPlan:
    if kind not in ARTICLE_TYPES or not title.strip() or not description.strip() or any(ord(c) < 32 for c in title):
        raise ValueError("nonempty title/body and workaround or permanent type required")
    cat, dest = folder(client, category, selected_folder)
    payload = {"title": title, "description": "<div>" + html.escape(description).replace("\n", "<br>") + "</div>",
               "status": 1, "article_type": ARTICLE_TYPES[kind], "folder_id": str(dest["id"]), "source": "problem"}
    inputs = dict(title=title, description=description, kind=kind, category=category, selected_folder=selected_folder)
    return SolutionPlan("solution_create", positive_id(dest["id"]), config.DOMAIN, "POST", "solutions/articles",
                        json.dumps(payload, sort_keys=True), journal.snapshot_hash({"category": cat, "folder": dest}) or "", json.dumps(inputs, sort_keys=True))


def send_create(plan: SolutionPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    def verify(response):
        raw = response.get("article", {}) if isinstance(response, dict) else {}
        ident = positive_id(raw.get("id"))
        exact = article(client, ident)
        results = {}
        for key in ("title", "description", "status", "article_type", "folder_id"):
            expected = plan.payload[key]
            if key not in exact:
                results[key] = "unavailable"
            else:
                matched = (isinstance(exact[key], str) and strip_html(exact[key]).strip() == strip_html(expected).strip()) if key == "description" else submitted_value_matches(exact[key], expected)
                results[key] = "matched" if matched else "mismatched"
        return {"article_id": ident, "verified": all(v == "matched" for v in results.values()), "fields": results, "readback": exact}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: create_plan(client, **json.loads(plan.inputs_json)), verify=verify)


def link_plan(client: Any, target: str | int, ident: int, *, unlink: bool = False) -> SolutionPlan:
    target = positive_id(parse_id(str(target), PROBLEMS))
    ident = positive_id(ident)
    problem = record(client, target)
    before = inventory(client, target, "solutions")
    exact = article(client, ident)
    if type(exact.get("article_type")) is not int or exact["article_type"] not in ARTICLE_TYPES.values():
        raise ReviewedWriteError("only proven workaround/permanent article types can be linked")
    exists = any(positive_id(r["id"]) == ident for r in before)
    if exists != unlink:
        raise ReviewedWriteError("article already linked" if exists else "article is not linked to this problem")
    return SolutionPlan("problem_solution_unlink" if unlink else "problem_solution_link", target, config.DOMAIN, "PUT",
                        f"problems/{target}/solutions/{ident}", json.dumps({"type": "unlink" if unlink else "link"}),
                        journal.snapshot_hash({"problem": problem, "solutions": before, "article": exact}) or "", json.dumps({"ident": ident, "unlink": unlink}))


def send_link(plan: SolutionPlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    inputs = json.loads(plan.inputs_json)
    def verify(_response):
        rows = inventory(client, plan.target, "solutions")
        exact = next((r for r in rows if positive_id(r["id"]) == inputs["ident"]), None)
        if inputs["unlink"]:
            matched = exact is None
        else:
            source = article(client, inputs["ident"])
            matched = exact is not None and exact.get("article_type") == source["article_type"]
        return {"verified": matched, "readback": rows}
    return execute(plan, client, reviewed_hash=reviewed_hash,
                   rebuild=lambda: link_plan(client, plan.target, **inputs), verify=verify)
