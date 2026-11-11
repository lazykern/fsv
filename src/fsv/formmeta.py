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

The workspace comes from ``fsv config set forms.workspace_id N`` (an
override) or, by default, from the form metadata itself (the workspace the
custom fields belong to), then from the account (the session's workspace
cookie, ``bootstrap/me``, ``workspaces``), then the workspace-less path.
When no path gives rules, :func:`rules_note` / :func:`rules_warnings` say
why in one line.
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

MODULES = {"change": ("change_form_fields", "change_fields"),
           "ticket": ("ticket_form_fields", "ticket_fields")}
FORM_TYPES = {"new": "new_form", "edit": "edit_form"}
cache.TTL.setdefault("form_rules", 3600)

# default fields whose record key differs from the form field name
RECORD_KEYS = {
    "department": "department_id",
    "group": "group_id",
    "agent": "agent_id",
    "change_window": "change_window_id",
    "requester": "requester_id",
    "requested_for": "requested_for_id",
}


def module_name(text: str) -> str:
    t = (text or "").strip().lower().rstrip("s")
    if t not in MODULES:
        raise ValueError(f"unsupported form {text!r}; supported: changes, tickets")
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


RULES_UNAVAILABLE_KEY = "unavailable"
RULES_WORKSPACE_KEY = "workspace"  # where the rules came from (informational)


def _ws_id(v: Any) -> Any:
    if isinstance(v, bool):
        return None
    if isinstance(v, int):
        return v
    if isinstance(v, str) and v.strip().isdigit():
        return int(v.strip())
    return None


def _add(out: list[Any], v: Any) -> None:
    w = _ws_id(v)
    if w is not None and w not in out:
        out.append(w)


_CURRENT_KEYS = ("current_workspace_id", "current_workspace", "selected_workspace_id", "active_workspace_id")
_PRIMARY_KEYS = ("primary_workspace_id", "default_workspace_id", "workspace_id")
_PRIMARY_FLAGS = ("primary", "is_primary", "default", "is_default", "current", "selected")


def workspaces_in(doc: Any) -> list[Any]:
    """Workspace ids an account document names, current/primary first.

    Reads ``current_workspace_id``-like keys, then ``primary``/``default``
    workspace keys, then ``workspaces: [{id, primary}]`` lists and
    ``workspace_ids`` (a few levels deep: ``{"user": {...}}``,
    ``{"account": {...}}``). Unknown shapes give ``[]``."""
    current: list[Any] = []
    primary: list[Any] = []
    rest: list[Any] = []

    def walk(o: Any, depth: int) -> None:
        if depth > 3 or not isinstance(o, Mapping):
            return
        for k in _CURRENT_KEYS:
            v = o.get(k)
            _add(current, v.get("id") if isinstance(v, Mapping) else v)
        for k in _PRIMARY_KEYS:
            _add(primary, o.get(k))
        for k in ("workspaces", "workspace_list"):
            rows = o.get(k)
            if isinstance(rows, list):
                for r in rows:
                    if isinstance(r, Mapping):
                        flagged = any(r.get(f) is True for f in _PRIMARY_FLAGS)
                        _add(primary if flagged else rest, r.get("id"))
                    else:
                        _add(rest, r)
        ids = o.get("workspace_ids")
        if isinstance(ids, list):
            for v in ids:
                _add(rest, v)
        for v in o.values():
            if isinstance(v, Mapping):
                walk(v, depth + 1)

    walk(doc, 0)
    out: list[Any] = []
    for v in current + primary + rest:
        _add(out, v)
    return out


def _cookie_workspace(client: Any) -> list[Any]:
    """A numeric ``*workspace*`` cookie of the session (the web remembers
    the workspace the agent last worked in), if the client exposes one."""
    jar = getattr(getattr(getattr(client, "_client", None), "cookies", None), "jar", None)
    out: list[Any] = []
    try:
        for ck in jar or []:
            if "workspace" in str(getattr(ck, "name", "")).lower():
                _add(out, getattr(ck, "value", None))
    except Exception:  # noqa: BLE001 - a cookie jar oddity never blocks a form
        return []
    return out


ACCOUNT_WORKSPACE_PATHS = ("bootstrap/me", "workspaces")


def account_workspaces(client: Any) -> list[Any]:
    """Workspace ids the account itself reports for the signed-in agent:
    the session's workspace cookie, ``GET /api/_/bootstrap/me``, then
    ``GET /api/_/workspaces`` (current/primary first). Optional endpoints:
    a 403/404 or an unknown shape just gives fewer candidates."""
    out = _cookie_workspace(client)
    for path in ACCOUNT_WORKSPACE_PATHS:
        try:
            doc = client.int_get(path)
        except APIError:
            continue
        except Exception as e:  # noqa: BLE001
            if type(e).__name__ == "SessionError":
                raise
            continue
        for w in workspaces_in(doc):
            _add(out, w)
        if out:
            break
    return out


def fetch_rules(client: Any, module: str, workspace_id: Any, *, fallback: Any = None,
                **query: Any) -> dict[str, Any]:
    """Business rules for a form, portable across tenants.

    Tries ``/api/_/ws/<ws>/business_rules`` for ``workspace_id`` (one id or
    a list of candidates, best first), then for the workspaces ``fallback``
    returns (a callable, asked only when every earlier candidate failed: the
    account's own workspaces), then the workspace-less
    ``/api/_/business_rules`` (accounts without workspaces). A candidate
    that answers with no rules is remembered and the next one is tried.
    A 403/404 on every path (rules not permitted for the role, or not a
    feature of the plan) gives an empty rule set with ``unavailable`` set
    to a one-line reason: the form then runs on the static
    required/visible flags only, and every review says so.
    """
    cands: list[Any] = []
    for w in (workspace_id if isinstance(workspace_id, (list, tuple)) else [workspace_id]):
        if w not in (None, "") and w not in cands:
            cands.append(w)
    tried: list[str] = []
    empty: dict[str, Any] | None = None
    last: APIError | None = None

    def attempt(path: str, params: dict[str, Any], ws: Any) -> dict[str, Any] | None:
        nonlocal last, empty
        try:
            raw = client.int_get(path, params)
        except APIError as e:
            if e.status in (403, 404):
                last = e
                tried.append(f"/api/_/{path} HTTP {e.status}")
                return None
            raise
        if raw in (None, "", [], {}):
            rules: dict[str, Any] = {"business_rules": [], "user": None}
        else:
            rules = normalize_rules(raw)
        if ws is not None:
            rules[RULES_WORKSPACE_KEY] = ws
        if rules.get("business_rules"):
            return rules
        if empty is None:
            empty = rules
        return None

    def ws_try(ws: Any) -> dict[str, Any] | None:
        q = business_rules_query(module, workspace_id=ws, **query)
        return attempt(q.path.removeprefix("/api/_/"), dict(q.params), ws)

    for ws in cands:
        got = ws_try(ws)
        if got is not None:
            return got
    if empty is not None:
        return empty  # the workspace answered: it has no rules for this form
    if fallback is not None:
        try:
            extra = [w for w in (fallback() or []) if w not in cands]
        except APIError:
            extra = []
        for ws in extra:
            cands.append(ws)
            got = ws_try(ws)
            if got is not None:
                return got
    if empty is not None:
        return empty
    q = business_rules_query(module, workspace_id=0, **query)
    got = attempt("business_rules", dict(q.params), None)
    if got is not None:
        return got
    if empty is not None:
        return empty
    if cands:
        why = (f"workspace {', '.join(map(str, cands))} and the workspace-less path answered "
               f"HTTP {last.status if last else '?'}")
    else:
        why = (f"no workspace found in the form metadata or the account, and /api/_/business_rules "
               f"answered HTTP {last.status if last else '?'}")
    return {"business_rules": [], "user": None, RULES_UNAVAILABLE_KEY: why,
            "unavailable_status": last.status if last is not None else None}


def rules_warnings(rules: Mapping[str, Any] | None) -> list[str]:
    """Review warning when the tenant gave no business rules."""
    why = (rules or {}).get(RULES_UNAVAILABLE_KEY)
    if not why:
        return []
    return [f"business rules could not be loaded ({why}); only the form's static required/visible "
            "settings were checked, the server may still ask for more. If the account has workspaces, "
            "`fsv config set forms.workspace_id N` picks the right one"]


def rules_note(rules: Mapping[str, Any] | None) -> str | None:
    """One line for stderr when the rules could not be loaded, else None."""
    why = (rules or {}).get(RULES_UNAVAILABLE_KEY)
    if not why:
        return None
    return (f"note: business rules not loaded ({why}); checking static required/visible flags only "
            "(override the workspace with `fsv config set forms.workspace_id N`)")


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


def workspace_candidates(*preferred: Any) -> list[Any]:
    """Workspaces to try for business rules, best first: the
    ``forms.workspace_id`` override alone when set, else the given ones
    (record workspace, form metadata workspace, ...)."""
    override = configured_workspace()
    if override not in (None, ""):
        return [override]
    out: list[Any] = []
    for w in preferred:
        if w not in (None, "") and w not in out:
            out.append(w)
    return out


def load_rules(client: Any, module: str, *preferred: Any, **query: Any) -> dict[str, Any]:
    """:func:`fetch_rules` with the workspace defaulted portably: the config
    override, else ``preferred`` (record / metadata workspaces), else the
    account's own workspaces, else the workspace-less path."""
    cands = workspace_candidates(*preferred)
    use_account = configured_workspace() in (None, "")
    return fetch_rules(client, module, cands, fallback=(lambda: account_workspaces(client)) if use_account else None,
                       **query)


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
    # the override, else the fields' workspace; accounts without one get the
    # account's own workspace or the workspace-less path (load_rules)
    ws = configured_workspace() or workspace_from_fields(fields[key])
    ckey = f"{module}-rules-{form_type}-ws{ws}" + (f"-{module_id}" if module_id is not None else "")
    rules = None if fresh else _cached(ckey)
    if rules is None:
        source = "server"
        rules = load_rules(client, module, ws, form_type=form_type, module_id=module_id)
        _save(ckey, "form_rules", rules)
    ws = rules.get(RULES_WORKSPACE_KEY, ws)
    return FormMeta(module, form_type, fields, rules, ws, time.time(), source, module_id)


# ---------------------------------------------------------------------------
# lookups


class ClientResolver:
    """``LookupResolver`` over the web's autocomplete endpoints."""

    def __init__(self, client: Any) -> None:
        self.client = client
        self._groups: list[dict[str, Any]] | None = None
        self._boot: dict[str, Any] | None = None

    def _bootstrap(self) -> dict[str, Any]:
        if self._boot is None:
            self._boot = self.client.int_get("bootstrap/agents_groups") or {}
        return self._boot

    def name_for(self, entity: str, id_: Any) -> str | None:
        """Display name of a group/agent by id from the (once-fetched,
        cached) bootstrap lists; None when unknown (e.g. requesters)."""
        key = "groups" if entity == "group" else "agents" if entity == "agent" else None
        if key is None:
            return None
        for row in self._bootstrap().get(key) or []:
            if not isinstance(row, Mapping):
                continue
            if str(row.get("id")) == str(id_) or str(row.get("user_id")) == str(id_):
                shown = row.get("name") or row.get("value")
                if shown:
                    return str(shown)
        return None

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
                self._groups = list(self._bootstrap().get("groups") or [])
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


_RECORD_OBJECTS = {"requester": ("requester",), "agent": ("agent", "responder", "owner"), "group": ("group",)}
_RECORD_NAMES = {"agent": ("owner_name", "responder_name", "agent_name"), "group": ("group_name",),
                 "requester": ("requester_name",)}


def _hint_name(name: str, vid: Any, record: Mapping[str, Any]) -> tuple[str | None, str | None]:
    """(display, email) the record itself carries for a lookup field, if any."""
    for key in _RECORD_OBJECTS.get(name, ()):
        obj = record.get(key)
        if isinstance(obj, Mapping) and (obj.get("id") in (None, vid) or str(obj.get("id")) == str(vid)
                                         or obj.get("user_id") == vid):
            shown = obj.get("name") or obj.get("value") or obj.get("full_name")
            if shown:
                return str(shown), _email(obj)
    for key in _RECORD_NAMES.get(name, ()):
        if record.get(key):
            return str(record[key]), None
    return None, None


def name_existing(existing: dict[str, Any], spec: FormSpec, record: Mapping[str, Any],
                  resolver: Any = None) -> dict[str, Any]:
    """Give id-only lookup values (requester, agent, group, ...) a display
    name: from the record's own nested objects, else the resolver's cached
    lookups (``name_for``). Ids nobody can name are left as they are."""
    namer = getattr(resolver, "name_for", None)

    def fix(f: Field, v: Any) -> Any:
        if not isinstance(v, LookupValue) or (v.display and v.display != str(v.id)):
            return v
        display, email = _hint_name(f.api_name, v.id, record)
        if display is None and namer is not None:
            try:
                display = namer(f.lookup_entity or {"lookup_group": "group", "lookup_agent": "agent",
                                                    "lookup_user": "user"}.get(f.kind, ""), v.id)
            except Exception:  # naming is cosmetic: never fail a plan for it
                display = None
        return LookupValue(v.id, display, email or v.email) if display else v

    for name, v in list(existing.items()):
        f = spec.by_name.get(name)
        if f is None:
            continue
        existing[name] = tuple(fix(f, x) for x in v) if isinstance(v, tuple) else fix(f, v)
    return existing


def apply_answers(state: FormState, answers: Mapping[str, Any], failed: set[str] | None = None) -> list[str]:
    """Set answers (label or internal name -> raw) in web field order.
    Returns every problem instead of stopping at the first. ``failed``, if
    given, collects the api names of fields whose answer was rejected."""
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
            if failed is not None:
                failed.add(name)
    return errors
