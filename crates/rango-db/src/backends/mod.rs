//! Database backends: SQLite and PostgreSQL.

pub mod postgres;
pub mod sqlite;

use async_trait::async_trait;
use rango_core::conf::DatabaseConfig;
use rango_core::exceptions::{Error, Result};

use crate::value::{Row, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vendor {
    Sqlite,
    Postgresql,
}

impl Vendor {
    /// `connection.ops.quote_name()`.
    pub fn quote(self, name: &str) -> String {
        if name.starts_with('"') && name.ends_with('"') { name.to_owned() } else { format!("\"{name}\"") }
    }

    pub fn from_engine(engine: &str) -> Result<Vendor> {
        match engine {
            "rango.db.backends.sqlite3" | "rango.db.backends.sqlite3" => Ok(Vendor::Sqlite),
            "rango.db.backends.postgresql" | "rango.db.backends.postgresql" => Ok(Vendor::Postgresql),
            other => Err(Error::improperly_configured(format!(
                "settings.DATABASES is improperly configured. Unsupported or unknown ENGINE {other:?}"
            ))),
        }
    }

    /// `allows_group_by_selected_pks`.
    pub fn group_by_pk_only(self) -> bool {
        self == Vendor::Postgresql
    }
}

/// A checked-out database connection.
#[async_trait]
pub trait Connection: Send {
    fn vendor(&self) -> Vendor;
    /// Run a query; `sql` uses `%s` placeholders.
    async fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<Row>>;
    /// Run a statement; returns rows affected.
    async fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64>;
    /// Run several statements without parameters.
    async fn execute_batch(&mut self, sql: &str) -> Result<()>;
    /// Run an INSERT and return the generated primary key (PostgreSQL: the `RETURNING` value).
    async fn insert(&mut self, sql: &str, params: &[Value], returning: bool) -> Result<Option<i64>>;
}

/// Open a pool for a `DATABASES` entry.
pub async fn connect(config: &DatabaseConfig) -> Result<Pool> {
    match Vendor::from_engine(&config.engine)? {
        Vendor::Sqlite => Ok(Pool::Sqlite(sqlite::SqlitePool::new(config)?)),
        Vendor::Postgresql => Ok(Pool::Postgres(postgres::PgPool::new(config)?)),
    }
}

pub enum Pool {
    Sqlite(sqlite::SqlitePool),
    Postgres(postgres::PgPool),
}

impl Pool {
    pub fn vendor(&self) -> Vendor {
        match self {
            Pool::Sqlite(_) => Vendor::Sqlite,
            Pool::Postgres(_) => Vendor::Postgresql,
        }
    }

    pub async fn acquire(&self) -> Result<Box<dyn Connection>> {
        match self {
            Pool::Sqlite(p) => Ok(Box::new(p.acquire().await?)),
            Pool::Postgres(p) => Ok(Box::new(p.acquire().await?)),
        }
    }
}

pub(crate) fn db_err(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string().into())
}

/// Convert `%s` placeholders: `?` (SQLite) or `$n` (PostgreSQL); `%%` becomes `%`.
pub fn convert_placeholders(sql: &str, vendor: Vendor) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut n = 0;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('s') => {
                    chars.next();
                    n += 1;
                    match vendor {
                        Vendor::Sqlite => out.push('?'),
                        Vendor::Postgresql => out.push_str(&format!("${n}")),
                    }
                    continue;
                }
                Some('%') => {
                    chars.next();
                    out.push('%');
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}
