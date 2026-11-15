//! `contrib.sessions`.

pub mod backends;
pub mod middleware;
pub mod models;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use indexmap::IndexMap;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_core::signing;
use rango_core::utils::crypto::get_random_string;
use rango_http::HttpRequest;
use serde_json::Value as Json;

pub use backends::SessionBackend;

pub const TEST_COOKIE_NAME: &str = "testcookie";
pub const TEST_COOKIE_VALUE: &str = "worked";
const VALID_KEY_CHARS: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

/// `set_expiry(value)`.
#[derive(Debug, Clone)]
pub enum Expiry {
    /// Seconds from now.
    Age(i64),
    /// An absolute moment.
    At(DateTime<Utc>),
    /// `0`: expire when the browser closes.
    BrowserClose,
    /// `None`: back to `SESSION_COOKIE_AGE`.
    Default,
}

#[derive(Default)]
struct State {
    key: Option<String>,
    data: IndexMap<String, Json>,
    loaded: bool,
    accessed: bool,
    modified: bool,
    expiry: Option<Expiry>,
    /// A key supplied by the client that no backend entry exists for.
    invalid_key: bool,
}

/// `request.session`: a shared handle (cheap to clone).
#[derive(Clone)]
pub struct Session {
    state: Arc<Mutex<State>>,
    backend: Arc<dyn SessionBackend>,
}

impl Session {
    pub fn new(backend: Arc<dyn SessionBackend>, key: Option<String>) -> Session {
        Session { state: Arc::new(Mutex::new(State { key, ..State::default() })), backend }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("session lock")
    }

    /// Load from the backend (the middleware does this when the request carries a session cookie).
    pub async fn load(&self) -> Result<()> {
        let key = self.lock().key.clone();
        let Some(key) = key else {
            self.lock().loaded = true;
            return Ok(());
        };
        let loaded = self.backend.load(&key).await?;
        let mut s = self.lock();
        match loaded {
            Some(d) => s.data = d,
            None => {
                s.key = None;
                s.invalid_key = true;
            }
        }
        s.loaded = true;
        Ok(())
    }

    pub fn session_key(&self) -> Option<String> {
        self.lock().key.clone()
    }

    pub fn get(&self, key: &str) -> Option<Json> {
        let mut s = self.lock();
        s.accessed = true;
        s.data.get(key).cloned()
    }

    pub fn get_as<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn set(&self, key: &str, value: impl serde::Serialize) {
        let mut s = self.lock();
        s.accessed = true;
        s.modified = true;
        s.data.insert(key.to_owned(), serde_json::to_value(value).unwrap_or(Json::Null));
    }

    pub fn pop(&self, key: &str) -> Option<Json> {
        let mut s = self.lock();
        s.accessed = true;
        let v = s.data.shift_remove(key);
        if v.is_some() {
            s.modified = true;
        }
        v
    }

    pub fn contains(&self, key: &str) -> bool {
        let mut s = self.lock();
        s.accessed = true;
        s.data.contains_key(key)
    }

    pub fn keys(&self) -> Vec<String> {
        let mut s = self.lock();
        s.accessed = true;
        s.data.keys().cloned().collect()
    }

    /// `setdefault`.
    pub fn setdefault(&self, key: &str, value: impl serde::Serialize) -> Json {
        if let Some(v) = self.get(key) {
            return v;
        }
        self.set(key, value);
        self.get(key).unwrap_or(Json::Null)
    }

    pub fn is_empty(&self) -> bool {
        let s = self.lock();
        s.key.is_none() && s.data.is_empty()
    }

    pub fn accessed(&self) -> bool {
        self.lock().accessed
    }

    pub fn modified(&self) -> bool {
        self.lock().modified
    }

    /// Mark modified after mutating nested data (`request.session.modified = True`).
    pub fn mark_modified(&self) {
        self.lock().modified = true;
    }

    pub fn set_test_cookie(&self) {
        self.set(TEST_COOKIE_NAME, TEST_COOKIE_VALUE);
    }

    pub fn test_cookie_worked(&self) -> bool {
        self.get(TEST_COOKIE_NAME).is_some_and(|v| v == TEST_COOKIE_VALUE)
    }

    pub fn delete_test_cookie(&self) {
        self.pop(TEST_COOKIE_NAME);
    }

    // ---- expiry

    pub fn set_expiry(&self, value: Expiry) {
        let mut s = self.lock();
        s.expiry = match value {
            Expiry::Default => None,
            other => Some(other),
        };
        s.modified = true;
    }

    fn expiry_of(&self) -> Option<Expiry> {
        self.lock().data.get("_session_expiry").and_then(|v| match v {
            Json::Number(n) => n.as_i64().map(Expiry::Age),
            Json::String(s) => DateTime::parse_from_rfc3339(s).ok().map(|d| Expiry::At(d.to_utc())),
            _ => None,
        })
    }

    fn effective_expiry(&self) -> Option<Expiry> {
        let pending = self.lock().expiry.clone();
        pending.or_else(|| self.expiry_of())
    }

    /// `get_expiry_age()`: seconds until the session expires.
    pub fn get_expiry_age(&self) -> i64 {
        match self.effective_expiry() {
            None | Some(Expiry::Default) => settings().session_cookie_age,
            Some(Expiry::Age(a)) => a,
            Some(Expiry::BrowserClose) => settings().session_cookie_age,
            Some(Expiry::At(d)) => (d - rango_core::utils::timezone::now()).num_seconds(),
        }
    }

    /// `get_expiry_date()`.
    pub fn get_expiry_date(&self) -> DateTime<Utc> {
        match self.effective_expiry() {
            Some(Expiry::At(d)) => d,
            _ => rango_core::utils::timezone::now() + Duration::seconds(self.get_expiry_age()),
        }
    }

    /// `get_expire_at_browser_close()`.
    pub fn get_expire_at_browser_close(&self) -> bool {
        match self.effective_expiry() {
            None | Some(Expiry::Default) => settings().session_expire_at_browser_close,
            Some(Expiry::BrowserClose) => true,
            _ => false,
        }
    }

    fn persist_expiry(&self) {
        let pending = self.lock().expiry.take();
        if let Some(e) = pending {
            let mut s = self.lock();
            match e {
                Expiry::Age(a) => s.data.insert("_session_expiry".into(), Json::from(a)),
                Expiry::At(d) => s.data.insert("_session_expiry".into(), Json::from(d.to_rfc3339())),
                Expiry::BrowserClose => s.data.insert("_session_expiry".into(), Json::from(0)),
                Expiry::Default => s.data.shift_remove("_session_expiry"),
            };
        }
    }

    // ---- persistence

    /// `save()`: write to the backend (creating a key when there is none).
    pub async fn save(&self) -> Result<()> {
        self.persist_expiry();
        let (key, data) = {
            let s = self.lock();
            (s.key.clone(), s.data.clone())
        };
        let age = self.get_expiry_age();
        match key {
            None => {
                let key = self.backend.create(&data, age).await?;
                self.lock().key = Some(key);
            }
            Some(k) => {
                if let Some(nk) = self.backend.save(&k, &data, age, false).await? {
                    self.lock().key = Some(nk);
                }
            }
        }
        self.lock().modified = false;
        Ok(())
    }

    /// `flush()`: delete the data and the key.
    pub async fn flush(&self) -> Result<()> {
        let key = {
            let mut s = self.lock();
            s.data.clear();
            s.accessed = true;
            s.modified = true;
            s.expiry = None;
            s.key.take()
        };
        if let Some(k) = key {
            self.backend.delete(&k).await?;
        }
        Ok(())
    }

    /// `cycle_key()`: new key, same data.
    pub async fn cycle_key(&self) -> Result<()> {
        let (old, data) = {
            let s = self.lock();
            (s.key.clone(), s.data.clone())
        };
        self.lock().key = None;
        self.save().await?;
        if let Some(o) = old {
            self.backend.delete(&o).await?;
        }
        self.lock().data = data;
        self.lock().modified = true;
        Ok(())
    }

    /// `exists(key)`.
    pub async fn exists(&self, key: &str) -> Result<bool> {
        self.backend.exists(key).await
    }

    pub(crate) fn invalid_key(&self) -> bool {
        self.lock().invalid_key
    }
}

/// `SessionBase.encode` / `decode`: signed, compressed JSON (same format as the default serializer).
pub fn encode(data: &IndexMap<String, Json>, key_salt: &str) -> Result<String> {
    signing::dumps(data, None, key_salt, true)
}

pub fn decode(encoded: &str, key_salt: &str) -> Option<IndexMap<String, Json>> {
    match signing::loads::<IndexMap<String, Json>>(encoded, None, key_salt, None, None) {
        Ok(Ok(d)) => Some(d),
        _ => None,
    }
}

pub fn new_session_key() -> String {
    get_random_string(32, VALID_KEY_CHARS)
}

/// `request.session` accessor.
pub trait RequestSessionExt {
    /// The request's session (panics if `SessionMiddleware` is not installed's `AttributeError`).
    fn session(&self) -> Session;
    fn try_session(&self) -> Option<Session>;
}

impl RequestSessionExt for HttpRequest {
    fn session(&self) -> Session {
        self.try_session().expect("The authentication/session features need 'rango.contrib.sessions.middleware.SessionMiddleware' in MIDDLEWARE")
    }
    fn try_session(&self) -> Option<Session> {
        self.ext::<Session>().map(|s| (*s).clone())
    }
}

pub(crate) fn bad_session(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string().into())
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.sessions"));
rango_db::register_initial_migration!("sessions", []);
