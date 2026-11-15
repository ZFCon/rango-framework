//! Database values and the conversions between Rust types and [`Value`].

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use rango_core::exceptions::{Error, Result};
use rust_decimal::Decimal;

/// One database value (a query parameter or a column).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(Decimal),
    Str(String),
    Bytes(Vec<u8>),
    Date(NaiveDate),
    Time(NaiveTime),
    /// Timezone-aware (UTC) datetime.
    DateTime(DateTime<Utc>),
    /// Naive datetime (`USE_TZ = false`).
    NaiveDateTime(NaiveDateTime),
    Duration(TimeDelta),
    Uuid(uuid::Uuid),
    Json(serde_json::Value),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
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
            Value::Bool(b) => Some(i64::from(*b)),
            _ => None,
        }
    }
}

/// Rust value → [`Value`] (query parameters).
pub trait ToValue {
    fn to_value(&self) -> Value;
}

/// [`Value`] → Rust value (model fields).
pub trait FromValue: Sized {
    fn from_value(v: Value) -> Result<Self>;
}

fn type_err<T>(v: &Value, want: &str) -> Result<T> {
    Err(Error::Other(format!("cannot convert {v:?} to {want}").into()))
}

macro_rules! int_value {
    ($($t:ty),*) => {$(
        impl ToValue for $t {
            fn to_value(&self) -> Value { Value::Int(*self as i64) }
        }
        impl FromValue for $t {
            fn from_value(v: Value) -> Result<Self> {
                match v {
                    Value::Int(i) => <$t>::try_from(i).or_else(|_| type_err(&Value::Int(i), stringify!($t))),
                    Value::Bool(b) => Ok(b as $t),
                    Value::Float(f) if f.fract() == 0.0 => Ok(f as $t),
                    Value::Decimal(d) => d.to_string().parse().or_else(|_| type_err(&Value::Decimal(d), stringify!($t))),
                    Value::Str(ref s) => s.parse().or_else(|_| type_err(&v, stringify!($t))),
                    other => type_err(&other, stringify!($t)),
                }
            }
        }
    )*};
}
int_value!(i8, i16, i32, i64, u8, u16, u32, u64, isize, usize);

impl ToValue for bool {
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
}
impl FromValue for bool {
    fn from_value(v: Value) -> Result<Self> {
        match v {
            Value::Bool(b) => Ok(b),
            Value::Int(i) => Ok(i != 0),
            other => type_err(&other, "bool"),
        }
    }
}

macro_rules! float_value {
    ($($t:ty),*) => {$(
        impl ToValue for $t {
            fn to_value(&self) -> Value { Value::Float(*self as f64) }
        }
        impl FromValue for $t {
            fn from_value(v: Value) -> Result<Self> {
                match v {
                    Value::Float(f) => Ok(f as $t),
                    Value::Int(i) => Ok(i as $t),
                    Value::Decimal(d) => d.to_string().parse().or_else(|_| type_err(&Value::Decimal(d), "float")),
                    other => type_err(&other, "float"),
                }
            }
        }
    )*};
}
float_value!(f32, f64);

impl ToValue for str {
    fn to_value(&self) -> Value {
        Value::Str(self.to_owned())
    }
}
impl ToValue for String {
    fn to_value(&self) -> Value {
        Value::Str(self.clone())
    }
}
impl FromValue for String {
    fn from_value(v: Value) -> Result<Self> {
        match v {
            Value::Str(s) => Ok(s),
            Value::Int(i) => Ok(i.to_string()),
            Value::Uuid(u) => Ok(u.to_string()),
            other => type_err(&other, "String"),
        }
    }
}

impl ToValue for Decimal {
    fn to_value(&self) -> Value {
        Value::Decimal(*self)
    }
}
impl FromValue for Decimal {
    fn from_value(v: Value) -> Result<Self> {
        match v {
            Value::Decimal(d) => Ok(d),
            Value::Str(s) => s.parse().or_else(|_| type_err(&Value::Str(s), "Decimal")),
            Value::Int(i) => Ok(Decimal::from(i)),
            Value::Float(f) => Decimal::try_from(f).or_else(|_| type_err(&Value::Float(f), "Decimal")),
            other => type_err(&other, "Decimal"),
        }
    }
}

macro_rules! simple_value {
    ($t:ty, $variant:ident, $name:literal) => {
        impl ToValue for $t {
            fn to_value(&self) -> Value {
                Value::$variant(self.clone())
            }
        }
        impl FromValue for $t {
            fn from_value(v: Value) -> Result<Self> {
                match v {
                    Value::$variant(x) => Ok(x),
                    other => type_err(&other, $name),
                }
            }
        }
    };
}
simple_value!(NaiveDate, Date, "NaiveDate");
simple_value!(NaiveTime, Time, "NaiveTime");
simple_value!(DateTime<Utc>, DateTime, "DateTime<Utc>");
simple_value!(NaiveDateTime, NaiveDateTime, "NaiveDateTime");
simple_value!(TimeDelta, Duration, "TimeDelta");
simple_value!(uuid::Uuid, Uuid, "Uuid");
simple_value!(Vec<u8>, Bytes, "Vec<u8>");
simple_value!(serde_json::Value, Json, "serde_json::Value");

impl<T: ToValue> ToValue for Option<T> {
    fn to_value(&self) -> Value {
        self.as_ref().map_or(Value::Null, ToValue::to_value)
    }
}
impl<T: FromValue> FromValue for Option<T> {
    fn from_value(v: Value) -> Result<Self> {
        if v.is_null() { Ok(None) } else { T::from_value(v).map(Some) }
    }
}
impl<T: ToValue + ?Sized> ToValue for &T {
    fn to_value(&self) -> Value {
        (**self).to_value()
    }
}
impl ToValue for Value {
    fn to_value(&self) -> Value {
        self.clone()
    }
}
impl FromValue for Value {
    fn from_value(v: Value) -> Result<Self> {
        Ok(v)
    }
}

/// A result row: column names (shared) plus values.
#[derive(Clone, Debug)]
pub struct Row {
    pub columns: std::sync::Arc<Vec<String>>,
    pub values: Vec<Value>,
}

impl Row {
    pub fn get(&self, idx: usize) -> &Value {
        &self.values[idx]
    }

    pub fn by_name(&self, name: &str) -> Option<&Value> {
        self.columns.iter().position(|c| c == name).map(|i| &self.values[i])
    }
}
