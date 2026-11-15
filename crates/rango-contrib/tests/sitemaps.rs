use chrono::{TimeZone, Utc};
use rango_contrib::sitemaps::{SitemapUrl, sitemap_index_xml, sitemap_xml};
use rango_core::conf::{self, Settings};

#[test]
fn sitemap_xml_matches_rango() {
    conf::configure(Settings { secret_key: "x".into(), use_tz: true, ..Settings::default() }).unwrap();
    let golden: serde_json::Value = serde_json::from_str(include_str!("golden/sitemaps.json")).unwrap();
    let urls = vec![
        SitemapUrl { location: "http://example.com/a/".into(), lastmod: Some(Utc.with_ymd_and_hms(2024, 1, 5, 12, 0, 0).unwrap()), changefreq: Some("daily".into()), priority: Some(0.5) },
        SitemapUrl { location: "http://example.com/b/?x=1&y=2".into(), ..SitemapUrl::default() },
    ];
    assert_eq!(sitemap_xml(&urls).unwrap(), golden["sitemap"].as_str().unwrap());
    let idx = vec![("http://example.com/sitemap-news.xml".to_owned(), Some(Utc.with_ymd_and_hms(2024, 3, 9, 8, 30, 0).unwrap())), ("http://example.com/sitemap-x.xml?p=2".to_owned(), None)];
    assert_eq!(sitemap_index_xml(&idx).unwrap(), golden["index"].as_str().unwrap());
}
