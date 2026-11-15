//! Migrations (`db.migrations`).

pub mod autodetector;
pub mod commands;
pub mod executor;
pub mod operations;
pub mod schema;
pub mod state;

pub use executor::{Migration, MigrationLoader, Target};
pub use operations::Operation;

/// `0001_initial` for a bundled (contrib) app, derived from its registered models.
///
/// Contrib apps ship no migration files; their initial schema is whatever the models are.
pub fn initial_for_app(app_label: &str, dependencies: &[(&str, &str)]) -> Migration {
    let reg = crate::models::meta::registry();
    let to = state::ProjectState::from_registry(&reg, Some(&[app_label.to_owned()]));
    let ops = autodetector::changes(&state::ProjectState::new(), &to).ok().and_then(|mut m| m.shift_remove(app_label)).unwrap_or_default();
    let mut m = Migration::new(app_label, "0001_initial").initial();
    for (a, n) in dependencies {
        m = m.dependency(a, n);
    }
    m.operations = ops;
    m
}

/// `register_initial_migration!("sessions", [("contenttypes", "0001_initial")])`: bundled apps' `0001_initial`.
#[macro_export]
macro_rules! register_initial_migration {
    ($app:expr, [$(($a:expr, $n:expr)),* $(,)?]) => {
        $crate::inventory::submit! {
            $crate::migrations::executor::MigrationRegistration { build: || $crate::migrations::initial_for_app($app, &[$(($a, $n)),*]) }
        }
    };
}
