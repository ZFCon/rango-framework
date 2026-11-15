//! `middleware.gzip.GZipMiddleware`.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use rango_core::exceptions::Result;
use rango_core::utils::text::compress_string;
use rango_http::cache_utils::patch_vary_headers;
use rango_http::{Body, HttpRequest, HttpResponse};

use crate::handlers::Middleware;

#[derive(Default)]
pub struct GZipMiddleware;

fn accepts_gzip(header: &str) -> bool {
    // the reference implementation: re.compile(r"\bgzip\b").search(header)
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\bgzip\b").expect("valid regex"));
    RE.is_match(header)
}

#[async_trait]
impl Middleware for GZipMiddleware {
    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if !response.is_streaming() && response.content()?.len() < 200 {
            return Ok(response);
        }
        patch_vary_headers(&mut response, &["Accept-Encoding"]);
        if !accepts_gzip(request.meta().get("HTTP_ACCEPT_ENCODING").map_or("", String::as_str)) || response.has_header("Content-Encoding") {
            return Ok(response);
        }
        if response.is_streaming() {
            // Compress chunk by chunk (buffered per chunk; each chunk is a complete gzip member).
            let body = std::mem::replace(response.body_mut(), Body::Bytes(Bytes::new()));
            if let Body::Stream(stream) = body {
                let mapped = stream.map(|c| c.map(|b| Bytes::from(compress_string(&b, None))));
                *response.body_mut() = Body::Stream(mapped.boxed());
            }
            response.headers.remove("Content-Length");
        } else {
            let compressed = compress_string(response.content()?, Some(100));
            if compressed.len() >= response.content()?.len() {
                return Ok(response);
            }
            let len = compressed.len();
            response.set_content(compressed);
            response.set_header("Content-Length", &len.to_string())?;
            if let Some(etag) = response.header("ETag").map(str::to_owned) {
                if etag.starts_with('"') {
                    response.set_header("ETag", &format!("W/{etag}"))?;
                }
            }
        }
        response.set_header("Content-Encoding", "gzip")?;
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.gzip.GZipMiddleware", || Ok(Arc::new(GZipMiddleware)));
