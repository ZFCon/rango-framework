//! `middleware.cache`: `UpdateCacheMiddleware`, `FetchFromCacheMiddleware`, `CacheMiddleware`
//! and the `cache_page` decorator.
//!
//! Cached responses are stored as JSON (`status`, `headers`, base64 `content`) under
//! `views.decorators.cache.cache_page.<prefix>.<METHOD>.<md5(url)>.<md5(vary values)>`.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use md5::{Digest, Md5};
use rango_core::cache::{Cache, Timeout, caches};
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_http::cache_utils::{get_max_age, has_vary_header, patch_response_headers};
use rango_http::{HttpRequest, HttpResponse};
use rango_urls::{IntoView, View};

use crate::handlers::Middleware;

fn md5hex(s: &str) -> String {
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Marks requests the update middleware should store the response of.
#[derive(Debug, Clone, Copy)]
struct CacheUpdate;

fn header_key(prefix: &str, url: &str) -> String {
    format!("views.decorators.cache.cache_header.{prefix}.{}", md5hex(url))
}

fn page_key(prefix: &str, url: &str, method: &str, vary: &[(String, String)]) -> String {
    let mut h = Md5::new();
    for (_, v) in vary {
        h.update(v.as_bytes());
    }
    let digest: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("views.decorators.cache.cache_page.{prefix}.{method}.{}.{digest}", md5hex(url))
}

fn vary_values(request: &HttpRequest, headers: &[String]) -> Vec<(String, String)> {
    headers.iter().map(|h| (h.clone(), request.meta().get(&format!("HTTP_{}", h.to_uppercase().replace('-', "_"))).cloned().unwrap_or_default())).collect()
}

/// `learn_cache_key`: remember which headers the response varies on.
async fn learn(cache: &Cache, request: &HttpRequest, vary_header: Option<String>, timeout: Timeout, prefix: &str) -> Result<String> {
    let url = request.build_absolute_uri(None)?;
    let vary: Vec<String> = vary_header.map(|v| v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty() && s != "*").collect()).unwrap_or_default();
    cache.set(&header_key(prefix, &url), &vary, timeout).await?;
    Ok(page_key(prefix, &url, request.method(), &vary_values(request, &vary)))
}

/// `FetchFromCacheMiddleware`.
pub struct FetchFromCacheMiddleware {
    pub alias: String,
    pub prefix: String,
}

impl Default for FetchFromCacheMiddleware {
    fn default() -> Self {
        let s = settings();
        Self { alias: s.cache_middleware_alias.clone(), prefix: s.cache_middleware_key_prefix.clone() }
    }
}

#[async_trait]
impl Middleware for FetchFromCacheMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        if !matches!(request.method(), "GET" | "HEAD") {
            return Ok(None);
        }
        let cache = caches(&self.alias)?;
        let url = request.build_absolute_uri(None)?;
        let Some(vary) = cache.get::<Vec<String>>(&header_key(&self.prefix, &url)).await? else {
            request.set_ext(CacheUpdate);
            return Ok(None);
        };
        let key = page_key(&self.prefix, &url, "GET", &vary_values(request, &vary));
        match cache.get::<serde_json::Value>(&key).await? {
            Some(v) => {
                let mut r = HttpResponse::new(base64::engine::general_purpose::STANDARD.decode(v["content"].as_str().unwrap_or("")).unwrap_or_default());
                r.set_status_code(v["status"].as_u64().unwrap_or(200) as u16)?;
                for (k, val) in v["headers"].as_array().into_iter().flatten().filter_map(|h| Some((h[0].as_str()?, h[1].as_str()?))) {
                    r.set_header(k, val)?;
                }
                Ok(Some(r))
            }
            None => {
                request.set_ext(CacheUpdate);
                Ok(None)
            }
        }
    }
}

/// `UpdateCacheMiddleware`.
pub struct UpdateCacheMiddleware {
    pub alias: String,
    pub prefix: String,
    pub seconds: i64,
}

impl Default for UpdateCacheMiddleware {
    fn default() -> Self {
        let s = settings();
        Self { alias: s.cache_middleware_alias.clone(), prefix: s.cache_middleware_key_prefix.clone(), seconds: s.cache_middleware_seconds }
    }
}

#[async_trait]
impl Middleware for UpdateCacheMiddleware {
    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if request.ext::<CacheUpdate>().is_none() || response.is_streaming() || !matches!(response.status_code, 200 | 304) {
            return Ok(response);
        }
        if !response.cookies.is_empty() && has_vary_header(&response, "Cookie") {
            return Ok(response);
        }
        if response.header("Cache-Control").is_some_and(|c| c.contains("private")) {
            return Ok(response);
        }
        let timeout = get_max_age(&response).unwrap_or(self.seconds);
        if timeout == 0 {
            return Ok(response);
        }
        patch_response_headers(&mut response, Some(timeout));
        if request.method() == "HEAD" {
            return Ok(response);
        }
        let payload = serde_json::json!({
            "status": response.status_code,
            "headers": response.headers.iter().map(|(k, v)| serde_json::json!([k, v])).collect::<Vec<_>>(),
            "content": base64::engine::general_purpose::STANDARD.encode(response.content()?),
        });
        let vary_header = response.header("Vary").map(str::to_owned);
        let cache = caches(&self.alias)?;
        let t = Timeout::Secs(timeout as f64);
        let key = learn(&cache, request, vary_header, t, &self.prefix).await?;
        cache.set(&key, &payload, t).await?;
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.cache.UpdateCacheMiddleware", || Ok(Arc::new(UpdateCacheMiddleware::default())));
crate::register_middleware!("rango.middleware.cache.FetchFromCacheMiddleware", || Ok(Arc::new(FetchFromCacheMiddleware::default())));

/// `CacheMiddleware`: both halves in one entry (put it last... or alone, as the reference implementation documents).
#[derive(Default)]
pub struct CacheMiddleware {
    update: UpdateCacheMiddleware,
    fetch: FetchFromCacheMiddleware,
}

#[async_trait]
impl Middleware for CacheMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        self.fetch.process_request(request).await
    }
    async fn process_response(&self, request: &HttpRequest, response: HttpResponse) -> Result<HttpResponse> {
        self.update.process_response(request, response).await
    }
}

crate::register_middleware!("rango.middleware.cache.CacheMiddleware", || Ok(Arc::new(CacheMiddleware::default())));

/// `@cache_page(timeout)`: cache one view's responses.
pub fn cache_page<M>(timeout: i64, view: impl IntoView<M>) -> View {
    let mw = Arc::new(CacheMiddleware { update: UpdateCacheMiddleware { seconds: timeout, ..UpdateCacheMiddleware::default() }, fetch: FetchFromCacheMiddleware::default() });
    super::csrf::decorator_from_middleware(mw, view)
}
