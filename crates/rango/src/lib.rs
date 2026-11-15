//! Rango: the reference for Rust. Module paths follow the (`core.signing` is
//! `rango::core::signing`, `urls` is `rango::urls`, `test.Client` is
//! `rango::test::Client`, ...).

pub use rango_core::{apps, conf, dispatch, utils};
pub use rango_core::{Error, Result, format_html, register_app, register_check, register_command};
pub use rango_db as db;
pub use rango_forms as forms;
pub use rango_http as http;
/// `rest_framework` + `OpenAPI schema generation`.
pub use rango_rest as rest;
pub use rango_middleware::register_middleware;
pub use rango_template as template;
pub use rango_template::{context, register_context_processor, register_library};
pub use rango_urls as urls;
/// `serde_json::json!`, for `TEMPLATES`-style settings.
pub use serde_json::json;
pub use rango_urls::{register_urlconf, reverse, urlpatterns};

/// `core`.
pub mod core {
    pub use rango_core::core_signals as signals;
    pub use rango_core::{checks, exceptions, management, paginator, signing, validators};
    pub use rango_middleware::{handlers, servers};
}

/// `contrib`.
pub mod contrib {
    pub use rango_contrib::{admin, auth, sitemaps, syndication, contenttypes, flatpages, messages, redirects, sessions, sites, staticfiles};
}

/// `middleware`.
pub mod middleware {
    pub use rango_middleware::middleware::*;
}

/// `views`.
pub mod views {
    pub use rango_views::{debug, decorators, defaults, generic, static_serve};
}

/// `shortcuts`.
pub mod shortcuts {
    pub use rango_views::shortcuts::*;

    use rango_core::exceptions::{Error, Result};
    use rango_db::models::{Model, QuerySet};
    use rango_db::Executor;

    /// `get_object_or_404(queryset)`: `Http404` instead of `DoesNotExist`.
    pub async fn get_object_or_404<M: Model>(qs: QuerySet<M>, db: &dyn Executor) -> Result<M> {
        match qs.get(db).await {
            Err(Error::ObjectDoesNotExist { model, .. }) => {
                Err(Error::Http404(format!("No {} matches the given query.", model.unwrap_or_default())))
            }
            other => other,
        }
    }

    /// `get_list_or_404(queryset)`: `Http404` when empty.
    pub async fn get_list_or_404<M: Model>(qs: QuerySet<M>, db: &dyn Executor) -> Result<Vec<M>> {
        let list = qs.fetch(db).await?;
        if list.is_empty() {
            return Err(Error::Http404(format!("No {} matches the given query.", M::label())));
        }
        Ok(list)
    }
}

/// `test`.
pub mod test {
    pub use rango_test::*;
}

/// Framework version.
pub const VERSION: &str = rango_core::management::VERSION;

/// `setup()`: populate the app registry from `INSTALLED_APPS` (and run `ready()` hooks).
pub async fn setup() -> Result<()> {
    rango_middleware::__link();
    rango_cli::__link();
    rango_contrib::__link();
    rango_rest::__link();
    rango_db::migrations::commands::__link();
    rango_core::setup().await
}
