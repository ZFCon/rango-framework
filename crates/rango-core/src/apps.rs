//! `apps`: `AppConfig` and the app registry.
//!
//! finds apps by importing the dotted names in `INSTALLED_APPS`. Rust has no import
//! by name, so each app registers itself at link time with [`register_app!`] and
//! `INSTALLED_APPS` selects among the registered apps by `name`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use futures_util::future::BoxFuture;

use crate::exceptions::{Error, Result};

/// `AppConfig`.
#[derive(Clone)]
pub struct AppConfig {
    /// Full name, e.g. `"rango.contrib.auth"` or `"polls"`.
    pub name: String,
    /// Short unique name; defaults to the last component of `name`.
    pub label: String,
    /// Human-readable name; defaults to the title-cased label.
    pub verbose_name: String,
    /// Filesystem path of the app (templates, static, locale, migrations live below it).
    pub path: Option<PathBuf>,
    /// Overrides `DEFAULT_AUTO_FIELD` for this app's models.
    pub default_auto_field: Option<String>,
    /// `AppConfig.ready()`: runs once, after every app is registered.
    pub ready: Option<fn() -> BoxFuture<'static, Result<()>>>,
}

impl AppConfig {
    pub fn new(name: &str) -> Self {
        let label = name.rsplit('.').next().unwrap_or(name).to_owned();
        Self {
            name: name.to_owned(),
            verbose_name: title(&label),
            label,
            path: None,
            default_auto_field: None,
            ready: None,
        }
    }

    pub fn label(mut self, label: &str) -> Self {
        self.label = label.to_owned();
        self.verbose_name = title(label);
        self
    }

    pub fn verbose_name(mut self, verbose_name: &str) -> Self {
        self.verbose_name = verbose_name.to_owned();
        self
    }

    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn ready(mut self, ready: fn() -> BoxFuture<'static, Result<()>>) -> Self {
        self.ready = Some(ready);
        self
    }
}

/// `str.title()`.
fn title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_letter = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            if prev_letter {
                out.extend(c.to_lowercase())
            } else {
                out.extend(c.to_uppercase())
            }
            prev_letter = true;
        } else {
            out.push(c);
            prev_letter = false;
        }
    }
    out
}

/// Link-time registration of an app. Create with [`register_app!`].
pub struct AppRegistration {
    pub build: fn() -> AppConfig,
}

inventory::collect!(AppRegistration);

/// `register_app!(AppConfig::new("polls").path(env!("CARGO_MANIFEST_DIR")))`.
#[macro_export]
macro_rules! register_app {
    ($config:expr) => {
        $crate::inventory::submit! {
            $crate::apps::AppRegistration { build: || $config }
        }
    };
}

/// The app registry (`apps.apps`).
#[derive(Default)]
pub struct Apps {
    configs: Vec<Arc<AppConfig>>,
    by_label: HashMap<String, usize>,
}

impl Apps {
    /// `get_app_configs()`, in `INSTALLED_APPS` order.
    pub fn get_app_configs(&self) -> impl Iterator<Item = &Arc<AppConfig>> {
        self.configs.iter()
    }

    /// `get_app_config(label)`.
    pub fn get_app_config(&self, label: &str) -> Result<&Arc<AppConfig>> {
        self.by_label
            .get(label)
            .map(|&i| &self.configs[i])
            .ok_or_else(|| {
                let mut msg = format!("No installed app with label '{label}'.");
                if let Some(c) = self
                    .configs
                    .iter()
                    .find(|c| c.name.rsplit('.').next() == Some(label))
                {
                    msg.push_str(&format!(" Did you mean '{}'?", c.label));
                }
                Error::LookupError(msg)
            })
    }

    /// `is_installed(app_name)`.
    pub fn is_installed(&self, name: &str) -> bool {
        self.configs.iter().any(|c| c.name == name)
    }
}

static APPS: RwLock<Option<Arc<Apps>>> = RwLock::new(None);

/// `apps.populate(installed_apps)` followed by every `ready()` hook. Idempotent per process:
/// a second call is an error ("populate() isn't reentrant" only while running;
/// afterwards it is a no-op, which is mirrored here).
pub async fn populate(installed_apps: &[String]) -> Result<()> {
    if APPS.read().expect("apps lock poisoned").is_some() {
        return Ok(());
    }
    let registered: HashMap<String, AppConfig> = inventory::iter::<AppRegistration>
        .into_iter()
        .map(|r| (r.build)())
        .map(|c| (c.name.clone(), c))
        .collect();

    let mut apps = Apps::default();
    for entry in installed_apps {
        let config = registered.get(entry).cloned().ok_or_else(|| {
            Error::improperly_configured(format!(
                "Cannot find app '{entry}'. Register it with rango::register_app! and link its crate."
            ))
        })?;
        apps.configs.push(Arc::new(config));
    }

    for (what, key) in [("labels", 0), ("names", 1)] {
        let mut seen = HashSet::new();
        let mut dupes: Vec<&str> = apps
            .configs
            .iter()
            .map(|c| {
                if key == 0 {
                    c.label.as_str()
                } else {
                    c.name.as_str()
                }
            })
            .filter(|v| !seen.insert(*v))
            .collect();
        if !dupes.is_empty() {
            dupes.sort_unstable();
            dupes.dedup();
            return Err(Error::improperly_configured(format!(
                "Application {what} aren't unique, duplicates: {}",
                dupes.join(", ")
            )));
        }
    }
    apps.by_label = apps
        .configs
        .iter()
        .enumerate()
        .map(|(i, c)| (c.label.clone(), i))
        .collect();

    let apps = Arc::new(apps);
    *APPS.write().expect("apps lock poisoned") = Some(Arc::clone(&apps));
    for config in apps.get_app_configs() {
        if let Some(ready) = config.ready {
            ready().await?;
        }
    }
    Ok(())
}

/// The populated registry; `AppRegistryNotReady` before [`populate`].
pub fn apps() -> Result<Arc<Apps>> {
    APPS.read()
        .expect("apps lock poisoned")
        .clone()
        .ok_or_else(|| Error::AppRegistryNotReady("Apps aren't loaded yet.".into()))
}

/// Reset the registry. Only for tests that need a pristine process state.
#[doc(hidden)]
pub fn __reset_for_tests() {
    *APPS.write().expect("apps lock poisoned") = None;
}
