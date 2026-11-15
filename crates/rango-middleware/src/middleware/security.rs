//! `middleware.security.SecurityMiddleware`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_http::{HttpRequest, HttpResponse};
use regex::Regex;

use crate::handlers::Middleware;

#[derive(Default)]
pub struct SecurityMiddleware;

#[async_trait]
impl Middleware for SecurityMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        let s = settings();
        if s.secure_ssl_redirect && !request.is_secure() {
            let path = request.path().trim_start_matches('/');
            let exempt = s.secure_redirect_exempt.iter().any(|p| Regex::new(p).is_ok_and(|r| r.is_match(path)));
            if !exempt {
                let host = match &s.secure_ssl_host {
                    Some(h) => h.clone(),
                    None => request.get_host()?,
                };
                return Ok(Some(HttpResponse::permanent_redirect(&format!("https://{host}{}", request.get_full_path()))?));
            }
        }
        Ok(None)
    }

    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        let s = settings();
        if s.secure_hsts_seconds > 0 && request.is_secure() && !response.has_header("Strict-Transport-Security") {
            let mut v = format!("max-age={}", s.secure_hsts_seconds);
            if s.secure_hsts_include_subdomains {
                v.push_str("; includeSubDomains");
            }
            if s.secure_hsts_preload {
                v.push_str("; preload");
            }
            response.set_header("Strict-Transport-Security", &v)?;
        }
        if s.secure_content_type_nosniff {
            response.headers.set_default("X-Content-Type-Options", "nosniff");
        }
        if !s.secure_referrer_policy.is_empty() {
            response.headers.set_default("Referrer-Policy", &s.secure_referrer_policy);
        }
        if !s.secure_cross_origin_opener_policy.is_empty() {
            response.headers.set_default("Cross-Origin-Opener-Policy", &s.secure_cross_origin_opener_policy);
        }
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.security.SecurityMiddleware", || Ok(Arc::new(SecurityMiddleware)));
