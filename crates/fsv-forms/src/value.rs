//! Form values: what Python stores in `FormState.values` and
//! `EvaluatedForm.values` (str, int, float, bool, tuples, `LookupValue`,
//! `date`, aware `datetime`), with Python `str()`, `repr()` and `==`.

use std::fmt;

use jiff::civil::Date;
use serde_json::{Map, Number, Value as Json};

use crate::pydt::{DateTimeValue, date_iso};
use crate::pytext::{casefold, squash_ws};

/// A resolved person/group/agent (or other lookup record).
#[derive(Clone, Debug, PartialEq)]
pub struct LookupValue {
    pub id: Json,
    pub display: String,
    pub email: Option<String>,
}

impl LookupValue {
    /// `LookupValue(id=...)` with no display text.
    pub fn id(id: impl Into<Json>) -> Self {
        Self {
            id: id.into(),
            display: String::new(),
            email: None,
        }
    }

    pub fn new(id: impl Into<Json>, display: impl Into<String>, email: Option<&str>) -> Self {
        Self {
            id: id.into(),
            display: display.into(),
            email: email.map(str::to_string),
        }
    }

    /// `repr(lookup_value)` (dataclass repr).
    pub fn py_repr(&self) -> String {
        let email = match &self.email {
            Some(e) => fsv_py::py_repr_str(e),
            None => "None".to_string(),
        };
        format!(
            "LookupValue(id={}, display={}, email={email})",
            fsv_py::py_repr(&self.id),
            fsv_py::py_repr_str(&self.display)
        )
    }
}

impl fmt::Display for LookupValue {
    /// `Name <email>`, else the name, the email or the id.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.display, &self.email) {
            (d, Some(e)) if !d.is_empty() && !e.is_empty() => write!(f, "{d} <{e}>"),
            (d, _) if !d.is_empty() => f.write_str(d),
            (_, Some(e)) if !e.is_empty() => f.write_str(e),
            _ => f.write_str(&fsv_py::py_str(&self.id)),
        }
    }
}

/// One form value or raw answer.
///
/// `List` is a Python tuple when stored by the engine (multi-selects,
/// multi-lookups, attachment paths) and a list when it is a raw answer;
/// [`Value::py_str`] renders tuples, [`Value::raw_repr`] lists.
/// `Json` carries any other server value (objects) unchanged.
#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Lookup(LookupValue),
    Date(Date),
    DateTime(DateTimeValue),
    Json(Json),
}

impl Value {
    pub fn str(s: impl Into<String>) -> Self {
        Value::Str(s.into())
    }

    pub fn list<I: IntoIterator<Item = Value>>(items: I) -> Self {
        Value::List(items.into_iter().collect())
    }

    /// `is_empty(value)`: None, `""` or an empty collection.
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Str(s) => s.is_empty(),
            Value::List(l) => l.is_empty(),
            Value::Json(Json::Null) => true,
            Value::Json(Json::String(s)) => s.is_empty(),
            Value::Json(Json::Array(a)) => a.is_empty(),
            _ => false,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null | Value::Json(Json::Null))
    }

    /// Python truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Lookup(_) | Value::Date(_) | Value::DateTime(_) => true,
            Value::Json(j) => crate::pytext::truthy(j),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Json(Json::String(s)) => Some(s),
            _ => None,
        }
    }

    /// Items of a list/tuple value.
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    /// `str(value)`.
    pub fn py_str(&self) -> String {
        match self {
            Value::Null => "None".to_string(),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => fsv_py::float_repr(*f),
            Value::Str(s) => s.clone(),
            Value::List(_) => self.py_repr(),
            Value::Lookup(l) => l.to_string(),
            Value::Date(d) => date_iso(*d),
            Value::DateTime(d) => d.py_str(),
            Value::Json(j) => fsv_py::py_str(j),
        }
    }

    /// `repr(value)` (lists render as tuples, like stored values).
    pub fn py_repr(&self) -> String {
        self.repr_with(true)
    }

    /// `repr(raw)` of a raw answer (lists render as lists).
    pub fn raw_repr(&self) -> String {
        self.repr_with(false)
    }

    fn repr_with(&self, tuple: bool) -> String {
        match self {
            Value::Str(s) => fsv_py::py_repr_str(s),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.repr_with(tuple)).collect();
                if !tuple {
                    format!("[{}]", parts.join(", "))
                } else if parts.len() == 1 {
                    format!("({},)", parts[0])
                } else {
                    format!("({})", parts.join(", "))
                }
            }
            Value::Lookup(l) => l.py_repr(),
            Value::Date(d) => format!("datetime.date({}, {}, {})", d.year(), d.month(), d.day()),
            Value::DateTime(d) => {
                let l = d.local;
                format!(
                    "datetime.datetime({}, {}, {}, {}, {}{})",
                    l.year(),
                    l.month(),
                    l.day(),
                    l.hour(),
                    l.minute(),
                    if l.second() != 0 {
                        format!(", {}", l.second())
                    } else {
                        String::new()
                    }
                )
            }
            Value::Json(j) => fsv_py::py_repr(j),
            other => other.py_str(),
        }
    }

    /// `norm(value)`: collapse whitespace, casefold `str(value)`.
    pub fn norm(&self) -> String {
        norm(&self.py_str())
    }

    /// JSON → value (`json.loads` result as Python sees it).
    pub fn from_json(v: &Json) -> Self {
        match v {
            Json::Null => Value::Null,
            Json::Bool(b) => Value::Bool(*b),
            Json::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else {
                    Value::Float(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            Json::String(s) => Value::Str(s.clone()),
            Json::Array(a) => Value::List(a.iter().map(Value::from_json).collect()),
            Json::Object(_) => Value::Json(v.clone()),
        }
    }

    /// Value → JSON as `json.dumps` (with fsv's `default`: dates as ISO
    /// strings) would write it. Lookup values are not serialisable.
    pub fn to_json(&self) -> Result<Json, crate::FormError> {
        Ok(match self {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(*b),
            Value::Int(i) => Json::from(*i),
            Value::Float(f) => Number::from_f64(*f).map(Json::Number).unwrap_or(Json::Null),
            Value::Str(s) => Json::String(s.clone()),
            Value::List(l) => Json::Array(l.iter().map(Value::to_json).collect::<Result<_, _>>()?),
            Value::Lookup(_) => {
                return Err(crate::FormError::Type(
                    "not JSON-serialisable: LookupValue".to_string(),
                ));
            }
            Value::Date(d) => Json::String(date_iso(*d)),
            Value::DateTime(d) => Json::String(d.isoformat()),
            Value::Json(j) => j.clone(),
        })
    }

    fn num(&self) -> Option<f64> {
        match self {
            Value::Bool(b) => Some(*b as i64 as f64),
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Json(Json::Number(n)) => n.as_f64(),
            Value::Json(Json::Bool(b)) => Some(*b as i64 as f64),
            _ => None,
        }
    }

    fn is_numeric(&self) -> bool {
        matches!(
            self,
            Value::Bool(_)
                | Value::Int(_)
                | Value::Float(_)
                | Value::Json(Json::Number(_))
                | Value::Json(Json::Bool(_))
        )
    }

    /// Python type identity for `type(a) is type(b)`.
    pub(crate) fn type_tag(&self) -> u8 {
        match self {
            Value::Null | Value::Json(Json::Null) => 0,
            Value::Bool(_) | Value::Json(Json::Bool(_)) => 1,
            Value::Int(_) => 2,
            Value::Float(_) => 3,
            Value::Json(Json::Number(n)) => {
                if n.is_i64() || n.is_u64() {
                    2
                } else {
                    3
                }
            }
            Value::Str(_) | Value::Json(Json::String(_)) => 4,
            Value::List(_) | Value::Json(Json::Array(_)) => 5,
            Value::Lookup(_) => 6,
            Value::Date(_) => 7,
            Value::DateTime(_) => 8,
            Value::Json(Json::Object(_)) => 9,
        }
    }
}

impl PartialEq for Value {
    /// Python `==`.
    fn eq(&self, other: &Self) -> bool {
        if self.is_numeric() && other.is_numeric() {
            return self.num() == other.num();
        }
        match (self, other) {
            (a, b) if a.is_null() && b.is_null() => true,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Lookup(a), Value::Lookup(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::DateTime(a), Value::DateTime(b)) => a == b,
            (Value::Json(a), Value::Json(b)) => a == b,
            (Value::List(a), Value::Json(Json::Array(b)))
            | (Value::Json(Json::Array(b)), Value::List(a)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| *x == Value::from_json(y))
            }
            _ => match (self.as_str(), other.as_str()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}
impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Value::Int(i as i64)
    }
}
impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}
impl From<LookupValue> for Value {
    fn from(l: LookupValue) -> Self {
        Value::Lookup(l)
    }
}
impl From<Date> for Value {
    fn from(d: Date) -> Self {
        Value::Date(d)
    }
}
impl From<DateTimeValue> for Value {
    fn from(d: DateTimeValue) -> Self {
        Value::DateTime(d)
    }
}
impl From<&Json> for Value {
    fn from(j: &Json) -> Self {
        Value::from_json(j)
    }
}
impl From<Json> for Value {
    fn from(j: Json) -> Self {
        Value::from_json(&j)
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map(Into::into).unwrap_or(Value::Null)
    }
}

/// Anything with a Python `str()` and `repr()`: choice keys and raw
/// answers are matched by `norm(str(x))` and reported by `repr(x)`.
pub trait PyObj {
    fn py_str(&self) -> String;
    fn py_repr(&self) -> String;
}

impl PyObj for Value {
    fn py_str(&self) -> String {
        Value::py_str(self)
    }
    fn py_repr(&self) -> String {
        self.raw_repr()
    }
}
impl PyObj for Json {
    fn py_str(&self) -> String {
        fsv_py::py_str(self)
    }
    fn py_repr(&self) -> String {
        fsv_py::py_repr(self)
    }
}
impl PyObj for str {
    fn py_str(&self) -> String {
        self.to_string()
    }
    fn py_repr(&self) -> String {
        fsv_py::py_repr_str(self)
    }
}
impl PyObj for String {
    fn py_str(&self) -> String {
        self.clone()
    }
    fn py_repr(&self) -> String {
        fsv_py::py_repr_str(self)
    }
}
impl PyObj for i64 {
    fn py_str(&self) -> String {
        self.to_string()
    }
    fn py_repr(&self) -> String {
        self.to_string()
    }
}

/// `norm(value)`: collapse whitespace, casefold.
pub fn norm(s: &str) -> String {
    casefold(&squash_ws(s))
}

/// Insertion-ordered `str -> Value` mapping (a Python dict of answers).
pub type Values = indexmap::IndexMap<String, Value>;

/// Convenience: build [`Values`] from JSON object entries.
pub fn values_from_json(obj: &Map<String, Json>) -> Values {
    obj.iter()
        .map(|(k, v)| (k.clone(), Value::from_json(v)))
        .collect()
}
