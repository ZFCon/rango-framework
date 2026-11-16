//! `utils.text` (HTML-aware truncation and gzip helpers arrive with the
//! template filters and `GZipMiddleware`).

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;
use unicode_properties::{GeneralCategory as G, UnicodeGeneralCategory};

use super::translation::{gettext, pgettext};
use crate::exceptions::{Error, Result, SuspiciousKind};

/// `str.isalnum() or c == "_"`, i.e. what `\w` means in `re` (Unicode mode).
pub fn is_word_char(c: char) -> bool {
    c == '_'
        || matches!(
            c.general_category(),
            G::UppercaseLetter
                | G::LowercaseLetter
                | G::TitlecaseLetter
                | G::ModifierLetter
                | G::OtherLetter
                | G::DecimalNumber
                | G::LetterNumber
                | G::OtherNumber
        )
}

/// `str.isspace()` (`\s` in Unicode mode).
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\x1c'..='\x1f')
}

/// `capfirst()`.
pub fn capfirst(x: &str) -> String {
    let mut chars = x.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

/// `normalize_newlines()`: CRLF and CR to LF.
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// `camel_case_to_spaces()`: split CamelCase and lowercase.
pub fn camel_case_to_spaces(value: &str) -> String {
    // Equivalent regex: re.sub(r"(((?<=[a-z])[A-Z])|([A-Z](?![A-Z]|$)))", r" \1", value).strip().lower()
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let after_lower = i > 0 && chars[i - 1].is_ascii_lowercase();
            let before_non_upper = chars.get(i + 1).is_some_and(|n| !n.is_ascii_uppercase());
            if after_lower || before_non_upper {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out.trim_matches(is_space).to_lowercase()
}

/// `slugify()`.
pub fn slugify(value: &str, allow_unicode: bool) -> String {
    let normalized: String = if allow_unicode {
        value.nfkc().collect()
    } else {
        value.nfkd().filter(char::is_ascii).collect()
    };
    let kept: String = normalized
        .to_lowercase()
        .chars()
        .filter(|&c| is_word_char(c) || is_space(c) || c == '-')
        .collect();
    let mut out = String::with_capacity(kept.len());
    let mut in_sep = false;
    for c in kept.chars() {
        if c == '-' || is_space(c) {
            in_sep = true;
        } else {
            if in_sep {
                out.push('-');
                in_sep = false;
            }
            out.push(c);
        }
    }
    if in_sep {
        out.push('-');
    }
    out.trim_matches(['-', '_']).to_owned()
}

/// `get_valid_filename()`; `SuspiciousFileOperation` if nothing usable is left.
pub fn get_valid_filename(name: &str) -> Result<String> {
    let s: String = name
        .trim_matches(is_space)
        .replace(' ', "_")
        .chars()
        .filter(|&c| is_word_char(c) || c == '-' || c == '.')
        .collect();
    if matches!(s.as_str(), "" | "." | "..") {
        return Err(Error::suspicious(
            SuspiciousKind::SuspiciousFileOperation,
            format!("Could not derive file name from '{name}'"),
        ));
    }
    Ok(s)
}

/// `get_text_list()`: `"a, b or c"`. `last_word` defaults to the translated "or".
pub fn get_text_list(items: &[impl AsRef<str>], last_word: Option<&str>) -> String {
    match items {
        [] => String::new(),
        [only] => only.as_ref().to_owned(),
        [init @ .., last] => {
            let head = init
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(&gettext(", "));
            let word = last_word.map_or_else(|| gettext("or"), str::to_owned);
            format!("{head} {word} {}", last.as_ref())
        }
    }
}

/// `phone2numeric()`.
pub fn phone2numeric(phone: &str) -> String {
    phone
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='c' => '2',
            'd'..='f' => '3',
            'g'..='i' => '4',
            'j'..='l' => '5',
            'm'..='o' => '6',
            'p'..='s' => '7',
            't'..='v' => '8',
            'w'..='z' => '9',
            c => c,
        })
        .collect()
}

static SMART_SPLIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?x)
        (?:
            [^\s'"]*
            (?:
                (?: "(?:[^"\\]|\\.)*" | '(?:[^'\\]|\\.)*' )
                [^\s'"]*
            )+
        ) | \S+"#,
    )
    .expect("valid regex")
});

/// `smart_split()`: split on whitespace keeping quoted phrases (and their quotes) together.
pub fn smart_split(text: &str) -> Vec<&str> {
    SMART_SPLIT.find_iter(text).map(|m| m.as_str()).collect()
}

/// `unescape_string_literal()`: `'"a \"bc\""'` becomes `a "bc"`. `None` if `s` is not quoted.
pub fn unescape_string_literal(s: &str) -> Option<String> {
    let q = s.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    if !s.ends_with(q) {
        return None;
    }
    let inner = s.get(1..s.len() - 1).unwrap_or("");
    Some(
        inner
            .replace(&format!("\\{q}"), &q.to_string())
            .replace("\\\\", "\\"),
    )
}

/// `trim_value()`: `(value, suffix)` where suffix notes how many chars were cut.
pub fn trim_value(value: &str, limit: usize) -> (String, String) {
    let len = value.chars().count();
    if len > limit {
        (
            value.chars().take(limit).collect(),
            format!("… <trimmed {} chars out of {len}>", len - limit),
        )
    } else {
        (value.to_owned(), String::new())
    }
}

/// `Truncator` (plain-text modes).
pub struct Truncator<'a>(&'a str);

impl<'a> Truncator<'a> {
    pub fn new(text: &'a str) -> Self {
        Self(text)
    }

    /// Truncate to at most `num` characters (combining marks do not count).
    pub fn chars(&self, num: i64, truncate: Option<&str>) -> String {
        if num <= 0 {
            return String::new();
        }
        let length = num as usize;
        let text: String = self.0.nfc().collect();
        let truncate_len = calculate_truncate_chars_length(length, truncate);
        let mut s_len = 0;
        let mut end_index: Option<usize> = None;
        for (i, ch) in text.char_indices() {
            if canonical_combining_class(ch) != 0 {
                continue;
            }
            s_len += 1;
            if end_index.is_none() && s_len > truncate_len {
                end_index = Some(i);
            }
            if s_len > length {
                return add_truncation_text(&text[..end_index.unwrap_or(0)], truncate);
            }
        }
        text
    }

    /// Truncate to at most `num` words (whitespace is normalized to single spaces).
    pub fn words(&self, num: i64, truncate: Option<&str>) -> String {
        if num <= 0 {
            return String::new();
        }
        let words: Vec<&str> = self.0.split(is_space).filter(|w| !w.is_empty()).collect();
        if words.len() > num as usize {
            add_truncation_text(&words[..num as usize].join(" "), truncate)
        } else {
            words.join(" ")
        }
    }
}

fn add_truncation_text(text: &str, truncate: Option<&str>) -> String {
    let default;
    let truncate = match truncate {
        Some(t) => t,
        None => {
            default = pgettext(
                "String to return when truncating text",
                "%(truncated_text)s…",
            );
            &default
        }
    };
    if truncate.contains("%(truncated_text)s") {
        return truncate.replace("%(truncated_text)s", text);
    }
    if text.ends_with(truncate) {
        return text.to_owned();
    }
    format!("{text}{truncate}")
}

fn calculate_truncate_chars_length(length: usize, replacement: Option<&str>) -> usize {
    let mut truncate_len = length;
    for ch in add_truncation_text("", replacement).chars() {
        if canonical_combining_class(ch) == 0 {
            truncate_len -= 1;
            if truncate_len == 0 {
                break;
            }
        }
    }
    truncate_len
}

/// `wrap()`: word-wrap preserving existing line breaks; long words are not split.
/// (A port of `textwrap.TextWrapper(break_long_words=False, break_on_hyphens=False,
/// replace_whitespace=False)`.)
pub fn wrap(text: &str, width: usize) -> String {
    let mut result: Vec<String> = Vec::new();
    for line in py_splitlines(text) {
        let wrapped = wrap_line(&expandtabs(line, 8), width);
        if wrapped.is_empty() {
            result.push(line.to_owned());
        } else {
            result.extend(wrapped);
        }
    }
    if text.ends_with('\n') {
        result.push(String::new());
    }
    result.join("\n")
}

fn wrap_line(text: &str, width: usize) -> Vec<String> {
    // Split into alternating whitespace / non-whitespace chunks.
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_ws = false;
    for c in text.chars() {
        let ws = matches!(c, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ');
        if !cur.is_empty() && ws != cur_ws {
            chunks.push(std::mem::take(&mut cur));
        }
        cur_ws = ws;
        cur.push(c);
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks.reverse();

    let is_blank = |s: &str| s.trim_matches(is_space).is_empty();
    let mut lines: Vec<String> = Vec::new();
    while !chunks.is_empty() {
        let mut cur_line: Vec<String> = Vec::new();
        let mut cur_len = 0;
        if !lines.is_empty() && chunks.last().is_some_and(|c| is_blank(c)) {
            chunks.pop();
        }
        while let Some(last) = chunks.last() {
            let l = last.chars().count();
            if cur_len + l <= width {
                cur_len += l;
                cur_line.push(chunks.pop().expect("non-empty"));
            } else {
                break;
            }
        }
        if chunks.last().is_some_and(|c| c.chars().count() > width) && cur_line.is_empty() {
            cur_line.push(chunks.pop().expect("non-empty"));
        }
        if cur_line.last().is_some_and(|c| is_blank(c)) {
            cur_line.pop();
        }
        if !cur_line.is_empty() {
            lines.push(cur_line.concat());
        }
    }
    lines
}

/// `str.splitlines()` (without keepends).
fn py_splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = s.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        let is_break = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if is_break {
            out.push(&s[start..i]);
            let mut next = i + c.len_utf8();
            if c == '\r' && iter.peek().is_some_and(|&(_, n)| n == '\n') {
                iter.next();
                next += 1;
            }
            start = next;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// `str.expandtabs(tabsize)`.
fn expandtabs(s: &str, tabsize: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        match c {
            '\t' => {
                let n = if tabsize == 0 {
                    0
                } else {
                    tabsize - col % tabsize
                };
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            '\n' | '\r' => {
                out.push(c);
                col = 0;
            }
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

/// `compress_string(s, max_random_bytes=...)`: gzip (level 6, mtime 0). With `max_random_bytes`
/// a random-length junk filename is added to the header (BREACH mitigation).
pub fn compress_string(data: &[u8], max_random_bytes: Option<usize>) -> Vec<u8> {
    use std::io::Write;
    let mut b = flate2::GzBuilder::new().mtime(0);
    if let Some(max) = max_random_bytes.filter(|&m| m > 0) {
        let n = rand::RngExt::random_range(&mut rand::rng(), 0..max);
        b = b.filename(vec![b'a'; n]);
    }
    let mut enc = b.write(Vec::new(), flate2::Compression::new(6));
    enc.write_all(data).expect("write to Vec");
    enc.finish().expect("finish gzip")
}
