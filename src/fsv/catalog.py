"""Service catalog: browse categories and items, load an item's form, build
a reviewed service-request plan, and place it.

Endpoints (the agent catalog, internal API; JSON for any agent):

* ``GET  /api/_/catalog/service-categories?category_hierarchy=true``
* ``GET  /api/_/catalog/service-items?aurora_search=true&page=N&per_page=30``
  (``meta.total_pages``) and ``?filter=frequently_requested``
* ``GET  /api/_/catalog/service-items/<display_id>``: the item form
* ``GET  /api/_/ws/<ws>/business_rules?...module_type=service_request&item_id=<id>``
* ``POST /api/_/attachments``: uploads, only after the review is confirmed
* ``POST /api/_/catalog/service-items/<display_id>/place-request``

A plan is the engine state, the exact request body (uploads shown as
``<upload N: name>`` placeholders, replaced by the attachment ids after the
files are uploaded), the files (name, size, sha256), the review, and a plan
hash over all of it. Nothing is uploaded or sent while building a plan.
"""
from __future__ import annotations

import datetime as dt
import os
import time
import unicodedata
from dataclasses import dataclass, field as dc_field
from typing import Any, Callable, Iterable, Mapping, Sequence

from fsv import config, journal
from fsv.change_flow import FlowExit, check_duplicate, review_diff, sha256_file
from fsv.change_flow import _acknowledge, _dropped, _messages, _notes  # shared review helpers
from fsv.drafts import Draft
from fsv.errors import APIError, SessionError
from fsv.formmeta import RULES_UNAVAILABLE_KEY, RULES_WORKSPACE_KEY, apply_answers, configured_workspace, find_field, load_rules, rules_warnings
from fsv.forms import (
    Field,
    FormError,
    FormSpec,
    FormState,
    LookupValue,
    MetadataFingerprint,
    PayloadError,
    RulesEngine,
    build_sr_payload,
    business_rules_query,
    metadata_fingerprint,
    plain_text,
    plan_hash,
)
from fsv.forms.spec import is_content_type
from fsv.forms.rules import is_empty
from fsv.review import Review

PER_PAGE = 30
MAX_PAGES = 50
SAFETY = "This sends a real service request to the catalog item's fulfilment team."
REQUESTER_MISSING = ("requester is not set: pass --requester EMAIL, add `requester:` to the draft, "
                     "or set defaults.sr_requester in the fsv config")
REQUESTED_FOR_NOTE = "Separate requested-for is available via --requested-for; requester remains unchanged."


def norm(value: Any) -> str:
    """NFC, collapsed whitespace, casefolded (choice values may hold double
    spaces or decomposed characters)."""
    return " ".join(unicodedata.normalize("NFC", str(value)).split()).casefold()


# ---------------------------------------------------------------------------
# reads


def _rows(data: Any, key: str) -> list[dict[str, Any]]:
    if isinstance(data, list):
        return [r for r in data if isinstance(r, dict)]
    if isinstance(data, Mapping):
        v = data.get(key)
        if isinstance(v, list):
            return [r for r in v if isinstance(r, dict)]
    return []


def list_categories(client: Any) -> list[dict[str, Any]]:
    """Categories, flattened; sub-categories (when the server nests them)
    get ``parent_id``."""
    data = client.int_get("catalog/service-categories", {"category_hierarchy": "true"})
    out: list[dict[str, Any]] = []

    def walk(rows: Iterable[Mapping[str, Any]], parent: Any) -> None:
        for r in rows:
            row = {k: v for k, v in r.items() if k not in ("sub_categories", "children")}
            if parent is not None and row.get("parent_id") is None:
                row["parent_id"] = parent
            out.append(row)
            kids = r.get("sub_categories") or r.get("children") or []
            if isinstance(kids, list):
                walk([k for k in kids if isinstance(k, Mapping)], r.get("id"))

    walk(_rows(data, "service_categories"), None)
    return out


def list_items(client: Any, *, popular: bool = False, max_pages: int = MAX_PAGES) -> list[dict[str, Any]]:
    """Every catalog item the agent can request (all pages), or the
    frequently requested ones."""
    if popular:
        return _rows(client.int_get("catalog/service-items", {"filter": "frequently_requested"}), "service_items")
    out: list[dict[str, Any]] = []
    seen: set[Any] = set()
    for page in range(1, max_pages + 1):
        data = client.int_get("catalog/service-items",
                              {"aurora_search": "true", "page": page, "per_page": PER_PAGE})
        rows = _rows(data, "service_items")
        for r in rows:
            key = r.get("id", r.get("display_id"))
            if key not in seen and not r.get("deleted"):
                seen.add(key)
                out.append(r)
        meta = data.get("meta") if isinstance(data, Mapping) else None
        total = (meta or {}).get("total_pages")
        if not rows or (isinstance(total, int) and page >= total) or (total is None and len(rows) < PER_PAGE):
            break
    return out


def resolve_category(categories: Sequence[Mapping[str, Any]], text: str) -> list[dict[str, Any]]:
    """A category by id or name (exact, else a unique part of the name) plus
    its sub-categories. ValueError lists the candidates otherwise."""
    q = str(text).strip()
    hits = [c for c in categories if str(c.get("id")) == q]
    if not hits:
        hits = [c for c in categories if norm(c.get("name", "")) == norm(q)]
    if not hits:
        hits = [c for c in categories if norm(q) in norm(c.get("name", ""))]
    if len(hits) != 1:
        names = ", ".join(str(c.get("name")) for c in (hits or categories)[:15])
        what = "several categories match" if hits else "no category matches"
        raise ValueError(f"{what} {q!r}; categories: {names}")
    picked = [dict(hits[0])]
    ids = {hits[0].get("id")}
    grew = True
    while grew:
        grew = False
        for c in categories:
            if c.get("parent_id") in ids and c.get("id") not in ids:
                ids.add(c.get("id"))
                picked.append(dict(c))
                grew = True
    return picked


def filter_items(items: Sequence[Mapping[str, Any]], *, category_ids: Iterable[Any] | None = None,
                 search: str | None = None, categories: Sequence[Mapping[str, Any]] = ()) -> list[dict[str, Any]]:
    names = {c.get("id"): str(c.get("name") or "") for c in categories}
    ids = None if category_ids is None else {str(i) for i in category_ids}
    words = norm(search).split() if search else []
    out = []
    for it in items:
        if ids is not None and str(it.get("category_id")) not in ids:
            continue
        hay = norm(" ".join(str(x or "") for x in (it.get("name"), it.get("short_description"),
                                                    it.get("category_name") or names.get(it.get("category_id")),
                                                    it.get("display_id"))))
        if words and not all(w in hay for w in words):
            continue
        out.append(dict(it))
    return out


def get_item(client: Any, display_id: Any) -> dict[str, Any]:
    data = client.int_get(f"catalog/service-items/{display_id}")
    item = data.get("service_item", data) if isinstance(data, Mapping) else None
    if not isinstance(item, Mapping) or not item.get("custom_fields") and item.get("display_id") is None:
        raise APIError(200, f"no catalog item {display_id} in the response", "GET", f"catalog/service-items/{display_id}")
    return dict(item)


def _item_ref(ref: Any) -> str:
    return str(ref).strip().removeprefix("#")


def find_item(client: Any, ref: Any) -> dict[str, Any]:
    """The item form for a display id (``77``, ``#77``) or a name (exact,
    case-insensitive; else a unique part of the name or short description)."""
    s = _item_ref(ref)
    if not s:
        raise ValueError("catalog item is empty")
    if s.isdigit():
        return get_item(client, int(s))
    items = list_items(client)
    exact = [i for i in items if norm(i.get("name", "")) == norm(s)]
    hits = exact or filter_items(items, search=s)
    if len(hits) == 1:
        return get_item(client, hits[0].get("display_id"))
    if not hits:
        raise ValueError(f"no catalog item matches {s!r}; list them with `fsv sr catalog --search TEXT`")
    shown = "; ".join(f"#{i.get('display_id')} {i.get('name')}" for i in hits[:10])
    raise ValueError(f"{len(hits)} catalog items match {s!r}: {shown}. Use the item id.")


def place_request(client: Any, display_id: Any, body: Mapping[str, Any]) -> Any:
    return client.int_post(f"catalog/service-items/{display_id}/place-request", dict(body))


def upload(client: Any, path: str) -> int:
    from fsv.create import upload_file

    return upload_file(path, client)


def created_ref(result: Any) -> dict[str, Any]:
    """``{"id", "display"}`` of the request the server created (shape
    tolerant: top level or one level down)."""
    cands: list[Mapping[str, Any]] = []
    if isinstance(result, Mapping):
        cands.append(result)
        cands.extend(v for v in result.values() if isinstance(v, Mapping))
    for c in cands:
        if c.get("id") is not None or c.get("display_id") is not None:
            shown = c.get("human_display_id") or (f"SR-{c['display_id']}" if c.get("display_id") is not None
                                                  else f"SR-{c['id']}")
            return {"id": c.get("id", c.get("display_id")), "display": str(shown)}
    return {"id": None, "display": "(no id in the response)"}


# ---------------------------------------------------------------------------
# item metadata


@dataclass
class ItemMeta:
    """An item form plus the business rules the web loads with it."""

    item: dict[str, Any]
    rules: dict[str, Any]
    workspace_id: Any
    fetched_at: float
    source: str = "server"
    _spec: FormSpec | None = dc_field(default=None, repr=False)

    @property
    def display_id(self) -> Any:
        return self.item.get("display_id")

    @property
    def name(self) -> str:
        return str(self.item.get("name") or "")

    @property
    def spec(self) -> FormSpec:
        if self._spec is None:
            self._spec = FormSpec.from_service_item(self.item)
        return self._spec

    def engine(self) -> RulesEngine:
        return RulesEngine(self.spec, self.rules, module="service_request", item_id=self.item.get("id"),
                           workspace_id=self.workspace_id)

    @property
    def fingerprint(self) -> MetadataFingerprint:
        return metadata_fingerprint(None, self.rules, self.item)

    def as_of(self) -> dict[str, Any]:
        fp = self.fingerprint
        return {
            "updated_at": fp.updated_at,
            "fetched_at": dt.datetime.fromtimestamp(self.fetched_at).astimezone().isoformat(timespec="seconds"),
            "source": self.source,
            "fingerprint": fp.hash[:16],
        }


def load_item_meta(client: Any, ref: Any, *, item: Mapping[str, Any] | None = None) -> ItemMeta:
    """Item form + business rules (always from the server: one GET each)."""
    raw = dict(item) if item is not None else find_item(client, ref)
    ws = configured_workspace() or raw.get("workspace_id")  # None: ask the account, else no workspaces
    rules = load_rules(client, "service_request", ws, item_id=raw.get("id"))
    return ItemMeta(raw, rules, rules.get(RULES_WORKSPACE_KEY, ws), time.time())


# ---------------------------------------------------------------------------
# answers


def _match_choice(f: Field, raw: Any) -> tuple[Any, str | None]:
    """Lenient choice input: exact (NFC, whitespace, case), else a unique
    prefix. Returns (value to set, note) and leaves anything else to the
    engine, which reports it with the valid choices."""
    if not isinstance(raw, str) or f.choice(raw) is not None:
        return raw, None
    key = norm(raw)
    pool = [c for c in f.choices if c.visible]
    exact = [c for c in pool if key in (norm(c.label), norm(c.value))]
    if len(exact) == 1:
        return exact[0].value, None
    pre = [c for c in pool if key and (norm(c.label).startswith(key) or norm(c.value).startswith(key))]
    if len(pre) == 1:
        return pre[0].value, f"{f.label}: {raw!r} matched the choice {pre[0].label!r}"
    return raw, None


def lenient_answers(spec: FormSpec, answers: Mapping[str, Any]) -> tuple[dict[str, Any], list[str]]:
    out: dict[str, Any] = {}
    notes: list[str] = []
    for k, v in answers.items():
        try:
            f = find_field(spec, k)
        except FormError:
            out[k] = v
            continue
        if f.kind == "dropdown":
            v, note = _match_choice(f, v)
            notes += [note] if note else []
        elif f.kind == "multi_select" and isinstance(v, list):
            vals = []
            for x in v:
                x2, note = _match_choice(f, x)
                vals.append(x2)
                notes += [note] if note else []
            v = vals
        out[k] = v
    return out, notes


def file_answers(spec: FormSpec, answers: Mapping[str, Any], base: str | os.PathLike[str] | None) -> dict[str, Any]:
    """File-field answers as paths (``~`` expanded, relative to ``base``)."""
    out = dict(answers)
    for k, v in answers.items():
        try:
            f = find_field(spec, k)
        except FormError:
            continue
        if f.kind != "attachment" or v in (None, "", []):
            continue
        items = v if isinstance(v, list) else [v]
        paths = []
        for p in items:
            s = os.path.expanduser(str(p))
            if base is not None and not os.path.isabs(s):
                s = os.path.join(str(base), s)
            paths.append(s)
        out[k] = paths
    return out


# ---------------------------------------------------------------------------
# plans


def rule_diagnostics(meta: ItemMeta, state: FormState) -> tuple[list[str], list[str]]:
    """Safe-to-display diagnostics and blockers, without rule operands or user data.

    The shared context check inspects all branches, even ones not evaluated due
    to boolean short circuiting. Warnings from the engine are not all fatal.
    """
    engine = state.rules
    warnings = rules_warnings(meta.rules)
    errors: list[str] = []
    if meta.rules.get(RULES_UNAVAILABLE_KEY) and meta.source != "static":
        errors.append("business rules are unavailable; use the web form")
    dependencies = engine.unavailable_dependencies("new")
    raw = list(engine.warnings)
    if dependencies or any("unknown field" in w or "none was given" in w for w in raw):
        errors.append("business rules reference unavailable fields or context; use the web form")
    if any("populate" in w and ("ignored" in w or "not a choice" in w or "unstable" in w) for w in raw):
        errors.append("business rule choice validation is incomplete; use the web form")
    if engine.unsupported:
        errors.append("business rules contain unsupported validation; use the web form")
    if any("unknown condition field" in w for w in dependencies) or any("condition on unknown field" in w for w in raw):
        warnings.append("A business rule condition references an unknown field.")
    if any("unknown action field" in w for w in dependencies) or any(
            "unknown field" in w and "condition on unknown field" not in w for w in raw):
        warnings.append("A business rule action references an unknown target.")
    if any(" context field" in w for w in dependencies) or any("none was given" in w for w in raw):
        warnings.append("Business rule source context is missing or incomplete.")
    if any(not ("unknown field" in w or "none was given" in w) for w in raw):
        warnings.append("A business rule diagnostic was reported; details are withheld to protect context data.")
    if any("unknown type" in w or "no section choice reveals" in w or
           "no choice reveals" in w for w in meta.spec.warnings):
        errors.append("catalog field validation is incomplete; use the web form")
    return warnings, errors


def unsupported_lookup_errors(state: FormState, draft: Draft) -> list[str]:
    """Never silently discard an answered lookup, even if now hidden."""
    answered = {f.api_name for f in state.spec.fields
                if f.kind == "lookup" and not f.sent and
                (not is_empty(state.values.get(f.api_name)) or
                 not is_empty(state.evaluated.values.get(f.api_name)))}
    for key, value in draft.answers.items():
        if is_empty(value):
            continue
        try:
            f = find_field(state.spec, key)
        except FormError:
            continue
        if f.kind == "lookup" and not f.sent:
            answered.add(f.api_name)
    return [f"{f.label}: catalog lookup submission is unverified; use the web form"
            for f in state.spec.ordered() if f.api_name in answered]


@dataclass
class Upload:
    placeholder: str
    path: str
    target: str  # file field api name, or "attachments"
    label: str


@dataclass
class SRPlan:
    meta: ItemMeta
    state: FormState
    draft: Draft
    payload: dict[str, Any] | None
    uploads: list[Upload]
    review: Review
    errors: list[str]
    requester: LookupValue | None = None
    identity: dict[str, Any] = dc_field(default_factory=dict)
    hash: str = ""
    action: str = "create"  # journal / duplicate-check compatible with changes

    @property
    def ok(self) -> bool:
        return not self.errors

    @property
    def target(self) -> str:
        return f"catalog-item/{self.meta.display_id}"

    @property
    def files(self) -> dict[str, list[str]]:
        out: dict[str, list[str]] = {}
        for u in self.uploads:
            out.setdefault(u.target, []).append(u.path)
        return out

    def to_json(self) -> dict[str, Any]:
        return {
            "plan_hash": self.hash or None,
            "action": "place-request",
            "item": {"id": self.meta.item.get("id"), "display_id": self.meta.display_id, "name": self.meta.name},
            "endpoint": f"POST /api/_/catalog/service-items/{self.meta.display_id}/place-request",
            "requester": None if self.requester is None else
            {"id": self.requester.id, "name": self.requester.display, "email": self.requester.email},
            "recipient_identity": self.identity,
            "payload": self.payload if self.ok else None,  # never a partial payload
            "uploads": [{"placeholder": u.placeholder, "target": u.label, **_digest(u.path)}
                        for u in self.uploads if os.path.isfile(u.path)],
            "safety": SAFETY,
            "review": self.review.to_dict(),
            "warnings": self.review.warnings,
            "errors": self.errors,
        }


def _digest(path: str) -> dict[str, Any]:
    d = sha256_file(path)
    d.pop("path", None)
    return d


def requester_line(req: LookupValue) -> str:
    who = req.display or req.email or str(req.id)
    return f"Requester: {who} (approvals route via {who}’s manager)"


def build_sr_plan(meta: ItemMeta, draft: Draft, *, resolver: Any, requester: LookupValue | None,
                  ack_all: bool = False, requested_for: LookupValue | None = None,
                  cc_emails: tuple[str, ...] = (), cc_ids: tuple[Any, ...] = ()) -> SRPlan:
    spec = meta.spec
    item = spec.item
    assert item is not None
    engine = meta.engine()
    if requested_for is not None:
        # Metadata may contain the actor's context, not the selected person's.
        # Rules needing requested-for attributes must fail closed until reloaded
        # for that exact identity; never evaluate them against the wrong user.
        from dataclasses import replace
        engine.context = replace(engine.context, requested_for=None)
    state = FormState(spec, engine, mode="new", resolver=resolver)
    errors: list[str] = []
    warnings: list[str] = list(spec.warnings) + [REQUESTED_FOR_NOTE]
    if draft.description_html is not None:
        errors.append("description_md/description_html: catalog items have no description field; "
                      "answer the item's own fields")
    if draft.planning:
        errors.append("planning: is for changes; service requests have no planning fields")
    answers, notes = lenient_answers(spec, draft.answers)
    warnings += notes
    failed: set[str] = set()
    errors += apply_answers(state, answers, failed)
    warnings += _acknowledge(state, draft.ack, ack_all)

    uploads: list[Upload] = []
    field_ph: dict[str, list[str]] = {}
    ev = state.evaluated
    for name in ev.visible:
        f = spec.by_name[name]
        v = ev.values.get(name)
        if f.kind != "attachment" or v in (None, (), [], ""):
            continue
        for p in v:
            if not isinstance(p, str):
                errors.append(f"{f.label}: give file paths (attachment ids are assigned after upload)")
                continue
            ph = f"<upload {len(uploads) + 1}: {os.path.basename(p)}>"
            uploads.append(Upload(ph, p, name, f.label))
            field_ph.setdefault(name, []).append(ph)
    general_ph: list[str] = []
    for p in draft.attachments:
        ph = f"<upload {len(uploads) + 1}: {os.path.basename(p)}>"
        uploads.append(Upload(ph, p, "attachments", "Other attachments"))
        general_ph.append(ph)
    errors += [f"file not found: {u.path}" for u in uploads if not os.path.isfile(u.path)]

    skip = {f"{spec.by_name[n].label} is required" for n in failed}  # one report per field
    problems = list(errors) + [p for p in state.errors() if p not in skip]
    rule_warnings, rule_errors = rule_diagnostics(meta, state)
    warnings += rule_warnings
    problems += rule_errors
    problems += unsupported_lookup_errors(state, draft)
    need = ev.attachments_required
    if (item.attachment_mandatory if need is None else need) and not draft.attachments:
        problems.append("this item requires at least one general attachment: add `attachments: [FILE]` "
                        "to the draft or pass --attach FILE")
    if draft.attachments and not item.allow_attachments:
        problems.append("this item does not accept general attachments")
    if requester is None:
        problems.insert(0, REQUESTER_MISSING)
    elif not requester.email:
        problems.insert(0, f"requester {requester} has no email address")
    from fsv.ticket_notes import _EMAIL
    if requested_for is not None and (not isinstance(requested_for, LookupValue) or
                                      not isinstance(requested_for.email, str) or not _EMAIL.fullmatch(requested_for.email)):
        problems.append("requested-for must resolve to a user with a plain email address")
    valid_cc = isinstance(cc_emails, (list, tuple)) and all(isinstance(v, str) and _EMAIL.fullmatch(v) for v in cc_emails)
    if not valid_cc:
        problems.append("CC recipients must be plain email addresses")

    payload = None
    if requester is not None and requester.email:
        for incomplete in ((False, True) if not problems else (True,)):
            try:
                payload = build_sr_payload(item, state, requester.email, field_attachments=field_ph,
                                           attachments=general_ph if item.allow_attachments else [],
                                           allow_incomplete=incomplete)
                if requested_for is not None and isinstance(requested_for, LookupValue) and requested_for.email:
                    payload["requested_for"] = requested_for.email
                if cc_emails and valid_cc:
                    payload["cc_emails"] = list(cc_emails)
                break
            except PayloadError as e:
                problems += [p for p in e.problems if p not in problems]
            except FormError as e:
                problems.append(str(e))
                break

    review = Review(title=f"New service request: {item.name}")
    review.rows.append(("Catalog item", f"{item.name} (#{item.display_id})"
                                        + (f" · {item.category_name}" if item.category_name else "")))
    review.rows.append(("Requester", str(requester) if requester is not None else "(not set)"))
    if requested_for is not None:
        review.rows.append(("Requested for", f"{requested_for} (user ID {requested_for.id})" if isinstance(requested_for, LookupValue) else str(requested_for)))
    if valid_cc and cc_emails:
        shown_cc = ", ".join(f"{email} (user ID {id_})" for email, id_ in zip(cc_emails, cc_ids)) if cc_ids and len(cc_ids) == len(cc_emails) else ", ".join(cc_emails)
        review.rows.append(("CC", shown_cc))
    subject = item.render_subject({k: v for k, v in ev.values.items()}) if item.subject_template else ""
    if subject:
        review.rows.append(("Subject (item template)", subject))
    for name in state.payload_values():
        f = spec.by_name[name]
        if f.kind == "attachment":
            text = ", ".join(os.path.basename(str(p)) for p in ev.values.get(name) or ())
        else:
            text = state.display(name) + (" (auto)" if name in ev.populated else "")
        review.rows.append((f.label, text))
    review.notify = [SAFETY] + ([requester_line(requester)] if requester is not None else [])
    review.messages = _messages(state)
    review.dropped = _dropped(state)
    review.notes = _notes(state)
    review.attachments = [{"target": u.label, **sha256_file(u.path)} for u in uploads if os.path.isfile(u.path)]
    review.errors = problems
    review.warnings = warnings
    review.metadata = meta.as_of()
    identity: dict[str, Any] = {}
    if requested_for is not None and isinstance(requested_for, LookupValue):
        identity["requested_for_id"] = requested_for.id
    if cc_ids:
        if len(cc_ids) != len(cc_emails):
            problems.append("CC identity count differs from addresses")
        identity["cc_ids"] = list(cc_ids)
    if identity:
        identity["tenant"] = config.DOMAIN
    plan = SRPlan(meta, state, draft, payload, uploads, review, problems, requester=requester, identity=identity)
    if payload is not None and not problems:
        hashed = {"request": payload, "target": plan.target}
        if identity:
            hashed["recipient_identity"] = identity
        plan.hash = plan_hash(hashed, plan.files)
        review.plan_hash = plan.hash
    return plan


# ---------------------------------------------------------------------------
# sending


class SendFailure(Exception):
    """Uploads or the place-request failed; ``uploaded`` lists every
    attachment id already on the server (unused when nothing was placed)."""

    def __init__(self, step: str, uploaded: list[dict[str, Any]], error: BaseException) -> None:
        self.step = step
        self.uploaded = uploaded
        self.error = error
        ids = ", ".join(f"{u['id']} ({u['name']})" for u in uploaded) or "none"
        super().__init__(f"{step} failed: {error}. Uploaded attachment ids: {ids}")


def _substitute(value: Any, ids: Mapping[str, Any]) -> Any:
    if isinstance(value, str) and value in ids:
        return ids[value]
    if isinstance(value, list):
        return [_substitute(v, ids) for v in value]
    if isinstance(value, dict):
        return {k: _substitute(v, ids) for k, v in value.items()}
    return value


def execute(plan: SRPlan, client: Any, *, upload_fn: Callable[[Any, str], int] = upload,
            allow_duplicate: bool = False) -> dict[str, Any]:
    """Upload the files, then place the request (journaled under the plan
    hash). Call only after the review was confirmed."""
    if not plan.ok or plan.payload is None or not plan.hash:
        raise FlowExit(2, "plan has problems: " + "; ".join(plan.errors))
    if plan.identity and plan.identity.get("tenant") != config.DOMAIN:
        raise FlowExit(2, "tenant changed since review; nothing was sent")
    duplicate_check(plan, allow_duplicate)
    _, preflight = rule_diagnostics(plan.meta, plan.state)
    preflight += unsupported_lookup_errors(plan.state, plan.draft)
    preflight += plan.state.errors()
    item = plan.meta.spec.item
    if item is None:
        preflight.append("not a service-item form")
    else:
        need = plan.state.evaluated.attachments_required
        if (item.attachment_mandatory if need is None else need) and not plan.draft.attachments:
            preflight.append("this item requires at least one general attachment")
        if plan.draft.attachments and not item.allow_attachments:
            preflight.append("this item does not accept general attachments")
    if preflight:
        raise FlowExit(2, "plan has problems: " + "; ".join(preflight))
    try:
        hashed = {"request": plan.payload, "target": plan.target}
        if plan.identity:
            hashed["recipient_identity"] = plan.identity
        current_hash = plan_hash(hashed, plan.files)
    except OSError as e:
        raise FlowExit(2, f"cannot verify reviewed files: {e}") from None
    if current_hash != plan.hash:
        raise FlowExit(2, "the reviewed payload or files changed; review again")
    before = len(journal.read())
    journal.record_plan("started", plan.hash, action=plan.action, target=plan.target)
    entries = journal.read()
    if len(entries) <= before or entries[-1].get("kind") != f"plan:{plan.action}" or entries[-1].get("plan") != plan.hash or entries[-1].get("event") != "started":
        raise FlowExit(5, "could not persist the request journal; nothing was uploaded or placed")
    uploaded: list[dict[str, Any]] = []
    ids: dict[str, Any] = {}
    try:
        for u in plan.uploads:
            att_id = upload_fn(client, u.path)
            ids[u.placeholder] = att_id
            uploaded.append({"id": att_id, "name": os.path.basename(u.path), "target": u.label})
    except BaseException as e:
        journal.record_plan("failed", plan.hash, action=plan.action, target=plan.target,
                            error=f"upload: {e}")  # nothing was placed
        raise SendFailure("upload", uploaded, e) from e
    body = _substitute(plan.payload, ids)
    missing = [p for p in ids if p in str(body)]
    if missing:  # pragma: no cover - placeholders are always replaced
        raise SendFailure("place-request", uploaded, RuntimeError(f"unreplaced upload placeholders: {missing}"))
    try:
        result = place_request(client, plan.meta.display_id, body)
    except BaseException as e:
        if isinstance(e, APIError) and 400 <= e.status < 500 and e.status not in (403, 408, 429):
            journal.record_plan("failed", plan.hash, action=plan.action, target=plan.target, error=f"HTTP {e.status}")
        # Ambiguous failures retain the started guard: never assume a 403,
        # timeout, throttling or server error means the request was not placed.
        raise SendFailure("place-request", uploaded, e) from e
    ref = created_ref(result)
    if ref["id"] is None:
        raise SendFailure("place-request", uploaded, RuntimeError("response has no exact request ID; outcome unknown"))
    steps = [{"step": "upload", "ok": True, "ids": [u["id"] for u in uploaded]},
             {"step": "place-request", "ok": True, "id": ref["id"]}]
    journal.record_plan("done", plan.hash, action=plan.action, target=plan.target, created_id=ref["id"], steps=steps)
    verification: dict[str, Any] = {"verified": False, "fields": {}}
    if plan.identity:
        try:
            raw = client.int_get(f"tickets/{ref['id']}")
            row = raw.get("ticket", raw) if isinstance(raw, dict) else None
            if not isinstance(row, dict) or str(row.get("id")) != str(ref["id"]):
                raise ValueError("identity mismatch")
            fields = {}
            for key in ("email", "requested_for", "cc_emails"):
                if key in body:
                    fields[key] = ("unavailable" if key not in row else
                                   "matched" if row[key] == body[key] else "mismatched")
            verification = {"verified": bool(fields) and all(v == "matched" for v in fields.values()), "fields": fields}
        except Exception:
            verification["warning"] = "Exact ticket read-back failed; do not resend."
        if not verification["verified"]:
            verification["warning"] = "Request recipient fields were not fully verified; do not resend."
    return {"created": ref, "result": result, "uploaded": uploaded, "body": body, "plan_hash": plan.hash,
            "steps": steps, "verification": verification}


def freshen(plan: SRPlan, client: Any, rebuild: Callable[[ItemMeta], SRPlan]) -> tuple[SRPlan, list[str] | None]:
    """Re-fetch the item and its rules right before sending. -> (plan to
    send, diff lines when the plan changed, else None)."""
    fresh = load_item_meta(client, plan.meta.display_id)
    if not fresh.fingerprint.changed(plan.meta.fingerprint):
        return plan, None
    new = rebuild(fresh)
    if new.hash and new.hash == plan.hash:
        return new, None
    return new, review_diff(plan, new) or ["catalog item or rules changed; plan hash differs"]  # type: ignore[arg-type]


def duplicate_check(plan: SRPlan, allow_duplicate: bool) -> None:
    events = [r for r in journal.read() if r.get("kind") == "plan:create" and r.get("plan") == plan.hash]
    if events and events[-1].get("event") == "started":
        raise FlowExit(4, "previous request outcome is unknown; inspect the server and journal before retry")
    check_duplicate(plan, allow_duplicate)  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# requested items (read side)


def label_map(defs: Iterable[Mapping[str, Any]]) -> dict[str, str]:
    """name -> label over item field definitions (any nesting the APIs use:
    ``fields``, ``nested_fields``, ``sections[].fields``)."""
    import html

    out: dict[str, str] = {}

    def walk(rows: Iterable[Any]) -> None:
        for r in rows or []:
            if not isinstance(r, Mapping):
                continue
            name, label = r.get("name"), r.get("label")
            if name and label and str(r.get("type")) not in ("1024",) \
                    and not is_content_type(str(r.get("field_type") or "")):
                out.setdefault(str(name), html.unescape(str(label)))
            walk(r.get("fields") or [])
            walk(r.get("nested_fields") or [])
            for s in r.get("sections") or []:
                if isinstance(s, Mapping):
                    walk(s.get("fields") or [])

    walk(defs)
    return out


def item_labels(client: Any, requested: Mapping[str, Any]) -> dict[str, str]:
    """Labels for a requested item's values: its embedded field definitions,
    else (or for names they miss) the catalog item form."""
    meta = requested.get("item") or {}
    labels = label_map(meta.get("custom_fields") or [])
    values = requested.get("custom_fields") or {}
    did = meta.get("display_id") or requested.get("service_item_display_id")
    if did is not None and any(k not in labels for k in values):
        try:
            spec = FormSpec.from_service_item(get_item(client, did))
            for f in spec.fields:
                if f.kind != "notice":
                    labels.setdefault(f.api_name, f.label)
        except (APIError, ValueError):
            pass
    return labels


def value_text(value: Any) -> str:
    if isinstance(value, list):
        return ", ".join(value_text(v) for v in value)
    if isinstance(value, Mapping):
        for k in ("name", "value", "label", "email"):
            if value.get(k):
                return str(value[k])
        return str(dict(value))
    if isinstance(value, bool):
        return "Yes" if value else "No"
    return " ".join(plain_text(str(value)).split()) if isinstance(value, str) and "<" in value else str(value)


def is_file_value(value: Any) -> bool:
    return isinstance(value, list) and bool(value) and all(
        isinstance(v, int) or (isinstance(v, Mapping) and (v.get("attachment_url") or v.get("canonical_url")
                                                           or v.get("name") and v.get("id") is not None))
        for v in value)


def safe_target(directory: str | os.PathLike[str], name: str) -> "os.PathLike[str]":
    """The path to write a server-supplied file name to inside ``directory``.
    Rejects (ValueError) empty names, ``..``, path separators, absolute or
    drive paths, control characters, and anything resolving outside."""
    from pathlib import Path
    import re

    n = str(name or "")
    if not n.strip():
        raise ValueError("empty file name")
    if ".." in n:
        raise ValueError(f"unsafe file name {n!r}: contains '..'")
    if "/" in n or "\\" in n:
        raise ValueError(f"unsafe file name {n!r}: contains a path separator")
    if os.path.isabs(n) or re.match(r"^[A-Za-z]:", n):
        raise ValueError(f"unsafe file name {n!r}: absolute path")
    if any(ord(ch) < 32 or ord(ch) == 127 for ch in n):
        raise ValueError(f"unsafe file name {n!r}: control characters")
    root = Path(directory).expanduser().resolve()
    target = (root / n).resolve()
    if target.parent != root:
        raise ValueError(f"unsafe file name {n!r}: resolves outside {root}")
    return target


def domain_url(path: str) -> str:
    return f"https://{config.DOMAIN}{path}"
