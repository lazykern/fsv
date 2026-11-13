//! Change maintenance windows (`change_windows.py`, captured agent UI wires).
//!
//! * list: `GET /api/_/maintenance_windows` -> `{"maintenance_windows": [...]}`.
//! * create: the overlap check `POST /api/_/change_windows/overlapping_windows`
//!   (`type: "blackout_events"`), then `POST /api/_/maintenance_windows` with
//!   `{"change_window": {...}}` -> 201 `{"maintenance_window": {id, ...}}`.
//! * delete: `DELETE /api/_/maintenance_windows/<id>`; readback is absence
//!   from the list.
//!
//! The UI overlap check covers blackout windows only, so fsv also reads the
//! maintenance window list and refuses any listed window whose time range
//! intersects the new one. Wall-clock times on the wire are read by the
//! tenant in the agent profile time zone.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::{AmbiguousOffset, Offset};
use serde_json::{Map, Value, json};

use fsv_core::client::Body;
use fsv_core::error::{Error, Result};
use fsv_core::pystr::{casefold, strip};
use fsv_forms::pydt::{DateTimeValue, datetime_fromisoformat, offset_text};
use fsv_forms::tz::{Zone, fold0_offset, localize, tz_name, zone};
use fsv_py::{py_repr_str, py_str};

use crate::reviewed_write::{WritePlan, execute, refused};
use crate::service::{Api, Journal, obj, snapshot_hash};
use crate::watchers::{positive_id, total_differs};

pub const OVERLAP_PATH: &str = "change_windows/overlapping_windows";
const ACTION_CREATE: &str = "change_window_create";

/// What `create_plan` reads besides the client: the change schema
/// (`schema.load(CHANGES)`), the configured workspace, `FSV_TZ` and the
/// signed-in agent (`views.current_user`).
pub struct WindowEnv<'a> {
    pub schema: &'a dyn Fn() -> Result<Value>,
    pub configured_workspace: Value,
    pub fsv_tz: Option<String>,
    pub current_user: &'a dyn Fn() -> Result<Value>,
}

fn is_refusal(e: &Error) -> bool {
    matches!(e, Error::Exit { code: 2, .. })
}

fn json_sorted(v: &Value) -> String {
    fsv_py::Dumps::default().sort_keys(true).dump(v)
}

/// Port of fsv.change_windows.list_windows.
pub fn list_windows(c: &dyn Api) -> Result<Vec<Value>> {
    let raw = c.int_get("maintenance_windows", None)?;
    let rows = match raw.get("maintenance_windows") {
        Some(Value::Array(rows)) if rows.iter().all(Value::is_object) => rows.clone(),
        _ => return Err(refused("maintenance window list unavailable")),
    };
    let mut ids = Vec::new();
    for r in &rows {
        ids.push(positive_id(r.get("id").unwrap_or(&Value::Null))?);
    }
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != ids.len() {
        return Err(refused("maintenance window list has duplicate ids"));
    }
    if let Some(Value::Object(meta)) = raw.get("meta")
        && total_differs(meta.get("total_count").unwrap_or(&Value::Null), rows.len())
    {
        return Err(refused(
            "maintenance window list is incomplete; nothing selected",
        ));
    }
    Ok(rows)
}

fn when_shape(raw: &str) -> bool {
    let b = raw.as_bytes();
    let digits = |r: std::ops::Range<usize>| r.into_iter().all(|i| b[i].is_ascii_digit());
    if b.len() < 16
        || !digits(0..4)
        || b[4] != b'-'
        || !digits(5..7)
        || b[7] != b'-'
        || !digits(8..10)
        || !matches!(b[10], b' ' | b'T')
        || !digits(11..13)
        || b[13] != b':'
        || !digits(14..16)
    {
        return false;
    }
    match &b[16..] {
        [] | [b'Z'] => true,
        [sign, h1, h2, b':', m1, m2] => {
            matches!(sign, b'+' | b'-') && [h1, h2, m1, m2].iter().all(|d| d.is_ascii_digit())
        }
        _ => false,
    }
}

/// `_parse_when`: `YYYY-MM-DD HH:MM` (wall clock), optionally with an offset.
fn parse_when(text: &str) -> Result<(DateTime, Option<Offset>)> {
    let raw = strip(text);
    if !when_shape(raw) {
        return Err(refused(format!(
            "window time {} must be 'YYYY-MM-DD HH:MM' (in --timezone; an offset such as +07:00 may be added)",
            py_repr_str(text)
        )));
    }
    let iso = match raw.strip_suffix('Z') {
        Some(head) => format!("{head}+00:00"),
        None => raw.to_string(),
    };
    datetime_fromisoformat(&iso).ok_or_else(|| {
        refused(format!(
            "window time {} is not a valid date and time",
            py_repr_str(text)
        ))
    })
}

/// `_when`: wire form of a tenant wall-clock time (`DD-MM-YYYY`, `h:mmam`).
fn wire_when(v: DateTime) -> (String, String) {
    let hour = match v.hour() % 12 {
        0 => 12,
        h => h,
    };
    (
        format!("{:02}-{:02}-{:04}", v.day(), v.month(), v.year()),
        format!(
            "{hour}:{:02}{}",
            v.minute(),
            if v.hour() < 12 { "am" } else { "pm" }
        ),
    )
}

/// `tenant_zone`: the signed-in agent's profile `time_zone`; `FSV_TZ` does
/// not change it. Unknown refuses.
fn tenant_zone(env: &WindowEnv<'_>) -> Result<(Zone, String)> {
    let me = (env.current_user)()?;
    if let Some(Value::String(name)) = me.get("time_zone")
        && let Some(tz) = zone(name)
    {
        return Ok((tz, name.clone()));
    }
    Err(refused(
        "tenant time zone unknown: the agent profile (/api/v2/agents/me) shows no readable time_zone, and the tenant reads window times in that zone; nothing was planned",
    ))
}

/// `input_zone`: `--timezone`, else `FSV_TZ`, else the tenant zone.
fn input_zone(
    explicit: Option<&str>,
    fsv_tz: Option<&str>,
    tenant: &Zone,
) -> Result<(Zone, String)> {
    for (value, source) in [(explicit, "--timezone"), (fsv_tz, "FSV_TZ")] {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            return match zone(value) {
                Some(tz) => Ok((tz, source.to_string())),
                None => Err(refused(format!(
                    "unknown time zone {} ({source}); use an IANA name such as America/New_York",
                    py_repr_str(value)
                ))),
            };
        }
    }
    Ok((tenant.clone(), "tenant time zone".to_string()))
}

fn civil_minutes(v: DateTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        v.year(),
        v.month(),
        v.day(),
        v.hour(),
        v.minute()
    )
}

/// `_render`: `YYYY-MM-DD HH:MM <zone> (+HH:MM)` of an instant in `tz`.
fn render(ts: Timestamp, tz: &Zone) -> String {
    let off = tz.offset_at(ts);
    format!(
        "{} {} ({})",
        civil_minutes(off.to_datetime(ts)),
        tz_name(tz),
        offset_text(off)
    )
}

fn aware(local: DateTime, offset: Offset) -> DateTimeValue {
    DateTimeValue::new(local, offset)
}

/// `_instant_of`: a typed time as an aware datetime (wall clock in `tz`
/// unless it carries an offset).
fn instant_of(text: &str, tz: &Zone, what: &str) -> Result<DateTimeValue> {
    let (local, offset) = parse_when(text)?;
    if let Some(off) = offset {
        return Ok(aware(local, off));
    }
    localize(local, tz).map_err(|e| refused(format!("window {what}: {e}")))
}

/// `_tenant_wall`: the instant as tenant wall clock; a wall-clock time the
/// tenant could read as two instants refuses.
fn tenant_wall(value: &DateTimeValue, tz: &Zone, what: &str) -> Result<DateTimeValue> {
    let ts = value.timestamp();
    let off = tz.offset_at(ts);
    let local = off.to_datetime(ts);
    if matches!(
        tz.tz.to_ambiguous_timestamp(local).offset(),
        AmbiguousOffset::Fold { .. }
    ) {
        return Err(refused(format!(
            "window {what} is {} in the tenant zone {}, which happens twice there (the clocks go back) and the wire carries no offset; choose another time",
            civil_minutes(local),
            tz_name(tz)
        )));
    }
    Ok(aware(local, off))
}

/// `isoformat(timespec="minutes")` of an aware datetime.
fn iso_minutes(v: &DateTimeValue) -> String {
    let l = v.local;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}{}",
        l.year(),
        l.month(),
        l.day(),
        l.hour(),
        l.minute(),
        offset_text(v.offset)
    )
}

fn fields(sch: &Value) -> Vec<Value> {
    sch.get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn truthy_str(v: Option<&Value>) -> Option<String> {
    v.filter(|v| fsv_py::truthy(v)).map(py_str)
}

/// `_change_type_criterion`: the captured `change_type=<label or id>` row.
fn change_type_criterion(sch: &Value, expr: &str) -> Result<Value> {
    let (name, raw) = match expr.split_once('=') {
        Some((n, r)) => (n, r),
        None => (expr, ""),
    };
    if !expr.contains('=')
        || !["change_type", "change type", "type"].contains(&casefold(strip(name)).as_str())
    {
        return Err(refused(
            "only the captured condition 'change_type=<label or id>' is supported",
        ));
    }
    let Some(field) = fields(sch)
        .into_iter()
        .find(|f| f.get("name").and_then(Value::as_str) == Some("change_type"))
        .filter(fsv_py::truthy)
    else {
        return Err(refused(
            "change_type field unavailable in change form metadata",
        ));
    };
    let wanted = strip(raw);
    let choices: Vec<Value> = field
        .get("choices")
        .and_then(Value::as_array)
        .map(|c| c.iter().filter(|c| c.is_object()).cloned().collect())
        .unwrap_or_default();
    let mut ids: Vec<i64> = Vec::new();
    for c in &choices {
        let label = truthy_str(c.get("value"))
            .or_else(|| truthy_str(c.get("name")))
            .unwrap_or_default();
        if py_str(c.get("id").unwrap_or(&Value::Null)) == wanted
            || casefold(&label) == casefold(wanted)
        {
            let id = positive_id(c.get("id").unwrap_or(&Value::Null))?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.len() != 1 {
        let shown = choices
            .iter()
            .map(|c| {
                truthy_str(c.get("value"))
                    .unwrap_or_else(|| py_str(c.get("id").unwrap_or(&Value::Null)))
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(refused(format!(
            "no unique change type {}; options: {shown}",
            py_repr_str(wanted)
        )));
    }
    Ok(json!({"name": "change_type", "operator": "is", "value": ids[0].to_string()}))
}

/// Port of fsv.change_windows.WindowPlan.
#[derive(Debug, Clone)]
pub struct WindowPlan {
    pub write: WritePlan,
    pub overlap_json: String,
    /// Aware ISO instants (`YYYY-MM-DDTHH:MM+HH:MM`, tenant offset).
    pub start: String,
    pub end: String,
    /// Tenant zone (IANA key) the wire's wall clock is read in.
    pub time_zone: String,
    /// Zone `--start`/`--end` were read in, with its source.
    pub input_zone: String,
    /// Hash of the same plan before zones were bound.
    pub legacy_hash: String,
    /// Listed windows intersecting this one at review time; `None` when a
    /// listed window's times are unreadable.
    pub overlapping: Option<Vec<i64>>,
    pub allow_overlap: bool,
}

impl WindowPlan {
    pub fn hash(&self) -> String {
        self.write.hash()
    }

    fn zones(&self) -> Result<(Zone, Zone)> {
        let Some(tenant) = zone(&self.time_zone) else {
            return Err(refused("tenant time zone unknown; nothing was created"));
        };
        let entered = self.input_zone.split(" (").next().unwrap_or_default();
        let entered = zone(entered).unwrap_or_else(|| tenant.clone());
        Ok((tenant, entered))
    }

    /// The reviewed start and end instants.
    pub fn instants(&self) -> (Timestamp, Timestamp) {
        let at = |s: &str| {
            datetime_fromisoformat(s)
                .and_then(|(l, o)| o.map(|o| aware(l, o).timestamp()))
                .unwrap_or(Timestamp::MIN)
        };
        (at(&self.start), at(&self.end))
    }

    /// Port of fsv.change_windows.WindowPlan.review.
    pub fn review(&self) -> Result<Map<String, Value>> {
        let (tenant, entered) = self.zones()?;
        let (s, e) = self.instants();
        let mut out = self.write.review();
        out.insert(
            "time_zone".into(),
            json!({"input": self.input_zone, "tenant": format!(
                "{} (agent profile time zone; the tenant reads the sent wall clock in it)", self.time_zone)}),
        );
        out.insert(
            "start".into(),
            json!({"input": render(s, &entered), "tenant": render(s, &tenant)}),
        );
        out.insert(
            "end".into(),
            json!({"input": render(e, &entered), "tenant": render(e, &tenant)}),
        );
        let overlap: Value = serde_json::from_str(&self.overlap_json).unwrap_or(Value::Null);
        out.insert(
            "overlap_check".into(),
            json!({"method": "POST", "path": format!("/api/_/{OVERLAP_PATH}"), "payload": overlap}),
        );
        out.insert(
            "existing_window_check".into(),
            json!({"method": "GET", "path": "/api/_/maintenance_windows", "rule": format!(
                "any listed window whose time range intersects {} -> {} refuses; unreadable window times refuse",
                render(s, &tenant), render(e, &tenant))}),
        );
        out.insert("allow_overlap".into(), json!(self.allow_overlap));
        out.insert(
            "overlapping_window_ids".into(),
            self.overlapping
                .as_ref()
                .map_or(json!("unknown"), |ids| json!(ids)),
        );
        let warning = self.overlap_warning();
        if !warning.is_empty() {
            out.insert("overlap_warning".into(), json!(warning));
        }
        out.insert(
            "warning".into(),
            json!("The existing-window check and the overlap check run first; any overlapping window, or an answer that does not show 'no overlap', refuses the create unless --allow-overlap. Single attempt; never resend an unknown outcome."),
        );
        Ok(out)
    }

    /// Review-time verdict of the window-list check (`""` when clear).
    pub fn overlap_warning(&self) -> String {
        let then = if self.allow_overlap {
            "you chose to create it anyway (--allow-overlap)"
        } else {
            "sending refuses unless --allow-overlap"
        };
        match &self.overlapping {
            None => format!(
                "overlap unknown: a listed window has unreadable start/end times; {then}"
            ),
            Some(ids) if !ids.is_empty() => format!(
                "overlapping windows exist: {}; {then}",
                join_ids(ids)
            ),
            Some(_) if self.allow_overlap => "--allow-overlap: no overlapping window is listed now, but an overlap found at send or an unreadable overlap answer will not refuse the create".into(),
            Some(_) => String::new(),
        }
    }
}

fn join_ids(ids: &[i64]) -> String {
    ids.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Inputs of [`create_plan`] (`create_plan(client, name, description, start,
/// end, criteria, alert_suppression=, timezone=, allow_overlap=)`).
pub struct CreateArgs<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub start: &'a str,
    pub end: &'a str,
    pub criteria: &'a [String],
    pub alert_suppression: bool,
    pub timezone: Option<&'a str>,
    pub allow_overlap: bool,
}

/// Port of fsv.change_windows.create_plan.
pub fn create_plan(c: &dyn Api, env: &WindowEnv<'_>, a: &CreateArgs<'_>) -> Result<WindowPlan> {
    if strip(a.name).is_empty() {
        return Err(refused("window name is required"));
    }
    if a.criteria.is_empty() {
        return Err(refused(
            "at least one --criteria change_type=<type> row is required (the window's 'Applicable to Changes' condition)",
        ));
    }
    for text in [a.start, a.end] {
        parse_when(text)?;
    }
    let (tenant, tenant_raw) = tenant_zone(env)?;
    let (entered, source) = input_zone(a.timezone, env.fsv_tz.as_deref(), &tenant)?;
    let s_at = instant_of(a.start, &entered, "start")?;
    let e_at = instant_of(a.end, &entered, "end")?;
    if e_at.timestamp() <= s_at.timestamp() {
        return Err(refused("window end must be after its start"));
    }
    let s_wall = tenant_wall(&s_at, &tenant, "start")?;
    let e_wall = tenant_wall(&e_at, &tenant, "end")?;
    let (sd, st) = wire_when(s_wall.local);
    let (ed, et) = wire_when(e_wall.local);
    let sch = (env.schema)()?;
    let rows = a
        .criteria
        .iter()
        .map(|c| change_type_criterion(&sch, c))
        .collect::<Result<Vec<_>>>()?;
    let ws = crate::runtime_workspace(env.configured_workspace.clone(), Some(&sch));
    if ws.is_null() {
        return Err(refused("workspace unavailable; set forms.workspace_id"));
    }
    let ws = positive_id(&ws)?;
    let payload = json!({"change_window": {
        "name": a.name, "description": a.description, "start_date": sd, "start_time": st,
        "end_date": ed, "end_time": et,
        "change_event": {"additional_info": {}, "criteria": rows},
        "workspace_id": ws, "alert_suppression": a.alert_suppression,
    }});
    let overlap = json!({
        "type": "blackout_events", "start_date": sd, "start_time": st, "end_date": ed,
        "end_time": et, "criteria": rows, "workspace_id": ws,
    });
    let change_type: Vec<Value> = fields(&sch)
        .into_iter()
        .filter(|f| f.get("name").and_then(Value::as_str) == Some("change_type"))
        .collect();
    let windows = list_windows(c)?;
    let snapshot = json!({"windows": windows, "change_type": change_type});
    let write_plan = |context: String| WritePlan {
        action: ACTION_CREATE.into(),
        target: 0,
        tenant: c.domain(),
        method: "POST".into(),
        path: "maintenance_windows".into(),
        payload_json: json_sorted(&payload),
        context_hash: context,
    };
    let legacy = write_plan(snapshot_hash(&snapshot)?);
    let input_label = format!("{} ({source})", tz_name(&entered));
    let mut bound = obj(snapshot.clone());
    bound.insert("tenant_zone".into(), json!([tz_name(&tenant), tenant_raw]));
    bound.insert("input_zone".into(), json!(input_label));
    bound.insert("start".into(), json!(s_at.isoformat()));
    bound.insert("end".into(), json!(e_at.isoformat()));
    bound.insert("allow_overlap".into(), json!(a.allow_overlap));
    let write = write_plan(snapshot_hash(&Value::Object(bound))?);
    let found = match existing_overlaps(&windows, s_at.timestamp(), e_at.timestamp(), &tenant) {
        Ok(ids) => Some(ids),
        Err(e) if is_refusal(&e) => None,
        Err(e) => return Err(e),
    };
    Ok(WindowPlan {
        write,
        overlap_json: json_sorted(&overlap),
        start: iso_minutes(&s_wall),
        end: iso_minutes(&e_wall),
        time_zone: tz_name(&tenant).to_string(),
        input_zone: input_label,
        legacy_hash: legacy.hash(),
        overlapping: found,
        allow_overlap: a.allow_overlap,
    })
}

/// `_instant`: a listed window time (`Z` means UTC).
fn instant(value: Option<&Value>) -> Option<(DateTime, Option<Offset>)> {
    let text = strip(value?.as_str()?);
    if text.is_empty() {
        return None;
    }
    let text = match text.strip_suffix(['Z', 'z']) {
        Some(head) => format!("{head}+00:00"),
        None => text.to_string(),
    };
    datetime_fromisoformat(&text)
}

/// Port of fsv.change_windows.existing_overlaps: ids of listed windows whose
/// half-open time range intersects `start..end`. Offset-less times count
/// when they intersect as tenant wall clock or as UTC; unreadable times
/// refuse.
pub fn existing_overlaps(
    rows: &[Value],
    start: Timestamp,
    end: Timestamp,
    tz: &Zone,
) -> Result<Vec<i64>> {
    let mut found = Vec::new();
    for row in rows {
        let (rs, re) = (instant(row.get("start_time")), instant(row.get("end_time")));
        let (Some(rs), Some(re)) = (rs, re) else {
            return Err(unreadable(row));
        };
        if rs.1.is_some() != re.1.is_some() {
            return Err(unreadable(row));
        }
        let frames: Vec<(Timestamp, Timestamp)> = match (rs.1, re.1) {
            (Some(a), Some(b)) => vec![(aware(rs.0, a).timestamp(), aware(re.0, b).timestamp())],
            _ => {
                let wall = |l: DateTime| aware(l, fold0_offset(l, tz)).timestamp();
                let utc = |l: DateTime| aware(l, Offset::UTC).timestamp();
                vec![(wall(rs.0), wall(re.0)), (utc(rs.0), utc(re.0))]
            }
        };
        if frames.iter().any(|(a, b)| *a < end && start < *b) {
            found.push(positive_id(row.get("id").unwrap_or(&Value::Null))?);
        }
    }
    Ok(found)
}

fn unreadable(row: &Value) -> Error {
    let times = json!({
        "start_time": row.get("start_time").cloned().unwrap_or(Value::Null),
        "end_time": row.get("end_time").cloned().unwrap_or(Value::Null),
    });
    refused(format!(
        "existing maintenance window {} has unreadable start/end times (shape {}); overlap unknown, nothing was created. Check `fsv changes windows list` yourself; --allow-overlap skips this check",
        py_str(row.get("id").unwrap_or(&Value::Null)),
        shape(&times, 0)
    ))
}

const FLAG_WORDS: [&str; 2] = ["overlap", "conflict"];
/// Exact boolean keys without a flag word; `match_found` is the tenant's
/// native answer to `overlapping_windows`.
const FLAG_KEYS: [&str; 1] = ["match_found"];
const COUNT_KEYS: [&str; 6] = [
    "count",
    "total",
    "total_count",
    "overlap_count",
    "overlapping_count",
    "conflict_count",
];

fn unspecified() -> Value {
    json!({"overlap": "reported without details"})
}

/// `_shape`: type-only outline of an answer (never values).
fn shape(value: &Value, depth: usize) -> String {
    match value {
        Value::Object(m) if depth < 2 => {
            let inner = m
                .iter()
                .take(8)
                .map(|(k, v)| format!("{k}: {}", shape(v, depth + 1)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{inner}{}}}", if m.len() <= 8 { "" } else { ", ..." })
        }
        Value::Object(_) => "object".into(),
        Value::Array(_) => "list".into(),
        Value::Bool(_) => "bool".into(),
        Value::String(_) => "text".into(),
        Value::Null => "null".into(),
        Value::Number(_) => "number".into(),
    }
}

fn flag_word(word: &str) -> bool {
    FLAG_WORDS.iter().any(|w| word.contains(w))
}

fn walk(
    obj: &Map<String, Value>,
    depth: usize,
    rows: &mut Vec<Value>,
    flags: &mut Vec<bool>,
) -> bool {
    for (key, value) in obj {
        let word = casefold(key);
        match value {
            Value::Array(items) => rows.extend(items.iter().cloned()),
            Value::Bool(b) if FLAG_KEYS.contains(&word.as_str()) || flag_word(&word) => {
                flags.push(*b);
            }
            Value::Number(n)
                if (n.is_u64() || n.as_i64().is_some_and(|i| i >= 0))
                    && (COUNT_KEYS.contains(&word.as_str()) || flag_word(&word)) =>
            {
                flags.push(n.as_u64().is_some_and(|v| v > 0));
            }
            Value::Object(inner) if depth == 0 => {
                if !walk(inner, 1, rows, flags) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// Port of fsv.change_windows._overlaps: conflicting windows from the
/// overlap-check answer; unknown shapes or contradictions refuse.
pub fn overlaps(response: &Value) -> Result<Vec<Value>> {
    let mut response = response.clone();
    if let Value::String(s) = &response {
        let word = casefold(strip(s));
        if word == "true" || word == "false" {
            response = Value::Bool(word == "true");
        }
    }
    match &response {
        Value::Bool(true) => return Ok(vec![unspecified()]),
        Value::Bool(false) => return Ok(Vec::new()),
        Value::Array(items) => return Ok(items.clone()),
        _ => {}
    }
    let mut rows = Vec::new();
    let mut flags = Vec::new();
    if let Value::Object(m) = &response
        && walk(m, 0, &mut rows, &mut flags)
    {
        let any = flags.iter().any(|f| *f);
        if any && rows.is_empty() {
            return Ok(vec![unspecified()]);
        }
        if !rows.is_empty() && !flags.is_empty() && !any {
            return Err(refused(
                "overlap check answer is contradictory (no-overlap flag with windows listed); nothing was created",
            ));
        }
        return Ok(rows);
    }
    Err(refused(format!(
        "overlap check answer not understood (shape {}); nothing was created. Check `fsv changes windows list` yourself; --allow-overlap skips this check",
        shape(&response, 0)
    )))
}

/// Port of fsv.change_windows.send_create. Whether overlaps refuse is
/// `plan.allow_overlap` (part of the reviewed hash).
pub fn send_create(
    plan: &WindowPlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
    rebuild: Option<&dyn Fn() -> Result<WindowPlan>>,
) -> Result<Value> {
    let allow = plan.allow_overlap;
    let Some(reviewed) = reviewed_hash else {
        return Ok(json!({"status": "cancelled", "verified": false}));
    };
    let hash = plan.hash();
    let guards: Vec<&str> = [hash.as_str(), plan.legacy_hash.as_str()]
        .into_iter()
        .filter(|h| !h.is_empty())
        .collect();
    let kind = format!("plan:{}", plan.write.action);
    let events: Vec<Value> = journal
        .read()
        .into_iter()
        .filter(|e| {
            e.get("kind").and_then(Value::as_str) == Some(kind.as_str())
                && e.get("plan")
                    .and_then(Value::as_str)
                    .is_some_and(|p| guards.contains(&p))
        })
        .collect();
    if let Some(msg) = fsv_core::journal::replay_refusal(
        &events,
        "window create was sent or outcome unknown; inspect the window list, do not replay",
    ) {
        return Err(refused(msg));
    }
    let changed = reviewed != hash
        || match rebuild {
            Some(r) => r()?.hash() != hash,
            None => false,
        };
    if changed {
        return Err(refused(
            "windows or metadata changed since review; review again (nothing was sent)",
        ));
    }
    let (start, end) = plan.instants();
    let existing = match list_windows(c)
        .and_then(|rows| existing_overlaps(&rows, start, end, &plan.zones()?.0))
    {
        Ok(ids) => Some(ids),
        Err(e) if is_refusal(&e) && allow => None,
        Err(e) => return Err(e),
    };
    if let Some(ids) = existing.as_ref().filter(|ids| !ids.is_empty())
        && !allow
    {
        return Err(refused(format!(
            "{} overlapping window(s) already listed ({}); nothing was created (review them, then pass --allow-overlap)",
            ids.len(),
            join_ids(ids)
        )));
    }
    let csrf = c.get_csrf()?;
    let answer = c.send_once(
        "POST",
        OVERLAP_PATH,
        Body::Raw(plan.overlap_json.clone().into_bytes()),
        None,
        &csrf,
    )?;
    let conflicts = match overlaps(&answer) {
        Ok(rows) => Some(rows),
        Err(e) if is_refusal(&e) && allow => None,
        Err(e) => return Err(e),
    };
    if let Some(rows) = conflicts.as_ref().filter(|r| !r.is_empty())
        && !allow
    {
        return Err(refused(format!(
            "{} overlapping window(s); nothing was created (review them, then pass --allow-overlap)",
            rows.len()
        )));
    }
    let payload = plan.write.payload();
    let name = payload["change_window"]["name"].clone();
    let verify = |response: &Value| -> Result<Map<String, Value>> {
        let row = response.get("maintenance_window").filter(|r| r.is_object());
        let wid = positive_id(row.and_then(|r| r.get("id")).unwrap_or(&Value::Null))?;
        let mut listed = Vec::new();
        for r in list_windows(c)? {
            if positive_id(r.get("id").unwrap_or(&Value::Null))? == wid {
                listed.push(r);
            }
        }
        let one = (listed.len() == 1).then(|| &listed[0]);
        let state = if one.is_some_and(|r| r.get("name").unwrap_or(&Value::Null) == &name) {
            "matched"
        } else {
            "mismatched"
        };
        let mut fields = Map::new();
        fields.insert("name".into(), json!(state));
        for (key, want) in [("start", start), ("end", end)] {
            let got = one.and_then(|r| instant(r.get(format!("{key}_time"))));
            let verdict = match got {
                Some((l, Some(o))) if aware(l, o).timestamp() == want => "matched",
                Some((_, Some(_))) => "mismatched",
                _ => "not exposed",
            };
            fields.insert(key.into(), json!(verdict));
        }
        let total = match (&conflicts, &existing) {
            (Some(cf), Some(ex)) => json!(cf.len() + ex.len()),
            _ => json!("unknown"),
        };
        let verified = !fields.values().any(|v| v == "mismatched");
        Ok(obj(json!({
            "window_id": wid, "fields": fields, "overlaps": total,
            "overlapping_window_ids": existing.as_ref().map_or(json!("unknown"), |ids| json!(ids)),
            "verified": verified,
        })))
    };
    let same = || Ok(plan.write.clone());
    execute(&plan.write, c, journal, Some(reviewed), &same, &verify)
}

/// Port of fsv.change_windows.delete_plan.
pub fn delete_plan(c: &dyn Api, ident: i64) -> Result<WritePlan> {
    let wid = positive_id(&json!(ident))?;
    let mut rows = Vec::new();
    for r in list_windows(c)? {
        if positive_id(r.get("id").unwrap_or(&Value::Null))? == wid {
            rows.push(r);
        }
    }
    if rows.len() != 1 {
        return Err(refused(format!("maintenance window {wid} not found")));
    }
    Ok(WritePlan {
        action: "change_window_delete".into(),
        target: wid,
        tenant: c.domain(),
        method: "DELETE".into(),
        path: format!("maintenance_windows/{wid}"),
        payload_json: "null".into(),
        context_hash: snapshot_hash(&json!({"window": rows[0]}))?,
    })
}

/// Port of fsv.change_windows.send_delete: readback is absence from the list.
pub fn send_delete(
    plan: &WritePlan,
    c: &dyn Api,
    journal: &dyn Journal,
    reviewed_hash: Option<&str>,
) -> Result<Value> {
    let verify = |_: &Value| -> Result<Map<String, Value>> {
        let mut gone = true;
        for r in list_windows(c)? {
            if positive_id(r.get("id").unwrap_or(&Value::Null))? == plan.target {
                gone = false;
            }
        }
        Ok(obj(json!({
            "window_id": plan.target,
            "fields": {"absent": if gone { "matched" } else { "mismatched" }},
            "verified": gone,
        })))
    };
    let rebuild = || delete_plan(c, plan.target);
    execute(plan, c, journal, reviewed_hash, &rebuild, &verify)
}

#[cfg(test)]
#[path = "change_windows_tests.rs"]
mod tests;
