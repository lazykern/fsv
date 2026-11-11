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
    FormRuleError,
    PayloadError,
    UnknownFieldError,
    UnsupportedRuleError,
    ValueParseError,
)
from fsv.forms.payload import (
    BR_VALIDATION_EXCLUDES,
    build_br_validation_excludes,
    build_change_payload,
    build_sr_payload,
    build_ticket_payload,
    decode_br_validation_excludes,
    encode_value,
    excludes_lists,
    format_requester,
    text_to_html,
)
from fsv.forms.plan import MetadataFingerprint, canonical_json, metadata_fingerprint, plan_hash, plan_matches
from fsv.forms.rules import (
    DEFAULT_MODEL,
    MODELS,
    SUPPORTED_ACTIONS,
    SUPPORTED_EVALUATE_ON,
    SUPPORTED_OPERATORS,
    Conflict,
    Decision,
    EvaluatedForm,
    Explanation,
    Message,
    Rule,
    RuleContext,
    RulesEngine,
)
from fsv.forms.spec import KINDS, Choice, Field, FormSpec, Section, ServiceItem, plain_text
from fsv.forms.state import Diff, FormState, LookupResolver, LookupValue
from fsv.forms.tz import resolve_timezone

__all__ = [
    "AmbiguousLookupError",
    "BR_VALIDATION_EXCLUDES",
    "Choice",
    "Conflict",
    "DEFAULT_MODEL",
    "Decision",
    "Diff",
    "EvaluatedForm",
    "Explanation",
    "Field",
    "FieldDisabledError",
    "FormError",
    "FormRuleError",
    "FormSpec",
    "FormState",
    "KINDS",
    "LookupResolver",
    "LookupValue",
    "MODELS",
    "Message",
    "MetadataFingerprint",
    "PayloadError",
    "Rule",
    "RuleContext",
    "RulesEngine",
    "SUPPORTED_ACTIONS",
    "SUPPORTED_EVALUATE_ON",
    "SUPPORTED_OPERATORS",
    "Section",
    "ServiceItem",
    "UnknownFieldError",
    "UnsupportedRuleError",
    "ValueParseError",
    "build_br_validation_excludes",
    "build_change_payload",
    "build_sr_payload",
    "build_ticket_payload",
    "canonical_json",
    "decode_br_validation_excludes",
    "encode_value",
    "excludes_lists",
    "format_requester",
    "metadata_fingerprint",
    "plain_text",
    "plan_hash",
    "plan_matches",
    "resolve_timezone",
    "text_to_html",
]
