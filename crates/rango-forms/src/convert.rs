//! Typed access to cleaned values (used by `#[derive(Form)]`).

use crate::value::Value;

/// Conversion of a cleaned [`Value`] into a Rust type.
pub trait FromFormValue: Sized {
    fn from_form_value(v: &Value) -> Option<Self>;
}

impl FromFormValue for String {
    fn from_form_value(v: &Value) -> Option<Self> {
        match v {
            Value::Str(s) => Some(s.clone()),
            Value::None => Some(String::new()),
            other => Some(other.to_text()),
        }
    }
}

impl FromFormValue for bool {
    fn from_form_value(v: &Value) -> Option<Self> {
        Some(v.truthy())
    }
}

macro_rules! ints {
    ($($t:ty),*) => {$(impl FromFormValue for $t {
        fn from_form_value(v: &Value) -> Option<Self> { v.as_i64().and_then(|i| <$t>::try_from(i).ok()) }
    })*};
}
ints!(i8, i16, i32, i64, u8, u16, u32, u64, isize, usize);

impl FromFormValue for f64 {
    fn from_form_value(v: &Value) -> Option<Self> {
        match v {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl FromFormValue for f32 {
    fn from_form_value(v: &Value) -> Option<Self> {
        f64::from_form_value(v).map(|f| f as f32)
    }
}

macro_rules! direct {
    ($($t:ty => $v:ident),*) => {$(impl FromFormValue for $t {
        fn from_form_value(v: &Value) -> Option<Self> { if let Value::$v(x) = v { Some(x.clone()) } else { None } }
    })*};
}
direct!(rust_decimal::Decimal => Decimal, chrono::NaiveDate => Date, chrono::NaiveTime => Time, chrono::NaiveDateTime => NaiveDateTime, chrono::DateTime<chrono::Utc> => DateTime, uuid::Uuid => Uuid);

impl FromFormValue for serde_json::Value {
    fn from_form_value(v: &Value) -> Option<Self> {
        match v {
            Value::Json(j) => Some(j.clone()),
            Value::None => Some(serde_json::Value::Null),
            _ => None,
        }
    }
}

impl<T: FromFormValue> FromFormValue for Option<T> {
    fn from_form_value(v: &Value) -> Option<Self> {
        if v.is_empty() { Some(None) } else { T::from_form_value(v).map(Some) }
    }
}

impl<T: FromFormValue> FromFormValue for Vec<T> {
    fn from_form_value(v: &Value) -> Option<Self> {
        match v {
            Value::List(l) => l.iter().map(T::from_form_value).collect(),
            Value::None => Some(vec![]),
            _ => None,
        }
    }
}
