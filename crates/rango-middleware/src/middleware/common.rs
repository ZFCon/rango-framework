//! `middleware.common.CommonMiddleware`: forbidden user agents, `PREPEND_WWW`,
//! `APPEND_SLASH` redirects and `Content-Length`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_core::utils::http::escape_leading_slashes;
use rango_http::{HttpRequest, HttpResponse};
use rango_urls::{RequestUrlconf, valid_match};
use regex::Regex;

use crate::handlers::Middleware;

#[derive(Default)]
pub struct CommonMiddleware;

fn urlconf(request: &HttpRequest) -> Option<String> {
    request.ext::<RequestUrlconf>().map(|u| u.0.clone())
}

fn should_redirect_with_slash(request: &HttpRequest) -> bool {
    if settings().append_slash && !request.path_info().ends_with('/') {
        let conf = urlconf(request);
        if valid_match(request.path_info(), conf.as_deref()).is_none() {
            if let Some(m) = valid_match(&format!("{}/", request.path_info()), conf.as_deref()) {
                return !m.func.flags.extra.contains_key("no_append_slash");
            }
        }
    }
    false
}

fn get_full_path_with_slash(request: &HttpRequest) -> Result<String> {
    let new_path = escape_leading_slashes(&request.get_full_path_with_slash());
    if settings().debug && matches!(request.method(), "POST" | "PUT" | "PATCH") {
        return Err(Error::Other(
            format!(
                "You called this URL via {m}, but the URL doesn't end in a slash and you have APPEND_SLASH set. Rango can't redirect to the slash URL while maintaining {m} data. Change your form to point to {u} (note the trailing slash), or set APPEND_SLASH=False in your Rango settings.",
                m = request.method(),
                u = request.get_full_path_with_slash()
            )
            .into(),
        ));
    }
    Ok(new_path)
}

#[async_trait]
impl Middleware for CommonMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        let s = settings();
        if let Some(ua) = request.meta().get("HTTP_USER_AGENT") {
            for pattern in &s.disallowed_user_agents {
                if Regex::new(pattern).map_err(|e| Error::improperly_configured(e.to_string()))?.is_match(ua) {
                    return Err(Error::PermissionDenied("Forbidden user agent".into()));
                }
            }
        }
        let host = request.get_host()?;
        if s.prepend_www && !host.is_empty() && !host.starts_with("www.") {
            let redirect = format!("{}://www.{host}", request.scheme());
            let path = if should_redirect_with_slash(request) { get_full_path_with_slash(request)? } else { request.get_full_path() };
            return Ok(Some(HttpResponse::permanent_redirect(&format!("{redirect}{path}"))?));
        }
        Ok(None)
    }

    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if response.status_code == 404 && should_redirect_with_slash(request) {
            return HttpResponse::permanent_redirect(&get_full_path_with_slash(request)?);
        }
        if !response.is_streaming() && !response.has_header("Content-Length") {
            let len = response.content()?.len();
            response.set_header("Content-Length", &len.to_string())?;
        }
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.common.CommonMiddleware", || Ok(Arc::new(CommonMiddleware)));
