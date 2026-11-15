//! Connections, transactions and the `DATABASES` registry (`db`).

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use rango_core::conf::try_settings;
use rango_core::exceptions::{Error, Result};
use tokio::sync::Mutex as AsyncMutex;

use crate::backends::{Connection, Pool, Vendor, connect};
use crate::value::{Row, Value};

static POOLS: LazyLock<AsyncMutex<HashMap<String, Arc<Pool>>>> = LazyLock::new(|| AsyncMutex::new(HashMap::new()));

/// Pool for a `DATABASES` alias (opened on first use).
pub async fn pool(alias: &str) -> Result<Arc<Pool>> {
    let mut pools = POOLS.lock().await;
    if let Some(p) = pools.get(alias) {
        return Ok(Arc::clone(p));
    }
    let settings = try_settings()?;
    let config = settings
        .databases
        .get(alias)
        .ok_or_else(|| Error::improperly_configured(format!("The connection '{alias}' doesn't exist.")))?
        .clone();
    let p = Arc::new(connect(&config).await?);
    pools.insert(alias.to_owned(), Arc::clone(&p));
    Ok(p)
}

/// Close every pool (tests that switch databases).
pub async fn close_all() {
    POOLS.lock().await.clear();
}

/// Where queries run: a [`Db`] (autocommit) or a [`Transaction`].
#[async_trait]
pub trait Executor: Send + Sync {
    fn alias(&self) -> &str;
    fn vendor(&self) -> Vendor;
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>>;
    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64>;
    async fn insert(&self, sql: &str, params: &[Value], returning: bool) -> Result<Option<i64>>;
    async fn execute_batch(&self, sql: &str) -> Result<()>;
}

/// A handle to a configured database (`connections["alias"]`). Cheap to clone.
#[derive(Clone, Debug)]
pub struct Db {
    alias: String,
}

impl Db {
    pub fn using(alias: &str) -> Db {
        Db { alias: alias.to_owned() }
    }

    /// The `default` database.
    pub fn default_db() -> Db {
        Db::using("default")
    }

    /// `transaction.atomic()`: run `f` in a transaction, committing on `Ok` and rolling back on `Err`.
    pub async fn atomic<T, F>(&self, f: F) -> Result<T>
    where
        F: for<'a> FnOnce(&'a Transaction) -> BoxFuture<'a, Result<T>>,
    {
        let tx = Transaction::begin(&self.alias).await?;
        match f(&tx).await {
            Ok(v) => {
                tx.commit().await?;
                Ok(v)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

fn vendor_of(alias: &str) -> Vendor {
    try_settings()
        .ok()
        .and_then(|s| s.databases.get(alias).and_then(|c| Vendor::from_engine(&c.engine).ok()))
        .unwrap_or(Vendor::Sqlite)
}

#[async_trait]
impl Executor for Db {
    fn alias(&self) -> &str {
        &self.alias
    }
    fn vendor(&self) -> Vendor {
        vendor_of(&self.alias)
    }
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        pool(&self.alias).await?.acquire().await?.query(sql, params).await
    }
    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        pool(&self.alias).await?.acquire().await?.execute(sql, params).await
    }
    async fn insert(&self, sql: &str, params: &[Value], returning: bool) -> Result<Option<i64>> {
        pool(&self.alias).await?.acquire().await?.insert(sql, params, returning).await
    }
    async fn execute_batch(&self, sql: &str) -> Result<()> {
        pool(&self.alias).await?.acquire().await?.execute_batch(sql).await
    }
}

/// An open transaction (savepoints for nesting).
pub struct Transaction {
    alias: String,
    conn: AsyncMutex<Box<dyn Connection>>,
    depth: std::sync::atomic::AtomicUsize,
    on_commit: std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl Transaction {
    async fn begin(alias: &str) -> Result<Transaction> {
        let mut conn = pool(alias).await?.acquire().await?;
        conn.execute_batch("BEGIN").await?;
        Ok(Transaction { alias: alias.to_owned(), conn: AsyncMutex::new(conn), depth: 0.into(), on_commit: std::sync::Mutex::new(Vec::new()) })
    }

    async fn commit(&self) -> Result<()> {
        self.conn.lock().await.execute_batch("COMMIT").await?;
        let callbacks: Vec<_> = std::mem::take(&mut *self.on_commit.lock().expect("on_commit lock"));
        for cb in callbacks {
            cb();
        }
        Ok(())
    }

    async fn rollback(&self) -> Result<()> {
        self.on_commit.lock().expect("on_commit lock").clear();
        self.conn.lock().await.execute_batch("ROLLBACK").await
    }

    /// Nested `atomic()`: a savepoint.
    pub async fn atomic<T, F>(&self, f: F) -> Result<T>
    where
        F: for<'a> FnOnce(&'a Transaction) -> BoxFuture<'a, Result<T>>,
    {
        let n = self.depth.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let name = format!("s{n}");
        self.conn.lock().await.execute_batch(&format!("SAVEPOINT {name}")).await?;
        let r = f(self).await;
        let res = match &r {
            Ok(_) => self.conn.lock().await.execute_batch(&format!("RELEASE SAVEPOINT {name}")).await,
            Err(_) => {
                let mut c = self.conn.lock().await;
                c.execute_batch(&format!("ROLLBACK TO SAVEPOINT {name}")).await?;
                c.execute_batch(&format!("RELEASE SAVEPOINT {name}")).await
            }
        };
        self.depth.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        res?;
        r
    }

    /// `transaction.on_commit(callback)`.
    pub fn on_commit(&self, f: impl FnOnce() + Send + 'static) {
        self.on_commit.lock().expect("on_commit lock").push(Box::new(f));
    }
}

#[async_trait]
impl Executor for Transaction {
    fn alias(&self) -> &str {
        &self.alias
    }
    fn vendor(&self) -> Vendor {
        vendor_of(&self.alias)
    }
    async fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
        self.conn.lock().await.query(sql, params).await
    }
    async fn execute(&self, sql: &str, params: &[Value]) -> Result<u64> {
        self.conn.lock().await.execute(sql, params).await
    }
    async fn insert(&self, sql: &str, params: &[Value], returning: bool) -> Result<Option<i64>> {
        self.conn.lock().await.insert(sql, params, returning).await
    }
    async fn execute_batch(&self, sql: &str) -> Result<()> {
        self.conn.lock().await.execute_batch(sql).await
    }
}

/// Run `fut` and map the error (helper for `?`-less code).
pub async fn run<T, F: Future<Output = Result<T>>>(fut: F) -> Result<T> {
    fut.await
}
