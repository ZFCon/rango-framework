//! `utils.feedgenerator` / `contrib.syndication`: RSS 2.0 and Atom 1.0 feeds.

use chrono::{DateTime, Utc};

/// An attached file (`Enclosure`).
#[derive(Debug, Clone)]
pub struct Enclosure {
    pub url: String,
    pub length: String,
    pub mime_type: String,
}

/// `add_item(...)` arguments.
#[derive(Debug, Clone, Default)]
pub struct FeedItem {
    pub title: String,
    pub link: String,
    pub description: Option<String>,
    pub author_email: Option<String>,
    pub author_name: Option<String>,
    pub author_link: Option<String>,
    pub pubdate: Option<DateTime<Utc>>,
    pub updateddate: Option<DateTime<Utc>>,
    pub comments: Option<String>,
    pub unique_id: Option<String>,
    pub unique_id_is_permalink: Option<bool>,
    pub categories: Vec<String>,
    pub item_copyright: Option<String>,
    pub ttl: Option<String>,
    pub enclosures: Vec<Enclosure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    /// `Rss201rev2Feed` (the `DefaultFeed`).
    Rss201rev2,
    Atom1,
}

/// `SyndicationFeed` with the metadata of the channel.
#[derive(Debug, Clone)]
pub struct Feed {
    pub kind: FeedKind,
    pub title: String,
    pub link: String,
    pub description: String,
    pub language: Option<String>,
    pub author_email: Option<String>,
    pub author_name: Option<String>,
    pub author_link: Option<String>,
    pub subtitle: Option<String>,
    pub categories: Vec<String>,
    pub feed_url: Option<String>,
    pub feed_copyright: Option<String>,
    pub feed_guid: Option<String>,
    pub ttl: Option<String>,
    pub items: Vec<FeedItem>,
}

impl Feed {
    pub fn new(kind: FeedKind, title: &str, link: &str, description: &str) -> Self {
        Feed { kind, title: title.into(), link: link.into(), description: description.into(), language: None, author_email: None, author_name: None, author_link: None, subtitle: None, categories: vec![], feed_url: None, feed_copyright: None, feed_guid: None, ttl: None, items: vec![] }
    }

    pub fn content_type(&self) -> &'static str {
        match self.kind {
            FeedKind::Rss201rev2 => "application/rss+xml; charset=utf-8",
            FeedKind::Atom1 => "application/atom+xml; charset=utf-8",
        }
    }

    pub fn add_item(&mut self, item: FeedItem) {
        self.items.push(item);
    }

    fn latest_post_date(&self) -> DateTime<Utc> {
        self.items.iter().flat_map(|i| [i.updateddate, i.pubdate]).flatten().max().unwrap_or_else(Utc::now)
    }

    /// `writeString("utf-8")`.
    pub fn write_string(&self) -> String {
        let mut x = Xml::default();
        x.out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        match self.kind {
            FeedKind::Rss201rev2 => self.write_rss(&mut x),
            FeedKind::Atom1 => self.write_atom(&mut x),
        }
        x.finish()
    }

    fn write_rss(&self, x: &mut Xml) {
        x.start("rss", &[("version", "2.0"), ("xmlns:atom", "http://www.w3.org/2005/Atom")]);
        x.start("channel", &[]);
        x.quick("title", Some(&self.title), &[]);
        x.quick("link", Some(&self.link), &[]);
        x.quick("description", Some(&self.description), &[]);
        if let Some(u) = &self.feed_url {
            x.quick("atom:link", None, &[("rel", "self"), ("href", u)]);
        }
        if let Some(l) = &self.language {
            x.quick("language", Some(l), &[]);
        }
        for c in &self.categories {
            x.quick("category", Some(c), &[]);
        }
        if let Some(c) = &self.feed_copyright {
            x.quick("copyright", Some(c), &[]);
        }
        x.quick("lastBuildDate", Some(&rfc2822(self.latest_post_date())), &[]);
        if let Some(t) = &self.ttl {
            x.quick("ttl", Some(t), &[]);
        }
        for i in &self.items {
            x.start("item", &[]);
            x.quick("title", Some(&i.title), &[]);
            x.quick("link", Some(&i.link), &[]);
            if let Some(d) = &i.description {
                x.quick("description", Some(d), &[]);
            }
            match (i.author_name.as_deref().filter(|s| !s.is_empty()), i.author_email.as_deref().filter(|s| !s.is_empty())) {
                (Some(n), Some(e)) => x.quick("author", Some(&format!("{e} ({n})")), &[]),
                (None, Some(e)) => x.quick("author", Some(e), &[]),
                (Some(n), None) => x.quick("dc:creator", Some(n), &[("xmlns:dc", "http://purl.org/dc/elements/1.1/")]),
                _ => {}
            }
            if let Some(p) = i.pubdate {
                x.quick("pubDate", Some(&rfc2822(p)), &[]);
            }
            if let Some(c) = &i.comments {
                x.quick("comments", Some(c), &[]);
            }
            if let Some(id) = &i.unique_id {
                let perm = i.unique_id_is_permalink.map(|b| b.to_string());
                match &perm {
                    Some(p) => x.quick("guid", Some(id), &[("isPermaLink", p)]),
                    None => x.quick("guid", Some(id), &[]),
                }
            }
            if let Some(t) = &i.ttl {
                x.quick("ttl", Some(t), &[]);
            }
            if let Some(e) = i.enclosures.first() {
                x.quick("enclosure", Some(""), &[("url", &e.url), ("length", &e.length), ("type", &e.mime_type)]);
            }
            for c in &i.categories {
                x.quick("category", Some(c), &[]);
            }
            x.end("item");
        }
        x.end("channel");
        x.end("rss");
    }

    fn write_atom(&self, x: &mut Xml) {
        let ns = "http://www.w3.org/2005/Atom";
        match &self.language {
            Some(l) => x.start("feed", &[("xmlns", ns), ("xml:lang", l)]),
            None => x.start("feed", &[("xmlns", ns)]),
        }
        x.quick("title", Some(&self.title), &[]);
        x.quick("link", Some(""), &[("rel", "alternate"), ("href", &self.link)]);
        if let Some(u) = &self.feed_url {
            x.quick("link", Some(""), &[("rel", "self"), ("href", u)]);
        }
        x.quick("id", Some(self.feed_guid.as_deref().unwrap_or(&self.link)), &[]);
        x.quick("updated", Some(&rfc3339(self.latest_post_date())), &[]);
        if let Some(n) = &self.author_name {
            x.start("author", &[]);
            x.quick("name", Some(n), &[]);
            if let Some(e) = &self.author_email {
                x.quick("email", Some(e), &[]);
            }
            if let Some(l) = &self.author_link {
                x.quick("uri", Some(l), &[]);
            }
            x.end("author");
        }
        if let Some(s) = &self.subtitle {
            x.quick("subtitle", Some(s), &[]);
        }
        for c in &self.categories {
            x.quick("category", Some(""), &[("term", c)]);
        }
        if let Some(c) = &self.feed_copyright {
            x.quick("rights", Some(c), &[]);
        }
        for i in &self.items {
            x.start("entry", &[]);
            x.quick("title", Some(&i.title), &[]);
            x.quick("link", Some(""), &[("href", &i.link), ("rel", "alternate")]);
            if let Some(p) = i.pubdate {
                x.quick("published", Some(&rfc3339(p)), &[]);
            }
            if let Some(u) = i.updateddate {
                x.quick("updated", Some(&rfc3339(u)), &[]);
            }
            if let Some(n) = &i.author_name {
                x.start("author", &[]);
                x.quick("name", Some(n), &[]);
                if let Some(e) = &i.author_email {
                    x.quick("email", Some(e), &[]);
                }
                if let Some(l) = &i.author_link {
                    x.quick("uri", Some(l), &[]);
                }
                x.end("author");
            }
            x.quick("id", Some(&i.unique_id.clone().unwrap_or_else(|| tag_uri(&i.link, i.pubdate))), &[]);
            if let Some(d) = &i.description {
                x.quick("summary", Some(d), &[("type", "html")]);
            }
            for e in &i.enclosures {
                x.quick("link", Some(""), &[("rel", "enclosure"), ("href", &e.url), ("length", &e.length), ("type", &e.mime_type)]);
            }
            for c in &i.categories {
                x.quick("category", Some(""), &[("term", c)]);
            }
            if let Some(c) = &i.item_copyright {
                x.quick("rights", Some(c), &[]);
            }
            x.end("entry");
        }
        x.end("feed");
    }
}

pub fn rfc2822(d: DateTime<Utc>) -> String {
    d.format("%a, %d %b %Y %H:%M:%S +0000").to_string()
}

pub fn rfc3339(d: DateTime<Utc>) -> String {
    d.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

/// `get_tag_uri(url, date)`.
pub fn tag_uri(url: &str, date: Option<DateTime<Utc>>) -> String {
    let (host, path, fragment) = match rango_core::utils::http::urlsplit(url) {
        Ok(p) => (p.hostname().unwrap_or_default(), p.path.clone(), p.fragment.clone()),
        Err(_) => (String::new(), String::new(), String::new()),
    };
    let d = date.map_or_else(String::new, |d| format!(",{}", d.format("%Y-%m-%d")));
    format!("tag:{host}{d}:{path}/{fragment}")
}

/// `SimplerXMLGenerator(short_empty_elements=True)`.
#[derive(Default)]
struct Xml {
    out: String,
    /// Name of a start tag still open for `/>`.
    pending: Option<String>,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn quoteattr(s: &str) -> String {
    let e = escape(s).replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;");
    if e.contains('"') { if e.contains('\'') { format!("\"{}\"", e.replace('"', "&quot;")) } else { format!("'{e}'") } } else { format!("\"{e}\"") }
}

impl Xml {
    fn finish_pending(&mut self) {
        if self.pending.take().is_some() {
            self.out.push('>');
        }
    }

    fn start(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.finish_pending();
        self.out.push('<');
        self.out.push_str(name);
        // the generator emits attributes sorted by name.
        let mut sorted: Vec<&(&str, &str)> = attrs.iter().collect();
        sorted.sort_by_key(|(k, _)| *k);
        for (k, v) in sorted {
            self.out.push_str(&format!(" {k}={}", quoteattr(v)));
        }
        self.pending = Some(name.to_owned());
    }

    fn chars(&mut self, s: &str) {
        if !s.is_empty() {
            self.finish_pending();
            self.out.push_str(&escape(s));
        }
    }

    fn end(&mut self, name: &str) {
        if self.pending.take().is_some() {
            self.out.push_str("/>");
        } else {
            self.out.push_str(&format!("</{name}>"));
        }
    }

    fn quick(&mut self, name: &str, contents: Option<&str>, attrs: &[(&str, &str)]) {
        self.start(name, attrs);
        if let Some(c) = contents {
            self.chars(c);
        }
        self.end(name);
    }

    fn finish(self) -> String {
        self.out
    }
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.syndication"));
