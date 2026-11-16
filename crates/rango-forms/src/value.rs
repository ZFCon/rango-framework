//! Values flowing through fields: submitted data is text, cleaned data is typed.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use rango_http::UploadedFile;
use rust_decimal::Decimal;

/// A cleaned/initial value (dynamic values in `cleaned_data` / `initial`).
#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(Decimal),
    Str(String),
    Date(NaiveDate),
    Time(NaiveTime),
    /// Timezone-aware (`USE_TZ = true`).
    DateTime(DateTime<Utc>),
    NaiveDateTime(NaiveDateTime),
    Duration(TimeDelta),
    Uuid(uuid::Uuid),
    Json(serde_json::Value),
    List(Vec<Value>),
    File(UploadedFile),
    /// `FileField.clean` result `False`: "clear the existing file".
    Cleared,
}

impl Value {
    /// `value in field.empty_values` (`None`, `''`, `[]`, `()`, `{}`).
    pub fn is_empty(&self) -> bool {
        match self {
            Value::None => true,
            Value::Str(s) => s.is_empty(),
            Value::List(l) => l.is_empty(),
            Value::Json(serde_json::Value::Object(o)) => o.is_empty(),
            _ => false,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Truthiness (empty / zero / false are false).
    pub fn truthy(&self) -> bool {
        match self {
            Value::None | Value::Cleared => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Decimal(d) => !d.is_zero(),
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Duration(d) => !d.is_zero(),
            Value::Json(j) => !matches!(j, serde_json::Value::Null | serde_json::Value::Bool(false)) && !j.as_str().is_some_and(str::is_empty),
            _ => true,
        }
    }

    /// `str(value)`.
    pub fn to_text(&self) -> String {
        match self {
            Value::None | Value::Cleared => "None".into(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) => "False".into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e16 { format!("{f:.1}") } else { f.to_string() }
            }
            Value::Decimal(d) => d.to_string(),
            Value::Str(s) => s.clone(),
            Value::Date(d) => d.format("%Y-%m-%d").to_string(),
            Value::Time(t) => {
                if chrono::Timelike::nanosecond(t) == 0 { t.format("%H:%M:%S").to_string() } else { t.format("%H:%M:%S%.6f").to_string() }
            }
            Value::DateTime(d) => d.format("%Y-%m-%d %H:%M:%S%:z").to_string(),
            Value::NaiveDateTime(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(),
            Value::Duration(d) => rango_core::utils::duration::duration_string(*d),
            Value::Uuid(u) => u.to_string(),
            Value::Json(j) => j.to_string(),
            Value::List(l) => format!("[{}]", l.iter().map(Value::repr).collect::<Vec<_>>().join(", ")),
            Value::File(f) => f.name.clone(),
        }
    }

    /// `repr(value)` (for lists of strings and `ErrorList` display).
    pub fn repr(&self) -> String {
        match self {
            Value::Str(s) => rango_core::exceptions::py_repr_str(s),
            other => other.to_text(),
        }
    }
}

macro_rules! from_impl {
    ($($t:ty => $v:ident $(as $c:ty)?),* $(,)?) => {$(impl From<$t> for Value { fn from(x: $t) -> Self { Value::$v(x $(as $c)?) } })*};
}
from_impl!(bool => Bool, i64 => Int, i32 => Int as i64, u32 => Int as i64, usize => Int as i64, f64 => Float, String => Str, NaiveDate => Date, NaiveTime => Time, NaiveDateTime => NaiveDateTime, DateTime<Utc> => DateTime, Decimal => Decimal, uuid::Uuid => Uuid, TimeDelta => Duration);

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_owned())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Self {
        o.map_or(Value::None, Into::into)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}

impl From<Value> for rango_db::Value {
    fn from(v: Value) -> Self {
        use rango_db::Value as D;
        match v {
            Value::None | Value::Cleared => D::Null,
            Value::Bool(b) => D::Bool(b),
            Value::Int(i) => D::Int(i),
            Value::Float(f) => D::Float(f),
            Value::Decimal(d) => D::Decimal(d),
            Value::Str(s) => D::Str(s),
            Value::Date(d) => D::Date(d),
            Value::Time(t) => D::Time(t),
            Value::DateTime(d) => D::DateTime(d),
            Value::NaiveDateTime(d) => D::NaiveDateTime(d),
            Value::Duration(d) => D::Duration(d),
            Value::Uuid(u) => D::Uuid(u),
            Value::Json(j) => D::Json(j),
            Value::List(l) => D::Json(serde_json::Value::Array(l.into_iter().map(|x| serde_json::Value::String(x.to_text())).collect())),
            Value::File(f) => D::Str(f.name),
        }
    }
}

impl From<rango_db::Value> for Value {
    fn from(v: rango_db::Value) -> Self {
        use rango_db::Value as D;
        match v {
            D::Null => Value::None,
            D::Bool(b) => Value::Bool(b),
            D::Int(i) => Value::Int(i),
            D::Float(f) => Value::Float(f),
            D::Decimal(d) => Value::Decimal(d),
            D::Str(s) => Value::Str(s),
            D::Bytes(b) => Value::Str(String::from_utf8_lossy(&b).into_owned()),
            D::Date(d) => Value::Date(d),
            D::Time(t) => Value::Time(t),
            D::DateTime(d) => Value::DateTime(d),
            D::NaiveDateTime(d) => Value::NaiveDateTime(d),
            D::Duration(d) => Value::Duration(d),
            D::Uuid(u) => Value::Uuid(u),
            D::Json(j) => Value::Json(j),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (None, None) | (Cleared, Cleared) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Decimal(a), Decimal(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Date(a), Date(b)) => a == b,
            (Time(a), Time(b)) => a == b,
            (DateTime(a), DateTime(b)) => a == b,
            (NaiveDateTime(a), NaiveDateTime(b)) => a == b,
            (Duration(a), Duration(b)) => a == b,
            (Uuid(a), Uuid(b)) => a == b,
            (Json(a), Json(b)) => a == b,
            (List(a), List(b)) => a == b,
            (File(a), File(b)) => a.name == b.name && a.size == b.size,
            (Int(a), Float(b)) | (Float(b), Int(a)) => (*a as f64) == *b,
            (Int(a), Decimal(b)) | (Decimal(b), Int(a)) => rust_decimal::Decimal::from(*a) == *b,
            _ => false,
        }
    }
}
