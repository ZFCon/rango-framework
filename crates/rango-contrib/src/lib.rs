//! Rango contrib apps (`contrib`).

pub mod admin;
pub mod auth;
pub mod contenttypes;
pub mod flatpages;
pub mod messages;
pub mod redirects;
pub mod sessions;
pub mod sitemaps;
pub mod syndication;
pub mod sites;
pub mod staticfiles;

/// Keep this crate's link-time registrations (apps, middleware, models, commands) alive.
#[doc(hidden)]
pub fn __link() {
    let _ = (
        sessions::middleware::SessionMiddleware::default as fn() -> _,
        auth::middleware::AuthenticationMiddleware::default as fn() -> _,
        messages::MessageMiddleware::default as fn() -> _,
        auth::commands::CreateSuperuser::default as fn() -> _,
        contenttypes::clear_cache as fn(),
        admin::capfirst as fn(&str) -> String,
        redirects::RedirectFallbackMiddleware::default as fn() -> _,
        staticfiles::CollectStatic::default as fn() -> _,
        flatpages::FlatpageFallbackMiddleware::default as fn() -> _,
    );
}
