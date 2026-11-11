//! The Properties pane (`views.load_properties`, `load_edit_rules`,
//! `_prop_value`): form fields in web order, visible per the edit-form
//! business rules, with display values. The result is what
//! [`crate::load_detail`] takes as [`crate::Properties`].
//!
//! The implementation is [`fsv_api::views`]; these keep this crate's
//! [`SessionError`] signatures.

use std::sync::Arc;

use jiff::tz::TimeZone;
use serde_json::Value;

use fsv_api::formmeta::MetaStore;
use fsv_api::service::Api;
use fsv_api::site::Prop;
use fsv_core::error::Error;
use fsv_core::resource::Resource;

use crate::source::SessionError;

#[cfg(test)]
use {
    crate::source::record_for_form,
    fsv_api::formmeta,
    fsv_api::views::{module_of, prop_value},
    fsv_forms::rules::EngineOptions,
    fsv_forms::{FormSpec, FormState, RulesEngine, StateOptions},
};

/// Only a session failure leaves the loaders as an error.
fn session(e: Error) -> SessionError {
    match e {
        Error::Session(msg) => SessionError(msg),
        other => SessionError(other.to_string()),
    }
}

/// `load_edit_rules`: [`fsv_api::views::load_edit_rules`].
pub fn load_edit_rules(
    c: &dyn Api,
    store: &dyn MetaStore,
    module: &str,
    record: &Value,
    raw_fields: &[Value],
) -> Result<(Option<Value>, Option<String>), SessionError> {
    fsv_api::views::load_edit_rules(c, store, module, record, raw_fields).map_err(session)
}

/// `load_properties`: [`fsv_api::views::load_properties`].
pub fn load_properties(
    c: Arc<dyn Api + Send + Sync>,
    store: &dyn MetaStore,
    res: &Resource,
    record: &Value,
    schema: Option<&Value>,
    tz: Option<&TimeZone>,
    user: Option<&Value>,
) -> Result<(Vec<Prop>, Option<String>), SessionError> {
    fsv_api::views::load_properties(&c, store, res, record, schema, tz, user).map_err(session)
}

#[cfg(test)]
mod golden;
