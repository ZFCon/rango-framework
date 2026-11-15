//! Session stores: `db`, `cache`, `cached_db`, `file`, `signed_cookies`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Duration;
use indexmap::IndexMap;
use rango_core::cache::{Timeout, caches};
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_db::models::Model;
use rango_db::Db;
use serde_json::Value as Json;

use super::models::Session as SessionRow;
use super::{decode, encode, new_session_key};

pub type Data = IndexMap<String, Json>;

/// A session store.
#[async_trait]
pub trait SessionBackend: Send + Sync {
    /// Salt used when signing the payload.
    fn key_salt(&self) -> &str {
        "rango.contrib.sessions.SessionStore"
    }
    /// `None` when the key is unknown or expired.
    async fn load(&self, key: &str) -> Result<Option<Data>>;
    async fn exists(&self, key: &str) -> Result<bool>;
    /// Store under a new unique key; returns the key.
    async fn create(&self, data: &Data, age: i64) -> Result<String>;
    /// Store under `key`; backends that derive the key from the data (signed cookies) return the new key.
    async fn save(&self, key: &str, data: &Data, age: i64, must_create: bool) -> Result<Option<String>>;
    async fn delete(&self, key: &str) -> Result<()>;
}

fn expiry(age: i64) -> chrono::DateTime<chrono::Utc> {
    rango_core::utils::timezone::now() + Duration::seconds(age)
}

// ------------------------------------------------------------------------------------ db

/// `contrib.sessions.backends.db`: rows of `rango_session`.
pub struct DbBackend {
    pub using: String,
}

impl Default for DbBackend {
    fn default() -> Self {
        Self { using: "default".into() }
    }
}

#[async_trait]
impl SessionBackend for DbBackend {
    async fn load(&self, key: &str) -> Result<Option<Data>> {
        let db = Db::using(&self.using);
        let now = rango_core::utils::timezone::now();
        match SessionRow::objects().filter_kw("session_key", key).filter_kw("expire_date__gt", now).get(&db).await {
            Ok(row) => Ok(decode(&row.session_data, self.key_salt())),
            Err(Error::ObjectDoesNotExist { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        SessionRow::objects().filter_kw("session_key", key).exists(&Db::using(&self.using)).await
    }

    async fn create(&self, data: &Data, age: i64) -> Result<String> {
        loop {
            let key = new_session_key();
            if self.exists(&key).await? {
                continue;
            }
            self.save(&key, data, age, true).await?;
            return Ok(key);
        }
    }

    async fn save(&self, key: &str, data: &Data, age: i64, must_create: bool) -> Result<Option<String>> {
        let db = Db::using(&self.using);
        let mut row = SessionRow { session_key: key.to_owned(), session_data: encode(data, self.key_salt())?, expire_date: expiry(age) };
        row.save_with(&db, must_create, None).await?;
        Ok(None)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        SessionRow::objects().filter_kw("session_key", key).delete(&Db::using(&self.using)).await?;
        Ok(())
    }
}

// ------------------------------------------------------------------------------------ cache

/// `contrib.sessions.backends.cache` (`cached_db` writes through to the database too).
pub struct CacheBackend {
    pub alias: String,
    pub db: Option<DbBackend>,
}

impl CacheBackend {
    pub fn new(alias: &str) -> Self {
        Self { alias: alias.to_owned(), db: None }
    }
    pub fn cached_db(alias: &str) -> Self {
        Self { alias: alias.to_owned(), db: Some(DbBackend::default()) }
    }
    fn cache_key(key: &str) -> String {
        format!("rango.contrib.sessions.cache{key}")
    }
}

#[async_trait]
impl SessionBackend for CacheBackend {
    fn key_salt(&self) -> &str {
        if self.db.is_some() { "rango.contrib.sessions.SessionStore" } else { "rango.contrib.sessions.SessionStore" }
    }

    async fn load(&self, key: &str) -> Result<Option<Data>> {
        let cache = caches(&self.alias)?;
        if let Some(d) = cache.get::<Data>(&Self::cache_key(key)).await? {
            return Ok(Some(d));
        }
        match &self.db {
            Some(db) => {
                let loaded = db.load(key).await?;
                if let Some(d) = &loaded {
                    cache.set(&Self::cache_key(key), d, Timeout::Secs(settings().session_cookie_age as f64)).await?;
                }
                Ok(loaded)
            }
            None => Ok(None),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(caches(&self.alias)?.has_key(&Self::cache_key(key)).await? || match &self.db {
            Some(db) => db.exists(key).await?,
            None => false,
        })
    }

    async fn create(&self, data: &Data, age: i64) -> Result<String> {
        loop {
            let key = new_session_key();
            if self.exists(&key).await? {
                continue;
            }
            self.save(&key, data, age, true).await?;
            return Ok(key);
        }
    }

    async fn save(&self, key: &str, data: &Data, age: i64, must_create: bool) -> Result<Option<String>> {
        if let Some(db) = &self.db {
            db.save(key, data, age, must_create).await?;
        }
        let cache = caches(&self.alias)?;
        let t = Timeout::Secs(age as f64);
        if must_create {
            if !cache.add(&Self::cache_key(key), data, t).await? {
                return Err(Error::Other("Session creation failed: key exists".into()));
            }
        } else {
            cache.set(&Self::cache_key(key), data, t).await?;
        }
        Ok(None)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        caches(&self.alias)?.delete(&Self::cache_key(key)).await?;
        if let Some(db) = &self.db {
            db.delete(key).await?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------------------------------ file

/// `contrib.sessions.backends.file`: `sessionid<key>` files in `SESSION_FILE_PATH`.
pub struct FileBackend {
    pub dir: PathBuf,
}

impl Default for FileBackend {
    fn default() -> Self {
        Self { dir: settings().session_file_path.clone().map_or_else(std::env::temp_dir, PathBuf::from) }
    }
}

impl FileBackend {
    fn path(&self, key: &str) -> Result<PathBuf> {
        if !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(Error::Suspicious { kind: rango_core::exceptions::SuspiciousKind::InvalidSessionKey, message: "Invalid characters in session key".into() });
        }
        Ok(self.dir.join(format!("sessionid{key}")))
    }
}

#[async_trait]
impl SessionBackend for FileBackend {
    async fn load(&self, key: &str) -> Result<Option<Data>> {
        let Ok(raw) = std::fs::read_to_string(self.path(key)?) else { return Ok(None) };
        let Some((exp, payload)) = raw.split_once('\n') else { return Ok(None) };
        if exp.parse::<i64>().is_ok_and(|e| e <= rango_core::utils::timezone::now().timestamp()) {
            let _ = std::fs::remove_file(self.path(key)?);
            return Ok(None);
        }
        Ok(decode(payload, self.key_salt()))
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.path(key)?.exists())
    }

    async fn create(&self, data: &Data, age: i64) -> Result<String> {
        loop {
            let key = new_session_key();
            if self.exists(&key).await? {
                continue;
            }
            self.save(&key, data, age, true).await?;
            return Ok(key);
        }
    }

    async fn save(&self, key: &str, data: &Data, age: i64, _must_create: bool) -> Result<Option<String>> {
        std::fs::create_dir_all(&self.dir).map_err(|e| Error::Other(Box::new(e)))?;
        let body = format!("{}\n{}", expiry(age).timestamp(), encode(data, self.key_salt())?);
        std::fs::write(self.path(key)?, body).map_err(|e| Error::Other(Box::new(e)))?;
        Ok(None)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let _ = std::fs::remove_file(self.path(key)?);
        Ok(())
    }
}

// ------------------------------------------------------------------------------------ signed cookies

/// `contrib.sessions.backends.signed_cookies`: the cookie *is* the (signed) data.
pub struct SignedCookieBackend;

#[async_trait]
impl SessionBackend for SignedCookieBackend {
    fn key_salt(&self) -> &str {
        "rango.contrib.sessions.backends.signed_cookies.SessionStore"
    }
    async fn load(&self, key: &str) -> Result<Option<Data>> {
        Ok(decode(key, self.key_salt()))
    }
    async fn exists(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }
    async fn create(&self, data: &Data, age: i64) -> Result<String> {
        Ok(self.save("", data, age, true).await?.unwrap_or_default())
    }
    async fn save(&self, _key: &str, data: &Data, _age: i64, _must_create: bool) -> Result<Option<String>> {
        Ok(Some(encode(data, self.key_salt())?))
    }
    async fn delete(&self, _key: &str) -> Result<()> {
        Ok(())
    }
}

/// `SESSION_ENGINE` -> backend.
pub fn backend_from_settings() -> Result<Arc<dyn SessionBackend>> {
    let s = settings();
    let engine = s.session_engine.clone();
    Ok(match engine.as_str() {
        "rango.contrib.sessions.backends.db" => Arc::new(DbBackend::default()),
        "rango.contrib.sessions.backends.cache" => Arc::new(CacheBackend::new(&s.session_cache_alias)),
        "rango.contrib.sessions.backends.cached_db" => Arc::new(CacheBackend::cached_db(&s.session_cache_alias)),
        "rango.contrib.sessions.backends.file" => Arc::new(FileBackend::default()),
        "rango.contrib.sessions.backends.signed_cookies" => Arc::new(SignedCookieBackend),
        other => return Err(Error::improperly_configured(format!("Unknown SESSION_ENGINE '{other}'"))),
    })
}
