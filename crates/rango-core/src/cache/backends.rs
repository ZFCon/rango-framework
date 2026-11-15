//! Cache backends: `locmem`, `filebased`, `dummy`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::{CacheBackend, CacheParams};
use crate::exceptions::Result;

// ----------------------------------------------------------------------------------- locmem

#[derive(Default)]
struct Store {
    data: HashMap<String, (Vec<u8>, Option<Instant>)>,
    /// Access order, oldest first (for culling).
    order: Vec<String>,
}

static LOCMEM: LazyLock<Mutex<HashMap<String, Arc<Mutex<Store>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// `LocMemCache`: per-process, shared by `LOCATION` name; culls when `MAX_ENTRIES` is exceeded.
pub struct LocMemCache {
    store: Arc<Mutex<Store>>,
    max_entries: usize,
    cull_frequency: usize,
}

impl LocMemCache {
    pub fn new(params: &CacheParams) -> Self {
        let store = LOCMEM.lock().expect("locmem registry").entry(params.location.clone()).or_default().clone();
        let opt = |k: &str, d: usize| params.options.get(k).and_then(|v| v.as_u64()).map_or(d, |v| v as usize);
        Self { store, max_entries: opt("MAX_ENTRIES", 300), cull_frequency: opt("CULL_FREQUENCY", 3) }
    }

    fn expired(e: &(Vec<u8>, Option<Instant>)) -> bool {
        e.1.is_some_and(|t| t <= Instant::now())
    }

    fn put(&self, s: &mut Store, key: &str, value: Vec<u8>, ttl: Option<Duration>) {
        if !s.data.contains_key(key) && s.data.len() >= self.max_entries {
            // _cull: drop expired entries, then every `cull_frequency`-th oldest (all when 0).
            let expired: Vec<String> = s.data.iter().filter(|(_, e)| Self::expired(e)).map(|(k, _)| k.clone()).collect();
            for k in expired {
                s.data.remove(&k);
                s.order.retain(|o| *o != k);
            }
            if s.data.len() >= self.max_entries {
                if self.cull_frequency == 0 {
                    s.data.clear();
                    s.order.clear();
                } else {
                    let n = s.data.len() / self.cull_frequency;
                    for k in s.order.drain(..n.min(s.order.len())).collect::<Vec<_>>() {
                        s.data.remove(&k);
                    }
                }
            }
        }
        s.data.insert(key.to_owned(), (value, ttl.map(|d| Instant::now() + d)));
        s.order.retain(|o| o != key);
        s.order.push(key.to_owned());
    }
}

#[async_trait]
impl CacheBackend for LocMemCache {
    async fn add(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<bool> {
        let mut s = self.store.lock().expect("locmem store");
        if s.data.get(key).is_some_and(|e| !Self::expired(e)) {
            return Ok(false);
        }
        self.put(&mut s, key, value, ttl);
        Ok(true)
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let mut s = self.store.lock().expect("locmem store");
        match s.data.get(key) {
            Some(e) if Self::expired(e) => {
                s.data.remove(key);
                s.order.retain(|o| o != key);
                Ok(None)
            }
            Some(e) => {
                let v = e.0.clone();
                s.order.retain(|o| o != key);
                s.order.push(key.to_owned());
                Ok(Some(v))
            }
            None => Ok(None),
        }
    }

    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<()> {
        let mut s = self.store.lock().expect("locmem store");
        self.put(&mut s, key, value, ttl);
        Ok(())
    }

    async fn touch(&self, key: &str, ttl: Option<Duration>) -> Result<bool> {
        let mut s = self.store.lock().expect("locmem store");
        match s.data.get_mut(key) {
            Some(e) if !Self::expired(e) => {
                e.1 = ttl.map(|d| Instant::now() + d);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn delete(&self, key: &str) -> Result<bool> {
        let mut s = self.store.lock().expect("locmem store");
        s.order.retain(|o| o != key);
        Ok(s.data.remove(key).is_some_and(|e| !Self::expired(&e)))
    }

    async fn clear(&self) -> Result<()> {
        let mut s = self.store.lock().expect("locmem store");
        s.data.clear();
        s.order.clear();
        Ok(())
    }
}

crate::register_cache_backend!("rango.core.cache.backends.locmem.LocMemCache", |p| Ok(Arc::new(LocMemCache::new(p))));

// ----------------------------------------------------------------------------------- dummy

/// `DummyCache`: accepts everything, stores nothing.
pub struct DummyCache;

#[async_trait]
impl CacheBackend for DummyCache {
    async fn add(&self, _: &str, _: Vec<u8>, _: Option<Duration>) -> Result<bool> {
        Ok(true)
    }
    async fn get(&self, _: &str) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
    async fn set(&self, _: &str, _: Vec<u8>, _: Option<Duration>) -> Result<()> {
        Ok(())
    }
    async fn touch(&self, _: &str, _: Option<Duration>) -> Result<bool> {
        Ok(true)
    }
    async fn delete(&self, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn clear(&self) -> Result<()> {
        Ok(())
    }
}

crate::register_cache_backend!("rango.core.cache.backends.dummy.DummyCache", |_| Ok(Arc::new(DummyCache)));

// ----------------------------------------------------------------------------------- file based

/// `FileBasedCache`: one `<md5(key)>.djcache` file per entry (expiry timestamp, then the value).
pub struct FileBasedCache {
    dir: PathBuf,
    max_entries: usize,
    cull_frequency: usize,
}

impl FileBasedCache {
    pub fn new(params: &CacheParams) -> Self {
        let opt = |k: &str, d: usize| params.options.get(k).and_then(|v| v.as_u64()).map_or(d, |v| v as usize);
        Self { dir: PathBuf::from(&params.location), max_entries: opt("MAX_ENTRIES", 300), cull_frequency: opt("CULL_FREQUENCY", 3) }
    }

    fn path(&self, key: &str) -> PathBuf {
        use md5::{Digest, Md5};
        let digest = Md5::digest(key.as_bytes());
        self.dir.join(format!("{}.djcache", digest.iter().map(|b| format!("{b:02x}")).collect::<String>()))
    }

    fn now() -> f64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
    }

    fn write(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(|e| crate::exceptions::Error::Other(Box::new(e)))?;
        self.cull();
        let expires = ttl.map_or(-1.0, |d| Self::now() + d.as_secs_f64());
        let mut buf = format!("{expires}\n").into_bytes();
        buf.extend_from_slice(value);
        std::fs::write(self.path(key), buf).map_err(|e| crate::exceptions::Error::Other(Box::new(e)))
    }

    fn read(&self, key: &str) -> Option<(f64, Vec<u8>)> {
        let raw = std::fs::read(self.path(key)).ok()?;
        let nl = raw.iter().position(|b| *b == b'\n')?;
        let exp: f64 = std::str::from_utf8(&raw[..nl]).ok()?.parse().ok()?;
        if exp >= 0.0 && exp <= Self::now() {
            let _ = std::fs::remove_file(self.path(key));
            return None;
        }
        Some((exp, raw[nl + 1..].to_vec()))
    }

    fn cull(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "djcache")).collect();
        if files.len() < self.max_entries {
            return;
        }
        if self.cull_frequency == 0 {
            files.iter().for_each(|f| {
                let _ = std::fs::remove_file(f);
            });
        } else {
            files.sort();
            for f in files.iter().step_by(self.cull_frequency) {
                let _ = std::fs::remove_file(f);
            }
        }
    }
}

#[async_trait]
impl CacheBackend for FileBasedCache {
    async fn add(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<bool> {
        if self.read(key).is_some() {
            return Ok(false);
        }
        self.write(key, &value, ttl)?;
        Ok(true)
    }
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.read(key).map(|(_, v)| v))
    }
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> Result<()> {
        self.write(key, &value, ttl)
    }
    async fn touch(&self, key: &str, ttl: Option<Duration>) -> Result<bool> {
        match self.read(key) {
            Some((_, v)) => {
                self.write(key, &v, ttl)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
    async fn delete(&self, key: &str) -> Result<bool> {
        Ok(std::fs::remove_file(self.path(key)).is_ok())
    }
    async fn clear(&self) -> Result<()> {
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "djcache")) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        Ok(())
    }
}

crate::register_cache_backend!("rango.core.cache.backends.filebased.FileBasedCache", |p| Ok(Arc::new(FileBasedCache::new(p))));
