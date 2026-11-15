//! PostgreSQL backend (tokio-postgres).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::BytesMut;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc};
use rango_core::conf::DatabaseConfig;
use rango_core::exceptions::{Error, Result};
use rust_decimal::Decimal;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type, to_sql_checked};

use super::{Connection, Vendor, convert_placeholders, db_err};
use crate::value::{Row, Value};

struct Shared {
    conn_str: String,
    idle: Mutex<Vec<tokio_postgres::Client>>,
    permits: Arc<Semaphore>,
}

pub struct PgPool(Arc<Shared>);

impl PgPool {
    pub fn new(config: &DatabaseConfig) -> Result<Self> {
        let mut parts = vec![format!("dbname={}", config.name)];
        if !config.host.is_empty() {
            parts.push(format!("host={}", config.host));
        }
        if !config.port.is_empty() {
            parts.push(format!("port={}", config.port));
        }
        if !config.user.is_empty() {
            parts.push(format!("user={}", config.user));
        }
        if !config.password.is_empty() {
            parts.push(format!("password={}", config.password));
        }
        let max = config.options.get("max_connections").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        Ok(Self(Arc::new(Shared { conn_str: parts.join(" "), idle: Mutex::new(Vec::new()), permits: Arc::new(Semaphore::new(max)) })))
    }

    pub async fn acquire(&self) -> Result<PgConn> {
        let permit = Arc::clone(&self.0.permits).acquire_owned().await.map_err(db_err)?;
        let existing = loop {
            let c = self.0.idle.lock().expect("pool lock").pop();
            match c {
                Some(c) if c.is_closed() => continue,
                other => break other,
            }
        };
        let client = match existing {
            Some(c) => c,
            None => {
                let (client, connection) = tokio_postgres::connect(&self.0.conn_str, tokio_postgres::NoTls).await.map_err(db_err)?;
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                client
            }
        };
        Ok(PgConn { client: Some(client), shared: Arc::clone(&self.0), _permit: permit })
    }
}

pub struct PgConn {
    client: Option<tokio_postgres::Client>,
    shared: Arc<Shared>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for PgConn {
    fn drop(&mut self) {
        if let Some(c) = self.client.take() {
            self.shared.idle.lock().expect("pool lock").push(c);
        }
    }
}

/// `interval` as a duration (microseconds; months/days are folded in as 30/1 days on read).
#[derive(Debug)]
struct Interval(TimeDelta);

impl ToSql for Interval {
    fn to_sql(&self, _ty: &Type, out: &mut BytesMut) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.extend_from_slice(&self.0.num_microseconds().unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&0i32.to_be_bytes());
        out.extend_from_slice(&0i32.to_be_bytes());
        Ok(IsNull::No)
    }
    fn accepts(ty: &Type) -> bool {
        *ty == Type::INTERVAL
    }
    to_sql_checked!();
}

impl<'a> FromSql<'a> for Interval {
    fn from_sql(_: &Type, raw: &'a [u8]) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let micros = i64::from_be_bytes(raw[0..8].try_into()?);
        let days = i32::from_be_bytes(raw[8..12].try_into()?);
        let months = i32::from_be_bytes(raw[12..16].try_into()?);
        Ok(Interval(TimeDelta::microseconds(micros) + TimeDelta::days(i64::from(days) + 30 * i64::from(months))))
    }
    fn accepts(ty: &Type) -> bool {
        *ty == Type::INTERVAL
    }
}

type Boxed = Box<dyn ToSql + Sync + Send>;

fn cannot(v: &Value, ty: &Type) -> Error {
    Error::Other(format!("cannot bind {v:?} to PostgreSQL type {ty}").into())
}

fn utc_naive(v: &Value) -> Option<NaiveDateTime> {
    match v {
        Value::DateTime(d) => Some(d.naive_utc()),
        Value::NaiveDateTime(d) => Some(*d),
        Value::Str(s) => NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").ok(),
        _ => None,
    }
}

fn bind(v: &Value, ty: &Type) -> Result<Boxed> {
    macro_rules! opt {
        ($t:ty, $val:expr) => {
            Box::new($val.map(|x| x as $t)) as Boxed
        };
    }
    if v.is_null() {
        return Ok(match *ty {
            Type::INT2 => Box::new(None::<i16>),
            Type::INT4 => Box::new(None::<i32>),
            Type::INT8 => Box::new(None::<i64>),
            Type::FLOAT4 => Box::new(None::<f32>),
            Type::FLOAT8 => Box::new(None::<f64>),
            Type::NUMERIC => Box::new(None::<Decimal>),
            Type::BOOL => Box::new(None::<bool>),
            Type::BYTEA => Box::new(None::<Vec<u8>>),
            Type::DATE => Box::new(None::<NaiveDate>),
            Type::TIME => Box::new(None::<NaiveTime>),
            Type::TIMESTAMP => Box::new(None::<NaiveDateTime>),
            Type::TIMESTAMPTZ => Box::new(None::<DateTime<Utc>>),
            Type::UUID => Box::new(None::<uuid::Uuid>),
            Type::JSON | Type::JSONB => Box::new(None::<serde_json::Value>),
            Type::INTERVAL => Box::new(None::<Interval>),
            _ => Box::new(None::<String>),
        });
    }
    Ok(match *ty {
        Type::INT2 => Box::new(v.as_i64().map(|i| i as i16).ok_or_else(|| cannot(v, ty))?),
        Type::INT4 => Box::new(v.as_i64().map(|i| i as i32).ok_or_else(|| cannot(v, ty))?),
        Type::INT8 => Box::new(v.as_i64().ok_or_else(|| cannot(v, ty))?),
        Type::FLOAT4 | Type::FLOAT8 => {
            let f = match v {
                Value::Float(f) => *f,
                Value::Int(i) => *i as f64,
                Value::Decimal(d) => d.to_string().parse().map_err(|_| cannot(v, ty))?,
                _ => return Err(cannot(v, ty)),
            };
            if *ty == Type::FLOAT4 { opt!(f32, Some(f)) } else { Box::new(f) }
        }
        Type::NUMERIC => Box::new(match v {
            Value::Decimal(d) => *d,
            Value::Int(i) => Decimal::from(*i),
            Value::Float(f) => Decimal::try_from(*f).map_err(|_| cannot(v, ty))?,
            Value::Str(s) => s.parse().map_err(|_| cannot(v, ty))?,
            _ => return Err(cannot(v, ty)),
        }),
        Type::BOOL => Box::new(match v {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            _ => return Err(cannot(v, ty)),
        }),
        Type::BYTEA => Box::new(match v {
            Value::Bytes(b) => b.clone(),
            _ => return Err(cannot(v, ty)),
        }),
        Type::DATE => Box::new(match v {
            Value::Date(d) => *d,
            Value::Str(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| cannot(v, ty))?,
            _ => return Err(cannot(v, ty)),
        }),
        Type::TIME => Box::new(match v {
            Value::Time(t) => *t,
            Value::Str(s) => NaiveTime::parse_from_str(s, "%H:%M:%S%.f").map_err(|_| cannot(v, ty))?,
            _ => return Err(cannot(v, ty)),
        }),
        Type::TIMESTAMP => Box::new(utc_naive(v).ok_or_else(|| cannot(v, ty))?),
        Type::TIMESTAMPTZ => Box::new(DateTime::<Utc>::from_naive_utc_and_offset(utc_naive(v).ok_or_else(|| cannot(v, ty))?, Utc)),
        Type::UUID => Box::new(match v {
            Value::Uuid(u) => *u,
            Value::Str(s) => s.parse().map_err(|_| cannot(v, ty))?,
            _ => return Err(cannot(v, ty)),
        }),
        Type::JSON | Type::JSONB => Box::new(match v {
            Value::Json(j) => j.clone(),
            Value::Str(s) => serde_json::from_str(s).map_err(|_| cannot(v, ty))?,
            _ => return Err(cannot(v, ty)),
        }),
        Type::INTERVAL => Box::new(Interval(match v {
            Value::Duration(d) => *d,
            Value::Int(us) => TimeDelta::microseconds(*us),
            _ => return Err(cannot(v, ty)),
        })),
        // text, varchar, bpchar, name, unknown...
        _ => Box::new(match v {
            Value::Str(s) => s.clone(),
            Value::Int(i) => i.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Decimal(d) => d.to_string(),
            Value::Uuid(u) => u.to_string(),
            other => crate::models::fields_db::adapt_to_text(other),
        }),
    })
}

fn read(row: &tokio_postgres::Row, i: usize) -> Result<Value> {
    let ty = row.columns()[i].type_().clone();
    macro_rules! get {
        ($t:ty, $f:expr) => {
            row.try_get::<_, Option<$t>>(i).map_err(db_err)?.map_or(Value::Null, $f)
        };
    }
    Ok(match ty {
        Type::BOOL => get!(bool, Value::Bool),
        Type::INT2 => get!(i16, |x| Value::Int(i64::from(x))),
        Type::INT4 => get!(i32, |x| Value::Int(i64::from(x))),
        Type::INT8 => get!(i64, Value::Int),
        Type::OID => get!(u32, |x| Value::Int(i64::from(x))),
        Type::FLOAT4 => get!(f32, |x| Value::Float(f64::from(x))),
        Type::FLOAT8 => get!(f64, Value::Float),
        Type::NUMERIC => get!(Decimal, Value::Decimal),
        Type::BYTEA => get!(Vec<u8>, Value::Bytes),
        Type::DATE => get!(NaiveDate, Value::Date),
        Type::TIME => get!(NaiveTime, Value::Time),
        Type::TIMESTAMP => get!(NaiveDateTime, Value::NaiveDateTime),
        Type::TIMESTAMPTZ => get!(DateTime<Utc>, Value::DateTime),
        Type::UUID => get!(uuid::Uuid, Value::Uuid),
        Type::JSON | Type::JSONB => get!(serde_json::Value, Value::Json),
        Type::INTERVAL => get!(Interval, |x| Value::Duration(x.0)),
        _ => get!(String, Value::Str),
    })
}

#[async_trait]
impl Connection for PgConn {
    fn vendor(&self) -> Vendor {
        Vendor::Postgresql
    }

    async fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let client = self.client.as_ref().expect("connection present");
        let stmt = client.prepare(&convert_placeholders(sql, Vendor::Postgresql)).await.map_err(db_err)?;
        let bound: Vec<Boxed> = params.iter().zip(stmt.params()).map(|(v, t)| bind(v, t)).collect::<Result<_>>()?;
        let refs: Vec<&(dyn ToSql + Sync)> = bound.iter().map(|b| &**b as &(dyn ToSql + Sync)).collect();
        let rows = client.query(&stmt, &refs).await.map_err(db_err)?;
        let cols = Arc::new(stmt.columns().iter().map(|c| c.name().to_owned()).collect::<Vec<_>>());
        rows.iter()
            .map(|r| Ok(Row { columns: Arc::clone(&cols), values: (0..r.len()).map(|i| read(r, i)).collect::<Result<_>>()? }))
            .collect()
    }

    async fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64> {
        let client = self.client.as_ref().expect("connection present");
        let stmt = client.prepare(&convert_placeholders(sql, Vendor::Postgresql)).await.map_err(db_err)?;
        let bound: Vec<Boxed> = params.iter().zip(stmt.params()).map(|(v, t)| bind(v, t)).collect::<Result<_>>()?;
        let refs: Vec<&(dyn ToSql + Sync)> = bound.iter().map(|b| &**b as &(dyn ToSql + Sync)).collect();
        client.execute(&stmt, &refs).await.map_err(db_err)
    }

    async fn insert(&mut self, sql: &str, params: &[Value], returning: bool) -> Result<Option<i64>> {
        if returning {
            let rows = self.query(sql, params).await?;
            return Ok(rows.first().and_then(|r| r.values.first()).and_then(Value::as_i64));
        }
        self.execute(sql, params).await?;
        Ok(None)
    }

    async fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.client.as_ref().expect("connection present").batch_execute(sql).await.map_err(db_err)
    }
}
