"""Form engine: build Freshservice forms from the web's own metadata, run its
business rules, validate answers, and build create payloads.

Typical use::

    spec = FormSpec.from_form_fields(change_form_fields_json, module="change")
    rules = RulesEngine(spec, business_rules_json, module="change")
    state = FormState(spec, rules, mode="new", resolver=my_resolver)
    diff = state.set("Infra Only", "No")       # -> fields appeared/required
    state.missing_required()                   # -> [Field, ...]
    state.to_review()                          # -> [(label, value), ...]
    body = build_change_payload(state, "jane@example.com", "Jane Doe")

No HTTP happens in this package; lookups go through an injected
:class:`LookupResolver`.
"""
from fsv.forms.errors import (
    AmbiguousLookupError,
    FieldDisabledError,
    FormError,
    PayloadError,
    UnknownFieldError,
    UnsupportedRuleError,
    ValueParseError,
)
from fsv.forms.payload import (
    BR_VALIDATION_EXCLUDES,
    build_change_payload,
    build_sr_payload,
    build_ticket_payload,
    encode_value,
    format_requester,
    text_to_html,
)
from fsv.forms.rules import (
    SUPPORTED_ACTIONS,
    SUPPORTED_OPERATORS,
    EvaluatedForm,
    Message,
    Rule,
    RulesEngine,
)
from fsv.forms.spec import KINDS, Choice, Field, FormSpec, Section, ServiceItem, plain_text
from fsv.forms.state import DEFAULT_TZ, Diff, FormState, LookupResolver, LookupValue

__all__ = [
    "AmbiguousLookupError",
    "BR_VALIDATION_EXCLUDES",
    "Choice",
    "DEFAULT_TZ",
    "Diff",
    "EvaluatedForm",
    "Field",
    "FieldDisabledError",
    "FormError",
    "FormSpec",
    "FormState",
    "KINDS",
    "LookupResolver",
    "LookupValue",
    "Message",
    "PayloadError",
    "Rule",
    "RulesEngine",
    "SUPPORTED_ACTIONS",
    "SUPPORTED_OPERATORS",
    "Section",
    "ServiceItem",
    "UnknownFieldError",
    "UnsupportedRuleError",
    "ValueParseError",
    "build_change_payload",
    "build_sr_payload",
    "build_ticket_payload",
    "encode_value",
    "format_requester",
    "plain_text",
    "text_to_html",
]
