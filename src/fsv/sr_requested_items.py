"""Captured SR requested-item text answers, Delivered stage, add and removal."""
from __future__ import annotations

import json
from typing import Any

from fsv import catalog, config
from fsv.attachments import owner_row
from fsv.formmeta import apply_answers
from fsv.forms import FormState
from fsv.readback import submitted_value_matches
from fsv.resources import TICKETS, parse_id
from fsv.reviewed_snapshot import stable_snapshot
from fsv.reviewed_write import ReviewedWriteError, WritePlan, execute
from fsv.watchers import positive_id


def inventory(client: Any, target: int) -> list[dict]:
    raw = client.int_get(f"tickets/{target}/requested_items")
    if not isinstance(raw, dict) or not isinstance(raw.get("requested_items"), list):
        raise ReviewedWriteError("requested-item inventory unavailable")
    rows = raw["requested_items"]
    meta = raw.get("meta", {})
    if any(meta.get(k) or raw.get(k) for k in ("has_more", "has_next", "next_page")):
        raise ReviewedWriteError("partial requested-item inventory")
    ids = [positive_id(r.get("id")) for r in rows if isinstance(r, dict)]
    if len(ids) != len(rows) or len(set(ids)) != len(ids):
        raise ReviewedWriteError("invalid or duplicate requested-item identities")
    for envelope in (raw, meta):
        if "total" in envelope and (type(envelope["total"]) is not int or envelope["total"] != len(rows)):
            raise ReviewedWriteError("incomplete requested-item inventory")
    if any(str(r.get("ticket_id")) != str(target) for r in rows):
        raise ReviewedWriteError("requested item belongs to a different or unavailable ticket")
    return sorted(rows, key=lambda r: positive_id(r["id"]))


def detail(client: Any, target: int, item: int) -> dict:
    raw = client.int_get(f"tickets/{target}/requested_items/{item}", {"view": "more_info"})
    if not isinstance(raw, dict) or not isinstance(raw.get("requested_item"), dict) or positive_id(raw["requested_item"].get("id")) != item:
        raise ReviewedWriteError("exact requested-item detail unavailable")
    return raw


def text_answers(client: Any, raw_item: dict, answers: dict, existing: dict | None = None) -> str:
    fields = raw_item.get("custom_fields")
    if not isinstance(fields, list) or not answers:
        raise ValueError("nonempty text answers and item field metadata are required")
    meta = catalog.load_item_meta(client, raw_item.get("display_id"), item=raw_item)
    for name, value in answers.items():
        found = [f for f in fields if f.get("name") == name]
        spec_field = meta.spec.by_name.get(name)
        if len(found) != 1 or spec_field is None or spec_field.kind not in {"text", "textarea"}:
            raise ReviewedWriteError("only captured plain text/paragraph answer types are supported")
        if not isinstance(value, str) or not value.strip() or "\x00" in value:
            raise ValueError("requested-item answers must be nonempty text")
        if found[0].get("deleted") or found[0].get("archived"):
            raise ReviewedWriteError("requested-item field is unavailable")
    # Reuse the catalog's conditional rule evaluator, never transplant ticket fields.
    state = FormState(meta.spec, meta.engine(), mode="edit" if existing is not None else "new", existing=existing)
    errors = apply_answers(state, answers)
    _, rule_errors = catalog.rule_diagnostics(meta, state)
    errors += state.errors() + rule_errors
    if errors:
        raise ReviewedWriteError("requested-item validation failed: " + "; ".join(errors))
    if any(name not in state.evaluated.visible for name in answers):
        raise ReviewedWriteError("requested-item answer is hidden")
    return meta.fingerprint.hash


def build_plan(client: Any, target: str | int, *, action: str, item: int | None = None,
               catalog_item: str | None = None, answers: dict | None = None, stage: int | None = None) -> WritePlan:
    cid = positive_id(parse_id(str(target), TICKETS))
    if action not in {"edit", "stage", "add", "remove"}:
        raise ValueError("unsupported requested-item action")
    parent = owner_row(client, "tickets", cid)
    if str(parent.get("ticket_type", "")).casefold() not in {"service request", "service_request"}:
        raise ReviewedWriteError("exact parent must be a service request")
    rows = inventory(client, cid)
    path, metadata_hash, snapshot = f"tickets/{cid}/requested_items", None, None
    if action == "add":
        if item is not None or stage is not None or not isinstance(catalog_item, str) or not catalog_item.isascii() or not catalog_item.isdigit() or int(catalog_item) <= 0:
            raise ValueError("add needs an exact positive catalog display ID as text, answers, and no existing item/stage")
        raw_item = catalog.get_item(client, catalog_item)
        if str(raw_item.get("display_id")) != catalog_item or raw_item.get("deleted"):
            raise ReviewedWriteError("catalog display identity changed/unavailable")
        metadata_hash = text_answers(client, raw_item, answers or {})
        method, payload = "POST", {"item_id": catalog_item, "custom_fields": answers}
    else:
        item = positive_id(item)
        exact = [r for r in rows if positive_id(r["id"]) == item]
        if len(exact) != 1 or catalog_item is not None:
            raise ReviewedWriteError("exact requested-item identity unavailable")
        path += f"/{item}"
        if action == "remove":
            if exact[0].get("is_parent") is not False or answers or stage is not None:
                raise ReviewedWriteError("only an explicitly non-parent item may be removed, without answer/stage edits")
            method, payload = "DELETE", None
        elif action == "stage":
            if type(stage) is not int or stage != 2 or answers:
                raise ValueError("only captured stage 2 (observed Delivered) is supported; no combined answer update")
            method, path, payload = "PATCH", path + "/update_stage", {"stage": stage}
        else:
            if stage is not None:
                raise ValueError("stage is a separate requested-item action")
            snapshot = detail(client, cid, item)
            row = snapshot["requested_item"]
            rules = snapshot.get("business_rules")
            if not isinstance(rules, dict):
                raise ReviewedWriteError("requested-item editability rules unavailable")
            for key in ("hidden_req_item_fields", "non_editable_item_fields"):
                values = rules.get(key)
                if not isinstance(values, dict) or values.get(str(item)) != []:
                    raise ReviewedWriteError("nonempty or unavailable item editability restrictions require a new capture")
            linked_item = exact[0].get("item", {})
            display_id = positive_id(linked_item.get("display_id"))
            raw_item = catalog.get_item(client, display_id)
            if str(raw_item.get("id")) != str(linked_item.get("id")) or str(raw_item.get("display_id")) != str(display_id):
                raise ReviewedWriteError("requested-item catalog identity changed")
            metadata_hash = text_answers(client, raw_item, answers or {}, row.get("custom_fields"))
            method, payload = "PUT", {"custom_fields": answers}
    intent = dict(target=cid, action=action, item=item, catalog_item=catalog_item, answers=answers, stage=stage)
    context = {"intent": intent, "parent": parent, "rows": rows, "detail": snapshot, "metadata": metadata_hash}
    return WritePlan("sr_requested_item_" + action, cid, config.DOMAIN, method, path, json.dumps(payload, sort_keys=True),
                     json.dumps(stable_snapshot(context), sort_keys=True))


def send(plan: WritePlan, client: Any, *, reviewed_hash: str | None) -> dict:
    before = json.loads(plan.context_hash)
    intent = before["intent"]
    def verify(response):
        rows = inventory(client, plan.target)
        old = {positive_id(r["id"]) for r in before["rows"]}
        current = {positive_id(r["id"]) for r in rows}
        ident = intent["item"]
        if intent["action"] == "add":
            returned = response.get("requested_item", {}) if isinstance(response, dict) else {}
            ident = positive_id(returned.get("id"))
        if intent["action"] == "remove":
            return {"item_id": ident, "verified": current == old - {ident}}
        exact = [r for r in rows if positive_id(r["id"]) == ident]
        if len(exact) != 1 or current != (old | {ident} if intent["action"] == "add" else old):
            return {"item_id": ident, "verified": False}
        if intent["action"] == "stage":
            actual = exact[0].get("stage", {})
            return {"item_id": ident, "verified": isinstance(actual, dict) and type(actual.get("id")) is int and actual["id"] == intent["stage"]}
        raw = detail(client, plan.target, ident)["requested_item"]
        actual = raw.get("custom_fields", {})
        fields = {k: "unavailable" if k not in actual else "matched" if submitted_value_matches(actual[k], v) else "mismatched"
                  for k, v in intent["answers"].items()}
        identity = True
        if intent["action"] == "add":
            # item_id changes namespaces between envelopes; use explicit nested display_id.
            identity = str(exact[0].get("item", {}).get("display_id")) == intent["catalog_item"] and ident not in old
        return {"item_id": ident, "fields": fields, "verified": identity and all(v == "matched" for v in fields.values())}
    return execute(plan, client, reviewed_hash=reviewed_hash, rebuild=lambda: build_plan(client, **intent), verify=verify)
