//! `middleware.locale.LocaleMiddleware`: pick the request's language from the
//! `rango_language` cookie or `Accept-Language`, activate it for the request, set `Content-Language`.
//!
//! `i18n_patterns` (language prefix in URLs) is not supported.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_core::utils::translation::{get_language, override_language};
use rango_http::cache_utils::patch_vary_headers;
use rango_http::{HttpRequest, HttpResponse};

use crate::handlers::{GetResponse, Middleware};

/// A supported language matching `code` (`en-GB` falls back to `en`), if any.
pub fn get_supported_language_variant(code: &str) -> Option<String> {
    let s = settings();
    let supported = |c: &str| s.languages.iter().any(|(l, _)| l.eq_ignore_ascii_case(c)) || s.language_code.eq_ignore_ascii_case(c) || rango_core::utils::translation::available_languages().iter().any(|l| l.eq_ignore_ascii_case(c));
    let code = code.to_lowercase().replace('_', "-");
    if supported(&code) {
        return Some(code);
    }
    let base = code.split('-').next().unwrap_or("").to_owned();
    if !base.is_empty() && supported(&base) {
        return Some(base);
    }
    s.languages.iter().map(|(l, _)| l.clone()).find(|l| l.starts_with(&format!("{base}-"))).map(|l| l.to_lowercase())
}

/// `parse_accept_lang_header`: `(language, quality)` sorted by quality, best first.
pub fn parse_accept_lang_header(header: &str) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = header
        .split(',')
        .filter_map(|part| {
            let mut it = part.trim().split(';');
            let lang = it.next()?.trim().to_owned();
            if lang.is_empty() {
                return None;
            }
            let q = it.next().and_then(|p| p.trim().strip_prefix("q=")).map_or(Some(1.0), |q| q.parse::<f64>().ok())?;
            Some((lang, q))
        })
        .collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v
}

/// `get_language_from_request(request)`.
pub fn get_language_from_request(request: &HttpRequest) -> String {
    let s = settings();
    if let Some(c) = request.cookies().get(&s.language_cookie_name).and_then(|c| get_supported_language_variant(c)) {
        return c;
    }
    if let Some(h) = request.meta().get("HTTP_ACCEPT_LANGUAGE") {
        for (lang, q) in parse_accept_lang_header(h) {
            if q > 0.0 {
                if lang == "*" {
                    break;
                }
                if let Some(l) = get_supported_language_variant(&lang) {
                    return l;
                }
            }
        }
    }
    s.language_code.to_lowercase()
}

#[derive(Default)]
pub struct LocaleMiddleware;

#[async_trait]
impl Middleware for LocaleMiddleware {
    async fn call(&self, request: HttpRequest, get_response: GetResponse) -> Result<HttpResponse> {
        let lang = get_language_from_request(&request);
        let mut response = override_language(&lang, get_response(request)).await;
        patch_vary_headers(&mut response, &["Accept-Language"]);
        if !response.has_header("Content-Language") {
            response.set_header("Content-Language", &lang)?;
        }
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.locale.LocaleMiddleware", || Ok(Arc::new(LocaleMiddleware)));

#[allow(dead_code)]
fn _l() -> String {
    get_language()
}
