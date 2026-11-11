//! Pure Freshservice form engine (port of Python `fsv.forms`, plus
//! `fsv.formui.fuzzy` and `fsv.formui.when`).
//!
//! The same metadata the web UI uses, interpreted locally: a [`FormSpec`]
//! (fields), a [`RulesEngine`] (business rules), a [`FormState`] (answers,
//! parsing, rule effects), a payload builder and plan hashes. No I/O: the
//! caller fetches metadata (see [`fetch`] for the request shapes) and
//! supplies a [`LookupResolver`] for people/group lookups.

pub mod errors;
pub mod fetch;
pub mod fuzzy;
pub mod payload;
pub mod plan;
pub mod pydt;
pub mod pytext;
pub mod rules;
pub mod spec;
pub mod state;
mod tables;
pub mod tz;
pub mod value;
pub mod when;

pub use errors::{FormError, Result};
pub use fetch::{
    BusinessRulesOptions, BusinessRulesQuery, RulesResponse, RulesShape, business_rules_query,
    parse_business_rules,
};
pub use fuzzy::{fuzzy_filter, score as fuzzy_score};
pub use payload::{
    ModuleOptions, SrOptions, build_br_validation_excludes, build_change_payload, build_sr_payload,
    build_ticket_payload, decode_br_validation_excludes,
};
pub use plan::{
    MetadataFingerprint, PlanFiles, metadata_fingerprint, plan_hash, plan_hash_entries,
    plan_matches,
};
pub use rules::{
    ChoiceRestriction, Conflict, Decision, EngineOptions, EvalOptions, EvaluatedForm, ExcludeName,
    Excludes, Explanation, Message, RuleContext, RulesEngine, is_empty,
};
pub use spec::{Choice, Field, FormSpec, Kind, Section, ServiceItem};
pub use state::{Ack, Diff, EventOrder, FormState, LookupResolver, StateOptions};
pub use tz::{RAILS_ZONES, Zone, localize, resolve_timezone, zone};
pub use value::{LookupValue, Value, Values, norm};
pub use when::{Parsed, parse_when};
