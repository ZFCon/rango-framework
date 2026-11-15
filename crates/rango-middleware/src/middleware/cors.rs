//! `CorsMiddleware`: CORS response headers and
//! preflight handling, driven by the `CORS_*` settings.
//!
//! Not ported: the `check_request_enabled` signal (use `CORS_URLS_REGEX` / origin regexes).

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_core::utils::http::urlsplit;
use rango_http::cache_utils::patch_vary_headers;
use rango_http::{HttpRequest, HttpResponse};
use regex::Regex;

use crate::handlers::{GetResponse, Middleware};

/// Python `re.match`: anchored at the start only.
fn re_match(pattern: &str, text: &str) -> bool {
    Regex::new(&format!("^(?:{pattern})")).is_ok_and(|r| r.is_match(text))
}

fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
    request.meta().get(&format!("HTTP_{}", name.to_uppercase().replace('-', "_"))).map(String::as_str)
}

fn is_enabled(request: &HttpRequest) -> bool {
    re_match(&settings().cors_urls_regex, request.path_info())
}

fn origin_allowed(origin: &str) -> bool {
    let s = settings();
    if origin == "null" && s.cors_allowed_origins.iter().any(|o| o == "null") {
        return true;
    }
    let Ok(url) = urlsplit(origin) else { return false };
    s.cors_allowed_origins.iter().filter_map(|o| urlsplit(o).ok()).any(|o| o.scheme == url.scheme && o.netloc == url.netloc) || s.cors_allowed_origin_regexes.iter().any(|p| re_match(p, origin))
}

/// `add_response_headers(request, response)`.
pub fn add_response_headers(request: &HttpRequest, response: &mut HttpResponse) -> Result<()> {
    if !is_enabled(request) {
        return Ok(());
    }
    patch_vary_headers(response, &["origin"]);
    let Some(origin) = header(request, "origin").filter(|o| !o.is_empty()) else { return Ok(()) };
    if urlsplit(origin).is_err() {
        return Ok(());
    }
    let s = settings();
    if !s.cors_allow_all_origins && !origin_allowed(origin) {
        return Ok(());
    }
    response.set_header("access-control-allow-origin", if s.cors_allow_all_origins && !s.cors_allow_credentials { "*" } else { origin })?;
    if s.cors_allow_credentials {
        response.set_header("access-control-allow-credentials", "true")?;
    }
    if !s.cors_expose_headers.is_empty() {
        response.set_header("access-control-expose-headers", &s.cors_expose_headers.join(", "))?;
    }
    if request.method() == "OPTIONS" {
        response.set_header("access-control-allow-headers", &s.cors_allow_headers.join(", "))?;
        response.set_header("access-control-allow-methods", &s.cors_allow_methods.join(", "))?;
        if s.cors_preflight_max_age != 0 {
            response.set_header("access-control-max-age", &s.cors_preflight_max_age.to_string())?;
        }
    }
    if s.cors_allow_private_network && header(request, "access-control-request-private-network") == Some("true") {
        response.set_header("access-control-allow-private-network", "true")?;
    }
    Ok(())
}

#[derive(Default)]
pub struct CorsMiddleware;

#[async_trait]
impl Middleware for CorsMiddleware {
    async fn call(&self, request: HttpRequest, get_response: GetResponse) -> Result<HttpResponse> {
        // Preflight requests are answered here and never reach the view.
        let preflight = is_enabled(&request) && request.method() == "OPTIONS" && header(&request, "access-control-request-method").is_some();
        let mut response = if preflight {
            let mut r = HttpResponse::new("");
            r.set_header("content-length", "0")?;
            r
        } else {
            get_response(request.clone()).await
        };
        add_response_headers(&request, &mut response)?;
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.cors.CorsMiddleware", || Ok(Arc::new(CorsMiddleware)));
