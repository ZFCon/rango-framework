//! `contrib.sitemaps`: sitemap and sitemap-index XML (rendered with the bundled templates).

use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rango_core::exceptions::{Error, Result};
use rango_core::paginator::Paginator;
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, Engine, Value as T};

static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut e = Engine::new();
    e.locmem(&[("sitemap.xml", include_str!("../../templates/sitemaps/sitemap.xml")), ("sitemap_index.xml", include_str!("../../templates/sitemaps/sitemap_index.xml"))]);
    e
});

/// One `<url>` entry.
#[derive(Debug, Clone, Default)]
pub struct SitemapUrl {
    /// Absolute URL.
    pub location: String,
    pub lastmod: Option<DateTime<Utc>>,
    pub changefreq: Option<String>,
    pub priority: Option<f64>,
}

/// `Sitemap`: a section of the site map.
#[async_trait]
pub trait Sitemap: Send + Sync {
    /// Absolute-path locations (`/blog/1/`) with optional metadata.
    async fn items(&self) -> Result<Vec<SitemapUrl>>;
    fn protocol(&self) -> Option<&str> {
        None
    }
    fn limit(&self) -> usize {
        50_000
    }
}

fn render(template: &str, key: &str, list: Vec<T>) -> Result<String> {
    let mut ctx = Context::new();
    ctx.set(key, T::List(list));
    Ok(ENGINE.render_to_string(template, &mut ctx)?)
}

fn dt(d: DateTime<Utc>) -> T {
    T::from(d)
}

/// Render `<urlset>` for already-absolute `urls`.
pub fn sitemap_xml(urls: &[SitemapUrl]) -> Result<String> {
    let list = urls
        .iter()
        .map(|u| {
            T::Map(
                [
                    ("location", T::Str(u.location.clone())),
                    ("lastmod", u.lastmod.map_or(T::None, dt)),
                    ("changefreq", u.changefreq.clone().map_or(T::None, T::Str)),
                    ("priority", T::Str(u.priority.map_or_else(String::new, |p| T::Float(p).to_str()))),
                    ("alternates", T::List(vec![])),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            )
        })
        .collect();
    render("sitemap.xml", "urlset", list)
}

/// Render `<sitemapindex>` for `(location, last_mod)` entries.
pub fn sitemap_index_xml(entries: &[(String, Option<DateTime<Utc>>)]) -> Result<String> {
    let list = entries.iter().map(|(l, m)| T::Map([("location".to_owned(), T::Str(l.clone())), ("last_mod".to_owned(), m.map_or(T::None, dt))].into_iter().collect())).collect();
    render("sitemap_index.xml", "sitemaps", list)
}

/// A named sitemap section.
pub type Sections = [(&'static str, Arc<dyn Sitemap>)];

async fn absolute(request: &HttpRequest, s: &dyn Sitemap, items: Vec<SitemapUrl>) -> Result<Vec<SitemapUrl>> {
    let host = request.get_host()?;
    let proto = s.protocol().map_or_else(|| request.scheme(), str::to_owned);
    Ok(items.into_iter().map(|mut u| {
        if u.location.starts_with('/') {
            u.location = format!("{proto}://{host}{}", u.location);
        }
        u
    }).collect())
}

/// `sitemaps.views.sitemap(request, sitemaps, section=None, page)`.
pub async fn sitemap(request: &HttpRequest, sections: &Sections, section: Option<&str>, page: usize) -> Result<HttpResponse> {
    let chosen: Vec<&(&str, Arc<dyn Sitemap>)> = match section {
        Some(s) => vec![sections.iter().find(|(n, _)| *n == s).ok_or_else(|| Error::Http404(format!("No sitemap available for section: '{s}'")))?],
        None => sections.iter().collect(),
    };
    let mut urls = Vec::new();
    for (_, s) in chosen {
        let items = absolute(request, &**s, s.items().await?).await?;
        let paginator = Paginator::new(items, s.limit());
        urls.extend(paginator.page(page as i64).await?.map_err(|_| Error::Http404(format!("Page {page} empty")))?.object_list);
    }
    Ok(HttpResponse::new(sitemap_xml(&urls)?).content_type("application/xml"))
}

/// `sitemaps.views.index(request, sitemaps, sitemap_url_name)`: one entry per section page.
pub async fn index(request: &HttpRequest, sections: &Sections, section_url: impl Fn(&str, usize) -> Result<String>) -> Result<HttpResponse> {
    let host = request.get_host()?;
    let mut entries = Vec::new();
    for (name, s) in sections {
        let items = s.items().await?;
        let pages = Paginator::new(items.clone(), s.limit()).num_pages().await?;
        let last = items.iter().map(|i| i.lastmod).collect::<Option<Vec<_>>>().and_then(|l| l.into_iter().max());
        let proto = s.protocol().map_or_else(|| request.scheme(), str::to_owned);
        for p in 1..=pages.max(1) {
            entries.push((format!("{proto}://{host}{}", section_url(name, p)?), last));
        }
    }
    Ok(HttpResponse::new(sitemap_index_xml(&entries)?).content_type("application/xml"))
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.sitemaps"));
