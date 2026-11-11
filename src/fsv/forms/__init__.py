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
from fsv.forms.fetch import BusinessRulesQuery, RulesResponse, business_rules_query, parse_business_rules
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
    DEFAULT_EVENT_REVERSE,
    DEFAULT_EVENT_SCOPE,
    DEFAULT_MODEL,
    DEFAULT_POPULATE,
    EVENT_SCOPES,
    MODELS,
    POPULATE_MODES,
    SUPPORTED_ACTIONS,
    SUPPORTED_EVALUATE_ON,
    SUPPORTED_OPERATORS,
    ChoiceRestriction,
    Conflict,
    Decision,
    EvaluatedForm,
    Explanation,
    Message,
    Rule,
    RuleContext,
    RulesEngine,
    offered_choices,
)
from fsv.forms.spec import KINDS, Choice, Field, FormSpec, Section, ServiceItem, plain_text
from fsv.forms.state import Diff, FormState, LookupResolver, LookupValue
from fsv.forms.tz import resolve_timezone

__all__ = [
    "AmbiguousLookupError",
    "BR_VALIDATION_EXCLUDES",
    "BusinessRulesQuery",
    "Choice",
    "ChoiceRestriction",
    "Conflict",
    "DEFAULT_EVENT_REVERSE",
    "DEFAULT_EVENT_SCOPE",
    "DEFAULT_MODEL",
    "DEFAULT_POPULATE",
    "Decision",
    "Diff",
    "EVENT_SCOPES",
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
    "POPULATE_MODES",
    "PayloadError",
    "Rule",
    "RuleContext",
    "RulesEngine",
    "RulesResponse",
    "SUPPORTED_ACTIONS",
    "SUPPORTED_EVALUATE_ON",
    "SUPPORTED_OPERATORS",
    "Section",
    "ServiceItem",
    "UnknownFieldError",
    "UnsupportedRuleError",
    "ValueParseError",
    "build_br_validation_excludes",
    "business_rules_query",
    "build_change_payload",
    "build_sr_payload",
    "build_ticket_payload",
    "canonical_json",
    "decode_br_validation_excludes",
    "encode_value",
    "excludes_lists",
    "format_requester",
    "metadata_fingerprint",
    "offered_choices",
    "parse_business_rules",
    "plain_text",
    "plan_hash",
    "plan_matches",
    "resolve_timezone",
    "text_to_html",
]
