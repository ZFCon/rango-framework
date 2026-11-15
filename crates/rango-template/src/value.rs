//! Template values and variable resolution.

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;

/// A callable usable in templates: `{{ obj.method }}` calls it with no arguments.
pub type Callable = Arc<dyn Fn() -> Value + Send + Sync>;

/// An arbitrary object exposed to templates (a form, a bound field, a model wrapper...).
pub trait TemplateObject: Send + Sync {
    /// `obj.key` / `obj["key"]`: attribute, item or no-argument method (already called).
    fn get(&self, key: &str) -> Option<Value>;
    /// `str(obj)`; return [`Value::Safe`] for ready-made HTML.
    fn display(&self) -> Value {
        Value::Str("<object>".into())
    }
    /// `iter(obj)`.
    fn iterate(&self) -> Option<Vec<Value>> {
        None
    }
    /// `bool(obj)`.
    fn truthy(&self) -> bool {
        true
    }
}

/// Shared handle to a [`TemplateObject`].
#[derive(Clone)]
pub struct Object(pub Arc<dyn TemplateObject>);

impl Object {
    pub fn new(o: impl TemplateObject + 'static) -> Value {
        Value::Object(Object(Arc::new(o)))
    }
}

/// A template value (what Python objects are in templates).
#[derive(Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(Decimal),
    Str(String),
    /// A string already safe for HTML output (`SafeString`).
    Safe(String),
    List(Vec<Value>),
    Map(IndexMap<String, Value>),
    Date(NaiveDate),
    Time(NaiveTime),
    DateTime(NaiveDateTime, Option<i32>),
    Callable(Callable),
    Object(Object),
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Callable(_) => f.write_str("<callable>"),
            Value::Object(_) => f.write_str("<object>"),
            other => f.write_str(&other.repr()),
        }
    }
}

fn py_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if f.fract() == 0.0 && f.abs() < 1e16 {
        return format!("{f:.1}");
    }
    let s = format!("{f}");
    s
}

impl Value {
    pub fn is_safe(&self) -> bool {
        match self {
            Value::Safe(_) => true,
            Value::Object(o) => o.0.display().is_safe(),
            _ => false,
        }
    }

    /// `iter(value)`: list items, dict keys, string characters, object iteration.
    pub fn iterate(&self) -> Option<Vec<Value>> {
        match self {
            Value::List(l) => Some(l.clone()),
            Value::Map(m) => Some(m.keys().map(|k| Value::Str(k.clone())).collect()),
            Value::Str(s) | Value::Safe(s) => Some(s.chars().map(|c| Value::Str(c.to_string())).collect()),
            Value::Object(o) => o.0.iterate(),
            _ => None,
        }
    }

    /// Python truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Value::None => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Decimal(d) => !d.is_zero(),
            Value::Str(s) | Value::Safe(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Map(m) => !m.is_empty(),
            Value::Object(o) => o.0.truthy(),
            Value::Date(_) | Value::Time(_) | Value::DateTime(..) | Value::Callable(_) => true,
        }
    }

    /// `str(value)`.
    pub fn to_str(&self) -> String {
        match self {
            Value::None => "None".into(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) => "False".into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => py_float(*f),
            Value::Decimal(d) => d.to_string(),
            Value::Str(s) | Value::Safe(s) => s.clone(),
            Value::List(_) | Value::Map(_) => self.repr(),
            Value::Date(d) => d.format("%Y-%m-%d").to_string(),
            Value::Time(t) => t.format("%H:%M:%S").to_string(),
            Value::DateTime(d, _) => d.format("%Y-%m-%d %H:%M:%S").to_string(),
            Value::Callable(_) => "<function>".into(),
            Value::Object(o) => o.0.display().to_str(),
        }
    }

    /// `repr(value)`.
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) | Value::Safe(s) => rango_core::exceptions::py_repr_str(s),
            Value::List(l) => format!("[{}]", l.iter().map(Value::repr).collect::<Vec<_>>().join(", ")),
            Value::Map(m) => format!(
                "{{{}}}",
                m.iter().map(|(k, v)| format!("{}: {}", rango_core::exceptions::py_repr_str(k), v.repr())).collect::<Vec<_>>().join(", ")
            ),
            Value::Decimal(d) => format!("Decimal('{d}')"),
            Value::Object(o) => o.0.display().repr(),
            other => other.to_str(),
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Bool(b) => Some(f64::from(u8::from(*b))),
            Value::Decimal(d) => d.to_string().parse().ok(),
            Value::Str(s) | Value::Safe(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(i64::from(*b)),
            Value::Float(f) if f.fract() == 0.0 => Some(*f as i64),
            Value::Str(s) | Value::Safe(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn len(&self) -> Option<usize> {
        match self {
            Value::Str(s) | Value::Safe(s) => Some(s.chars().count()),
            Value::List(l) => Some(l.len()),
            Value::Map(m) => Some(m.len()),
            Value::Object(o) => o.0.iterate().map(|l| l.len()),
            _ => None,
        }
    }

    /// Python `==`.
    pub fn eq_py(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::None, Value::None) => true,
            (Value::Str(a) | Value::Safe(a), Value::Str(b) | Value::Safe(b)) => a == b,
            (Value::List(a), Value::List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_py(y)),
            (Value::Map(a), Value::Map(b)) => a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|o| v.eq_py(o))),
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::DateTime(a, _), Value::DateTime(b, _)) => a == b,
            (Value::Object(a), Value::Object(b)) => Arc::ptr_eq(&a.0, &b.0),
            (a, b) => match (a.as_number(), b.as_number()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            },
        }
    }

    fn as_number(&self) -> Option<f64> {
        match self {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Decimal(_) => self.as_f64(),
            _ => None,
        }
    }

    /// Python ordering comparison; `None` when the types are not comparable.
    pub fn cmp_py(&self, other: &Value) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Str(a) | Value::Safe(a), Value::Str(b) | Value::Safe(b)) => Some(a.cmp(b)),
            (Value::Date(a), Value::Date(b)) => Some(a.cmp(b)),
            (Value::DateTime(a, _), Value::DateTime(b, _)) => Some(a.cmp(b)),
            (Value::List(a), Value::List(b)) => {
                for (x, y) in a.iter().zip(b) {
                    match x.cmp_py(y)? {
                        std::cmp::Ordering::Equal => {}
                        o => return Some(o),
                    }
                }
                Some(a.len().cmp(&b.len()))
            }
            (a, b) => a.as_number()?.partial_cmp(&b.as_number()?),
        }
    }

    /// `x in self`.
    pub fn contains(&self, x: &Value) -> bool {
        match self {
            Value::List(l) => l.iter().any(|v| v.eq_py(x)),
            Value::Map(m) => m.contains_key(&x.to_str()),
            Value::Str(s) | Value::Safe(s) => s.contains(&x.to_str()),
            Value::Object(o) => o.0.iterate().is_some_and(|l| l.iter().any(|v| v.eq_py(x))),
            _ => false,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_owned())
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
macro_rules! from_int {
    ($($t:ty),*) => {$(impl From<$t> for Value { fn from(i: $t) -> Self { Value::Int(i as i64) } })*};
}
from_int!(i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);
impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}
impl From<Decimal> for Value {
    fn from(d: Decimal) -> Self {
        Value::Decimal(d)
    }
}
impl From<NaiveDate> for Value {
    fn from(d: NaiveDate) -> Self {
        Value::Date(d)
    }
}
impl From<NaiveTime> for Value {
    fn from(t: NaiveTime) -> Self {
        Value::Time(t)
    }
}
impl From<DateTime<Utc>> for Value {
    fn from(d: DateTime<Utc>) -> Self {
        Value::DateTime(d.naive_utc(), Some(0))
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Self {
        o.map_or(Value::None, Into::into)
    }
}
impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Value::None,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => n.as_i64().map(Value::Int).unwrap_or_else(|| Value::Float(n.as_f64().unwrap_or(0.0))),
            serde_json::Value::String(s) => Value::Str(s),
            serde_json::Value::Array(a) => Value::List(a.into_iter().map(Value::from).collect()),
            serde_json::Value::Object(o) => Value::Map(o.into_iter().map(|(k, v)| (k, Value::from(v))).collect()),
        }
    }
}

/// Convert any `Serialize` value (a model, a struct, a map) into a template value.
pub fn to_value<T: Serialize + ?Sized>(v: &T) -> Value {
    serde_json::to_value(v).map(Value::from).unwrap_or(Value::None)
}

/// Mark a string safe (`mark_safe`).
pub fn safe(s: impl Into<String>) -> Value {
    Value::Safe(s.into())
}

/// `context! { "name" => value, ... }`.
#[macro_export]
macro_rules! context {
    ($($k:expr => $v:expr),* $(,)?) => {{
        let mut m = $crate::Context::new();
        $(m.set($k, $crate::Value::from($v));)*
        m
    }};
}
