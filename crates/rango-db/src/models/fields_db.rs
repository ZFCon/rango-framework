//! Value adaptation between Rust field semantics and each database
//! (`get_db_prep_value` / `from_db_value`).

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use rango_core::conf::try_settings;
use rango_core::exceptions::{Error, Result};
use rust_decimal::Decimal;

use crate::backends::Vendor;
use crate::models::meta::{Field, FieldType};
use crate::value::Value;

/// `str(datetime)` as the SQLite adapter writes it.
pub fn fmt_naive_datetime(d: &NaiveDateTime) -> String {
    use chrono::Timelike;
    if d.nanosecond() / 1000 == 0 { d.format("%Y-%m-%d %H:%M:%S").to_string() } else { d.format("%Y-%m-%d %H:%M:%S%.6f").to_string() }
}

fn fmt_time(t: &NaiveTime) -> String {
    use chrono::Timelike;
    if t.nanosecond() / 1000 == 0 { t.format("%H:%M:%S").to_string() } else { t.format("%H:%M:%S%.6f").to_string() }
}

/// Text form of values that have no native SQLite type.
pub fn adapt_to_text(v: &Value) -> String {
    match v {
        Value::Date(d) => d.format("%Y-%m-%d").to_string(),
        Value::Time(t) => fmt_time(t),
        Value::DateTime(d) => fmt_naive_datetime(&d.naive_utc()),
        Value::NaiveDateTime(d) => fmt_naive_datetime(d),
        Value::Decimal(d) => d.to_string(),
        Value::Uuid(u) => u.simple().to_string(),
        Value::Json(j) => rango_core::utils::json::dumps(j, false).unwrap_or_default(),
        Value::Duration(d) => d.num_microseconds().unwrap_or(0).to_string(),
        Value::Str(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

/// Prepare a parameter for `field`'s column on `vendor`.
pub fn prep_param(vendor: Vendor, field: &Field, v: Value) -> Value {
    if v.is_null() {
        return v;
    }
    match vendor {
        Vendor::Sqlite => match (field.ty, v) {
            (FieldType::Decimal, Value::Decimal(d)) => {
                Value::Str(field.decimal_places.map_or_else(|| d.to_string(), |p| format!("{:.*}", p as usize, d.round_dp(p))))
            }
            (_, Value::Duration(d)) => Value::Int(d.num_microseconds().unwrap_or(0)),
            (_, v @ (Value::Date(_) | Value::Time(_) | Value::DateTime(_) | Value::NaiveDateTime(_) | Value::Uuid(_) | Value::Json(_) | Value::Decimal(_))) => {
                Value::Str(adapt_to_text(&v))
            }
            (_, v) => v,
        },
        Vendor::Postgresql => v,
    }
}

fn parse_naive_dt(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f"))
        .ok()
        .or_else(|| DateTime::parse_from_rfc3339(s).ok().map(|d| d.naive_utc()))
}

/// Convert a raw database value to the field's canonical [`Value`] (`from_db_value`).
pub fn from_db(vendor: Vendor, field: &Field, v: Value) -> Result<Value> {
    if v.is_null() {
        return Ok(v);
    }
    let use_tz = try_settings().map(|s| s.use_tz).unwrap_or(true);
    let ty = if field.ty.is_relation() { FieldType::Integer } else { field.ty };
    let bad = |v: &Value| Error::Other(format!("cannot read {v:?} as {}", field.ty.class_name()).into());
    Ok(match (ty, v) {
        (FieldType::Boolean, Value::Int(i)) => Value::Bool(i != 0),
        (FieldType::DateTime, Value::Str(s)) => {
            let n = parse_naive_dt(&s).ok_or_else(|| bad(&Value::Str(s.clone())))?;
            if use_tz { Value::DateTime(DateTime::<Utc>::from_naive_utc_and_offset(n, Utc)) } else { Value::NaiveDateTime(n) }
        }
        (FieldType::DateTime, Value::NaiveDateTime(n)) if use_tz => Value::DateTime(DateTime::<Utc>::from_naive_utc_and_offset(n, Utc)),
        (FieldType::DateTime, Value::DateTime(d)) if !use_tz => Value::NaiveDateTime(d.naive_utc()),
        (FieldType::Date, Value::Str(s)) => Value::Date(NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|_| bad(&Value::Str(s)))?),
        (FieldType::Time, Value::Str(s)) => Value::Time(
            NaiveTime::parse_from_str(&s, "%H:%M:%S%.f").or_else(|_| NaiveTime::parse_from_str(&s, "%H:%M:%S")).map_err(|_| bad(&Value::Str(s)))?,
        ),
        (FieldType::Decimal, Value::Float(f)) => {
            let d = Decimal::try_from(f).map_err(|_| bad(&Value::Float(f)))?;
            Value::Decimal(field.decimal_places.map_or(d, |p| d.round_dp(p)))
        }
        (FieldType::Decimal, Value::Int(i)) => Value::Decimal(Decimal::from(i)),
        (FieldType::Decimal, Value::Str(s)) => Value::Decimal(s.parse().map_err(|_| bad(&Value::Str(s)))?),
        (FieldType::Decimal, Value::Decimal(d)) => Value::Decimal(field.decimal_places.map_or(d, |p| d.round_dp(p))),
        (FieldType::Uuid, Value::Str(s)) => Value::Uuid(s.parse().map_err(|_| bad(&Value::Str(s)))?),
        (FieldType::Duration, Value::Int(us)) => Value::Duration(TimeDelta::microseconds(us)),
        (FieldType::Json, Value::Str(s)) => Value::Json(serde_json::from_str(&s).map_err(|_| bad(&Value::Str(s)))?),
        (FieldType::Float, Value::Int(i)) => Value::Float(i as f64),
        (_, v) => {
            let _ = vendor;
            v
        }
    })
}
