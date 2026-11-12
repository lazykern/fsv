//! Captured canned-response reads and local insertion into a reviewed reply
//! (`canned.py`).
//!
//! Detail payload keys are not a portable template schema: the caller selects
//! an explicit JSON pointer from `show`. No placeholders, attachments or
//! signatures are expanded, and the complete source is rechecked before
//! sending.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use fsv_core::error::{Error, Result};
use fsv_py::py_str;

use crate::conversations::{self, ConversationArgs, ConversationPlan, MailArgs};
use crate::formmeta::MetaStore;
use crate::notes::BodyFormat;
use crate::reviewed_write::refused;
use crate::service::{Api, snapshot_hash};
use crate::watchers::positive_id;

/// Port of fsv.canned.folders.
pub fn folders(c: &dyn Api) -> Result<Value> {
    c.int_get("canned_response_folders", None)
}

/// Port of fsv.canned.responses.
pub fn responses(c: &dyn Api, folder_id: &Value) -> Result<Value> {
    let id = positive_id(folder_id)?;
    c.int_get(
        &format!("canned_response_folders/{id}/canned_responses"),
        None,
    )
}

/// Port of fsv.canned.show.
pub fn show(c: &dyn Api, response_id: &Value) -> Result<Value> {
    let id = positive_id(response_id)?;
    c.int_get(&format!("canned_responses/{id}"), None)
}

/// Port of fsv.canned.content: the string an RFC 6901 object pointer
/// selects, never a guessed key.
pub fn content(raw: &Value, pointer: &str) -> Result<String> {
    static PLACEHOLDER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\{\{|\{%|\$\{|\[\[").expect("regex"));
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(Error::value(
            "content pointer must start with /; inspect canned show first",
        ));
    };
    let mut value = raw;
    for part in rest.split('/') {
        let bytes = part.as_bytes();
        if bytes
            .iter()
            .enumerate()
            .any(|(i, b)| *b == b'~' && !matches!(bytes.get(i + 1), Some(b'0' | b'1')))
        {
            return Err(Error::value("invalid JSON pointer escape"));
        }
        let key = part.replace("~1", "/").replace("~0", "~");
        value = value
            .as_object()
            .and_then(|m| m.get(&key))
            .ok_or_else(|| Error::value("content pointer does not identify a source field"))?;
    }
    let text = match value.as_str() {
        Some(s) if !fsv_core::pystr::strip(s).is_empty() => s,
        _ => {
            return Err(Error::value(
                "selected canned content must be nonempty text/HTML",
            ));
        }
    };
    if PLACEHOLDER.is_match(text) {
        return Err(Error::value(
            "unresolved canned placeholders require the web editor",
        ));
    }
    Ok(text.to_string())
}

/// Port of fsv.canned.reply_plan: an HTML reply from the selected content,
/// bound to the complete source.
pub fn reply_plan(
    c: &dyn Api,
    store: &dyn MetaStore,
    target: &str,
    response_id: &Value,
    pointer: &str,
    sender: &str,
) -> Result<ConversationPlan> {
    let id = positive_id(response_id)?;
    let raw = show(c, &json!(id))?;
    let args = ConversationArgs {
        format: BodyFormat::Html,
        ..ConversationArgs::default()
    };
    let mail = MailArgs {
        bcc: &[],
        sender: Some(sender),
    };
    let plan =
        conversations::build_plan_mail(c, store, target, &content(&raw, pointer)?, &args, &mail)?;
    Ok(ConversationPlan {
        canned_id: Some(id),
        canned_pointer: Some(pointer.to_string()),
        canned_hash: Some(snapshot_hash(&raw)?),
        ..plan
    })
}

/// Port of fsv.canned.freshen: the source still hashes the same and still
/// selects the reviewed body.
pub fn freshen(plan: &ConversationPlan, c: &dyn Api) -> Result<()> {
    let changed = || refused("canned source changed; review again");
    let raw = show(c, &json!(plan.canned_id))?;
    if Some(snapshot_hash(&raw)?) != plan.canned_hash {
        return Err(changed());
    }
    let body = plan.payload().get("body").cloned().unwrap_or(Value::Null);
    if json!(content(
        &raw,
        plan.canned_pointer.as_deref().unwrap_or_default()
    )?) != body
    {
        return Err(changed());
    }
    Ok(())
}

/// Port of fsv.canned.scenarios.
pub fn scenarios(c: &dyn Api) -> Result<Value> {
    c.int_get("scenario_automations", None)
}

/// Port of fsv.canned.scenario: the exact inventory row; there is no
/// captured detail endpoint.
pub fn scenario(c: &dyn Api, scenario_id: &Value) -> Result<Map<String, Value>> {
    let raw = scenarios(c)?;
    let rows = match &raw {
        Value::Object(m) => m.get("scenario_automations").unwrap_or(&Value::Null),
        other => other,
    };
    let Some(rows) = rows.as_array() else {
        return Err(Error::value(
            "scenario inventory is not a list; inspect scenarios list",
        ));
    };
    let want = positive_id(scenario_id)?.to_string();
    let matches: Vec<&Map<String, Value>> = rows
        .iter()
        .filter_map(Value::as_object)
        .filter(|r| py_str(r.get("id").unwrap_or(&Value::Null)) == want)
        .collect();
    match matches[..] {
        [one] => Ok(one.clone()),
        _ => Err(Error::value(
            "scenario identity is missing or ambiguous in this inventory",
        )),
    }
}
