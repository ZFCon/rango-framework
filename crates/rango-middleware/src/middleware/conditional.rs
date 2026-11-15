//! `middleware.http.ConditionalGetMiddleware`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::exceptions::Result;
use rango_core::utils::crypto::HashAlgorithm;
use rango_core::utils::http::{parse_http_date_safe, quote_etag};
use rango_http::cache_utils::get_conditional_response;
use rango_http::{HttpRequest, HttpResponse};

use crate::handlers::Middleware;

#[derive(Default)]
pub struct ConditionalGetMiddleware;

#[async_trait]
impl Middleware for ConditionalGetMiddleware {
    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if request.method() != "GET" {
            return Ok(response);
        }
        let no_store = response.header("Cache-Control").is_some_and(|c| c.split(',').any(|h| h.trim().eq_ignore_ascii_case("no-store")));
        if !no_store && !response.has_header("ETag") && !response.is_streaming() {
            let digest: String = HashAlgorithm::Md5.digest(response.content()?).iter().map(|b| format!("{b:02x}")).collect();
            response.set_header("ETag", &quote_etag(&digest))?;
        }
        let etag = response.header("ETag").map(str::to_owned);
        let last_modified = response.header("Last-Modified").and_then(parse_http_date_safe);
        if etag.is_some() || last_modified.is_some() {
            return Ok(get_conditional_response(request, etag.as_deref(), last_modified, Some(response)).expect("response was provided"));
        }
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.http.ConditionalGetMiddleware", || Ok(Arc::new(ConditionalGetMiddleware)));
