"""Where the web gets a form's business rules, and how to read the response.

The forms package does no HTTP: :func:`business_rules_query` builds the
request the web UI sends when it opens a form, and
:func:`parse_business_rules` reads either response shape.

The server *filters* the rules for the context in the query (module, catalog
item, form type, requester, record), so fetch them with the same parameters
the web uses::

    q = business_rules_query("ticket", workspace_id=2)
    data = http_get(q.url())          # caller's HTTP client
    rules = parse_business_rules(data)
    engine = RulesEngine(spec, rules.raw, context=RuleContext(module="ticket", user=rules.user,
                                                                             system={"portal": False}))

Two response shapes exist:

* ``{"business_rules": [...], "user": {...}}``: what ``ember_api=true``
  returns; every rule has ``condition_data`` and ``action_data``. This is the
  one to evaluate.
* ``{"business_rule": [...]}``: the plain ``GET .../business_rules`` listing
  (admin view): every rule of the workspace with ``position`` but **without**
  conditions or actions. It cannot be evaluated (``evaluable`` is False and
  :class:`~fsv.forms.RulesEngine` refuses it).
"""
from __future__ import annotations

from dataclasses import dataclass, field as dc_field
from typing import Any, Mapping
from urllib.parse import urlencode

_FORM_TYPES = {"new": "new_form", "edit": "edit_form", "new_form": "new_form", "edit_form": "edit_form"}
# service-catalog item rules name custom fields "customField.<name>"; module
# forms use "customFields.<name>" (as the web sends cf_name_prefix)
_SR_MODULES = frozenset({"service_request", "sr"})


@dataclass(frozen=True)
class BusinessRulesQuery:
    """``GET /api/_/ws/<workspace>/business_rules?...`` as the web sends it."""

    workspace_id: Any
    params: tuple[tuple[str, str], ...]

    @property
    def path(self) -> str:
        return f"/api/_/ws/{self.workspace_id}/business_rules"

    def query_string(self) -> str:
        return urlencode(list(self.params), safe=".")

    def url(self, base: str = "") -> str:
        """Path and query (prefixed with ``base``, e.g. ``https://acme.freshservice.com``)."""
        return f"{base.rstrip('/')}{self.path}?{self.query_string()}"


def _null(v: Any) -> str:
    return "null" if v is None or v == "" else str(v)


def _blank(v: Any) -> str:
    return "" if v is None else str(v)


def business_rules_query(
    module_type: str,
    *,
    workspace_id: Any,
    item_id: Any = None,
    form_type: str = "new_form",
    requester: Any = None,
    requested_for: Any = None,
    user_type: str | None = None,
    module_id: Any = None,
    include_workspace: bool = False,
    filter_by_default_form_action: bool | None = None,
    cf_name_prefix: str | None = None,
) -> BusinessRulesQuery:
    """The business-rules request for one form context.

    ``module_type``: ``change``, ``ticket``, ``service_request``, ...;
    ``item_id``: the catalog item's ``id`` (service requests), else None;
    ``form_type``: ``new``/``new_form`` or ``edit``/``edit_form``;
    ``requester`` / ``requested_for``: user ids when known (empty otherwise);
    ``module_id``: the record's display id on an edit form;
    ``user_type``: None sends ``null`` (what the agent UI sends).
    ``filter_by_default_form_action`` defaults to True except for service
    requests (where the web omits it); ``cf_name_prefix`` defaults to
    ``customField.`` for service requests and ``customFields.`` otherwise.
    """
    if form_type not in _FORM_TYPES:
        raise ValueError(f"form_type must be one of {sorted(_FORM_TYPES)}")
    if workspace_id in (None, ""):
        raise ValueError("workspace_id is required (the workspace the form belongs to)")
    sr = module_type in _SR_MODULES
    if filter_by_default_form_action is None:
        filter_by_default_form_action = not sr
    if cf_name_prefix is None:
        cf_name_prefix = "customField." if sr else "customFields."
    params: list[tuple[str, str]] = [
        ("ember_api", "true"),
        ("module_type", "service_request" if sr else str(module_type)),
        ("item_id", _null(item_id)),
        ("requester", _blank(requester)),
        ("requested_for", _blank(requested_for)),
        ("include_workspace", "true" if include_workspace else "false"),
    ]
    if filter_by_default_form_action:
        params.append(("filter_by_default_form_action", "true"))
    params += [
        ("form_type", _FORM_TYPES[form_type]),
        ("user_type", _null(user_type)),
        ("module_id", _blank(module_id)),
        ("cf_name_prefix", cf_name_prefix),
    ]
    return BusinessRulesQuery(workspace_id, tuple(params))


@dataclass
class RulesResponse:
    """A parsed business-rules response.

    ``shape``: ``"form"`` (``business_rules`` + ``user``, evaluable),
    ``"listing"`` (``business_rule``, no conditions/actions) or ``"list"``
    (a bare list). ``raw`` is what :class:`~fsv.forms.RulesEngine` takes.
    """

    rules: list[Mapping[str, Any]]
    user: Mapping[str, Any] | None = None
    shape: str = "form"
    evaluable: bool = True
    raw: Any = dc_field(default=None, repr=False)


def parse_business_rules(data: Any) -> RulesResponse:
    """Read either response shape (see the module docstring)."""
    if isinstance(data, Mapping):
        if "business_rules" in data:
            rules, shape = list(data.get("business_rules") or []), "form"
        elif "business_rule" in data:
            rules, shape = list(data.get("business_rule") or []), "listing"
        else:
            raise ValueError("unrecognised business_rules response (no business_rules / business_rule key)")
        user = data.get("user") if isinstance(data.get("user"), Mapping) else None
    elif isinstance(data, list):
        rules, shape, user = list(data), "list", None
    else:
        raise ValueError("unrecognised business_rules response")
    evaluable = all("condition_data" in r or "action_data" in r for r in rules)
    if shape == "listing" and rules and not evaluable:
        raw: Any = {"business_rule": rules}
    else:
        raw = {"business_rules": rules, "user": user} if user is not None else {"business_rules": rules}
    return RulesResponse(rules=rules, user=user, shape=shape, evaluable=evaluable, raw=raw)
