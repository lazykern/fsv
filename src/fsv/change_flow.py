"""Create and update changes through the form engine.

Faces (YAML ``--from``/``--edit``, ``--set``, presets, legacy flags) all end
here: a :class:`Plan` (engine state + exact request body + files + plan
hash + review), then confirmation, a metadata freshness check, and the
writes, journaled under the plan hash.

Order of writes on create: the change itself, then attachments (uploaded
only now, after confirmation), then planning fields. A failure after the
change exists reports the created id and every uploaded attachment id.
"""
from __future__ import annotations

import datetime as dt
import hashlib
import os
import sys
from dataclasses import dataclass, field as dc_field
from pathlib import Path
from typing import Any, Callable, Mapping

from fsv import journal
from fsv.drafts import (
    Draft,
    load_hints,
    matching_hints,
    parse_template,
    render_template,
)
from fsv.errors import APIError, SessionError
from fsv.formmeta import (
    RECORD_KEYS,
    ClientResolver,
    FormMeta,
    apply_answers,
    existing_from_record,
    find_field,
    is_description,
    is_requester,
    is_status,
    load_meta,
    name_existing,
    rules_warnings,
)
from fsv.forms import (
    Field,
    FormError,
    FormState,
    LookupValue,
    PayloadError,
    build_change_payload,
    encode_value,
    plan_hash,
    plan_matches,
    plain_text,
)
from fsv.review import Review
from fsv.safety import norm, write_risks

DUPLICATE_WINDOW_HOURS = 24


class FlowExit(Exception):
    """Stop with a message and a documented exit code."""

    def __init__(self, code: int, message: str = "") -> None:
        super().__init__(message)
        self.code = code
        self.message = message


class PartialFailure(Exception):
    def __init__(self, created_id: Any, uploaded: list[dict[str, Any]], steps: list[dict[str, Any]],
                 error: BaseException) -> None:
        self.created_id = created_id
        self.uploaded = uploaded
        self.steps = steps
        self.error = error
        ids = ", ".join(str(u["id"]) for u in uploaded) or "none"
        super().__init__(f"change {created_id} was written, then a later step failed: {error}. "
                         f"Uploaded attachment ids: {ids}")


def interactive() -> bool:
    """Prompts allowed? (patched in tests)."""
    return sys.stdin.isatty()


def sha256_file(path: str) -> dict[str, Any]:
    h = hashlib.sha256()
    size = 0
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 16), b""):
            h.update(chunk)
            size += len(chunk)
    return {"name": os.path.basename(path), "size": size, "sha256": h.hexdigest(), "path": path}


@dataclass
class Plan:
    action: str  # create | update
    meta: FormMeta
    state: FormState
    draft: Draft
    payload: dict[str, Any] | None
    files: dict[str, list[str]]
    planning: list[dict[str, Any]]
    review: Review
    errors: list[str]
    requester: LookupValue | None = None
    target: int | None = None
    hash: str = ""
    risks: list[str] = dc_field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.errors

    def to_json(self) -> dict[str, Any]:
        return {
            "plan_hash": self.hash or None,
            "action": self.action,
            "target": self.target,
            "payload": self.payload if self.ok else None,  # never a partial payload
            "after": {
                "attachments": [sha256_file(p) for p in self.files.get("attachments", []) if os.path.isfile(p)],
                "planning": [
                    {"label": p["label"], "field": p["field_id"], "description_html": p["description_html"],
                     "files": [sha256_file(f) for f in p["files"] if os.path.isfile(f)]}
                    for p in self.planning
                ],
            },
            "review": self.review.to_dict(),
            "warnings": self.review.warnings,
            "errors": self.errors,
        }


# ---------------------------------------------------------------------------
# building plans


def _field_by_type(meta: FormMeta, field_type: str) -> Field | None:
    return next((f for f in meta.spec.fields if f.field_type == field_type), None)


def legacy_answers(meta: FormMeta, *, subject: str | None = None, priority: str | None = None,
                   agent: str | None = None, group: str | None = None) -> dict[str, Any]:
    """Old create/update flags as engine answers (keyed by internal name)."""
    out: dict[str, Any] = {}
    for ftype, value in (("default_subject", subject), ("default_priority", priority),
                         ("default_agent", agent), ("default_group", group)):
        if value is None:
            continue
        f = _field_by_type(meta, ftype)
        if f is None:
            raise FlowExit(2, f"this form has no {ftype.removeprefix('default_')} field")
        out[f.api_name] = value
    return out


def _split_special(meta: FormMeta, draft: Draft) -> tuple[dict[str, Any], Any, Any, list[str]]:
    """-> (answers without status/requester, status raw, requester raw, errors)."""
    answers: dict[str, Any] = {}
    status_raw = requester_raw = None
    errors: list[str] = []
    for k, v in draft.answers.items():
        try:
            f = find_field(meta.spec, k)
        except FormError:
            answers[k] = v  # apply_answers reports it
            continue
        if is_status(f):
            status_raw = v
        elif is_requester(f):
            requester_raw = v
        else:
            answers[k] = v
    if draft.description_html is not None:
        desc = next((f for f in meta.spec.fields if is_description(f)), None)
        if desc is None:
            errors.append("this form has no description field")
        else:
            answers = {k: v for k, v in answers.items() if meta.spec.find(k) is not desc}
            answers[desc.api_name] = draft.description_html
    return answers, status_raw, requester_raw, errors


def _acknowledge(state: FormState, acks: list[str], ack_all: bool) -> list[str]:
    warnings = []
    if ack_all or any(norm(a) == "all" for a in acks):
        state.acknowledge()
        return warnings
    for a in acks:
        try:
            state.acknowledge(a)
        except ValueError:
            warnings.append(f"ack {a!r}: no blocking message matches (ignored)")
    return warnings


def _resolve_planning(meta: FormMeta, draft: Draft) -> tuple[list[dict[str, Any]], list[str]]:
    from fsv.create import planning_server_key, resolve_planning_definition

    out, errors = [], []
    for label, entry in draft.planning.items():
        try:  # by label first: internal names differ between accounts
            defn = resolve_planning_definition(label, meta.raw_fields)
        except Exception as e:  # APIError(0, "planning field not found ...")
            errors.append(str(getattr(e, "body", e)))
            continue
        files = list(entry.get("files") or [])
        out.append({"label": label, "field_id": planning_server_key(defn),
                    "field_label": str(defn.get("label") or label), "files": files,
                    "description_html": entry.get("description_html")})
    return out, errors


def _file_errors(paths: list[str]) -> list[str]:
    return [f"file not found: {p}" for p in paths if not os.path.isfile(p)]


def real_conflicts(ev: Any) -> list[Any]:
    """Genuine rule conflicts: two rules that BOTH match in the final state
    and give opposite actions on one field property. The engine also logs
    rules that matched earlier in the event replay but no longer match (an
    ordinary Yes/No pair where only one applies); those are not conflicts."""
    from fsv.forms.rules import Conflict

    current = {str(r) for r in ev.matched}
    out = []
    for c in ev.conflicts:
        live = [d for d in c.decisions if str(d.rule_id) in current]
        if len({str(d.rule_id) for d in live}) < 2:
            continue
        positive = {d.action in ("show", "mandate", "disable") for d in live}
        if len(positive) < 2:
            continue
        winner = c.winner if c.winner in live else live[-1]
        out.append(Conflict(c.field, c.property, tuple(live), winner))
    return out


def only_option(state: FormState, name: str) -> tuple[Any, Any] | None:
    """(choice, restriction) when a rule limits ``name`` to exactly one
    real (non-empty) choice."""
    r = state.evaluated.restricted.get(name)
    if r is None:
        return None
    offered = [c for c in state.choices(name) if c.visible and c.label and str(c.value) != ""]
    return (offered[0], r) if len(offered) == 1 else None


def _rule_of(r: Any) -> str:
    return str(r.rule_name or r.rule_id)


def _auto_suffix(state: FormState, name: str) -> str:
    """Review-row suffix: how a rule shaped this value."""
    ev = state.evaluated
    if name in ev.populated:
        return " (auto)"
    one = only_option(state, name)
    if one is not None and norm(state.display(name)) == norm(one[0].label):
        return f" (only option allowed by rule {_rule_of(one[1])})"
    r = ev.restricted.get(name)
    if r is not None:
        return f" (choices limited by rule {_rule_of(r)})"
    return ""


def _unsent_suffix(f: Any) -> str:
    return "" if getattr(f, "sent", True) else " (not sent: fsv does not know its request key yet)"


def _notes(state: FormState) -> list[str]:
    ev = state.evaluated
    spec = state.spec
    out = []
    for name, rid in ev.populated.items():
        if name in ev.visible:
            out.append(f"{spec.by_name[name].label} = {state.display(name)} (filled in by rule {rid})")
    for name, r in ev.restricted.items():
        if name in ev.visible:
            offered = [c.label for c in state.choices(name) if c.visible]
            out.append(f"{spec.by_name[name].label}: choices limited to {', '.join(offered) or '(none)'} "
                       f"by rule {_rule_of(r)!r}")
    for name in sorted(ev.disabled, key=lambda n: ev.visible.index(n) if n in ev.visible else 1 << 20):
        d = ev.decision(name, "disabled")
        out.append(f"{spec.by_name[name].label}: read-only" + (f" ({d.describe()})" if d else ""))
    for name, (old, rid) in ev.overridden.items():
        f = spec.by_name[name]
        out.append(f"{f.label}: your value {state.display(name, old)!r} was replaced by rule {rid}")
    out.extend(f"conflict: {c.describe()}" for c in real_conflicts(ev))
    return out


def _messages(state: FormState) -> list[dict[str, Any]]:
    acked = state.acknowledged
    return [{"text": m.text, "blocking": m.blocking, "acknowledged": m.blocking and m.key in acked,
             "rule": m.rule_name or m.rule_id} for m in state.evaluated.messages]


def _dropped(state: FormState) -> list[str]:
    out = []
    for f, v, d in state.dropped():
        why = f"hidden by rule {d.rule_name!r}" if d and d.action == "hide" else "hidden"
        out.append(f"{f.label} = {state.display(f.api_name, v)} ({why})")
    return out


def _hint_lines(state: FormState, section: str) -> tuple[list[str], list[str]]:
    try:
        hints = load_hints(section)
    except ValueError as e:
        return [], [str(e)]
    return matching_hints(state, hints)


def _attachment_rows(files: dict[str, list[str]], planning: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    labels = {f"planning:{p['field_id']}": f"planning: {p['label']}" for p in planning}
    for target, paths in files.items():
        for p in paths:
            if os.path.isfile(p):
                rows.append({"target": labels.get(target, target), **sha256_file(p)})
    return rows


def _hash(request: Any, files: dict[str, list[str]], planning: list[dict[str, Any]], target: Any = None) -> str:
    doc: dict[str, Any] = {"request": request,
                           "after": {"planning": {p["field_id"]: p["description_html"] for p in planning}}}
    if target is not None:
        doc["target"] = target
    return plan_hash(doc, {k: v for k, v in files.items() if v})


def build_create_plan(
    meta: FormMeta,
    draft: Draft,
    *,
    resolver: Any,
    requester: LookupValue | None,
    ack_all: bool = False,
    allow_incomplete: bool = False,
) -> Plan:
    spec = meta.spec
    state = FormState(spec, meta.engine(), mode="new", resolver=resolver)
    answers, status_raw, _req, errors = _split_special(meta, draft)
    warnings: list[str] = list(spec.warnings) + rules_warnings(meta.rules)
    req_field = next((f for f in spec.fields if is_requester(f)), None)
    if requester is not None and req_field is not None:
        try:
            state.set(req_field.api_name, requester)
        except FormError as e:
            errors.append(str(e))
    failed: set[str] = set()
    errors += apply_answers(state, answers, failed)
    warnings += _acknowledge(state, draft.ack, ack_all)

    status_field = next((f for f in spec.fields if is_status(f)), None)
    status_value = status_field.default if status_field else None
    risks: list[str] = []
    if status_raw not in (None, "") and status_field is not None:
        try:
            status_value = state.parse(status_field, status_raw)
        except FormError as e:
            errors.append(str(e))
        if status_value != status_field.default:
            label = status_field.choice(status_value).label if status_field.choice(status_value) else status_value
            risks.append(f"status on create -> {label} (not the form default; may start approvals/workflows)")

    skip = {f"{req_field.label} is required"} if req_field is not None else set()
    skip |= {f"{spec.by_name[n].label} is required" for n in failed}  # one report per field
    form_problems = [p for p in state.errors() if p not in skip]
    if requester is None:
        form_problems.insert(0, "requester is not set: pass --requester EMAIL, add `requester:` to the draft, "
                                "or `fsv config set defaults.requester EMAIL`")
    planning, perr = _resolve_planning(meta, draft)
    files: dict[str, list[str]] = {"attachments": list(draft.attachments)}
    for p in planning:
        files[f"planning:{p['field_id']}"] = p["files"]
    file_errors = _file_errors([x for paths in files.values() for x in paths])
    hard = errors + perr + file_errors
    problems = hard + ([] if allow_incomplete else form_problems)
    if allow_incomplete and form_problems:
        warnings.append("--no-validate: sending without the client-side required check: " + "; ".join(form_problems))

    payload = None
    if requester is not None:
        try:
            payload = build_change_payload(state, requester.email, requester.display, status=status_value,
                                           allow_incomplete=bool(problems) or allow_incomplete)
        except PayloadError as e:
            problems += [p for p in e.problems if p not in problems]
        except FormError as e:
            problems.append(str(e))

    review = Review(title="New Change" + (f": {state.get('subject')}" if spec.find("subject") and state.get("subject")
                                          else ""))
    if requester is not None:
        review.rows.append(("Requester", str(requester)))
    else:
        review.rows.append(("Requester", "(not set)"))
    for name in state.payload_values():
        f = spec.by_name[name]
        if is_requester(f) or is_status(f):
            continue
        review.rows.append((f.label, state.display(name) + _auto_suffix(state, name) + _unsent_suffix(f)))
    if status_field is not None:
        st = status_field.choice(status_value)
        review.rows.append((f"{status_field.label} on create",
                            f"{st.label if st else status_value}" + (" (form default)" if status_value == status_field.default
                                                                     else "")))
    notify = []
    for ftype, what in (("default_agent", "assigned agent"), ("default_group", "assigned group")):
        f = _field_by_type(meta, ftype)
        if f is not None and f.api_name in state.payload_values():
            notify.append(f"{f.label}: {state.display(f.api_name)} ({what}; Freshservice usually notifies on assignment)")
    if requester is not None:
        notify.append(f"Requester: {requester} (requester notifications)")
    hint_lines, hint_problems = _hint_lines(state, "changes")
    notify += hint_lines
    review.notify = notify
    review.messages = _messages(state)
    review.dropped = _dropped(state)
    review.notes = _notes(state)
    review.attachments = _attachment_rows(files, planning)
    review.risks = risks
    review.errors = problems
    review.warnings = warnings + hint_problems
    review.metadata = meta.as_of()
    plan = Plan("create", meta, state, draft, payload, files, planning, review, problems, requester=requester,
                risks=risks)
    if payload is not None and not problems:
        plan.hash = _hash(payload, files, planning)
        review.plan_hash = plan.hash
    return plan


def _normcmp(f: Field, e: Any) -> Any:
    if e is None or e == "" or e == [] or e == ():
        return None
    if f.kind in ("datetime",) and isinstance(e, str):
        try:
            t = dt.datetime.fromisoformat(e.replace("Z", "+00:00"))
            if t.tzinfo:
                return t.astimezone(dt.timezone.utc).isoformat()
        except ValueError:
            return e
    if isinstance(e, (list, tuple)):
        return sorted(str(x) for x in e)
    if isinstance(e, bool):
        return e
    if f.kind in ("text", "textarea", "richtext"):
        return str(e).strip()
    return str(e)


def _encode(state: FormState, f: Field, v: Any) -> Any:
    if v is None or v == "" or v == () or v == []:
        return None
    try:
        return encode_value(state.spec, f, v, state.evaluated.values)
    except Exception:
        if isinstance(v, LookupValue):
            return v.id
        if isinstance(v, tuple):
            return [x.id if isinstance(x, LookupValue) else x for x in v]
        return v


def build_update_plan(
    meta: FormMeta,
    draft: Draft,
    record: Mapping[str, Any],
    target: int,
    *,
    resolver: Any,
    ack_all: bool = False,
    status_label: Callable[[Any], str] | None = None,
) -> Plan:
    spec = meta.spec
    existing = name_existing(existing_from_record(spec, record), spec, record, resolver)
    state = FormState(spec, meta.engine(), mode="edit", resolver=resolver, existing=existing)
    answers: dict[str, Any] = dict(draft.answers)
    errors: list[str] = []
    if draft.description_html is not None:
        desc = next((f for f in spec.fields if is_description(f)), None)
        if desc is None:
            errors.append("this form has no description field")
        else:
            answers = {k: v for k, v in answers.items() if spec.find(k) is not desc}
            answers[desc.api_name] = draft.description_html
    answered: set[str] = set()
    for k in answers:
        f = spec.find(k)
        if f is not None:
            answered.add(f.api_name)
    failed: set[str] = set()
    errors += apply_answers(state, answers, failed)
    warnings = list(spec.warnings) + rules_warnings(meta.rules) + _acknowledge(state, draft.ack, ack_all)
    problems = errors + [p for p in state.errors()
                         if p not in {f"{spec.by_name[n].label} is required" for n in failed}]

    top: dict[str, Any] = {}
    custom: dict[str, Any] = {}
    changes: list[tuple[str, str, str]] = []
    ev = state.evaluated
    for f in spec.ordered():
        name = f.api_name
        if f.kind in ("notice", "attachment"):
            continue
        if name not in answered and name not in ev.populated:
            continue
        if name not in ev.visible:
            continue
        new_v = ev.values.get(name)
        old_v = existing.get(name)
        new_e, old_e = _encode(state, f, new_v), _encode(state, f, old_v)
        if _normcmp(f, new_e) == _normcmp(f, old_e):
            continue
        if not f.sent:
            changes.append((f.label, state.display(name, old_v) if old_v is not None else "",
                            state.display(name) + _unsent_suffix(f)))
            continue
        if f.is_custom:
            custom[name] = new_e
        else:
            top[RECORD_KEYS.get(name, name)] = new_e
        changes.append((f.label, state.display(name, old_v) if old_v is not None else "",
                        state.display(name) + _auto_suffix(state, name)))
    body: dict[str, Any] = dict(top)
    if custom:
        body["custom_fields"] = custom
    planning, perr = _resolve_planning(meta, draft)
    files: dict[str, list[str]] = {"attachments": list(draft.attachments)}
    for p in planning:
        files[f"planning:{p['field_id']}"] = p["files"]
    problems += perr + _file_errors([x for paths in files.values() for x in paths])

    flat = {k: v for k, v in body.items() if k != "custom_fields"}
    if status_label is None:
        sf = next((f for f in spec.fields if is_status(f)), None)

        def status_label(v: Any) -> str:
            ch = sf.choice(v) if sf is not None else None
            return ch.label if ch is not None else str(v)
    risks = write_risks("changes", flat, current=dict(record), status_label=status_label)
    review = Review(title=f"Update change {target}")
    review.changes = changes
    review.notify, hint_problems = _hint_lines(state, "changes")
    review.messages = _messages(state)
    review.dropped = _dropped(state)
    review.notes = _notes(state)
    review.attachments = _attachment_rows(files, planning)
    review.risks = risks
    review.errors = problems
    review.warnings = warnings + hint_problems
    review.metadata = meta.as_of()
    plan = Plan("update", meta, state, draft, body, files, planning, review, problems, target=target, risks=risks)
    if not problems and (body or any(files.values()) or planning):
        plan.hash = _hash(body, files, planning, target)
        review.plan_hash = plan.hash
    return plan


# ---------------------------------------------------------------------------
# executing plans


def check_duplicate(plan: Plan, allow_duplicate: bool) -> None:
    if plan.action != "create" or allow_duplicate:
        return
    hit = journal.recent_plan(plan.hash, action="create", hours=DUPLICATE_WINDOW_HOURS)
    if hit is None:
        return
    if hit.get("event") == "done":
        what = f"was already sent at {hit.get('ts')} (created {hit.get('created_id')})"
    else:
        what = f"was started at {hit.get('ts')} with no recorded outcome (it may have been created)"
    raise FlowExit(4, f"this exact plan {what}. Check `fsv journal tail`; pass --allow-duplicate to send it again.")


ATTACH_FLOWS = ("after-create", "before-create")


def resolve_attach_flow(option: str | None = None) -> str:
    """Attachment flow for ``changes create``: the option, else config
    ``changes.attach_flow``, else ``after-create``.

    * ``after-create`` (default): create the change, upload each file, then
      PUT ``{"attachments": [ids]}`` on the new change.
    * ``before-create``: upload each file first (still only after you
      confirm), then include ``attachments: [ids]`` in the create body.

    TODO: neither order is verified against what the web UI sends on create;
    confirm with a browser capture and make the verified one the default.
    """
    from fsv import config

    value = option or str((config.load_config().get("changes") or {}).get("attach_flow") or "") or ATTACH_FLOWS[0]
    if value not in ATTACH_FLOWS:
        raise FlowExit(2, f"attach flow {value!r}: expected one of {', '.join(ATTACH_FLOWS)}")
    return value


def execute(plan: Plan, client: Any, *, duplicate: str = "append", attach_flow: str | None = None) -> dict[str, Any]:
    """Send the plan. Raises PartialFailure when the change was written but a
    later step (upload, planning) failed."""
    from fsv.create import attach_files_to_change, post_change_body, put_change_body, update_planning_field, upload_file

    if not plan.ok:
        raise FlowExit(2, "plan has problems: " + "; ".join(plan.errors))
    steps: list[dict[str, Any]] = []
    uploaded: list[dict[str, Any]] = []
    paths = plan.files.get("attachments") or []
    early = plan.action == "create" and bool(paths) and resolve_attach_flow(attach_flow) == "before-create"
    body = dict(plan.payload or {})
    journal.record_plan("started", plan.hash, action=plan.action, target=plan.target)
    try:
        if early:
            ids = []
            for p in paths:
                att_id = upload_file(p, client)
                ids.append(att_id)
                uploaded.append({"id": att_id, "name": os.path.basename(p)})
            body["attachments"] = ids
        if plan.action == "create":
            result = post_change_body(body, client)
            cid = result.get("id")
        else:
            cid = plan.target
            result = put_change_body(cid, plan.payload or {}, client) if plan.payload else {"id": cid}
    except BaseException as e:
        rejected = isinstance(e, (APIError, RuntimeError)) and not isinstance(e, SessionError)
        if rejected or isinstance(e, SessionError):
            journal.record_plan("failed", plan.hash, action=plan.action, target=plan.target, error=str(e))
        if uploaded:
            err_out = ", ".join(f"{u['id']} ({u['name']})" for u in uploaded)
            sys.stderr.write(f"note: files were uploaded before the change was rejected (unused attachment ids: {err_out})\n")
        raise
    steps.append({"step": plan.action, "ok": True, "id": cid})
    if early:
        steps.append({"step": "attachments", "ok": True, "ids": body["attachments"]})
    try:
        if early:
            pass
        elif paths and plan.action == "create":
            ids = []
            for p in paths:
                att_id = upload_file(p, client)
                ids.append(att_id)
                uploaded.append({"id": att_id, "name": os.path.basename(p)})
            put_change_body(cid, {"attachments": ids}, client)
            steps.append({"step": "attachments", "ok": True, "ids": ids})
        elif paths:
            attach_files_to_change(cid, list(paths), client, duplicate=duplicate)
            steps.append({"step": "attachments", "ok": True, "files": [os.path.basename(p) for p in paths]})
        for p in plan.planning:
            update_planning_field(cid, p["field_id"], description=p["description_html"], file_paths=p["files"],
                                  c=client, duplicate=duplicate, label=p.get("field_label") or p["label"])
            steps.append({"step": f"planning:{p['field_id']}", "ok": True})
    except BaseException as e:
        steps.append({"step": "after", "ok": False, "error": str(e)[:200]})
        journal.record_plan("done", plan.hash, action=plan.action, target=plan.target, created_id=cid, steps=steps,
                            error=str(e))
        raise PartialFailure(cid, uploaded, steps, e) from e
    journal.record_plan("done", plan.hash, action=plan.action, target=plan.target, created_id=cid, steps=steps)
    return {"id": cid, "result": result, "steps": steps, "uploaded": uploaded, "plan_hash": plan.hash}


def review_diff(old: Plan, new: Plan) -> list[str]:
    """What changed between two plans (rows, payload keys)."""
    out = []
    a, b = dict(old.review.rows), dict(new.review.rows)
    for k in dict.fromkeys([*a, *b]):
        if a.get(k) != b.get(k):
            out.append(f"{k}: {a.get(k, '—')} → {b.get(k, '—')}")
    if not out and old.payload != new.payload:
        keys = sorted(set(old.payload or {}) | set(new.payload or {}))
        out += [f"payload {k} changed" for k in keys if (old.payload or {}).get(k) != (new.payload or {}).get(k)]
    if old.errors != new.errors:
        out += [f"problem: {e}" for e in new.errors if e not in old.errors]
    return out


def freshen(plan: Plan, client: Any, rebuild: Callable[[FormMeta], Plan]) -> tuple[Plan, list[str] | None]:
    """Re-fetch the metadata synchronously. -> (plan to send, diff lines if
    the plan changed, else None)."""
    mode = "new" if plan.meta.form_type == "new_form" else "edit"
    fresh = load_meta(client, plan.meta.module, mode, fresh=True, module_id=plan.meta.module_id)
    if not fresh.fingerprint.changed(plan.meta.fingerprint):
        return plan, None
    new = rebuild(fresh)
    if new.hash and new.hash == plan.hash:
        return new, None
    return new, review_diff(plan, new) or ["form metadata changed (fields or rules); plan hash differs"]


# ---------------------------------------------------------------------------
# editor face


def edit_loop(
    build: Callable[[Draft], Plan],
    draft: Draft,
    *,
    title: str,
    requester_text: str | None = None,
    edit_text: Callable[..., Any] | None = None,
    max_rounds: int = 20,
) -> tuple[Draft, Plan]:
    """Open the annotated YAML draft; reopen it, regenerated with the errors
    at the top, until the form is complete. Raises EditorAbort."""
    from fsv.editor import EditorAbort, edit_text as _edit_text

    edit = edit_text or _edit_text
    plan = build(draft)
    errors: list[str] = []
    for _ in range(max_rounds):
        text = template_for(plan, draft, title=title, errors=errors, requester_text=requester_text)
        base = parse_template(text)
        baseline = base.answers
        new = edit(text, parse_template, suffix=".yaml", comment_prefix="#")
        keep = set(draft.answers)
        answers = {k: v for k, v in new.answers.items()
                   if k in keep or _yaml_norm(v) != _yaml_norm(baseline.get(k))}
        desc = new.description_html
        if draft.description_html is None and desc is not None and base.description_html is not None \
                and desc.strip() == base.description_html.strip():
            desc = None  # shown for reference, not edited
        draft = Draft(answers=answers, requester=new.requester, description_html=desc,
                      attachments=new.attachments, planning=new.planning, ack=new.ack, preset=draft.preset,
                      base_dir=new.base_dir)
        plan = build(draft)
        if plan.ok:
            return draft, plan
        errors = list(plan.errors)
    raise EditorAbort("aborted: too many rounds with errors")


def _yaml_norm(v: Any) -> Any:
    if isinstance(v, list):
        return [norm(x) for x in v]
    return None if v in (None, "") else norm(v)


def template_for(plan: Plan, draft: Draft, *, title: str, errors: list[str] | None = None,
                 requester_text: str | None = None, base_dir: str | None = None) -> str:
    spec = plan.state.spec
    unknown: dict[str, Any] = {}
    hidden: dict[str, Any] = {}
    for k, v in draft.answers.items():
        f = spec.find(k)
        if f is None:
            unknown[k] = v
        elif f.api_name not in plan.state.evaluated.visible and not is_status(f) and not is_requester(f):
            hidden[k] = v
    req = requester_text
    if req is None and plan.requester is not None:
        req = plan.requester.email or plan.requester.display
    if req is None:
        req = draft.requester
    return render_template(plan.state, title=title, errors=errors, draft=draft, requester=req, unknown=unknown,
                           hidden=hidden, base_dir=base_dir)


def save_draft(plan: Plan, draft: Draft, path: str) -> str:
    p = Path(os.path.expanduser(path))
    text = template_for(plan, draft, title="fsv change draft (saved)", base_dir=str(p.resolve().parent))
    p.write_text(text, encoding="utf-8")
    return str(p)


def plan_text(plan: Plan) -> str:
    """Plain-text one-liners for logs/notifications."""
    return f"{plan.action} plan {plan.hash[:16]}" if plan.hash else f"{plan.action} plan (incomplete)"


def expect(plan: Plan, expected: str | None) -> None:
    if not expected:
        return
    if not plan.hash:
        raise FlowExit(2, "--expect-plan given but the plan is incomplete: " + "; ".join(plan.errors))
    if not plan_matches(expected, plan.hash):
        raise FlowExit(4, f"plan changed: expected {expected}, computed {plan.hash}. Review again with --dry-run.")


def plain(text: str) -> str:
    return " ".join(plain_text(text).split())


def default_resolver(client: Any) -> ClientResolver:
    return ClientResolver(client)
