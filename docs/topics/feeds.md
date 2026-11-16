# Syndication feeds

`rango::contrib::syndication` builds RSS 2.0 and Atom 1.0 documents. You describe a channel with `Feed`, add `FeedItem`s, and return `write_string()` from a view.

## Build a feed

```rust
use chrono::{TimeZone, Utc};
use rango::contrib::syndication::{Feed, FeedItem, FeedKind};

let mut feed = Feed::new(FeedKind::Rss201rev2, "Polls", "https://example.com/", "Latest polls");
feed.language = Some("en".into());
feed.feed_url = Some("https://example.com/feed/".into());
feed.add_item(FeedItem {
    title: "What's up?".into(),
    link: "https://example.com/polls/1/".into(),
    description: Some("<p>Vote now.</p>".into()),
    pubdate: Some(Utc.with_ymd_and_hms(2024, 1, 5, 12, 0, 0).unwrap()),
    unique_id: Some("poll-1".into()),
    unique_id_is_permalink: Some(false),
    ..FeedItem::default()
});
let xml = feed.write_string();
```

`Feed::new(kind, title, link, description)` takes the required channel fields. `FeedKind::Rss201rev2` produces RSS 2.0; `FeedKind::Atom1` produces Atom 1.0. The `kind` field can be changed later to render the same data in the other format.

## Serve it

```rust
use rango::http::{HttpRequest, HttpResponse};
use rango::Result;

async fn latest(_req: HttpRequest) -> Result<HttpResponse> {
    let feed = build_feed().await?; // your function returning a Feed
    Ok(HttpResponse::new(feed.write_string()).content_type(feed.content_type()))
}
```

`content_type()` returns `application/rss+xml; charset=utf-8` or `application/atom+xml; charset=utf-8`. Output starts with an XML declaration (`encoding="utf-8"`). Text and attribute values are escaped for you; do not pre-escape titles. A `description` that contains HTML is escaped as text, as feed readers expect.

## Channel fields (`Feed`)

| Field | Type | RSS element | Atom element |
|---|---|---|---|
| `title`, `link`, `description` | `String` | `title`, `link`, `description` | `title`, `link rel=alternate`; `description` is not written |
| `language` | `Option<String>` | `language` | `xml:lang` on `feed` |
| `author_name`, `author_email`, `author_link` | `Option<String>` | not written | `author` (`name`, `email`, `uri`); only written when `author_name` is set |
| `subtitle` | `Option<String>` | not written | `subtitle` |
| `categories` | `Vec<String>` | `category` | `category term=` |
| `feed_url` | `Option<String>` | `atom:link rel=self` | `link rel=self` |
| `feed_copyright` | `Option<String>` | `copyright` | `rights` |
| `feed_guid` | `Option<String>` | not written | `id` (defaults to `link`) |
| `ttl` | `Option<String>` | `ttl` | not written |
| `items` | `Vec<FeedItem>` | `item` | `entry` |

The last-modified timestamp (`lastBuildDate` / `updated`) is the newest `updateddate` or `pubdate` among the items, or the current time for an empty feed. Dates are always UTC: RFC 2822 (`Fri, 05 Jan 2024 12:00:00 +0000`) in RSS, RFC 3339 (`2024-01-05T12:00:00+00:00`) in Atom.

## Item fields (`FeedItem`)

`FeedItem` implements `Default`, so set what you need and use `..FeedItem::default()`.

| Field | Type | Notes |
|---|---|---|
| `title`, `link` | `String` | Required in practice. |
| `description` | `Option<String>` | RSS `description`; Atom `summary type="html"`. |
| `author_name`, `author_email`, `author_link` | `Option<String>` | RSS: `author` is `email (name)`, or just the email; with only a name it is written as `dc:creator`. `author_link` is Atom only. Atom writes an `author` element only when `author_name` is set. |
| `pubdate` | `Option<DateTime<Utc>>` | RSS `pubDate`; Atom `published`. |
| `updateddate` | `Option<DateTime<Utc>>` | Atom `updated`. Not written in RSS. |
| `comments` | `Option<String>` | RSS `comments`. Not written in Atom. |
| `unique_id` | `Option<String>` | RSS `guid`; Atom `id`. |
| `unique_id_is_permalink` | `Option<bool>` | RSS `guid isPermaLink`. |
| `categories` | `Vec<String>` | `category`. |
| `item_copyright` | `Option<String>` | Atom `rights`. Not written in RSS. |
| `ttl` | `Option<String>` | RSS `ttl`. |
| `enclosures` | `Vec<Enclosure>` | RSS writes only the first; Atom writes one `link rel=enclosure` per enclosure. |

Atom requires an `id` per entry. When `unique_id` is `None`, one is generated as a tag URI from the link and publication date (`tag_uri(url, date)`), for example `tag:example.com,2024-01-05:/polls/1//frag`.

An attachment:

```rust
use rango::contrib::syndication::Enclosure;

let enclosure = Enclosure {
    url: "https://example.com/ep1.mp3".into(),
    length: "123456".into(),   // bytes, as a string
    mime_type: "audio/mpeg".into(),
};
```

## Feeds from the database

Feeds are plain data, so build them from a query in your handler. Item links must be absolute; build them from `request.get_host()` and `reverse!`.

```rust
use rango::contrib::syndication::{Feed, FeedItem, FeedKind};

async fn build_feed(req: &HttpRequest) -> Result<Feed> {
    let base = format!("{}://{}", req.scheme(), req.get_host()?);
    let db = rango::db::Db::default_db();
    let mut feed = Feed::new(FeedKind::Atom1, "Polls", &format!("{base}/"), "Latest polls");
    for q in Question::objects().order_by(&["-pub_date"]).fetch(&db).await?.into_iter().take(10) {
        feed.add_item(FeedItem {
            title: q.question_text.clone(),
            link: format!("{base}{}", rango::reverse!("polls:detail", q.id.unwrap())?),
            pubdate: Some(q.pub_date),
            ..FeedItem::default()
        });
    }
    Ok(feed)
}
```

(`Question` stands for your own model.)

## Helpers

- `rfc2822(DateTime<Utc>) -> String` and `rfc3339(DateTime<Utc>) -> String` format dates the way the feeds do.
- `tag_uri(url: &str, date: Option<DateTime<Utc>>) -> String`.

## Not supported yet

- There is no feed view class or URL helper: you write the handler and the query.
- Only the Atom and RSS 2.0 formats exist; no other feed formats or custom XML namespaces.
- `Feed` writes at most the fields listed above; there is no hook for extra elements.

## See also

- [Sitemaps](sitemaps.md)
- [Requests and responses](http/requests-responses.md)
- [Querying](db/queries.md)
