"""Lookups the web form runs while you type, for the form UI.

* Requesters: ``GET /search/autocomplete/requesters?q=<text>&qf=details``
  (``results[{id, value, details: "Name <email>", email, ...}]``), the same
  call the web's requester box makes.
* Groups and agents: the web filters ``GET /api/_/bootstrap/agents_groups``
  locally; so does fsv (fetched once per session). When the role may not
  read it (403), agents fall back to the agent autocomplete.
* Other lookup fields: the engine resolver (``lookup_choices`` links).

A 403/404 becomes :class:`LookupUnavailable`; the form shows a dim note
instead of failing.
"""
from __future__ import annotations

import threading
from typing import Any, Mapping, Sequence

from fsv.errors import APIError
from fsv.forms import Field, LookupValue

from .fuzzy import fuzzy_filter


class LookupUnavailable(Exception):
    """The tenant or the agent's role does not offer this lookup."""

    def __init__(self, what: str, status: Any = None) -> None:
        self.what = what
        self.status = status
        why = f"HTTP {status}" if status else "not offered"
        super().__init__(f"{what} is not available for your role ({why})")


def _email_of(row: Mapping[str, Any]) -> str | None:
    for k in ("email", "primary_email"):
        v = row.get(k)
        if isinstance(v, str) and "@" in v:
            return v
    d = row.get("details")
    if isinstance(d, str) and "<" in d and d.rstrip().endswith(">"):
        return d.rsplit("<", 1)[1].rstrip("> ").strip() or None
    if isinstance(d, str) and "@" in d:
        return d.strip()
    return None


def person(row: Mapping[str, Any], id_key: str = "id") -> LookupValue:
    name = row.get("value") or row.get("name")
    details = row.get("details")
    if not name and isinstance(details, str):
        name = details.split("<", 1)[0].strip()
    return LookupValue(row.get(id_key), str(name or row.get(id_key) or ""), _email_of(row))


def search_requesters(client: Any, query: str, *, limit: int = 20) -> list[LookupValue]:
    """People matching ``query`` (name or email), as the web's requester box."""
    q = (query or "").strip()
    if not q:
        return []
    try:
        rows = client.autocomplete("requesters", q, {"qf": "details"})
    except APIError as e:
        if e.status in (403, 404):
            raise LookupUnavailable("requester search", e.status) from None
        raise
    out = [person(r) for r in rows or [] if isinstance(r, Mapping) and r.get("id") is not None
           and not r.get("deleted")]
    return out[:limit]


class AgentsGroups:
    """The ``bootstrap/agents_groups`` lists, fetched once, filtered locally."""

    def __init__(self, client: Any) -> None:
        self.client = client
        self._data: Mapping[str, Any] | None = None
        self._error: LookupUnavailable | None = None
        self._lock = threading.Lock()

    def _load(self) -> Mapping[str, Any]:
        with self._lock:
            if self._error is not None:
                raise self._error
            if self._data is None:
                try:
                    data = self.client.int_get("bootstrap/agents_groups")
                except APIError as e:
                    if e.status in (403, 404):
                        self._error = LookupUnavailable("the agents and groups list", e.status)
                        raise self._error from None
                    raise
                self._data = data if isinstance(data, Mapping) else {}
            return self._data

    def has(self, key: str) -> bool:
        """Whether the bootstrap answer carries a ``key`` list at all."""
        return isinstance(self._load().get(key), list)

    def groups(self) -> list[LookupValue]:
        rows = self._load().get("groups") or []
        return [LookupValue(g["id"], str(g.get("name") or g.get("value") or g["id"]))
                for g in rows if isinstance(g, Mapping) and g.get("id") is not None]

    def agents(self) -> list[LookupValue]:
        rows = self._load().get("agents") or []
        out = []
        for a in rows:
            if not isinstance(a, Mapping) or "(Deactivated)" in str(a.get("value") or a.get("name") or ""):
                continue
            key = "user_id" if a.get("user_id") is not None else "id"
            if a.get(key) is not None:
                out.append(person(a, key))
        return out

    def filter(self, entity: str, query: str, *, limit: int = 50) -> list[LookupValue]:
        pool = self.groups() if entity == "group" else self.agents()
        idx = fuzzy_filter(query, [(p.display, p.email or "") for p in pool])
        return [pool[i] for i in idx[:limit]]


def search_field(client: Any, resolver: Any, boot: AgentsGroups, field: Field, entity: str,
                 query: str) -> Sequence[LookupValue]:
    """Suggestions for a lookup field."""
    if entity == "user":
        return search_requesters(client, query)
    if entity in ("group", "agent"):
        try:
            if entity == "group" or boot.has("agents"):
                return boot.filter(entity, query)
        except LookupUnavailable:
            if entity == "group":
                raise
        if not query.strip():
            return []
        try:
            return list(resolver.lookup("agent", query, field))
        except APIError as e:
            if e.status in (403, 404):
                raise LookupUnavailable("agent search", e.status) from None
            raise
    if not query.strip():
        return []
    try:
        return list(resolver.lookup(entity, query, field))
    except APIError as e:
        if e.status in (403, 404):
            raise LookupUnavailable(f"{field.label} search", e.status) from None
        raise
