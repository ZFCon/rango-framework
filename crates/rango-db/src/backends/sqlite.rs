//! SQLite backend (rusqlite, driven from `spawn_blocking`).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rango_core::conf::DatabaseConfig;
use rango_core::exceptions::Result;
use rusqlite::types::{Value as SqlValue, ValueRef};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{Connection, Vendor, convert_placeholders, db_err};
use crate::value::{Row, Value};

struct Shared {
    path: String,
    memory: bool,
    idle: Mutex<Vec<rusqlite::Connection>>,
    permits: Arc<Semaphore>,
}

pub struct SqlitePool(Arc<Shared>);

fn open(path: &str) -> Result<rusqlite::Connection> {
    let conn = rusqlite::Connection::open(path).map_err(db_err)?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;").map_err(db_err)?;
    register_functions(&conn)?;
    Ok(conn)
}

/// Functions the reference implementation registers on every SQLite connection (REGEXP and friends).
fn register_functions(conn: &rusqlite::Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    conn.create_scalar_function("regexp", 2, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let pat: String = ctx.get(0)?;
        let text: Option<String> = ctx.get(1)?;
        Ok(text.map(|t| regex::Regex::new(&pat).map(|r| r.is_match(&t)).unwrap_or(false)))
    })
    .map_err(db_err)?;
    conn.create_scalar_function("rango_date_extract", 2, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let (lookup, v): (String, Option<String>) = (ctx.get(0)?, ctx.get(1)?);
        Ok(v.and_then(|s| extract(&lookup, &s)))
    })
    .map_err(db_err)?;
    for n in [4usize, 3] {
        conn.create_scalar_function("rango_datetime_extract", n as i32, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
            let (lookup, v): (String, Option<String>) = (ctx.get(0)?, ctx.get(1)?);
            Ok(v.and_then(|s| extract(&lookup, &s)))
        })
        .map_err(db_err)?;
    }
    Ok(())
}

fn parse_dt(s: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .ok()
        .or_else(|| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)))
}

fn extract(lookup: &str, s: &str) -> Option<i64> {
    use chrono::{Datelike, Timelike};
    let dt = parse_dt(s)?;
    Some(match lookup {
        "year" => i64::from(dt.year()),
        "iso_year" => i64::from(dt.iso_week().year()),
        "month" => i64::from(dt.month()),
        "day" => i64::from(dt.day()),
        "week_day" => i64::from(dt.weekday().num_days_from_sunday()) + 1,
        "iso_week_day" => i64::from(dt.weekday().number_from_monday()),
        "week" => i64::from(dt.iso_week().week()),
        "quarter" => i64::from((dt.month() - 1) / 3 + 1),
        "hour" => i64::from(dt.hour()),
        "minute" => i64::from(dt.minute()),
        "second" => i64::from(dt.second()),
        _ => return None,
    })
}

impl SqlitePool {
    pub fn new(config: &DatabaseConfig) -> Result<Self> {
        let name = if config.name.is_empty() { ":memory:" } else { &config.name };
        let memory = name == ":memory:" || name.starts_with("file::memory:");
        let max = if memory { 1 } else { 8 };
        Ok(Self(Arc::new(Shared { path: name.to_owned(), memory, idle: Mutex::new(Vec::new()), permits: Arc::new(Semaphore::new(max)) })))
    }

    pub async fn acquire(&self) -> Result<SqliteConn> {
        let permit = Arc::clone(&self.0.permits).acquire_owned().await.map_err(db_err)?;
        let existing = self.0.idle.lock().expect("pool lock").pop();
        let conn = match existing {
            Some(c) => c,
            None => open(&self.0.path)?,
        };
        Ok(SqliteConn { conn: Some(conn), shared: Arc::clone(&self.0), _permit: permit })
    }
}

pub struct SqliteConn {
    conn: Option<rusqlite::Connection>,
    shared: Arc<Shared>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for SqliteConn {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            // In-memory databases must keep their single connection alive.
            let _ = self.shared.memory;
            self.shared.idle.lock().expect("pool lock").push(c);
        }
    }
}

fn to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Int(i) => SqlValue::Integer(*i),
        Value::Float(f) => SqlValue::Real(*f),
        Value::Str(s) => SqlValue::Text(s.clone()),
        Value::Bytes(b) => SqlValue::Blob(b.clone()),
        // Everything else is converted by the field layer before it reaches the backend.
        other => SqlValue::Text(crate::models::fields_db::adapt_to_text(other)),
    }
}

fn from_sql(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(t) => Value::Str(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

#[async_trait]
impl Connection for SqliteConn {
    fn vendor(&self) -> Vendor {
        Vendor::Sqlite
    }

    async fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        let sql = convert_placeholders(sql, Vendor::Sqlite);
        let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
        let conn = self.conn.take().expect("connection present");
        let (conn, result) = tokio::task::spawn_blocking(move || {
            let r = (|| -> std::result::Result<Vec<Row>, rusqlite::Error> {
                let mut stmt = conn.prepare_cached(&sql)?;
                let cols = Arc::new(stmt.column_names().into_iter().map(|s| s.to_owned()).collect::<Vec<_>>());
                let n = cols.len();
                let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(Row { columns: Arc::clone(&cols), values: (0..n).map(|i| from_sql(row.get_ref_unwrap(i))).collect() });
                }
                Ok(out)
            })();
            (conn, r)
        })
        .await
        .map_err(db_err)?;
        self.conn = Some(conn);
        result.map_err(db_err)
    }

    async fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64> {
        let sql = convert_placeholders(sql, Vendor::Sqlite);
        let params: Vec<SqlValue> = params.iter().map(to_sql).collect();
        let conn = self.conn.take().expect("connection present");
        let (conn, result) = tokio::task::spawn_blocking(move || {
            let r = conn.execute(&sql, rusqlite::params_from_iter(params.iter()));
            (conn, r)
        })
        .await
        .map_err(db_err)?;
        self.conn = Some(conn);
        result.map(|n| n as u64).map_err(db_err)
    }

    async fn insert(&mut self, sql: &str, params: &[Value], _returning: bool) -> Result<Option<i64>> {
        self.insert_impl(sql, params).await
    }

    async fn execute_batch(&mut self, sql: &str) -> Result<()> {
        let sql = sql.to_owned();
        let conn = self.conn.take().expect("connection present");
        let (conn, result) = tokio::task::spawn_blocking(move || {
            let r = conn.execute_batch(&sql);
            (conn, r)
        })
        .await
        .map_err(db_err)?;
        self.conn = Some(conn);
        result.map_err(db_err)
    }
}

impl SqliteConn {
    async fn insert_impl(&mut self, sql: &str, params: &[Value]) -> Result<Option<i64>> {
        self.execute(sql, params).await?;
        Ok(Some(self.last_insert_rowid()))
    }

    /// `cursor.lastrowid` of the last INSERT on this connection.
    pub fn last_insert_rowid(&self) -> i64 {
        self.conn.as_ref().map_or(0, |c| c.last_insert_rowid())
    }
}
