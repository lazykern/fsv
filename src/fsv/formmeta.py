"""Form metadata for the CLI faces: fetch, cache, fingerprint, and build
engine objects (``fsv.forms``) for a module form.

The web loads two things for a new/edit change form, and so does fsv:

* the field list: ``GET /api/_/change_form_fields``
* the business rules: ``GET /api/_/ws/<workspace>/business_rules?...``,
  built by :func:`fsv.forms.business_rules_query` and read with
  :func:`fsv.forms.parse_business_rules` (``{"business_rules": [...],
  "user": {...}}``; a ``{"business_rule": [...]}`` answer is accepted only
  when its rules carry conditions and actions)

Both are cached (never when empty). Writes re-fetch them synchronously just
before submitting (:func:`load_meta` with ``fresh=True``) and compare the
fingerprint, so a rule or choice edited since the draft was reviewed is
noticed.

The workspace comes from ``fsv config set forms.workspace_id N`` or, by
default, from the form metadata itself (the workspace the custom fields
belong to).
"""
from __future__ import annotations

import collections
import datetime as dt
import time
from dataclasses import dataclass, field as dc_field
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence

from fsv import cache, config
from fsv.errors import APIError
from fsv.forms import (
    Field,
    FormError,
    FormSpec,
    FormState,
    LookupValue,
    MetadataFingerprint,
    RulesEngine,
    business_rules_query,
    metadata_fingerprint,
    parse_business_rules,
)
from fsv.safety import norm

MODULES = {"change": ("change_form_fields", "change_fields")}
FORM_TYPES = {"new": "new_form", "edit": "edit_form"}
cache.TTL.setdefault("form_rules", 3600)

# default fields whose record key differs from the form field name
RECORD_KEYS = {
    "department": "department_id",
    "group": "group_id",
    "agent": "agent_id",
    "change_window": "change_window_id",
    "requester": "requester_id",
}


def module_name(text: str) -> str:
    t = (text or "").strip().lower().rstrip("s")
    if t not in MODULES:
        raise ValueError(f"unsupported form {text!r}; supported: changes")
    return t


def _cache_path(name: str) -> Path:
    return config.CACHE_DIR / "forms" / f"{config.cache_domain_key()}--{name}.json"


def _cached(name: str) -> Any:
    doc, stale = cache.load(_cache_path(name))
    if doc is None or stale:
        return None
    data = doc.get("data")
    return data if _non_empty(data) else None


def _non_empty(data: Any) -> bool:
    if isinstance(data, Mapping):
        return any(isinstance(v, list) and v for v in data.values())
    return bool(data)


def _save(name: str, kind: str, data: Any) -> None:
    if not _non_empty(data):
        return  # never cache an empty answer (expired session, outage)
    try:
        cache.save(_cache_path(name), kind, data)
    except OSError:
        pass


def normalize_rules(raw: Any) -> dict[str, Any]:
    """``{"business_rules": [...], "user": {...}}`` from either response shape."""
    try:
        parsed = parse_business_rules(raw)
    except ValueError as e:
        raise APIError(200, str(e), "GET", "business_rules") from None
    if not parsed.evaluable:
        raise APIError(200, "the server returned the business-rule listing without conditions/actions; "
                            "fsv cannot evaluate the form from it (use the web)", "GET", "business_rules")
    return {"business_rules": list(parsed.rules), "user": parsed.user}


def workspace_from_fields(fields: Sequence[Mapping[str, Any]]) -> Any:
    """The workspace the custom fields belong to (rules live there), else the
    most common workspace in the field list."""
    custom = [f.get("workspace_id") for f in fields if not f.get("default_field") and f.get("workspace_id") is not None]
    pool = custom or [f.get("workspace_id") for f in fields if f.get("workspace_id") is not None]
    if not pool:
        return None
    return collections.Counter(pool).most_common(1)[0][0]


def configured_workspace() -> Any:
    v = (config.load_config().get("forms") or {}).get("workspace_id")
    return int(v) if isinstance(v, str) and v.isdigit() else v


@dataclass
class FormMeta:
    module: str
    form_type: str  # new_form | edit_form
    form_fields: dict[str, Any]
    rules: dict[str, Any]
    workspace_id: Any
    fetched_at: float
    source: str  # "server" | "cache"
    module_id: Any = None  # record display id (edit form)
    _spec: FormSpec | None = dc_field(default=None, repr=False)

    @property
    def fields_key(self) -> str:
        return MODULES[self.module][1]

    @property
    def raw_fields(self) -> list[dict[str, Any]]:
        return list(self.form_fields.get(self.fields_key) or [])

    @property
    def user(self) -> Mapping[str, Any] | None:
        return self.rules.get("user")

    @property
    def fingerprint(self) -> MetadataFingerprint:
        return metadata_fingerprint(self.form_fields, self.rules)

    def as_of(self) -> dict[str, Any]:
        fp = self.fingerprint
        return {
            "updated_at": fp.updated_at,
            "fetched_at": dt.datetime.fromtimestamp(self.fetched_at).astimezone().isoformat(timespec="seconds"),
            "source": self.source,
            "fingerprint": fp.hash[:16],
        }

    @property
    def spec(self) -> FormSpec:
        if self._spec is None:
            self._spec = FormSpec.from_form_fields(self.form_fields, module=self.module)
        return self._spec

    def engine(self) -> RulesEngine:
        return RulesEngine(self.spec, self.rules, module=self.module, workspace_id=self.workspace_id)


def load_meta(client: Any, module: str = "change", mode: str = "new", *, fresh: bool = False,
              module_id: Any = None) -> FormMeta:
    """Form fields + rules for ``module``; ``fresh`` bypasses the cache (writes).
    ``module_id``: the record's display id on an edit form."""
    module = module_name(module)
    form_type = FORM_TYPES[mode]
    path, key = MODULES[module]
    source = "cache"
    fields = None if fresh else _cached(f"{module}-fields")
    if fields is None:
        source = "server"
        raw = client.int_get(path)
        if not isinstance(raw, Mapping) or not raw.get(key):
            raise APIError(200, f"no form fields returned for {module}", "GET", path)
        fields = {key: raw[key]}
        _save(f"{module}-fields", "schema", fields)
    ws = configured_workspace() or workspace_from_fields(fields[key])
    if ws in (None, ""):
        raise ValueError("cannot tell which workspace the form belongs to; "
                         "set it with `fsv config set forms.workspace_id N`")
    ckey = f"{module}-rules-{form_type}-ws{ws}" + (f"-{module_id}" if module_id is not None else "")
    rules = None if fresh else _cached(ckey)
    if rules is None:
        source = "server"
        q = business_rules_query(module, workspace_id=ws, form_type=form_type, module_id=module_id)
        rules = normalize_rules(client.int_get(q.path.removeprefix("/api/_/"), dict(q.params)))
        _save(ckey, "form_rules", rules)
    return FormMeta(module, form_type, fields, rules, ws, time.time(), source, module_id)


# ---------------------------------------------------------------------------
# lookups


class ClientResolver:
    """``LookupResolver`` over the web's autocomplete endpoints."""

    def __init__(self, client: Any) -> None:
        self.client = client
        self._groups: list[dict[str, Any]] | None = None

    def lookup(self, entity: str, query: str, field: Field) -> Sequence[LookupValue]:
        c = self.client
        if entity == "user":
            rows = c.autocomplete("requesters", query, {"all_users": "true"})
            return [_person(r, r.get("id")) for r in rows if r.get("id") is not None and not r.get("deleted")]
        if entity == "agent":
            rows = c.autocomplete("agents", query)
            return [_person(r, r.get("user_id")) for r in rows
                    if r.get("user_id") is not None and "(Deactivated)" not in str(r.get("value") or "")]
        if entity == "group":
            if self._groups is None:
                data = c.int_get("bootstrap/agents_groups")
                self._groups = list((data or {}).get("groups") or [])
            q = norm(query)
            return [LookupValue(g["id"], str(g.get("name") or g["id"])) for g in self._groups
                    if g.get("id") is not None and q in norm(g.get("name"))]
        if field.lookup_link:
            rows = c.lookup_choices(field.lookup_link, query)
            return [LookupValue(r["id"], str(r.get("name") or r.get("value") or r["id"]), _email(r))
                    for r in rows if r.get("id") is not None]
        return []


def _email(r: Mapping[str, Any]) -> str | None:
    for k in ("email", "details", "primary_email"):
        v = r.get(k)
        if isinstance(v, str) and "@" in v:
            return v
    return None


def _person(r: Mapping[str, Any], id_: Any) -> LookupValue:
    return LookupValue(id_, str(r.get("value") or r.get("name") or id_), _email(r))


def resolve_requester(client: Any, value: str, pick: Callable[[list[LookupValue]], LookupValue] | None = None) -> LookupValue:
    """Exact requester match (email or full name, NFC/case-insensitive).
    Anything else is an error listing candidates, unless ``pick`` chooses."""
    q = str(value or "").strip()
    if not q:
        raise ValueError("requester is empty")
    rows = client.autocomplete("requesters", q, {"all_users": "true"})
    found = [_person(r, r.get("id")) for r in rows if r.get("id") is not None and not r.get("deleted")]
    key = norm(q)
    exact = [p for p in found if key in (norm(p.email), norm(p.display), norm(str(p)))]
    if len(exact) == 1 and exact[0].email:
        return exact[0]
    cands = exact or found
    if pick is not None and cands:
        return pick(cands)
    if not cands:
        raise ValueError(f"no requester matches {q!r}; use the exact email address")
    shown = "; ".join(str(p) for p in cands[:8])
    what = "several exact matches" if exact else "no exact match"
    raise ValueError(f"requester {q!r}: {what}: {shown}. Use the exact email address.")


# ---------------------------------------------------------------------------
# answers -> state


def find_field(spec: FormSpec, key: str) -> Field:
    """Field by internal name or label; ambiguous labels are an error."""
    k = str(key).strip()
    if k in spec.by_name:
        return spec.by_name[k]
    hits = [f for f in spec.fields if f.on_form and norm(f.label) == norm(k)]
    if len(hits) > 1:
        names = ", ".join(f.api_name for f in hits)
        raise FormError(f"label {k!r} is ambiguous (fields: {names}); use the internal name")
    return spec.field(k)


def is_status(f: Field) -> bool:
    return f.field_type == "default_status"


def is_requester(f: Field) -> bool:
    return f.field_type == "default_requester"


def is_description(f: Field) -> bool:
    return f.field_type == "default_description"


def existing_from_record(spec: FormSpec, record: Mapping[str, Any]) -> dict[str, Any]:
    """Map a change record (``GET /api/_/changes/<id>``) to form values."""
    custom = record.get("custom_fields") or {}
    out: dict[str, Any] = {}
    for f in spec.fields:
        name = f.api_name
        if f.is_custom:
            v = custom.get(name) if isinstance(custom, Mapping) else None
        else:
            v = record.get(RECORD_KEYS.get(name, name), record.get(name))
            if isinstance(v, Mapping) and "id" in v:
                v = v["id"]
        if isinstance(v, Mapping) and set(v) >= {"id"}:
            v = LookupValue(v["id"], str(v.get("value") or v.get("name") or v["id"]))
        elif f.kind in ("lookup_user", "lookup_group", "lookup_agent", "lookup") and v not in (None, "", []):
            if isinstance(v, list):
                v = tuple(x if isinstance(x, LookupValue) else LookupValue(x) for x in v)
            elif not isinstance(v, LookupValue):
                v = LookupValue(v)
        if v in (None, "", [], {}):
            continue
        out[name] = tuple(v) if isinstance(v, list) else v
    return out


def apply_answers(state: FormState, answers: Mapping[str, Any]) -> list[str]:
    """Set answers (label or internal name -> raw) in web field order.
    Returns every problem instead of stopping at the first."""
    spec = state.spec
    errors: list[str] = []
    resolved: dict[str, Any] = {}
    for key, raw in answers.items():
        try:
            f = find_field(spec, key)
        except FormError as e:
            errors.append(str(e))
            continue
        resolved[f.api_name] = raw
    for name in state.rules.position_order(resolved):
        try:
            state.set(name, resolved[name])
        except FormError as e:
            errors.append(str(e))
    return errors
