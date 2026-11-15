use chrono::{TimeZone, Utc};
use rango_contrib::syndication::{Enclosure, Feed, FeedItem, FeedKind};

#[test]
fn feeds_match_rango() {
    let golden: serde_json::Value = serde_json::from_str(include_str!("golden/feeds.json")).unwrap();
    for (name, kind) in [("rss", FeedKind::Rss201rev2), ("atom", FeedKind::Atom1)] {
        let mut f = Feed::new(kind, "My <blog>", "http://example.com/", "All & more");
        f.language = Some("en".into());
        f.author_name = Some("Ann".into());
        f.author_email = Some("ann@example.com".into());
        f.author_link = Some("http://example.com/ann".into());
        f.subtitle = Some("sub".into());
        f.categories = vec!["a".into(), "b".into()];
        f.feed_url = Some("http://example.com/feed/".into());
        f.feed_copyright = Some("(c) me".into());
        f.ttl = Some("30".into());
        f.add_item(FeedItem {
            title: "First".into(),
            link: "http://example.com/1/#frag".into(),
            description: Some("Body <b>one</b>".into()),
            author_name: Some("Bob".into()),
            author_email: Some("bob@example.com".into()),
            pubdate: Some(Utc.with_ymd_and_hms(2024, 1, 5, 12, 0, 0).unwrap()),
            updateddate: Some(Utc.with_ymd_and_hms(2024, 2, 6, 7, 8, 9).unwrap()),
            comments: Some("http://example.com/1/#c".into()),
            unique_id: Some("id-1".into()),
            unique_id_is_permalink: Some(false),
            categories: vec!["x".into()],
            item_copyright: Some("(c) item".into()),
            enclosures: vec![Enclosure { url: "http://example.com/a.mp3".into(), length: "1234".into(), mime_type: "audio/mpeg".into() }],
            ..FeedItem::default()
        });
        f.add_item(FeedItem { title: "Second".into(), link: "http://example.com/2/".into(), author_name: Some("Cy".into()), pubdate: Some(Utc.with_ymd_and_hms(2024, 3, 1, 0, 0, 0).unwrap()), ..FeedItem::default() });
        if std::env::var("DUMP_FEEDS").is_ok() {
            std::fs::write(format!("{}/feed_{name}.xml", env!("CARGO_TARGET_TMPDIR")), f.write_string()).unwrap();
        }
        assert_eq!(f.write_string(), golden[name].as_str().unwrap(), "{name}");
    }
}
