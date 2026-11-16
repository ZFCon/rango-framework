//! `utils.html` (the parts that do not depend on later milestones;
//! `urlize`/`smart_urlquote` arrive with the template filters).

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::safestring::{SafeString, ToHtml, mark_safe};
use super::text::normalize_newlines;
use crate::exceptions::{Error, Result, SuspiciousKind};

const MAX_STRIP_TAGS_DEPTH: usize = 50;

/// `escape()`: always escapes (may double-escape); use [`conditional_escape`] to avoid that.
pub fn escape(text: &str) -> SafeString {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            c => out.push(c),
        }
    }
    mark_safe(out)
}

/// `conditional_escape()`: escape unless the value is already safe.
pub fn conditional_escape<T: ToHtml + ?Sized>(text: &T) -> SafeString {
    text.conditional_escape()
}

/// `format_html()`: like `format!`, but every positional/named argument is passed through
/// [`conditional_escape`] and the result is marked safe. (Inline `{captured}` identifiers are
/// *not* escaped, so pass values as arguments.)
#[macro_export]
macro_rules! format_html {
    ($fmt:literal $(, $name:ident = $arg:expr)+ $(,)?) => {
        $crate::utils::safestring::mark_safe(::std::format!(
            $fmt $(, $name = $crate::utils::html::conditional_escape(&$arg))+
        ))
    };
    ($fmt:literal $(, $arg:expr)* $(,)?) => {
        $crate::utils::safestring::mark_safe(::std::format!(
            $fmt $(, $crate::utils::html::conditional_escape(&$arg))*
        ))
    };
}

/// `format_html_join()`: join already-built fragments with a conditionally-escaped separator.
pub fn format_html_join<T: ToHtml + ?Sized>(
    sep: &T,
    fragments: impl IntoIterator<Item = SafeString>,
) -> SafeString {
    let sep = conditional_escape(sep);
    let parts: Vec<String> = fragments.into_iter().map(String::from).collect();
    mark_safe(parts.join(sep.as_str()))
}

/// `escapejs()`: hex-encode characters for use inside JavaScript strings.
pub fn escapejs(value: &str) -> SafeString {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\u005C"),
            '\'' => out.push_str("\\u0027"),
            '"' => out.push_str("\\u0022"),
            '>' => out.push_str("\\u003E"),
            '<' => out.push_str("\\u003C"),
            '&' => out.push_str("\\u0026"),
            '=' => out.push_str("\\u003D"),
            '-' => out.push_str("\\u002D"),
            ';' => out.push_str("\\u003B"),
            '`' => out.push_str("\\u0060"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 32 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    mark_safe(out)
}

/// `json_script()`: JSON in a `<script type="application/json">` tag, safe anywhere but
/// inside a tag attribute.
pub fn json_script<T: Serialize + ?Sized>(
    value: &T,
    element_id: Option<&str>,
) -> serde_json::Result<SafeString> {
    let json = super::json::dumps(value, false)?
        .replace('>', "\\u003E")
        .replace('<', "\\u003C")
        .replace('&', "\\u0026");
    let json = mark_safe(json);
    Ok(match element_id {
        Some(id) if !id.is_empty() => {
            format_html!(
                "<script id=\"{}\" type=\"application/json\">{}</script>",
                id,
                json
            )
        }
        _ => format_html!("<script type=\"application/json\">{}</script>", json),
    })
}

static PARA_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{2,}").expect("valid regex"));

/// `linebreaks()`: newlines to `<p>` and `<br>`.
pub fn linebreaks(value: &str, autoescape: bool) -> String {
    let value = normalize_newlines(value);
    PARA_SPLIT
        .split(&value)
        .map(|p| {
            if autoescape {
                format!("<p>{}</p>", escape(p).replace('\n', "<br>"))
            } else {
                format!("<p>{}</p>", p.replace('\n', "<br>"))
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

static BETWEEN_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">\s+<").expect("valid regex"));

/// `strip_spaces_between_tags()`.
pub fn strip_spaces_between_tags(value: &str) -> String {
    BETWEEN_TAGS.replace_all(value, "><").into_owned()
}

/// `avoid_wrapping()`: keep a value on one line by using non-breaking spaces.
pub fn avoid_wrapping(value: &str) -> String {
    value.replace(' ', "\u{a0}")
}

static LONG_OPEN_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[a-zA-Z][^>]{1000,}").expect("valid regex"));

/// One pass of tag removal. Mirrors the `MLStripper` on top of lenient
/// `HTMLParser(convert_charrefs=False)`: tags, comments, declarations and processing
/// instructions are dropped; text and character/entity references are kept verbatim.
fn strip_once(value: &str) -> String {
    let b = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let next = value[i..].find('<').map_or(b.len(), |p| i + p);
            out.push_str(&value[i..next]);
            i = next;
            continue;
        }
        let rest = &value[i..];
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map(|p| p + 3)
        } else if rest.len() > 1
            && (rest.as_bytes()[1].is_ascii_alphabetic()
                || matches!(rest.as_bytes()[1], b'/' | b'!' | b'?'))
        {
            tag_end(rest)
        } else {
            // `<` not starting markup is plain data.
            out.push('<');
            i += 1;
            continue;
        };
        match end {
            Some(n) => i += n,
            None => {
                // Unterminated markup: HTMLParser flushes it as data up to the next `<`.
                let next = value[i + 1..].find('<').map_or(b.len(), |p| i + 1 + p);
                out.push_str(&value[i..next]);
                i = next;
            }
        }
    }
    out
}

/// Length of the tag at the start of `s` (through its closing `>`), honouring quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, &c) in s.as_bytes().iter().enumerate().skip(1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') if s[..i].ends_with('=') || s[..i].trim_end().ends_with('=') => {
                quote = Some(c)
            }
            (None, b'>') => return Some(i + 1),
            _ => {}
        }
    }
    None
}

/// `strip_tags()`: HTML with all tags removed. Errors with `SuspiciousOperation` on
/// pathological nesting.
pub fn strip_tags(value: &str) -> Result<String> {
    for m in LONG_OPEN_TAG.find_iter(value) {
        if m.as_str().matches('<').count() >= MAX_STRIP_TAGS_DEPTH {
            return Err(Error::suspicious(SuspiciousKind::Operation, ""));
        }
    }
    let mut value = value.to_owned();
    let mut depth = 0;
    while value.contains('<') && value.contains('>') {
        if depth >= MAX_STRIP_TAGS_DEPTH {
            return Err(Error::suspicious(SuspiciousKind::Operation, ""));
        }
        let new_value = strip_once(&value);
        if value.matches('<').count() == new_value.matches('<').count() {
            break;
        }
        value = new_value;
        depth += 1;
    }
    Ok(value)
}
