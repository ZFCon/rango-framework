//! `HttpHeaders` (request) and `ResponseHeaders` (response): case-insensitive, order-preserving.

use indexmap::IndexMap;

/// `BadHeaderError`: header names/values must not contain newlines.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BadHeaderError(pub String);

/// Case-insensitive header map. Lookup accepts `Content-Type`, `content-type` or
/// `content_type`; the first spelling used on insertion is kept for output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    items: IndexMap<String, (String, String)>,
}

fn key(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

impl Headers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request headers from a WSGI-style `META` map (`HTTP_X_FOO` → `X-Foo`).
    pub fn from_meta<'a>(meta: impl IntoIterator<Item = (&'a String, &'a String)>) -> Self {
        let mut h = Self::new();
        for (k, v) in meta {
            let name = if let Some(rest) = k.strip_prefix("HTTP_") {
                rest
            } else if k == "CONTENT_TYPE" || k == "CONTENT_LENGTH" {
                k.as_str()
            } else {
                continue;
            };
            let display: String = name
                .split('_')
                .map(|p| {
                    let mut c = p.chars();
                    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect())
                })
                .collect::<Vec<_>>()
                .join("-");
            let _ = h.set(&display, v);
        }
        h
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.items.get(&key(name)).map(|(_, v)| v.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.items.contains_key(&key(name))
    }

    /// `headers[name] = value`; rejects header injection (`BadHeaderError`).
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), BadHeaderError> {
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err(BadHeaderError(format!("Header values can't contain newlines (got {value:?} for header {name:?})")));
        }
        let k = key(name);
        let display = self.items.get(&k).map_or_else(|| name.to_owned(), |(d, _)| d.clone());
        self.items.insert(k, (display, value.to_owned()));
        Ok(())
    }

    /// `setdefault`.
    pub fn set_default(&mut self, name: &str, value: &str) {
        if !self.contains(name) {
            let _ = self.set(name, value);
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<String> {
        self.items.shift_remove(&key(name)).map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.items.values().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
