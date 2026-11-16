//! `core.cache`: the cache framework.
//!
//! Values are stored serialized (JSON bytes), so any `Serialize + DeserializeOwned` type can be
//! cached; values are stored as JSON.

pub mod backends;

use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::conf::try_settings;
use crate::exceptions::{Error, Result};

/// `timeout=` argument of the cache methods.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Timeout {
    /// `DEFAULT_TIMEOUT`: the cache's `TIMEOUT` setting.
    Default,
    /// `None`: never expires.
    Never,
    /// Seconds; `0` (or negative) means "do not cache".
    Secs(f64),
}

impl From<u64> for Timeout {
    fn from(s: u64) -> Self {
        Timeout::Secs(s as f64)
    }
}

/// Storage primitive of a cache backend; keys are already prefixed/versioned.
#[async_trait]
pub trait CacheBackend: Send + Sync {
    async fn add(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<bool>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<()>;
    async fn touch(&self, key: &str, ttl: Option<Duration>) -> Result<bool>;
    async fn delete(&self, key: &str) -> Result<bool>;
    async fn has_key(&self, key: &str) -> Result<bool> {
        Ok(self.get(key).await?.is_some())
    }
    async fn clear(&self) -> Result<()>;
    async fn close(&self) {}
}

/// Parameters shared by every backend (`BaseCache.__init__`).
#[derive(Debug, Clone)]
pub struct CacheParams {
    pub default_timeout: Option<f64>,
    pub key_prefix: String,
    pub version: i64,
    pub location: String,
    pub options: serde_json::Value,
}

impl CacheParams {
    pub fn from_config(cfg: &serde_json::Value) -> CacheParams {
        let timeout = match cfg.get("TIMEOUT") {
            None => Some(300.0),
            Some(serde_json::Value::Null) => None,
            Some(v) => v.as_f64(),
        };
        CacheParams {
            default_timeout: timeout,
            key_prefix: cfg.get("KEY_PREFIX").and_then(|v| v.as_str()).unwrap_or("").to_owned(),
            version: cfg.get("VERSION").and_then(|v| v.as_i64()).unwrap_or(1),
            location: match cfg.get("LOCATION") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(","),
                _ => String::new(),
            },
            options: cfg.get("OPTIONS").cloned().unwrap_or(serde_json::Value::Null),
        }
    }
}

type KeyFunc = Arc<dyn Fn(&str, &str, i64) -> String + Send + Sync>;

/// `caches["alias"]`.
#[derive(Clone)]
pub struct Cache {
    backend: Arc<dyn CacheBackend>,
    pub params: CacheParams,
    key_func: KeyFunc,
}

/// `default_key_func`: `prefix:version:key`.
pub fn default_key_func(key: &str, prefix: &str, version: i64) -> String {
    format!("{prefix}:{version}:{key}")
}

impl Cache {
    pub fn new(backend: Arc<dyn CacheBackend>, params: CacheParams) -> Self {
        Self { backend, params, key_func: Arc::new(default_key_func) }
    }

    /// `KEY_FUNCTION`.
    pub fn with_key_func(mut self, f: impl Fn(&str, &str, i64) -> String + Send + Sync + 'static) -> Self {
        self.key_func = Arc::new(f);
        self
    }

    /// `make_key(key, version)`.
    pub fn make_key(&self, key: &str, version: Option<i64>) -> String {
        (self.key_func)(key, &self.params.key_prefix, version.unwrap_or(self.params.version))
    }

    fn ttl(&self, t: Timeout) -> Option<Option<Duration>> {
        // Some(None) = never expires, Some(Some(d)) = ttl, None = do not store.
        let secs = match t {
            Timeout::Default => self.params.default_timeout,
            Timeout::Never => None,
            Timeout::Secs(s) => Some(s),
        };
        match secs {
            None => Some(None),
            Some(s) if s <= 0.0 => None,
            Some(s) => Some(Some(Duration::from_secs_f64(s))),
        }
    }

    fn key(&self, key: &str, version: Option<i64>) -> String {
        let k = self.make_key(key, version);
        if k.len() > 250 || k.chars().any(|c| c.is_control() || c == ' ') {
            tracing::warn!(target: "rango.cache", "Cache key contains characters that will cause errors if used with memcached: {k:?}");
        }
        k
    }

    fn enc<T: Serialize + ?Sized>(v: &T) -> Result<Vec<u8>> {
        serde_json::to_vec(v).map_err(|e| Error::Other(Box::new(e)))
    }

    fn dec<T: DeserializeOwned>(b: &[u8]) -> Result<T> {
        serde_json::from_slice(b).map_err(|e| Error::Other(Box::new(e)))
    }

    /// `add(key, value, timeout)`: store only if the key is absent; returns whether it stored.
    pub async fn add<T: Serialize + ?Sized>(&self, key: &str, value: &T, timeout: Timeout) -> Result<bool> {
        match self.ttl(timeout) {
            None => Ok(false),
            Some(ttl) => self.backend.add(&self.key(key, None), Self::enc(value)?, ttl).await,
        }
    }

    /// `get(key)`.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.backend.get(&self.key(key, None)).await? {
            Some(b) => Ok(Some(Self::dec(&b)?)),
            None => Ok(None),
        }
    }

    pub async fn get_versioned<T: DeserializeOwned>(&self, key: &str, version: i64) -> Result<Option<T>> {
        match self.backend.get(&self.key(key, Some(version))).await? {
            Some(b) => Ok(Some(Self::dec(&b)?)),
            None => Ok(None),
        }
    }

    /// `set(key, value, timeout)`.
    pub async fn set<T: Serialize + ?Sized>(&self, key: &str, value: &T, timeout: Timeout) -> Result<()> {
        match self.ttl(timeout) {
            // A non-positive timeout deletes the key.
            None => {
                self.backend.delete(&self.key(key, None)).await?;
                Ok(())
            }
            Some(ttl) => self.backend.set(&self.key(key, None), Self::enc(value)?, ttl).await,
        }
    }

    /// `touch(key, timeout)`.
    pub async fn touch(&self, key: &str, timeout: Timeout) -> Result<bool> {
        match self.ttl(timeout) {
            None => self.backend.delete(&self.key(key, None)).await,
            Some(ttl) => self.backend.touch(&self.key(key, None), ttl).await,
        }
    }

    pub async fn delete(&self, key: &str) -> Result<bool> {
        self.backend.delete(&self.key(key, None)).await
    }

    pub async fn has_key(&self, key: &str) -> Result<bool> {
        self.backend.has_key(&self.key(key, None)).await
    }

    /// `get_many(keys)`: only the keys that exist.
    pub async fn get_many<T: DeserializeOwned>(&self, keys: &[&str]) -> Result<indexmap::IndexMap<String, T>> {
        let mut out = indexmap::IndexMap::new();
        for k in keys {
            if let Some(v) = self.get::<T>(k).await? {
                out.insert((*k).to_owned(), v);
            }
        }
        Ok(out)
    }

    /// `set_many(data, timeout)`; returns the keys that failed to be set (always empty here).
    pub async fn set_many<T: Serialize>(&self, data: &[(&str, T)], timeout: Timeout) -> Result<Vec<String>> {
        for (k, v) in data {
            self.set(k, v, timeout).await?;
        }
        Ok(vec![])
    }

    pub async fn delete_many(&self, keys: &[&str]) -> Result<()> {
        for k in keys {
            self.delete(k).await?;
        }
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        self.backend.clear().await
    }

    /// `incr(key, delta)`; raises `ValueError` ("Key '…' not found") when missing.
    pub async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let Some(v) = self.get::<i64>(key).await? else { return Err(Error::Other(format!("Key '{key}' not found").into())) };
        let new = v + delta;
        // Keep the remaining TTL: plain `set` with the default timeout like locmem does is close enough.
        self.set(key, &new, Timeout::Default).await?;
        Ok(new)
    }

    pub async fn decr(&self, key: &str, delta: i64) -> Result<i64> {
        self.incr(key, -delta).await
    }

    /// `get_or_set(key, default, timeout)`.
    pub async fn get_or_set<T: Serialize + DeserializeOwned>(&self, key: &str, default: impl FnOnce() -> T, timeout: Timeout) -> Result<T> {
        if let Some(v) = self.get::<T>(key).await? {
            return Ok(v);
        }
        let v = default();
        self.add(key, &v, timeout).await?;
        // Re-read (another writer may have won the add).
        Ok(self.get::<T>(key).await?.unwrap_or(v))
    }

    pub async fn close(&self) {
        self.backend.close().await;
    }

    // ---- blocking variants (templates and other synchronous code); fine for in-process and file backends.

    pub fn get_blocking<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        futures_util::future::FutureExt::now_or_never(self.get::<T>(key)).unwrap_or_else(|| Err(Error::Other("cache backend needs an async context".into())))
    }

    pub fn set_blocking<T: Serialize + ?Sized>(&self, key: &str, value: &T, timeout: Timeout) -> Result<()> {
        futures_util::future::FutureExt::now_or_never(self.set(key, value, timeout)).unwrap_or_else(|| Err(Error::Other("cache backend needs an async context".into())))
    }
}

/// Factory for a backend from its `CACHES` entry.
pub type BackendFactory = fn(&CacheParams) -> Result<Arc<dyn CacheBackend>>;

pub struct CacheBackendRegistration {
    pub name: &'static str,
    pub build: BackendFactory,
}

inventory::collect!(CacheBackendRegistration);

/// `register_cache_backend!("rango.core.cache.backends.redis.RedisCache", |params| ...)`.
#[macro_export]
macro_rules! register_cache_backend {
    ($name:expr, $build:expr) => {
        $crate::inventory::submit! { $crate::cache::CacheBackendRegistration { name: $name, build: $build } }
    };
}

static CACHES: LazyLock<Mutex<Option<(String, Arc<indexmap::IndexMap<String, Arc<Cache>>>)>>> = LazyLock::new(|| Mutex::new(None));

fn normalize(name: &str) -> String {
    name.strip_prefix("rango.").map_or_else(|| name.to_owned(), |rest| format!("rango.{rest}"))
}

fn build_all() -> Result<indexmap::IndexMap<String, Arc<Cache>>> {
    let cfg = try_settings()?.caches.clone();
    let mut out = indexmap::IndexMap::new();
    let Some(map) = cfg.as_object() else { return Ok(out) };
    for (alias, c) in map {
        let backend_name = normalize(c.get("BACKEND").and_then(|v| v.as_str()).unwrap_or("rango.core.cache.backends.locmem.LocMemCache"));
        let reg = inventory::iter::<CacheBackendRegistration>.into_iter().find(|r| r.name == backend_name).ok_or_else(|| Error::improperly_configured(format!("Could not find backend '{backend_name}'")))?;
        let params = CacheParams::from_config(c);
        out.insert(alias.clone(), Arc::new(Cache::new((reg.build)(&params)?, params)));
    }
    Ok(out)
}

/// `caches[alias]`.
pub fn caches(alias: &str) -> Result<Arc<Cache>> {
    let key = try_settings()?.caches.to_string();
    let mut slot = CACHES.lock().expect("caches lock");
    if slot.as_ref().is_none_or(|(k, _)| *k != key) {
        *slot = Some((key.clone(), Arc::new(build_all()?)));
    }
    let (_, all) = slot.as_ref().expect("just set");
    all.get(alias).cloned().ok_or_else(|| Error::improperly_configured(format!("The connection '{alias}' doesn't exist.")))
}

/// `core.cache.cache`: the `default` cache.
pub fn cache() -> Result<Arc<Cache>> {
    caches("default")
}
