//! Status meaning from runtime metadata, never from English labels
//! (`status_semantics.py`).
//!
//! Freshservice's built-in statuses keep fixed internal ids whatever a tenant
//! renames or translates them to (tickets: 2 Open, 3 Pending, 4 Resolved,
//! 5 Closed; problems: 1 Open, 3 Closed). Custom statuses get other ids and
//! never resolve or close a record. A role ("open", "pending", "resolved",
//! "closed") is identified only when the record's own runtime status choices
//! contain its internal id exactly once; anything else is ambiguous and
//! callers refuse rather than guess from a label.
//!
//! The words "Open", "Pending", "Resolved" and "Closed" appear only as CLI
//! vocabulary (`fsv tickets resolve`, `--set-status Pending`): they name a
//! role, which is then looked up by internal id in the tenant's choices.

use serde_json::Value;

use fsv_core::pystr::{casefold, strip};
use fsv_py::py_repr_str;

use crate::choice_ids::runtime_choice_id;

/// `BUILTIN_IDS`: role -> internal id, per module, in Python dict order.
pub fn builtin_ids(module: &str) -> Option<&'static [(&'static str, i64)]> {
    match module {
        "ticket" => Some(&[("open", 2), ("pending", 3), ("resolved", 4), ("closed", 5)]),
        "problem" => Some(&[("open", 1), ("closed", 3)]),
        _ => None,
    }
}

/// `TERMINAL_ROLES`.
pub const TERMINAL_ROLES: [&str; 2] = ["resolved", "closed"];

/// `StatusSemanticsError` (a `ValueError`): a status role cannot be
/// identified from runtime metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticsError(pub String);

impl std::fmt::Display for SemanticsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

type Res<T> = std::result::Result<T, SemanticsError>;

/// Port of status_semantics.Role: a status meaning requested by a command
/// (not a tenant label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Open,
    Pending,
    Resolved,
    Closed,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Open => "open",
            Role::Pending => "pending",
            Role::Resolved => "resolved",
            Role::Closed => "closed",
        }
    }

    pub fn from_name(name: &str) -> Option<Role> {
        Some(match name {
            "open" => Role::Open,
            "pending" => Role::Pending,
            "resolved" => Role::Resolved,
            "closed" => Role::Closed,
            _ => return None,
        })
    }
}

/// A requested status: a [`Role`] or a tenant label (`Role | str`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted<'a> {
    Role(Role),
    Label(&'a str),
}

impl<'a> From<&'a str> for Wanted<'a> {
    fn from(label: &'a str) -> Self {
        Wanted::Label(label)
    }
}

impl<'a> From<&'a String> for Wanted<'a> {
    fn from(label: &'a String) -> Self {
        Wanted::Label(label)
    }
}

impl From<Role> for Wanted<'_> {
    fn from(role: Role) -> Self {
        Wanted::Role(role)
    }
}

/// A runtime status choice: a raw metadata dict or a parsed form choice.
pub trait StatusChoice {
    /// `_ident`.
    fn ident(&self) -> Option<i64>;
    /// `_visible`.
    fn visible(&self) -> bool;
    /// `_label`.
    fn shown(&self) -> String;
    /// The two texts `select` matches a tenant label against.
    fn texts(&self) -> [String; 2];
}

impl StatusChoice for Value {
    fn ident(&self) -> Option<i64> {
        runtime_choice_id(self.as_object().and_then(|m| m.get("id")), 1)
    }
    fn visible(&self) -> bool {
        self.as_object()
            .and_then(|m| m.get("visible"))
            .is_none_or(|v| v != &Value::Bool(false))
    }
    fn shown(&self) -> String {
        let m = self.as_object();
        let get = |k: &str| m.and_then(|m| m.get(k)).filter(|v| fsv_py::truthy(v));
        get("value")
            .or_else(|| get("label"))
            .or_else(|| get("name"))
            .map_or_else(
                || fsv_py::py_str(m.and_then(|m| m.get("id")).unwrap_or(&Value::Null)),
                fsv_py::py_str,
            )
    }
    fn texts(&self) -> [String; 2] {
        let m = self.as_object();
        let text = |k: &str| {
            m.and_then(|m| m.get(k))
                .filter(|v| fsv_py::truthy(v))
                .map(fsv_py::py_str)
                .unwrap_or_default()
        };
        [text("value"), text("label")]
    }
}

impl StatusChoice for fsv_forms::Choice {
    fn ident(&self) -> Option<i64> {
        runtime_choice_id(self.id.as_ref(), 1)
    }
    fn visible(&self) -> bool {
        self.visible
    }
    fn shown(&self) -> String {
        if !self.label.is_empty() {
            self.label.clone()
        } else if !self.value.is_empty() {
            self.value.clone()
        } else {
            self.id.as_ref().map(fsv_py::py_str).unwrap_or_default()
        }
    }
    fn texts(&self) -> [String; 2] {
        [self.label.clone(), self.value.clone()]
    }
}

fn module_of(module: &str) -> Res<&'static str> {
    let lowered = strip(module).to_lowercase();
    let m = lowered.trim_end_matches('s');
    match m {
        "ticket" => Ok("ticket"),
        "problem" => Ok("problem"),
        _ => Err(SemanticsError(format!(
            "no status semantics for {}",
            py_repr_str(module)
        ))),
    }
}

fn count<T: StatusChoice>(choices: &[&T], ident: i64) -> usize {
    choices.iter().filter(|c| c.ident() == Some(ident)).count()
}

/// Port of status_semantics.find: the runtime choice for `role` (by
/// internal id); refuse when absent or duplicated.
pub fn find<'c, T: StatusChoice>(choices: &[&'c T], module: &str, role: Role) -> Res<&'c T> {
    let name = role.name();
    let ids = builtin_ids(module_of(module)?).unwrap_or(&[]);
    let Some((_, ident)) = ids.iter().find(|(n, _)| *n == name) else {
        return Err(SemanticsError(format!(
            "{module} has no built-in {} status",
            py_repr_str(name)
        )));
    };
    let hits: Vec<&&T> = choices
        .iter()
        .filter(|c| c.ident() == Some(*ident))
        .collect();
    if hits.len() != 1 {
        return Err(SemanticsError(format!(
            "the {name} status is not identifiable from the runtime status metadata (the built-in internal id is {}); pass the tenant's own status label explicitly",
            if hits.is_empty() {
                "missing"
            } else {
                "duplicated"
            }
        )));
    }
    Ok(hits[0])
}

/// Port of status_semantics.role_of: the built-in role of a status id, only
/// when the runtime choices carry that id; `None` for custom or unknown
/// statuses.
pub fn role_of<T: StatusChoice>(
    choices: &[&T],
    module: &str,
    value: Option<&Value>,
) -> Res<Option<&'static str>> {
    let Some(ident) = runtime_choice_id(value, 1) else {
        return Ok(None);
    };
    if count(choices, ident) != 1 {
        return Ok(None);
    }
    let ids = builtin_ids(module_of(module)?).unwrap_or(&[]);
    Ok(ids.iter().find(|(_, b)| *b == ident).map(|(n, _)| *n))
}

/// Port of status_semantics.closes: `Some(true)` the status resolves/closes
/// the record; `Some(false)` it does not; `None` ambiguous (the runtime
/// choices do not carry every built-in terminal id, so a custom-looking
/// status cannot be ruled out).
pub fn closes<T: StatusChoice>(
    choices: &[&T],
    module: &str,
    value: Option<&Value>,
) -> Res<Option<bool>> {
    if let Some(role) = role_of(choices, module, value)? {
        return Ok(Some(TERMINAL_ROLES.contains(&role)));
    }
    let Some(ident) = runtime_choice_id(value, 1) else {
        return Ok(None);
    };
    if count(choices, ident) != 1 {
        return Ok(None);
    }
    let ids = builtin_ids(module_of(module)?).unwrap_or(&[]);
    let all_terminal_present = ids
        .iter()
        .filter(|(n, _)| TERMINAL_ROLES.contains(n))
        .all(|(_, t)| count(choices, *t) == 1);
    Ok(if all_terminal_present {
        Some(false)
    } else {
        None
    })
}

/// Port of status_semantics.select: a visible runtime choice for a [`Role`]
/// (by internal id) or a tenant label (exact, case-insensitive). An English
/// role word that is not a tenant label is read as that role. Refuses
/// ambiguity.
pub fn select<'c, T: StatusChoice>(
    choices: &[&'c T],
    module: &str,
    wanted: Wanted<'_>,
) -> Res<&'c T> {
    let pool: Vec<&'c T> = choices.iter().copied().filter(|c| c.visible()).collect();
    let text = match wanted {
        Wanted::Role(role) => return find(&pool, module, role),
        Wanted::Label(text) => strip(text),
    };
    let folded = casefold(text);
    let hits: Vec<&'c T> = pool
        .iter()
        .copied()
        .filter(|c| !folded.is_empty() && c.texts().iter().any(|t| casefold(t) == folded))
        .collect();
    match hits.len() {
        1 => return Ok(hits[0]),
        0 => {}
        _ => {
            return Err(SemanticsError(format!(
                "status {} matches several runtime statuses; use a unique label",
                py_repr_str(text)
            )));
        }
    }
    let ids = builtin_ids(module_of(module)?).unwrap_or(&[]);
    if let Some(role) = ids
        .iter()
        .find(|(n, _)| *n == folded)
        .and_then(|(n, _)| Role::from_name(n))
    {
        return find(&pool, module, role);
    }
    let shown: Vec<String> = pool.iter().take(12).map(|c| c.shown()).collect();
    let mut msg = format!("status {} is not a runtime status label", py_repr_str(text));
    if !shown.is_empty() {
        msg.push_str(&format!("; choices: {}", shown.join(", ")));
    }
    Err(SemanticsError(msg))
}

/// Port of safety.status_closes: decided by the built-in internal status
/// ids present in the runtime `choices`, never by the label. Resources
/// without built-in status semantics are always unknown.
pub fn status_closes<T: StatusChoice>(
    resource: &str,
    value: Option<&Value>,
    choices: &[&T],
) -> Option<bool> {
    let module = resource.strip_suffix('s').unwrap_or(resource);
    builtin_ids(module)?;
    closes(choices, module, value).ok().flatten()
}

#[cfg(test)]
mod tests;
