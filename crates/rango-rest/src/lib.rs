//! Rango REST framework: REST framework building blocks (`rest_framework`) plus an
//! OpenAPI 3 schema generator and documentation views (`OpenAPI schema generation`).
//!
//! ```ignore
//! let mut router = Router::new();
//! router.register("articles", ModelViewSet::new(ModelSerializer::<Article>::all()?).search_fields(&["title"]));
//! urlpatterns![
//!     path("api/", include(router.urls())),
//!     path("api/schema/", schema::schema_view()),
//!     path("api/docs/", schema::swagger_view("/api/schema/")),
//! ]
//! ```

pub mod auth;
pub mod authtoken;
pub mod exceptions;
pub mod pagination;
pub mod routers;
pub mod schema;
pub mod serializers;
pub mod settings;
pub mod status;
pub mod views;
pub mod viewsets;

pub use auth::{Authentication, Permission, Throttle};
pub use exceptions::ApiException;
pub use pagination::Pagination;
pub use routers::Router;
pub use serializers::{Field, ModelSerializer, Serializer};
pub use views::{ApiRequest, Response, ViewConfig, api_view};
pub use viewsets::{ModelViewSet, ViewSet};

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.rest"));

/// Keep this crate's link-time registrations alive.
#[doc(hidden)]
pub fn __link() {
    let _ = (schema::SpectacularCommand::default as fn() -> _, authtoken::obtain_auth_token as fn() -> _);
}
