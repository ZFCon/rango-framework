//! `utils.safestring`.
//!
//! the reference implementation marks strings safe by subclassing `str`. Rust uses a newtype: [`SafeString`] is HTML
//! that must not be escaped again; plain `str`/`String` is untrusted text. [`ToHtml`] is the
//! `__html__` protocol.

use std::borrow::Cow;
use std::fmt;
use std::ops::{Add, Deref};

/// `SafeString`: text already safe for HTML output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SafeString(String);

impl SafeString {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Append other safe HTML (`SafeString + SafeString` stays safe).
    pub fn push_safe(&mut self, other: &SafeString) {
        self.0.push_str(&other.0);
    }
}

/// `mark_safe(s)`: declare `s` safe. The caller vouches for it.
pub fn mark_safe(s: impl Into<String>) -> SafeString {
    SafeString(s.into())
}

impl Deref for SafeString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SafeString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SafeString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<SafeString> for String {
    fn from(s: SafeString) -> String {
        s.0
    }
}

impl Add<&SafeString> for SafeString {
    type Output = SafeString;
    fn add(mut self, rhs: &SafeString) -> SafeString {
        self.0.push_str(&rhs.0);
        self
    }
}

/// The `__html__` protocol plus `conditional_escape`'s decision: safe values pass through,
/// everything else is HTML-escaped.
pub trait ToHtml {
    fn conditional_escape(&self) -> SafeString;
}

impl ToHtml for str {
    fn conditional_escape(&self) -> SafeString {
        super::html::escape(self)
    }
}

impl ToHtml for String {
    fn conditional_escape(&self) -> SafeString {
        self.as_str().conditional_escape()
    }
}

impl ToHtml for Cow<'_, str> {
    fn conditional_escape(&self) -> SafeString {
        self.as_ref().conditional_escape()
    }
}

impl ToHtml for SafeString {
    fn conditional_escape(&self) -> SafeString {
        self.clone()
    }
}

impl<T: ToHtml + ?Sized> ToHtml for &T {
    fn conditional_escape(&self) -> SafeString {
        (**self).conditional_escape()
    }
}

macro_rules! to_html_via_display {
    ($($t:ty),*) => {$(
        impl ToHtml for $t {
            fn conditional_escape(&self) -> SafeString {
                super::html::escape(&self.to_string())
            }
        }
    )*};
}
to_html_via_display!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char
);
