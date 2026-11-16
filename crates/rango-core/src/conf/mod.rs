//! `conf`: global settings, `settings.configure()`, `override_settings`.
//!
//! There is no import-time magic: the project builds a typed [`Settings`] value (struct-update syntax over
//! `Settings::default()`) and hands it to [`configure`] once, early in `main`.
//!
//! ```
//! use rango_core::conf::{configure, settings, Settings};
//! configure(Settings { debug: true, secret_key: "s3cret".into(), ..Settings::default() }).unwrap();
//! assert!(settings().debug);
//! ```

mod global_settings;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, RwLock};

use serde::{Deserialize, Serialize};

pub use global_settings::Settings;

use crate::exceptions::{Error, Result};

/// Free-form setting values.
pub type Json = serde_json::Value;

pub(crate) fn vs<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

pub(crate) fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

/// One entry of `DATABASES`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    #[serde(rename = "ENGINE")]
    pub engine: String,
    #[serde(rename = "NAME")]
    pub name: String,
    #[serde(rename = "USER")]
    pub user: String,
    #[serde(rename = "PASSWORD")]
    pub password: String,
    #[serde(rename = "HOST")]
    pub host: String,
    #[serde(rename = "PORT")]
    pub port: String,
    #[serde(rename = "OPTIONS")]
    pub options: Json,
    #[serde(rename = "ATOMIC_REQUESTS")]
    pub atomic_requests: bool,
    #[serde(rename = "AUTOCOMMIT")]
    pub autocommit: bool,
    /// Seconds a connection may be reused; `None` = unlimited.
    #[serde(rename = "CONN_MAX_AGE")]
    pub conn_max_age: Option<u64>,
    #[serde(rename = "CONN_HEALTH_CHECKS")]
    pub conn_health_checks: bool,
    #[serde(rename = "DISABLE_SERVER_SIDE_CURSORS")]
    pub disable_server_side_cursors: bool,
    #[serde(rename = "TIME_ZONE")]
    pub time_zone: Option<String>,
    #[serde(rename = "TEST")]
    pub test: DatabaseTestConfig,
}

/// `DATABASES[alias]["TEST"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseTestConfig {
    #[serde(rename = "CHARSET")]
    pub charset: Option<String>,
    #[serde(rename = "COLLATION")]
    pub collation: Option<String>,
    #[serde(rename = "MIGRATE")]
    pub migrate: bool,
    #[serde(rename = "MIRROR")]
    pub mirror: Option<String>,
    #[serde(rename = "NAME")]
    pub name: Option<String>,
}

impl Default for DatabaseTestConfig {
    fn default() -> Self {
        Self {
            charset: None,
            collation: None,
            migrate: true,
            mirror: None,
            name: None,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            engine: "rango.db.backends.dummy".into(),
            name: String::new(),
            user: String::new(),
            password: String::new(),
            host: String::new(),
            port: String::new(),
            options: serde_json::json!({}),
            atomic_requests: false,
            autocommit: true,
            conn_max_age: Some(0),
            conn_health_checks: false,
            disable_server_side_cursors: false,
            time_zone: None,
            test: DatabaseTestConfig::default(),
        }
    }
}

static GLOBAL: RwLock<Option<Arc<Settings>>> = RwLock::new(None);

tokio::task_local! {
    static SCOPED: Arc<Settings>;
}

static DEFAULTS_JSON: LazyLock<Json> =
    LazyLock::new(|| serde_json::to_value(Settings::default()).expect("settings serialize"));

/// `settings.configure(...)`: install the process-wide settings. Errors if already configured.
pub fn configure(settings: Settings) -> Result<()> {
    let mut g = GLOBAL.write().expect("settings lock poisoned");
    if g.is_some() {
        return Err(Error::improperly_configured("Settings already configured."));
    }
    *g = Some(Arc::new(settings));
    Ok(())
}

/// `settings.configured`.
pub fn configured() -> bool {
    SCOPED.try_with(|_| ()).is_ok() || GLOBAL.read().expect("settings lock poisoned").is_some()
}

/// Current settings (innermost [`override_settings`] scope, else the configured settings).
pub fn try_settings() -> Result<Arc<Settings>> {
    if let Ok(s) = SCOPED.try_with(Arc::clone) {
        return Ok(s);
    }
    GLOBAL
        .read()
        .expect("settings lock poisoned")
        .clone()
        .ok_or_else(|| {
            Error::improperly_configured(
                "Requested settings, but settings are not configured. \
             You must call rango::conf::configure() before accessing settings.",
            )
        })
}

/// Current settings; panics with the `ImproperlyConfigured` message when unconfigured
/// (use [`try_settings`] to handle that).
pub fn settings() -> Arc<Settings> {
    try_settings().unwrap_or_else(|e| panic!("{e}"))
}

/// `@override_settings` for async code: `fut` (and everything it awaits on the same task) sees
/// the modified settings. Tasks spawned with `tokio::spawn` do not inherit the override.
pub async fn override_settings<F: Future>(modify: impl FnOnce(&mut Settings), fut: F) -> F::Output {
    SCOPED.scope(overridden(modify), fut).await
}

/// `override_settings` for synchronous code.
pub fn override_settings_sync<R>(
    modify: impl FnOnce(&mut Settings),
    body: impl FnOnce() -> R,
) -> R {
    SCOPED.sync_scope(overridden(modify), body)
}

fn overridden(modify: impl FnOnce(&mut Settings)) -> Arc<Settings> {
    let mut s = (*settings()).clone();
    modify(&mut s);
    Arc::new(s)
}

impl Settings {
    /// `settings.SECRET_KEY`, raising `ImproperlyConfigured` when empty (as the reference implementation does on access).
    pub fn secret_key(&self) -> Result<&str> {
        if self.secret_key.is_empty() {
            return Err(Error::improperly_configured(
                "The SECRET_KEY setting must not be empty.",
            ));
        }
        Ok(&self.secret_key)
    }

    /// `settings.is_overridden(NAME)`.
    ///
    /// Divergence: a typed struct cannot remember *assignment*, so this is "differs from the
    /// the reference implementation default" (explicitly setting a setting to its default reads as not overridden).
    pub fn is_overridden(&self, name: &str) -> bool {
        let now = serde_json::to_value(self).expect("settings serialize");
        now.get(name) != DEFAULTS_JSON.get(name)
    }

    /// `{NAME: value}` for every setting, as `diffsettings --all` sees them.
    pub fn to_map(&self) -> BTreeMap<String, Json> {
        match serde_json::to_value(self).expect("settings serialize") {
            Json::Object(m) => m.into_iter().collect(),
            _ => unreachable!("Settings serializes to an object"),
        }
    }

    /// the default value of `NAME`, if it is a known setting.
    pub fn default_of(name: &str) -> Option<&'static Json> {
        DEFAULTS_JSON.get(name)
    }
}

/// Reset the process-wide settings. Only for tests that need a pristine process state.
#[doc(hidden)]
pub fn __reset_for_tests() {
    *GLOBAL.write().expect("settings lock poisoned") = None;
}
