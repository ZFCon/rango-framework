//! Per-site cache middleware.

use std::sync::atomic::{AtomicUsize, Ordering};

use rango::conf::{self, Settings};
use rango::http::{HttpRequest, HttpResponse};
use rango::test::Client;
use rango::urls::path;
use rango::{Result, register_urlconf, urlpatterns};

static HITS: AtomicUsize = AtomicUsize::new(0);

async fn counter(_: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new(format!("hit {}", HITS.fetch_add(1, Ordering::SeqCst) + 1)))
}

register_urlconf!("cache.urls", || urlpatterns![path("c/", counter)]);

#[tokio::test]
async fn cache_middleware() {
    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("cache.urls".into()),
        middleware: ["UpdateCacheMiddleware", "FetchFromCacheMiddleware"].map(|m| format!("rango.middleware.cache.{m}")).to_vec(),
        cache_middleware_seconds: 60,
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();
    let c = Client::new().unwrap();
    assert_eq!(c.get("/c/").await.unwrap().text(), "hit 1");
    let second = c.get("/c/").await.unwrap();
    assert_eq!(second.text(), "hit 1", "served from cache");
    assert!(second.header("Cache-Control").is_some_and(|v| v.contains("max-age=60")));
    assert_eq!(c.get("/c/?x=1").await.unwrap().text(), "hit 2", "the query string is part of the key");
    assert_eq!(c.post("/c/", &[]).await.unwrap().text(), "hit 3", "POST is never cached");
}
