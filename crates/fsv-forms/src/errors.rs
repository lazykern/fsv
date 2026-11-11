//! Errors raised by the form engine (`fsv.forms.errors`).
//!
//! Every variant except [`FormError::Value`] and [`FormError::Type`] is a
//! Python `FormError` subclass. The CLI catches `FormError` around form
//! flows and exits 2; plain `ValueError`s (`Value`) also map to exit 2 via
//! `errors.exit_code_for`. A `FormError` that escapes those handlers maps
//! to exit 1 in Python ([`FormError::is_value_error`] tells them apart).

use crate::value::{LookupValue, Value};

#[derive(Debug, thiserror::Error)]
pub enum FormError {
    /// `FormError(message)`: the base class raised directly (e.g. by formmeta).
    #[error("{0}")]
    Other(String),

    /// `UnknownFieldError`: no field matches the API name or label.
    #[error("{}", unknown_field_message(.name, .suggestions))]
    UnknownField {
        name: String,
        suggestions: Vec<String>,
    },

    /// `ValueParseError`: a raw value could not be parsed/validated.
    #[error("{field}: {message}")]
    ValueParse {
        field: String,
        raw: Value,
        message: String,
        options: Vec<String>,
    },

    /// `AmbiguousLookupError`: a lookup matched several records.
    #[error("{}", ambiguous_message(.field, .query, .candidates))]
    AmbiguousLookup {
        field: String,
        query: String,
        candidates: Vec<LookupValue>,
    },

    /// `FieldDisabledError`.
    #[error("{0}")]
    FieldDisabled(String),

    /// `FormRuleError` (`unsupported`: the `UnsupportedRuleError` subclass).
    #[error("{message}")]
    FormRule {
        message: String,
        rule_id: serde_json::Value,
        rule_name: Option<String>,
        unsupported: bool,
    },

    /// `PayloadError`: problems joined by `"; "`.
    #[error("{}", payload_message(.problems))]
    Payload { problems: Vec<String> },

    /// A lookup resolver failed (Python: whatever the resolver raised,
    /// usually an `APIError`). Downcast the source to recover it.
    #[error("{0}")]
    Lookup(Box<dyn std::error::Error + Send + Sync>),

    /// A plain Python `ValueError` (not a `FormError`).
    #[error("{0}")]
    Value(String),

    /// A plain Python `TypeError`.
    #[error("{0}")]
    Type(String),
}

fn unknown_field_message(name: &str, suggestions: &[String]) -> String {
    let mut msg = format!("unknown field: {}", fsv_py::py_repr_str(name));
    if !suggestions.is_empty() {
        msg.push_str(&format!(" (did you mean: {})", suggestions.join(", ")));
    }
    msg
}

fn ambiguous_message(field: &str, query: &str, candidates: &[LookupValue]) -> String {
    let shown: Vec<String> = candidates
        .iter()
        .take(10)
        .map(|c| {
            if c.display.is_empty() {
                fsv_py::py_str(&c.id)
            } else {
                c.display.clone()
            }
        })
        .collect();
    format!(
        "{field}: {} matches {} records: {}",
        fsv_py::py_repr_str(query),
        candidates.len(),
        shown.join(", ")
    )
}

fn payload_message(problems: &[String]) -> String {
    if problems.is_empty() {
        "invalid payload".to_string()
    } else {
        problems.join("; ")
    }
}

impl FormError {
    pub(crate) fn parse(field: &str, raw: &Value, message: impl Into<String>) -> Self {
        FormError::ValueParse {
            field: field.to_string(),
            raw: raw.clone(),
            message: message.into(),
            options: vec![],
        }
    }

    pub(crate) fn parse_opts(
        field: &str,
        raw: &Value,
        message: impl Into<String>,
        options: Vec<String>,
    ) -> Self {
        FormError::ValueParse {
            field: field.to_string(),
            raw: raw.clone(),
            message: message.into(),
            options,
        }
    }

    pub(crate) fn payload<S: Into<String>>(problem: S) -> Self {
        FormError::Payload {
            problems: vec![problem.into()],
        }
    }

    /// True for Python `FormError` subclasses (caught by the CLI's form
    /// handlers), false for plain `ValueError`/`TypeError`/resolver errors.
    pub fn is_form_error(&self) -> bool {
        !matches!(
            self,
            FormError::Value(_) | FormError::Type(_) | FormError::Lookup(_)
        )
    }

    /// True when Python raises a `ValueError` (exit code 2 through
    /// `errors.exit_code_for`).
    pub fn is_value_error(&self) -> bool {
        matches!(self, FormError::Value(_))
    }

    /// Exit code the CLI uses when it catches this error around a form flow
    /// (`except FormError: exit 2`; `ValueError` maps to 2 as well).
    pub fn exit_code(&self) -> i32 {
        match self {
            FormError::Type(_) | FormError::Lookup(_) => 1,
            _ => 2,
        }
    }

    /// Options offered for a `ValueParse` error (empty otherwise).
    pub fn options(&self) -> &[String] {
        match self {
            FormError::ValueParse { options, .. } => options,
            _ => &[],
        }
    }

    /// Problems of a `Payload` error (empty otherwise).
    pub fn problems(&self) -> &[String] {
        match self {
            FormError::Payload { problems } => problems,
            _ => &[],
        }
    }
}

pub type Result<T, E = FormError> = std::result::Result<T, E>;

#[cfg(test)]
mod py_tests;
