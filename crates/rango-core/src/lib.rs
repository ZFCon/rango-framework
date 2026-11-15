//! Rango core: the framework runtime shared by every other crate.
//!
//! Module names are: `conf`, `core::signing` → [`signing`], `core::exceptions` →
//! [`exceptions`], `utils`, and so on.

pub mod apps;
pub mod cache;
pub mod checks;
pub mod conf;
pub mod core_signals;
pub mod dispatch;
pub mod exceptions;
pub mod files;
pub mod mail;
pub mod management;
pub mod paginator;
pub mod security_checks;
pub mod signing;
pub mod utils;
pub mod validators;

pub use exceptions::{Error, Result};

/// `setup()`: populate the app registry from `INSTALLED_APPS` (and run app `ready()` hooks).
pub async fn setup() -> Result<()> {
    let installed = conf::try_settings()?.installed_apps.clone();
    apps::populate(&installed).await
}

#[doc(hidden)]
pub use inventory;
