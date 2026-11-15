//! `utils.datastructures`.

use indexmap::{IndexMap, IndexSet};

/// `OrderedSet`: insertion-ordered set.
pub type OrderedSet<T> = IndexSet<T>;

/// `MultiValueDictKeyError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Key {0:?} not found")]
pub struct MultiValueDictKeyError(pub String);

/// `MultiValueDict`: a dict whose values are lists, where plain access yields the *last* value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiValueDict<V> {
    map: IndexMap<String, Vec<V>>,
}

impl<V> Default for MultiValueDict<V> {
    fn default() -> Self {
        Self {
            map: IndexMap::new(),
        }
    }
}

impl<V> MultiValueDict<V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `MultiValueDict({key: [values]})`.
    pub fn from_lists(lists: impl IntoIterator<Item = (String, Vec<V>)>) -> Self {
        Self {
            map: lists.into_iter().collect(),
        }
    }

    /// Build from `(key, value)` pairs, appending repeated keys (the shape of a query string).
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, V)>) -> Self {
        let mut d = Self::new();
        for (k, v) in pairs {
            d.appendlist(k, v);
        }
        d
    }

    /// `d[key]` / `d.get(key)`: the last value, `None` if missing or its list is empty.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key).and_then(|l| l.last())
    }

    /// `d[key]` raising `MultiValueDictKeyError` when missing.
    pub fn try_get(&self, key: &str) -> Result<&V, MultiValueDictKeyError> {
        self.get(key)
            .ok_or_else(|| MultiValueDictKeyError(key.to_owned()))
    }

    /// `getlist(key)`: all values, empty if missing.
    pub fn getlist(&self, key: &str) -> &[V] {
        self.map.get(key).map_or(&[], Vec::as_slice)
    }

    /// `d[key] = value` (replaces the list with `[value]`).
    pub fn set(&mut self, key: impl Into<String>, value: V) {
        self.map.insert(key.into(), vec![value]);
    }

    pub fn setlist(&mut self, key: impl Into<String>, list: Vec<V>) {
        self.map.insert(key.into(), list);
    }

    /// `setdefault(key, default)`: returns the (possibly new) last value.
    pub fn setdefault(&mut self, key: impl Into<String>, default: V) -> &V {
        let list = self.map.entry(key.into()).or_default();
        if list.is_empty() {
            list.push(default);
        }
        list.last().expect("non-empty")
    }

    pub fn setlistdefault(&mut self, key: impl Into<String>, default: Vec<V>) -> &[V] {
        self.map.entry(key.into()).or_insert(default)
    }

    pub fn appendlist(&mut self, key: impl Into<String>, value: V) {
        self.map.entry(key.into()).or_default().push(value);
    }

    /// `update(...)`: *extends* existing lists rather than replacing them.
    pub fn update(&mut self, other: impl IntoIterator<Item = (String, V)>) {
        for (k, v) in other {
            self.appendlist(k, v);
        }
    }

    /// `items()`: `(key, last value)`.
    pub fn items(&self) -> impl Iterator<Item = (&str, &V)> {
        self.map
            .iter()
            .filter_map(|(k, l)| l.last().map(|v| (k.as_str(), v)))
    }

    /// `lists()`: `(key, all values)`.
    pub fn lists(&self) -> impl Iterator<Item = (&str, &[V])> {
        self.map.iter().map(|(k, l)| (k.as_str(), l.as_slice()))
    }

    /// `values()`: last value of each key.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.map.values().filter_map(|l| l.last())
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Vec<V>> {
        self.map.shift_remove(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `dict()`: plain mapping to last values.
    pub fn to_dict(&self) -> IndexMap<&str, &V> {
        self.items().collect()
    }
}
