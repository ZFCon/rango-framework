//! Path converters (`urls.converters`): `<int:pk>`, `<slug:s>`, ...

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::view::Arg;

/// A `<converter:name>` in a `path()` route.
pub trait PathConverter: Send + Sync {
    /// Regex fragment matched by the parameter.
    fn regex(&self) -> &str;
    /// `to_clean`: `None` means "does not match" (the `ValueError`).
    fn to_clean(&self, value: &str) -> Option<Arg>;
    /// `to_url`: `None` means the value cannot be reversed.
    fn to_url(&self, value: &str) -> Option<String> {
        Some(value.to_owned())
    }
}

struct IntConverter;
struct StringConverter;
struct SlugConverter;
struct PathConv;
struct UuidConverter;

impl PathConverter for IntConverter {
    fn regex(&self) -> &str {
        "[0-9]+"
    }
    fn to_clean(&self, v: &str) -> Option<Arg> {
        v.parse().ok().map(Arg::Int)
    }
}
impl PathConverter for StringConverter {
    fn regex(&self) -> &str {
        "[^/]+"
    }
    fn to_clean(&self, v: &str) -> Option<Arg> {
        Some(Arg::Str(v.to_owned()))
    }
}
impl PathConverter for SlugConverter {
    fn regex(&self) -> &str {
        "[-a-zA-Z0-9_]+"
    }
    fn to_clean(&self, v: &str) -> Option<Arg> {
        Some(Arg::Str(v.to_owned()))
    }
}
impl PathConverter for PathConv {
    fn regex(&self) -> &str {
        ".+"
    }
    fn to_clean(&self, v: &str) -> Option<Arg> {
        Some(Arg::Str(v.to_owned()))
    }
}
impl PathConverter for UuidConverter {
    fn regex(&self) -> &str {
        "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
    }
    fn to_clean(&self, v: &str) -> Option<Arg> {
        v.parse().ok().map(Arg::Uuid)
    }
}

static CONVERTERS: LazyLock<RwLock<HashMap<String, Arc<dyn PathConverter>>>> = LazyLock::new(|| {
    let mut m: HashMap<String, Arc<dyn PathConverter>> = HashMap::new();
    m.insert("int".into(), Arc::new(IntConverter));
    m.insert("path".into(), Arc::new(PathConv));
    m.insert("slug".into(), Arc::new(SlugConverter));
    m.insert("str".into(), Arc::new(StringConverter));
    m.insert("uuid".into(), Arc::new(UuidConverter));
    RwLock::new(m)
});

/// `register_converter(converter, type_name)`.
pub fn register_converter(type_name: &str, converter: impl PathConverter + 'static) {
    CONVERTERS.write().expect("converters lock").insert(type_name.to_owned(), Arc::new(converter));
}

/// Look up a converter by its `<type:...>` name.
pub fn get_converter(name: &str) -> Option<Arc<dyn PathConverter>> {
    CONVERTERS.read().expect("converters lock").get(name).cloned()
}
