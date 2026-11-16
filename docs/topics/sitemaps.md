# Sitemaps

`rango::contrib::sitemaps` renders `sitemap.xml` documents and sitemap indexes from sections you define. You implement the `Sitemap` trait for each section of your site and call `sitemap` or `index` from a view.

## Enable the app

```rust
use rango::conf::Settings;

let settings = Settings {
    installed_apps: vec!["rango.contrib.sitemaps".into()],
    ..Settings::default()
};
```

The app registers the bundled XML templates itself; it has no models and needs no migration.

## Define a section

A section implements `Sitemap`. Only `items` is required.

```rust
use async_trait::async_trait;
use rango::contrib::sitemaps::{Sitemap, SitemapUrl};
use rango::Result;

struct StaticPages;

#[async_trait]
impl Sitemap for StaticPages {
    async fn items(&self) -> Result<Vec<SitemapUrl>> {
        Ok(vec![
            SitemapUrl { location: "/".into(), priority: Some(1.0), ..SitemapUrl::default() },
            SitemapUrl { location: "/about/".into(), changefreq: Some("monthly".into()), ..SitemapUrl::default() },
        ])
    }
}
```

| Item | Default | Meaning |
|---|---|---|
| `async fn items(&self) -> Result<Vec<SitemapUrl>>` | required | The entries of the section. Query the database here; `items` is async and may fail. |
| `fn protocol(&self) -> Option<&str>` | `None` | Scheme used to build absolute URLs. `None` uses the request's scheme. |
| `fn limit(&self) -> usize` | `50_000` | Maximum entries per page. Larger sections are split across pages. |

`SitemapUrl` fields:

| Field | Type | Rendered as |
|---|---|---|
| `location` | `String` | `<loc>`. A value starting with `/` is made absolute with the request's scheme and host; any other value is used as given. |
| `lastmod` | `Option<DateTime<Utc>>` | `<lastmod>` formatted `YYYY-MM-DD`. Omitted when `None`. |
| `changefreq` | `Option<String>` | `<changefreq>`. The value is not validated. |
| `priority` | `Option<f64>` | `<priority>`. Omitted when `None`. |

## Serve a sitemap

`sitemap` is a plain async function, not a view. Call it from your own handler and map the URL yourself.

```rust
use std::sync::Arc;
use rango::contrib::sitemaps::{sitemap, Sitemap};
use rango::http::{HttpRequest, HttpResponse};
use rango::urls::path;
use rango::{register_urlconf, urlpatterns, Result};

fn sections() -> Vec<(&'static str, Arc<dyn Sitemap>)> {
    vec![("static", Arc::new(StaticPages))]
}

async fn sitemap_view(req: HttpRequest) -> Result<HttpResponse> {
    let page = req.query().get("p").and_then(|p| p.parse().ok()).unwrap_or(1);
    sitemap(&req, &sections(), None, page).await
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("sitemap.xml", sitemap_view),
]);
```

Signature: `sitemap(request, sections: &Sections, section: Option<&str>, page: usize) -> Result<HttpResponse>`.

- `section: None` merges every section into one `<urlset>`. `Some("static")` renders only that section.
- An unknown section name returns `Error::Http404("No sitemap available for section: '...'")`.
- `page` is 1-based. A page past the end returns a 404.
- The response has content type `application/xml`.
- The host comes from `request.get_host()`, so `allowed_hosts` applies.

## Sitemap index with several files

When you have several sections, or a section larger than its `limit`, serve an index that links to one file per section page. `index` takes a closure that builds the path for a given section name and page number.

```rust
use rango::contrib::sitemaps::index;

async fn index_view(req: HttpRequest) -> Result<HttpResponse> {
    index(&req, &sections(), |name, page| {
        Ok(if page == 1 { format!("/sitemap-{name}.xml") } else { format!("/sitemap-{name}.xml?p={page}") })
    })
    .await
}

async fn section_view(req: HttpRequest, name: String) -> Result<HttpResponse> {
    let page = req.query().get("p").and_then(|p| p.parse().ok()).unwrap_or(1);
    sitemap(&req, &sections(), Some(&name), page).await
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("sitemap.xml", index_view),
    path("sitemap-<str:name>.xml", section_view),
]);
```

Each `<sitemap>` entry gets an absolute `<loc>`. Its `<lastmod>` is the newest `lastmod` of the section, and is left out if any item of that section has no `lastmod`. A section with no items still produces one entry (page 1).

## Render without a request

`sitemap_xml(&[SitemapUrl]) -> Result<String>` and `sitemap_index_xml(&[(String, Option<DateTime<Utc>>)]) -> Result<String>` render the XML directly. Locations are used exactly as given, so pass absolute URLs. Use them for static generation or tests.

```rust
use rango::contrib::sitemaps::sitemap_xml;

let xml = sitemap_xml(&[SitemapUrl { location: "https://example.com/a/".into(), ..SitemapUrl::default() }])?;
```

## Not supported yet

- Per-item `alternates` (`xhtml:link` language variants) are always rendered empty.
- No built-in `Sitemap` for models, no generic sitemap, and no ping command. Write `items` yourself.
- No per-item protocol; `protocol` applies to a whole section.

## See also

- [Sites, flat pages and redirects](sites-flatpages-redirects.md)
- [Syndication feeds](feeds.md)
- [URL dispatcher](http/urls.md)
- [Testing](testing.md)
