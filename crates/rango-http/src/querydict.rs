//! `QueryDict`: the immutable-by-default `MultiValueDict` of query strings and form posts.

use rango_core::conf::try_settings;
use rango_core::exceptions::{Error, Result, SuspiciousKind};
use rango_core::utils::datastructures::MultiValueDict;
use rango_core::utils::http::{quote_plus, unquote};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryDict {
    inner: MultiValueDict<String>,
    mutable: bool,
}

fn parse_qsl(query: &str, max_num_fields: Option<usize>) -> Result<Vec<(String, String)>> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(max) = max_num_fields {
        if query.matches('&').count() + 1 > max {
            return Err(Error::suspicious(SuspiciousKind::TooManyFieldsSent, "The number of GET/POST parameters exceeded settings.DATA_UPLOAD_MAX_NUMBER_FIELDS."));
        }
    }
    Ok(query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (unquote(&k.replace('+', " ")), unquote(&v.replace('+', " ")))
        })
        .collect())
}

impl QueryDict {
    pub fn new() -> Self {
        Self::default()
    }

    /// `QueryDict(query_string, mutable=...)`; honours `DATA_UPLOAD_MAX_NUMBER_FIELDS`.
    pub fn parse(query_string: &str, mutable: bool) -> Result<Self> {
        let max = try_settings().ok().map(|s| s.data_upload_max_number_fields).filter(|&m| m > 0).map(|m| m as usize);
        Ok(Self { inner: MultiValueDict::from_pairs(parse_qsl(query_string, max)?), mutable })
    }

    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>, mutable: bool) -> Self {
        Self { inner: MultiValueDict::from_pairs(pairs), mutable }
    }

    fn check_mutable(&self) -> Result<()> {
        if self.mutable { Ok(()) } else { Err(Error::Other("This QueryDict instance is immutable".into())) }
    }

    pub fn is_mutable(&self) -> bool {
        self.mutable
    }

    /// `copy()`: a mutable copy.
    pub fn copy(&self) -> Self {
        Self { inner: self.inner.clone(), mutable: true }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key).map(String::as_str)
    }

    pub fn getlist(&self, key: &str) -> &[String] {
        self.inner.getlist(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    pub fn items(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner.items().map(|(k, v)| (k, v.as_str()))
    }

    pub fn lists(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.inner.lists()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.inner.keys()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        self.check_mutable()?;
        self.inner.set(key, value.to_owned());
        Ok(())
    }

    pub fn setlist(&mut self, key: &str, values: Vec<String>) -> Result<()> {
        self.check_mutable()?;
        self.inner.setlist(key, values);
        Ok(())
    }

    pub fn appendlist(&mut self, key: &str, value: &str) -> Result<()> {
        self.check_mutable()?;
        self.inner.appendlist(key, value.to_owned());
        Ok(())
    }

    pub fn remove(&mut self, key: &str) -> Result<Option<Vec<String>>> {
        self.check_mutable()?;
        Ok(self.inner.remove(key))
    }

    /// `urlencode(safe=None)`.
    pub fn urlencode(&self, safe: Option<&str>) -> String {
        let enc = |s: &str| match safe {
            Some(safe) => rango_core::utils::http::quote(s, safe).replace("%20", "+"),
            None => quote_plus(s),
        };
        self.inner
            .lists()
            .flat_map(|(k, vs)| vs.iter().map(move |v| (k, v)))
            .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
            .collect::<Vec<_>>()
            .join("&")
    }
}
