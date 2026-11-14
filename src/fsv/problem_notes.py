"""Reviewed problem notes: plan-hash bound send, ambiguity journal and an
independent readback of the note body by listing the problem's notes.

The captured browser body is ``{"body": ...}`` only: the tenant rejects a
``private`` key with HTTP 400, so visibility is not controllable and the
tenant default applies.
"""
from __future__ import annotations

from fsv.tenant import tenant_json

import json
import re
from typing import Any, Callable

import typer

from fsv import config
from fsv.readback import same_text
from fsv.resources import PROBLEMS, format_id, parse_id
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute

ACTION = "problem_note"
VISIBILITY = ("problem notes accept only a body (captured `{body}`; the tenant rejects `private` with "
              "HTTP 400): visibility is not controllable and the tenant default applies")


def build_plan(target: int | str, body: str) -> WritePlan:
    """Pure local preparation; the body is sent verbatim (HTML or plain text)."""
    cid = parse_id(str(target), PROBLEMS)
    if cid <= 0:
        raise ValueError("problem ID must be positive")
    if not isinstance(body, str) or not body.strip() or "\x00" in body:
        raise ValueError("note body must be nonempty text without NUL characters")
    payload = {"body": body}
    return WritePlan(ACTION, cid, config.DOMAIN, "POST", f"problems/{cid}/notes",
                     json.dumps(payload, ensure_ascii=False, sort_keys=True), "")


def review(plan: WritePlan) -> dict[str, Any]:
    return {**plan.review(), "id": format_id({"id": plan.target}, PROBLEMS), "body": plan.payload,
            "warnings": ["visibility: " + VISIBILITY]}


def _returned_id(response: Any) -> int | str | None:
    note = response.get("note") if isinstance(response, dict) and isinstance(response.get("note"), dict) else response
    value = note.get("id") if isinstance(note, dict) else None
    if isinstance(value, int) and not isinstance(value, bool) and value > 0:
        return value
    if isinstance(value, str) and re.fullmatch(r"[0-9]+", value) and int(value) > 0:
        return value
    return None


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict[str, Any]:
    body = plan.payload["body"]

    def rebuild() -> WritePlan:
        return build_plan(plan.target, body)

    def verify(response: Any) -> dict[str, Any]:
        from fsv import views
        nid = _returned_id(response)
        out: dict[str, Any] = {"note_id": nid, "verified": False, "readback": None,
                               "identity_verified": False, "readback_complete": False}
        if nid is None:
            out["warning"] = "note response has no exact note ID; outcome unknown, check the problem before any retry"
            return out
        rows, meta, missing = views.collect_pages(
            client, f"problems/{plan.target}/notes", "notes",
            {"page": 1, "per_page": 30, "include": "user"}, max_pages=50, what="notes")
        exact = next((r for r in rows if isinstance(r, dict) and str(r.get("id")) == str(nid)), None)
        out["readback"] = exact
        out["readback_complete"] = not missing and not meta.get("partial")
        out["identity_verified"] = exact is not None
        if exact is not None:
            actual = exact.get("body") if "body" in exact else exact.get("body_html")
            out["fields"] = {"body": "unavailable" if actual is None else
                             "matched" if same_text(actual, body) else "mismatched"}
            out["verified"] = out["fields"]["body"] == "matched" and out["readback_complete"]
        return out

    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=rebuild, verify=verify)


def add_note(client_factory: Callable[[], Any], id_: str, body: str, *, public: bool, dry_run: bool,
             yes: bool, no_input: bool, expect_plan: str | None, json_out: bool) -> None:
    try:
        if public:
            raise ValueError("--public: " + VISIBILITY)
        plan = build_plan(id_, body)
        doc = review(plan)
        if dry_run:
            typer.echo(tenant_json({"status": "dry_run", "dry_run": True, **doc}, ensure_ascii=False, indent=2))
            return
        if expect_plan is not None and expect_plan != plan.hash:
            raise ReviewedWriteError("note review hash does not match; review the exact body again")
        if yes:
            if expect_plan != plan.hash:
                raise ReviewedWriteError("--yes requires the full --expect-plan from a prior --dry-run review")
        elif no_input:
            raise ReviewedWriteError("sending requires --yes and --expect-plan in no-input mode")
        else:
            typer.echo(tenant_json(doc, ensure_ascii=False, indent=2), err=True)
            if not typer.confirm("Add this exact reviewed note?", default=False):
                typer.echo("Cancelled; no note was sent.")
                return
        result = send(plan, client_factory(), reviewed_hash=plan.hash)
        if json_out:
            typer.echo(tenant_json(result, ensure_ascii=False, indent=2))
        else:
            typer.echo(f"added note {result.get('note_id')} on problem {plan.target} "
                       f"(visibility: tenant default; not controllable on problem notes); "
                       f"verified: {str(bool(result.get('verified'))).lower()}")
            if result.get("warning"):
                typer.echo(result["warning"], err=True)
    except (ValueError, ReviewedWriteError) as exc:
        typer.echo(str(exc), err=True)
        raise typer.Exit(2) from None
