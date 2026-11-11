"""``FormSession``: the form UI's model, independent of Textual.

It wraps one live :class:`fsv.forms.FormState` (the same engine, rules and
replay order the plan builders use), the draft being filled in, the
lookups, and the mode. Every widget change goes through :meth:`set`, which
returns the rule notes to show ("+ Staging Start (required) appeared
because Cloud Only = No"). :meth:`build_plan` hands the draft to the
existing plan builders (``change_flow.build_create_plan`` /
``build_update_plan`` / ``catalog.build_sr_plan``), so the review, payload,
plan hash, journal, duplicate check, freshness check and upload-after-
confirm are exactly the ones the YAML and ``--set`` faces use.

Values are stored in the draft already parsed (choice values, aware
date-times, ``LookupValue``), so building the plan never re-resolves a
lookup over the network.
"""
from __future__ import annotations

import copy
import datetime as dt
import os
import shlex
from dataclasses import dataclass, field as dc_field
from pathlib import Path
from typing import Any, Callable

from fsv import change_flow as flow
from fsv import config
from fsv.drafts import Draft
from fsv.formmeta import (
    ClientResolver,
    existing_from_record,
    is_description,
    is_requester,
    is_status,
    name_existing,
    resolve_requester,
    rules_warnings,
)
from fsv.forms import Field, FormError, FormState, LookupValue, plain_text
from fsv.forms.spec import is_planning_type
from fsv.forms.tz import zone

from . import lookups, when
from .fuzzy import fuzzy_filter

WIDGET_FOR_KIND = {
    "text": "text", "textarea": "textarea", "richtext": "richtext", "number": "number",
    "checkbox": "checkbox", "dropdown": "choice", "department": "choice", "nested": "choice",
    "multi_select": "multi", "lookup_user": "lookup", "lookup_group": "lookup", "lookup_agent": "lookup",
    "lookup": "lookup", "date": "date", "datetime": "datetime", "attachment": "file", "notice": "notice",
}
ENTITY_FOR_KIND = {"lookup_user": "user", "lookup_group": "group", "lookup_agent": "agent"}
REQUESTER = "__requester__"
ATTACHMENTS = "__attachments__"
PLANNING = "planning:"
MAX_EFFECT_CHOICES = 30


def _plain(text: str, limit: int = 600) -> str:
    s = " ".join(plain_text(text or "").split())
    return s if len(s) <= limit else s[: limit - 1] + "…"


@dataclass
class FormItem:
    """One row of the form (a spec field or a form-level input)."""

    key: str
    label: str
    widget: str  # text number textarea richtext checkbox choice multi lookup date datetime file notice
    field: Field | None = None
    entity: str | None = None
    help: str = ""
    placeholder: str = ""
    allowed_types: tuple[str, ...] = ()
    max_files: int | None = None

    @property
    def synthetic(self) -> bool:
        return self.field is None


@dataclass
class ItemView:
    visible: bool
    required: bool
    disabled: bool = False
    disabled_reason: str = ""
    value: Any = None
    text: str = ""
    error: str = ""
    note: str = ""


@dataclass
class Suggestion:
    label: str
    detail: str
    value: Any
    selected: bool = False


@dataclass
class Outcome:
    notes: list[str] = dc_field(default_factory=list)
    error: str = ""


@dataclass
class FormResult:
    """What the form screen dismisses with."""

    action: str  # submit | save_draft | cancel
    state: FormState | None = None
    plan: Any = None
    saved_path: str | None = None
    session: "FormSession | None" = None


def _fork(state: FormState) -> FormState:
    """A throw-away copy to try a value (``set`` replaces ``evaluated``)."""
    s = copy.copy(state)
    s.values = dict(state.values)
    s.sources = dict(state.sources)
    s._seq = list(state._seq)
    s._cleared = set(state._cleared)
    s._events = list(state._events)
    s.acknowledged = set(state.acknowledged)
    return s


def check_files(raw: Any, *, allowed: tuple[str, ...] = (), max_files: int | None = None,
                max_bytes: int | None = None) -> tuple[list[str], str]:
    """Paths from ``a.pdf; b.png`` (or a list) -> (absolute paths, error)."""
    items = raw if isinstance(raw, (list, tuple)) else str(raw or "").replace("\n", ";").split(";")
    paths = [os.path.abspath(os.path.expanduser(str(p).strip())) for p in items if str(p).strip()]
    if max_files is not None and len(paths) > max_files:
        return paths, f"at most {max_files} file(s)"
    norm_types = {t.lower() if t.startswith(".") else "." + t.lower() for t in allowed}
    for p in paths:
        name = os.path.basename(p)
        if not os.path.exists(p):
            return paths, f"{name}: file not found"
        if not os.path.isfile(p):
            return paths, f"{name}: not a file"
        if norm_types and os.path.splitext(p)[1].lower() not in norm_types:
            return paths, f"{name}: allowed file types are {', '.join(sorted(norm_types))}"
        size = os.path.getsize(p)
        if size == 0:
            return paths, f"{name}: the file is empty"
        if max_bytes is not None and size > max_bytes:
            return paths, f"{name}: {size / 1e6:.1f} MB is over the {max_bytes / 1e6:.0f} MB limit"
    return paths, ""


def human_size(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:,} {unit}" if unit == "B" else f"{n:.1f} {unit}"
        n /= 1024  # type: ignore[assignment]
    return str(n)  # pragma: no cover


def _max_bytes() -> int | None:
    v = (config.load_config().get("forms") or {}).get("max_file_mb")
    try:
        return int(float(v) * 1_000_000) if v not in (None, "") else None
    except (TypeError, ValueError):
        return None


class FormSession:
    """The form model. Build with :func:`change_session` / :func:`sr_session`."""

    def __init__(self, *, kind: str, mode: str, meta: Any, client: Any, title: str, record: Any = None,
                 target: Any = None, requester: LookupValue | None = None, requester_note: str = "",
                 dry_run: bool = False, status_label: Callable[[Any], str] | None = None,
                 resolver: Any = None) -> None:
        if kind not in ("change", "sr", "ticket") or mode not in ("new", "edit") or (kind == "ticket" and mode != "new"):
            raise ValueError("kind must be change|sr|ticket and mode new|edit (ticket: new only)")
        self.kind, self.mode, self.meta, self.client = kind, mode, meta, client
        self.title = title
        self.record = record or {}
        self.target = target
        self.dry_run = dry_run
        self.status_label = status_label
        self.resolver = resolver or ClientResolver(client)
        self.boot = lookups.AgentsGroups(client)
        self.draft = Draft()
        self.requester = requester
        self.requester_note = requester_note
        self.touched: set[str] = set()
        self.show_all_errors = False
        self.errors_by_key: dict[str, str] = {}
        self.ack = False
        self.plan: Any = None
        self.saved_path: str | None = None
        self._effects: dict[Any, list[str]] = {}
        spec = meta.spec
        self.existing: dict[str, Any] = {}
        if mode == "edit":
            self.existing = name_existing(existing_from_record(spec, self.record), spec, self.record, self.resolver)
        self.state = FormState(spec, meta.engine(), mode=mode, resolver=self.resolver,
                               existing=self.existing or None)
        self.tz, self.tz_source = self._timezone()
        req = self._requester_field()
        if req is not None and requester is not None and self._requester_is_special():
            try:
                self.state.set(req.api_name, requester)
            except FormError:
                pass
        if requester is not None:
            self.draft.requester = requester.email or requester.display
        desc = self._description_field()
        self._desc_prefill = ""
        self._desc_md: str | None = None
        if desc is not None and mode == "edit":
            self._desc_prefill = plain_text(str(self.existing.get(desc.api_name) or ""))
        self.items: list[FormItem] = self._build_items()
        self.by_key = {it.key: it for it in self.items}
        self.load_notes: list[str] = self._preselect_all()
        self.load_notes = rules_warnings(meta.rules) + self.load_notes
        if requester_note:
            self.load_notes.insert(0, requester_note)

    # -- structure --------------------------------------------------------
    @property
    def spec(self) -> Any:
        return self.meta.spec

    def _requester_field(self) -> Field | None:
        return next((f for f in self.spec.fields if is_requester(f)), None)

    def _description_field(self) -> Field | None:
        return next((f for f in self.spec.fields if is_description(f)), None) if self.kind == "change" else None

    def _requester_is_special(self) -> bool:
        """The plan builder takes the requester separately (new change, SR)."""
        return self.kind == "sr" or self.mode == "new"

    def _timezone(self) -> tuple[dt.tzinfo, str]:
        tz, source = self.state.tz, self.state.tz_source
        if source != "system time zone" or self.meta.rules.get("user"):
            return tz, source
        check = getattr(self.client, "check_session", None)  # agent profile (portable fallback)
        if check is None:
            return tz, source
        try:
            me = check() or {}
        except Exception:
            return tz, source
        name = me.get("time_zone") if isinstance(me, dict) else None
        found = zone(name)
        return (found, f"Freshservice user time zone ({name})") if found is not None else (tz, source)

    def _build_items(self) -> list[FormItem]:
        out: list[FormItem] = []
        if self.kind == "sr" or (self.kind == "ticket" and self._requester_field() is None):
            out.append(FormItem(REQUESTER, "Requester", "lookup", entity="user",
                                help="Select the requester by name or email."))
        for f in self.spec.ordered():
            if self.kind in ("change", "ticket") and self.mode == "new" and is_status(f):
                continue  # status on create is the form default (shown on the review)
            widget = WIDGET_FOR_KIND.get(f.kind, "text")
            entity = ENTITY_FOR_KIND.get(f.kind, f.lookup_entity or "lookup") if widget == "lookup" else None
            out.append(FormItem(f.api_name, f.label, widget, field=f, entity=entity, help=_plain(f.help),
                                placeholder=_plain(f.placeholder, 80), allowed_types=tuple(f.allowed_file_types),
                                max_files=f.max_files))
        if self.kind == "change":
            out.append(FormItem(ATTACHMENTS, "Attachments", "file",
                                help="Files for the change's Attachments section; uploaded only after you "
                                     "confirm. Separate several with ';'."))
            for f in self.spec.fields:  # every planning type, keyed by label (names differ per account)
                if is_planning_type(f.field_type):
                    out.append(FormItem(PLANNING + f.label, f"{f.label} (planning files)", "file",
                                        help=(_plain(f.help) + " " if f.help else "")
                                        + "Planning field files, attached after the change is saved."))
        else:
            item = self.spec.item
            if item is not None and item.allow_attachments:
                out.append(FormItem(ATTACHMENTS, "Attachments", "file",
                                    help="General files for the request; uploaded only after you confirm."))
        return out

    # -- reading ----------------------------------------------------------
    def view(self, key: str) -> ItemView:
        it = self.by_key[key]
        ev = self.state.evaluated
        if key == REQUESTER:
            v = ItemView(True, True, value=self.requester, text=str(self.requester or ""))
        elif key == ATTACHMENTS:
            need = False
            if self.kind == "sr" and self.spec.item is not None:
                need = ev.attachments_required if ev.attachments_required is not None \
                    else self.spec.item.attachment_mandatory
            v = ItemView(True, bool(need), value=list(self.draft.attachments),
                         text="; ".join(self.draft.attachments))
        elif key.startswith(PLANNING):
            files = (self.draft.planning.get(key[len(PLANNING):]) or {}).get("files") or []
            v = ItemView(True, False, value=list(files), text="; ".join(files))
        else:
            f = it.field
            assert f is not None
            name = f.api_name
            visible = name in ev.visible
            value = ev.values.get(name)
            if is_requester(f) and self._requester_is_special():
                value = self.requester
            v = ItemView(visible, name in ev.required, value=value, text=self.text_for(it, value))
            if name in ev.disabled:
                d = ev.decision(name, "disabled")
                v.disabled, v.disabled_reason = True, (d.describe() if d else "read-only")
            r = ev.restricted.get(name)
            if r is not None:
                offered = [c.label for c in self.state.choices(name) if c.visible and c.label]
                v.note = f"choices limited by rule {r.rule_name or r.rule_id!s}: " + (", ".join(offered[:8]) or "none")
            if name in ev.unavailable:
                v.error = next((e for e in self.state.unavailable_errors() if e.startswith(f.label)), "not offered now")
        if key in self.errors_by_key:
            v.error = self.errors_by_key[key]
        elif not v.error and v.visible and v.required and self._empty(it, v.value) \
                and (key in self.touched or self.show_all_errors):
            v.error = "required"
        return v

    @staticmethod
    def _empty(it: FormItem, value: Any) -> bool:
        if it.widget == "checkbox":
            return value is not True
        return value in (None, "", (), [])

    def text_for(self, it: FormItem, value: Any) -> str:
        if value in (None, "", (), []):
            return ""
        if it.widget in ("date", "datetime"):
            return when.to_text(value, self.tz)
        if it.widget == "choice" and it.field is not None:
            return self.state.display(it.field.api_name, value)
        if it.widget == "multi" and it.field is not None:
            return self.state.display(it.field.api_name, value)
        if it.widget == "file":
            return "; ".join(str(p) for p in value)
        if it.widget == "richtext" and self.kind == "change":
            return self._desc_text
        if isinstance(value, LookupValue):
            return str(value)
        if isinstance(value, tuple):
            return ", ".join(str(x) for x in value)
        return str(value)

    @property
    def _desc_text(self) -> str:
        return self._desc_md if self._desc_md is not None else self._desc_prefill

    def visible_keys(self) -> list[str]:
        return [it.key for it in self.items if self.view(it.key).visible]

    def problems(self) -> list[str]:
        """Why Submit is not available yet (blocking messages are
        acknowledged on the review, so they are not listed)."""
        out: list[str] = []
        special = self._requester_is_special()
        if special and self.requester is None and (self.kind in ("sr", "ticket") or self._requester_field() is not None):
            out.append("Requester is required")
        req = self._requester_field()
        for e in self.state.errors():
            if e.startswith("blocked by rule"):
                continue
            if special and req is not None and e == f"{req.label} is required":
                continue
            out.append(e)
        if self.kind == "sr" and ATTACHMENTS in self.by_key and self.view(ATTACHMENTS).required \
                and not self.draft.attachments:
            out.append("Attachments: at least one file is required")
        for k, e in self.errors_by_key.items():
            if self.view(k).visible:
                out.append(f"{self.by_key[k].label}: {e}")
        return list(dict.fromkeys(out))

    def missing_keys(self) -> list[str]:
        out = []
        for it in self.items:
            v = self.view(it.key)
            if v.visible and v.required and self._empty(it, v.value):
                out.append(it.key)
        return out

    def help_for(self, key: str) -> dict[str, Any]:
        it = self.by_key[key]
        v = self.view(key)
        info: dict[str, Any] = {"label": it.label, "required": v.required, "help": it.help,
                                "placeholder": it.placeholder, "choices": [], "effects": [], "why": [],
                                "kind": it.widget, "disabled": v.disabled_reason, "note": v.note}
        if it.widget in ("choice", "multi") and it.field is not None:
            info["choices"] = [(c.label, c.description) for c in self._pool(it)]
        if it.widget in ("date", "datetime"):
            info["why"].append(f"time zone: {self.tz_source}" if it.widget == "datetime" else "date only")
        if it.field is not None:
            for e in self.state.explain([it.field.api_name]):
                if e.decision is not None:
                    info["why"].append(str(e))
        info["effects"] = self.effects(key)
        return info

    def _pool(self, it: FormItem) -> list[Any]:
        assert it.field is not None
        try:
            pool = self.state.choices(it.field.api_name)
        except FormError:
            return []
        return [c for c in pool if c.visible and c.label and str(c.value) != ""]

    def suggestions(self, key: str, query: str) -> tuple[list[Suggestion], str]:
        """(suggestions, note). May call the network for people lookups;
        call it from a worker thread."""
        it = self.by_key[key]
        if it.widget in ("choice", "multi"):
            f = it.field
            assert f is not None
            pool = self._pool(it)
            if not pool and f.is_nested_level:
                parent = self.spec.by_name.get(f.parent or "")
                return [], f"choose {parent.label if parent else 'the level above'} first"
            current = self.state.evaluated.values.get(f.api_name)
            chosen = set(current) if isinstance(current, tuple) else {current}
            idx = fuzzy_filter(query, [(c.label, c.description) for c in pool])
            return [Suggestion(pool[i].label, pool[i].description, pool[i].value, pool[i].value in chosen)
                    for i in idx], ("" if idx else "no choice matches")
        if it.widget == "lookup":
            entity = it.entity or "lookup"
            if entity in ("user", "lookup") and not query.strip():
                return [], "type part of a name or email"
            try:
                rows = lookups.search_field(self.client, self.resolver, self.boot, it.field or Field(key, it.label, "lookup_user"),
                                            entity, query)
            except lookups.LookupUnavailable as e:
                out: list[Suggestion] = []
                if entity == "user" and "@" in query:
                    q = query.strip()
                    out.append(Suggestion(f"use {q}", "typed address (search unavailable)", LookupValue(None, q, q)))
                return out, f"{e}; type the exact email" if entity == "user" else str(e)
            return [Suggestion(str(r), "", r) for r in rows], ("" if rows else "no match")
        return [], ""

    # -- effects ("Picking No will show ...") --------------------------------
    def effects(self, key: str) -> list[str]:
        it = self.by_key[key]
        f = it.field
        if f is None or it.widget not in ("choice", "checkbox") or f.api_name in self.state.evaluated.disabled:
            return []
        cache_key = (key, repr(sorted(self.state.values.items(), key=lambda kv: kv[0])))
        if cache_key in self._effects:
            return self._effects[cache_key]
        if it.widget == "checkbox":
            options = [("ticked", True), ("unticked", False)]
        else:
            options = [(c.label, c.value) for c in self._pool(it)[:MAX_EFFECT_CHOICES]]
        current = self.state.evaluated.values.get(f.api_name)
        out = []
        for label, value in options:
            if value == current or (it.widget == "checkbox" and bool(current) == value):
                continue
            s2 = _fork(self.state)
            try:
                d = s2.set(f.api_name, value)
            except FormError:
                continue
            parts = self._diff_parts(s2, d)
            if parts:
                verb = "Ticking" if value is True else "Unticking" if value is False else f"Picking {label}"
                out.append(f"{verb} will " + "; ".join(parts))
        self._effects[cache_key] = out
        return out

    def _labels(self, names: list[str], state: FormState | None = None) -> str:
        st = state or self.state
        return ", ".join(self.spec.by_name[n].label + (" (required)" if n in st.evaluated.required else "")
                         for n in names)

    def _diff_parts(self, st: FormState, d: Any) -> list[str]:
        parts = []
        if d.appeared:
            parts.append("show " + self._labels(d.appeared, st))
        if d.disappeared:
            parts.append("hide " + ", ".join(self.spec.by_name[n].label for n in d.disappeared))
        req = [n for n in d.became_required if n not in d.appeared]
        if req:
            parts.append("make required: " + ", ".join(self.spec.by_name[n].label for n in req))
        opt = [n for n in d.no_longer_required if n in st.evaluated.visible]
        if opt:
            parts.append("make optional: " + ", ".join(self.spec.by_name[n].label for n in opt))
        for n, labels in d.restricted.items():
            parts.append(f"limit {self.spec.by_name[n].label} to {', '.join(labels) or 'nothing'}")
        if d.unrestricted:
            parts.append("offer every choice again in " + ", ".join(self.spec.by_name[n].label for n in d.unrestricted))
        for m in d.messages_added:
            parts.append(f"show the message {m.text!r}")
        return parts

    # -- writing ----------------------------------------------------------
    def set(self, key: str, raw: Any) -> Outcome:
        """Apply one widget value; returns the rule notes or the error."""
        it = self.by_key[key]
        self.touched.add(key)
        self.errors_by_key.pop(key, None)
        if key == REQUESTER or (it.field is not None and is_requester(it.field) and self._requester_is_special()):
            return self._set_requester(it, raw)
        if it.widget == "file" and it.field is None:
            paths, err = check_files(raw, max_bytes=_max_bytes())
            if err:
                self.errors_by_key[key] = err
                return Outcome(error=err)
            if key == ATTACHMENTS:
                self.draft.attachments = paths
            else:
                label = key[len(PLANNING):]
                if paths:
                    self.draft.planning[label] = {"files": paths, "description_html": None}
                else:
                    self.draft.planning.pop(label, None)
            return Outcome()
        f = it.field
        assert f is not None
        name = f.api_name
        if it.widget == "file":
            paths, err = check_files(raw, allowed=it.allowed_types, max_files=it.max_files, max_bytes=_max_bytes())
            if err:
                self.errors_by_key[key] = err
                return Outcome(error=err)
            raw = tuple(paths) or None
        if it.widget == "richtext" and self.kind == "change":
            return self._set_description(it, str(raw or ""))
        before = self.state.display(name) if self.state.evaluated.values.get(name) is not None else ""
        try:
            diff = self.state.set(name, raw)
        except FormError as e:
            self.errors_by_key[key] = str(e)
            return Outcome(error=str(e))
        self._remember(name)
        notes = self._notes(f, diff, before)
        notes += self._preselect(list(diff.restricted))
        return Outcome(notes)

    def mark_invalid(self, key: str, error: str) -> None:
        """An editor holds input it cannot read: keep the stored value but
        report ``error`` until fixed (an empty error withdraws it)."""
        if error:
            self.touched.add(key)
            self.errors_by_key[key] = error
        else:
            self.errors_by_key.pop(key, None)

    def input_errors(self) -> list[str]:
        """Unresolved input errors on visible fields (stop the submit)."""
        return [f"{self.by_key[k].label}: {e}" for k, e in self.errors_by_key.items()
                if k in self.by_key and self.view(k).visible]

    def _remember(self, name: str) -> None:
        value = self.state.values.get(name) if self.state.sources.get(name) == "user" else None
        if value is None:
            if self.mode == "edit":
                self.draft.answers[name] = None  # clearing a field is a change
            else:
                self.draft.answers.pop(name, None)
        else:
            self.draft.answers[name] = value

    def _set_requester(self, it: FormItem, raw: Any) -> Outcome:
        if raw is not None and not isinstance(raw, LookupValue):
            err = "pick a person from the list"
            self.errors_by_key[it.key] = err
            return Outcome(error=err)
        if raw is not None and self.kind in ("sr", "ticket") and not raw.email:
            err = f"{raw} has no email address"
            self.errors_by_key[it.key] = err
            return Outcome(error=err)
        self.requester = raw
        self.draft.requester = (raw.email or raw.display) if raw is not None else None
        req = self._requester_field()
        if req is not None and self.kind in ("change", "ticket"):
            try:
                self.state.set(req.api_name, raw)
            except FormError as e:
                return Outcome(error=str(e))
        return Outcome()

    def _set_description(self, it: FormItem, md: str) -> Outcome:
        from fsv.mdlite import to_html

        assert it.field is not None
        name = it.field.api_name
        self._desc_md = md
        if self.mode == "edit" and md.strip() == self._desc_prefill.strip():
            self.draft.description_html = None
            html = self.existing.get(name)
        else:
            html = to_html(md) if md.strip() else None
            self.draft.description_html = html if html is not None else ("" if self.mode == "edit" else None)
        try:
            diff = self.state.set(name, html)
        except FormError as e:
            self.errors_by_key[it.key] = str(e)
            return Outcome(error=str(e))
        return Outcome(self._notes(it.field, diff, ""))

    def _notes(self, f: Field, diff: Any, before: str) -> list[str]:
        ev = self.state.evaluated
        now = self.state.display(f.api_name) or "(empty)"
        cause = f"{f.label} = {now}"
        out = []
        if diff.appeared:
            out.append(f"+ {self._labels(diff.appeared)} appeared because {cause}")
        if diff.disappeared:
            out.append(f"- {', '.join(self.spec.by_name[n].label for n in diff.disappeared)} hidden because {cause}")
        req = [n for n in diff.became_required if n not in diff.appeared and n in ev.visible]
        if req:
            out.append(f"* {', '.join(self.spec.by_name[n].label for n in req)} now required because {cause}")
        opt = [n for n in diff.no_longer_required if n in ev.visible]
        if opt:
            out.append(f"{', '.join(self.spec.by_name[n].label for n in opt)} no longer required")
        for n, labels in diff.restricted.items():
            out.append(f"{self.spec.by_name[n].label}: choices limited to {', '.join(labels) or 'nothing'}")
        for n in diff.unrestricted:
            out.append(f"{self.spec.by_name[n].label}: every choice is offered again")
        for m in diff.messages_added:
            out.append(f"! {m.text}" + (" (blocking: acknowledge it on the review)" if m.blocking else ""))
        if diff.cleared:
            out.append(f"{', '.join(self.spec.by_name[n].label for n in diff.cleared)} cleared ({f.label} changed)")
        return out

    def _preselect(self, names: list[str]) -> list[str]:
        """A rule left exactly one choice on an empty field: pick it (the
        review says so), like the YAML draft does."""
        out = []
        ev = self.state.evaluated
        for n in names:
            if n not in ev.visible or n in ev.disabled or ev.values.get(n) not in (None, "", ()):
                continue
            f = self.spec.by_name[n]
            if f.kind not in ("dropdown", "department", "nested"):
                continue
            one = flow.only_option(self.state, n)
            if one is None:
                continue
            ch, r = one
            try:
                diff = self.state.set(n, ch.value)
            except FormError:
                continue
            self._remember(n)
            out.append(f"{f.label} = {ch.label} (the only choice rule {r.rule_name or r.rule_id!s} allows)")
            out += self._notes(f, diff, "")
            ev = self.state.evaluated
        return out

    def _preselect_all(self) -> list[str]:
        return self._preselect(list(self.state.evaluated.restricted))

    # -- plans --------------------------------------------------------------
    def build_plan(self, ack: bool | None = None, meta: Any = None) -> Any:
        from fsv import catalog as cat

        ack = self.ack if ack is None else ack
        m = meta or self.meta
        if self.kind == "change" and self.mode == "new":
            plan = flow.build_create_plan(m, self.draft, resolver=self.resolver, requester=self.requester, ack_all=ack)
        elif self.kind == "change":
            plan = flow.build_update_plan(m, self.draft, self.record, self.target, resolver=self.resolver,
                                          ack_all=ack, status_label=self.status_label)
        elif self.kind == "ticket":
            from fsv import ticket_flow
            plan = ticket_flow.build_create_plan(m, self.draft, resolver=self.resolver, requester=self.requester, ack_all=ack)
        else:
            plan = cat.build_sr_plan(m, self.draft, resolver=self.resolver, requester=self.requester, ack_all=ack)
        pending = [e for e in self.input_errors() if e not in plan.errors]
        if pending:  # what the person typed last was not taken: never submit the older value silently
            plan.errors.extend(pending)
            plan.review.errors = list(plan.review.errors) + [e for e in pending if e not in plan.review.errors]
            plan.hash = ""
            plan.review.plan_hash = ""
        if meta is None:
            self.plan = plan
        return plan

    def default_draft_path(self) -> str:
        name = "ticket-draft.yaml" if self.kind == "ticket" else "sr-draft.yaml" if self.kind == "sr" else (
            f"change-{self.target}-draft.yaml" if self.mode == "edit" else "change-draft.yaml")
        return str(Path.cwd() / name)

    def save_draft(self, path: str, plan: Any = None) -> str:
        plan = plan or self.build_plan()
        p = Path(os.path.expanduser(path))
        if self.kind == "sr":
            from fsv.cli_sr import render_sr_template

            p.write_text(render_sr_template(plan, base_dir=str(p.resolve().parent)), encoding="utf-8")
        else:
            flow.save_draft(plan, self.draft, str(p))
        self.saved_path = str(p)
        return str(p)

    def command_for(self, path: str, plan: Any) -> str:
        h = (plan.hash or "")[:16]
        q = shlex.quote(path)
        ack = " --ack" if self.ack else ""
        if self.kind == "sr":
            return f"fsv sr create {self.meta.display_id} --from {q} --yes --expect-plan {h}{ack}"
        if self.kind == "ticket":
            return f"fsv tickets create --from {q} --yes --expect-plan {h}{ack}"
        if self.mode == "edit":
            return f"fsv changes update {self.target} --from {q} --yes --expect-plan {h}{ack}"
        return f"fsv changes create --from {q} --yes --expect-plan {h}{ack}"

    def send(self, plan: Any, *, allow_duplicate: bool = False) -> dict[str, Any]:
        """The non-interactive send path for embedding apps (TUI): journal
        duplicate check, metadata freshness re-check (refuses when the plan
        changed), then the writes (uploads only now)."""
        from fsv import catalog as cat

        if self.dry_run:
            raise flow.FlowExit(2, "dry run: nothing is sent")
        if self.kind == "ticket":
            from fsv import ticket_flow
            return ticket_flow.send(plan, self.client, lambda m: self.build_plan(meta=m), allow_duplicate=allow_duplicate)
        if self.kind == "sr":
            cat.duplicate_check(plan, allow_duplicate)
            new, diff = cat.freshen(plan, self.client, lambda m: self.build_plan(meta=m))
            if diff is not None:
                raise flow.FlowExit(4, "the item form changed since the review: " + "; ".join(diff))
            return cat.execute(new, self.client)
        flow.check_duplicate(plan, allow_duplicate or self.mode == "edit")
        new, diff = flow.freshen(plan, self.client, lambda m: self.build_plan(meta=m))
        if diff is not None:
            raise flow.FlowExit(4, "form metadata changed since the review: " + "; ".join(diff))
        return flow.execute(new, self.client)


# ---------------------------------------------------------------------------
# constructors


def _default_requester(client: Any, key: str) -> tuple[LookupValue | None, str]:
    v = str((config.load_config().get("defaults") or {}).get(key) or "").strip()
    if not v:
        return None, ""
    try:
        return resolve_requester(client, v), f"Requester {v} from config defaults.{key}"
    except (ValueError, Exception) as e:  # never block the form on a default
        return None, f"config defaults.{key} = {v!r} not used: {' '.join(str(e).split())}"


def change_session(client: Any, *, mode: str = "new", target: Any = None, record: Any = None, meta: Any = None,
                   requester: LookupValue | None = None, dry_run: bool = False,
                   status_label: Callable[[Any], str] | None = None, title: str | None = None) -> FormSession:
    """A new-change form, or an edit form for ``record`` (``target`` = id)."""
    from fsv.formmeta import load_meta

    if mode == "edit" and (record is None or "custom_fields" not in record):
        # a list row is not enough: the form compares against the full record
        data = client.int_get(f"changes/{target}")
        record = data.get("change", data) if isinstance(data, dict) else {}
    if meta is None:
        mid = (((record or {}).get("display_id") or (record or {}).get("id") or target)
               if mode == "edit" else None)
        meta = load_meta(client, "change", mode, module_id=mid)
    note = ""
    if mode == "new" and requester is None:
        requester, note = _default_requester(client, "requester")
    if title is None:
        title = "New Change" if mode == "new" else f"Edit change {record.get('human_display_id') or target}"
    return FormSession(kind="change", mode=mode, meta=meta, client=client, title=title, record=record, target=target,
                       requester=requester, requester_note=note, dry_run=dry_run, status_label=status_label)


def sr_session(client: Any, item_ref: Any = None, *, meta: Any = None, requester: LookupValue | None = None,
               dry_run: bool = False) -> FormSession:
    """A service-request form for a catalog item (id or name)."""
    from fsv import catalog as cat

    meta = meta or cat.load_item_meta(client, item_ref)
    note = ""
    if requester is None:
        requester, note = _default_requester(client, "sr_requester")
    item = meta.spec.item
    title = f"New service request: {item.name if item else meta.name}"
    return FormSession(kind="sr", mode="new", meta=meta, client=client, title=title, requester=requester,
                       requester_note=note, dry_run=dry_run)


def ticket_session(client: Any, *, meta: Any = None, requester: LookupValue | None = None,
                   dry_run: bool = False) -> FormSession:
    """Reusable incident form; requester is never implicitly the signed-in agent."""
    from fsv.formmeta import load_meta

    meta = meta or load_meta(client, "ticket", "new")
    note = ""
    if requester is None:
        requester, note = _default_requester(client, "requester")
    return FormSession(kind="ticket", mode="new", meta=meta, client=client, title="New Incident",
                       requester=requester, requester_note=note, dry_run=dry_run)


def open_session(kind: str, **ctx: Any) -> FormSession:
    if kind in ("change", "changes"):
        return change_session(**ctx)
    if kind in ("sr", "service_request"):
        return sr_session(**ctx)
    if kind in ("ticket", "tickets", "incident"):
        return ticket_session(**ctx)
    raise ValueError(f"unknown form kind {kind!r}; expected change, ticket or sr")
