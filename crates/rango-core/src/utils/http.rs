//! `utils.http`, plus the `urllib.parse` pieces it relies on.

use std::collections::HashSet;
use std::sync::LazyLock;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Datelike, TimeZone, Utc};
use percent_encoding::percent_decode_str;
use regex::Regex;
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

use super::datastructures::MultiValueDict;

pub const RFC3986_GENDELIMS: &str = ":/?#[]@";
pub const RFC3986_SUBDELIMS: &str = "!$&'()*+,;=";
pub const MAX_URL_LENGTH: usize = 2048;
pub const MAX_URL_REDIRECT_LENGTH: usize = 16384;

fn quote_impl(s: &str, safe: &str, plus: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(b as char);
        } else if plus && b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `urllib.parse.quote_plus`.
pub fn quote_plus(s: &str) -> String {
    quote_impl(s, "", true)
}

/// `urllib.parse.quote(s, safe=...)`.
pub fn quote(s: &str, safe: &str) -> String {
    quote_impl(s, safe, false)
}

/// `urllib.parse.unquote`.
pub fn unquote(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// `urlencode(query)` for `(key, value)` pairs.
pub fn urlencode<K: AsRef<str>, V: AsRef<str>>(query: impl IntoIterator<Item = (K, V)>) -> String {
    query
        .into_iter()
        .map(|(k, v)| format!("{}={}", quote_plus(k.as_ref()), quote_plus(v.as_ref())))
        .collect::<Vec<_>>()
        .join("&")
}

/// `urlencode(multi_value_dict)`: every value of every key, as with `doseq=True`.
pub fn urlencode_multi(query: &MultiValueDict<String>) -> String {
    urlencode(
        query
            .lists()
            .flat_map(|(k, vs)| vs.iter().map(move |v| (k, v.as_str()))),
    )
}

/// `http_date()`: `Wdy, DD Mon YYYY HH:MM:SS GMT`. `None` means now.
pub fn http_date(epoch_seconds: Option<f64>) -> String {
    let dt: DateTime<Utc> = match epoch_seconds {
        Some(s) => Utc
            .timestamp_opt(s.floor() as i64, 0)
            .single()
            .unwrap_or_default(),
        None => Utc::now(),
    };
    dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not in a valid HTTP date format")]
pub struct HttpDateError(pub String);

static RFC1123_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\w{3}, (?P<day>[0-9]{2}) (?P<mon>\w{3}) (?P<year>[0-9]{4}) (?P<hour>[0-9]{2}):(?P<min>[0-9]{2}):(?P<sec>[0-9]{2}) GMT$").expect("valid regex")
});
static RFC850_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\w{6,9}, (?P<day>[0-9]{2})-(?P<mon>\w{3})-(?P<year>[0-9]{2}) (?P<hour>[0-9]{2}):(?P<min>[0-9]{2}):(?P<sec>[0-9]{2}) GMT$").expect("valid regex")
});
static ASCTIME_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\w{3} (?P<mon>\w{3}) (?P<day>[ 0-9][0-9]) (?P<hour>[0-9]{2}):(?P<min>[0-9]{2}):(?P<sec>[0-9]{2}) (?P<year>[0-9]{4})$").expect("valid regex")
});
const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// `parse_http_date()`: seconds since the epoch (UTC).
pub fn parse_http_date(date: &str) -> Result<i64, HttpDateError> {
    let err = || HttpDateError(date.to_owned());
    let m = [&*RFC1123_DATE, &*RFC850_DATE, &*ASCTIME_DATE]
        .into_iter()
        .find_map(|re| re.captures(date))
        .ok_or_else(err)?;
    let num = |name: &str| m[name].trim().parse::<u32>().map_err(|_| err());
    let mut year = num("year")? as i32;
    if year < 100 {
        let current_year = Utc::now().year();
        let current_century = current_year - (current_year % 100);
        year += if year - (current_year % 100) > 50 {
            current_century - 100
        } else {
            current_century
        };
    }
    let month = MONTHS
        .iter()
        .position(|&x| x == m["mon"].to_lowercase())
        .ok_or_else(err)? as u32
        + 1;
    Utc.with_ymd_and_hms(
        year,
        month,
        num("day")?,
        num("hour")?,
        num("min")?,
        num("sec")?,
    )
    .single()
    .map(|d| d.timestamp())
    .ok_or_else(err)
}

/// `parse_http_date_safe()`.
pub fn parse_http_date_safe(date: &str) -> Option<i64> {
    parse_http_date(date).ok()
}

/// `base36_to_int()`.
pub fn base36_to_int(s: &str) -> Result<u64, String> {
    if s.len() > 13 {
        return Err("Base36 input too large".into());
    }
    u64::from_str_radix(s, 36).map_err(|e| e.to_string())
}

/// `int_to_base36()`.
pub fn int_to_base36(mut i: u64) -> String {
    const CHARS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if i < 36 {
        return (CHARS[i as usize] as char).to_string();
    }
    let mut out = Vec::new();
    while i != 0 {
        out.push(CHARS[(i % 36) as usize]);
        i /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii")
}

/// `urlsafe_base64_encode()`: no padding.
pub fn urlsafe_base64_encode(s: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(s)
}

/// `urlsafe_base64_decode()`: tolerates stripped (or present) padding.
pub fn urlsafe_base64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    // the decoder silently discards characters outside the base64 alphabet.
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/'))
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect();
    URL_SAFE_NO_PAD.decode(cleaned)
}

static ETAG_MATCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\A((?:W/)?"[^"]*")\z"#).expect("valid regex"));

/// `parse_etags()`.
pub fn parse_etags(etag_str: &str) -> Vec<String> {
    if etag_str.trim() == "*" {
        return vec!["*".into()];
    }
    etag_str
        .split(',')
        .filter_map(|e| ETAG_MATCH.captures(e.trim()).map(|c| c[1].to_owned()))
        .collect()
}

/// `quote_etag()`.
pub fn quote_etag(etag_str: &str) -> String {
    if ETAG_MATCH.is_match(etag_str) {
        etag_str.to_owned()
    } else {
        format!("\"{etag_str}\"")
    }
}

/// `is_same_domain()`.
pub fn is_same_domain(host: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    let pattern = pattern.to_lowercase();
    (pattern.starts_with('.') && (host.ends_with(&pattern) || host == &pattern[1..]))
        || pattern == host
}

/// `urllib.parse.SplitResult`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SplitResult {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

impl SplitResult {
    /// `.hostname`: host without userinfo/port/brackets, lowercased; `None` if empty.
    pub fn hostname(&self) -> Option<String> {
        let hostinfo = self
            .netloc
            .rsplit_once('@')
            .map_or(self.netloc.as_str(), |(_, h)| h);
        let host = match hostinfo.split_once('[') {
            Some((_, rest)) => rest.split(']').next().unwrap_or(""),
            None => hostinfo.split(':').next().unwrap_or(""),
        };
        (!host.is_empty()).then(|| host.to_lowercase())
    }

    /// `urlunsplit`.
    pub fn unsplit(&self) -> String {
        let mut url = self.path.clone();
        if !self.netloc.is_empty() || (!self.scheme.is_empty() && url.starts_with("//")) {
            if !url.is_empty() && !url.starts_with('/') {
                url.insert(0, '/');
            }
            url = format!("//{}{url}", self.netloc);
        }
        if !self.scheme.is_empty() {
            url = format!("{}:{url}", self.scheme);
        }
        if !self.query.is_empty() {
            url = format!("{url}?{}", self.query);
        }
        if !self.fragment.is_empty() {
            url = format!("{url}#{}", self.fragment);
        }
        url
    }
}

/// Split a URL into scheme, netloc, path, query and fragment. `Err` for an invalid URL.
pub fn urlsplit(url: &str) -> Result<SplitResult, String> {
    let mut url: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut r = SplitResult::default();
    if let Some(i) = url.find(':') {
        let head = &url[..i];
        if i > 0
            && head.starts_with(|c: char| c.is_ascii_alphabetic())
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        {
            r.scheme = head.to_lowercase();
            url = url[i + 1..].to_owned();
        }
    }
    if let Some(rest) = url.strip_prefix("//") {
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        r.netloc = rest[..end].to_owned();
        url = rest[end..].to_owned();
        let (open, close) = (r.netloc.contains('['), r.netloc.contains(']'));
        if open != close {
            return Err("Invalid IPv6 URL".into());
        }
        if open {
            let host = r
                .netloc
                .split_once('[')
                .map_or("", |(_, t)| t)
                .split(']')
                .next()
                .unwrap_or("");
            if !host.starts_with(['v', 'V']) && host.parse::<std::net::Ipv6Addr>().is_err() {
                return Err(format!(
                    "{host:?} does not appear to be an IPv4 or IPv6 address"
                ));
            }
        }
    }
    if let Some((u, f)) = url.split_once('#') {
        r.fragment = f.to_owned();
        url = u.to_owned();
    }
    if let Some((u, q)) = url.split_once('?') {
        r.query = q.to_owned();
        url = u.to_owned();
    }
    r.path = url;
    Ok(r)
}

/// `url_has_allowed_host_and_scheme()`.
pub fn url_has_allowed_host_and_scheme(
    url: &str,
    allowed_hosts: &HashSet<String>,
    require_https: bool,
) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return false;
    }
    check_allowed(url, allowed_hosts, require_https)
        && check_allowed(&url.replace('\\', "/"), allowed_hosts, require_https)
}

fn check_allowed(url: &str, allowed_hosts: &HashSet<String>, require_https: bool) -> bool {
    if url.starts_with("///") || url.len() > MAX_URL_LENGTH {
        return false;
    }
    let Ok(info) = urlsplit(url) else {
        return false;
    };
    if info.netloc.is_empty() && !info.scheme.is_empty() {
        return false;
    }
    if url
        .chars()
        .next()
        .is_some_and(|c| c.general_category_group() == GeneralCategoryGroup::Other)
    {
        return false;
    }
    let scheme = if info.scheme.is_empty() && !info.netloc.is_empty() {
        "http"
    } else {
        &info.scheme
    };
    let valid: &[&str] = if require_https {
        &["https"]
    } else {
        &["http", "https"]
    };
    (info.netloc.is_empty() || allowed_hosts.contains(&info.netloc))
        && (scheme.is_empty() || valid.contains(&scheme))
}

/// `escape_leading_slashes()`.
pub fn escape_leading_slashes(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("/%2F{rest}"),
        None => url.to_owned(),
    }
}

/// `content_disposition_header()`.
pub fn content_disposition_header(as_attachment: bool, filename: Option<&str>) -> Option<String> {
    match filename {
        Some(name) if !name.is_empty() => {
            let disposition = if as_attachment {
                "attachment"
            } else {
                "inline"
            };
            let quotable = name
                .chars()
                .all(|c| matches!(c, '\t' | ' ' | '\x21'..='\x7e'));
            let expr = if quotable {
                format!(
                    "filename=\"{}\"",
                    name.replace('\\', "\\\\").replace('"', "\\\"")
                )
            } else {
                format!("filename*=utf-8''{}", quote(name, "/"))
            };
            Some(format!("{disposition}; {expr}"))
        }
        _ if as_attachment => Some("attachment".into()),
        _ => None,
    }
}

/// `parse_header_parameters()`: main value (lowercased) and parameters of a Content-Type-like
/// header. Handles quoted strings and RFC 2231 (`name*=charset''pct`, `name*0=...` continuations).
pub fn parse_header_parameters(line: &str) -> Result<(String, Vec<(String, String)>), String> {
    if line.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let Some((key, rest)) = line.split_once(';') else {
        return Ok((line.trim().to_lowercase(), Vec::new()));
    };
    let mut raw: Vec<(String, String)> = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ';').is_some() {}
        let mut name = String::new();
        while let Some(c) = chars.next_if(|c| *c != '=' && *c != ';') {
            name.push(c);
        }
        let name = name.trim().to_lowercase();
        if chars.next_if_eq(&'=').is_none() {
            if name.is_empty() {
                break;
            }
            raw.push((name, String::new()));
            continue;
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'"').is_some() {
            while let Some(c) = chars.next() {
                match c {
                    '\\' => value.extend(chars.next()),
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(c) = chars.next_if(|c| *c != ';') {
                value.push(c);
            }
            value = value.trim().to_owned();
        }
        if !name.is_empty() {
            raw.push((name, value));
        }
    }

    // Collapse RFC 2231 continuations / extended values.
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let (name, value) = &raw[i];
        if let Some(base) = name.strip_suffix('*').filter(|b| !b.contains('*')) {
            out.push((base.to_owned(), decode_rfc2231(value)?));
        } else if let Some((base, idx)) = name.split_once('*') {
            let base = base.to_owned();
            let _ = idx;
            let mut joined = String::new();
            let mut extended = false;
            while i < raw.len() && raw[i].0.starts_with(&format!("{base}*")) {
                extended |= raw[i].0.ends_with('*');
                joined.push_str(&raw[i].1);
                i += 1;
            }
            out.push((
                base,
                if extended {
                    decode_rfc2231(&joined)?
                } else {
                    joined
                },
            ));
            continue;
        } else {
            out.push((name.clone(), value.clone()));
        }
        i += 1;
    }
    Ok((key.trim().to_lowercase(), out))
}

fn decode_rfc2231(v: &str) -> Result<String, String> {
    let mut parts = v.splitn(3, '\'');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(enc), Some(_lang), Some(data)) => {
            if !enc.is_empty()
                && !matches!(
                    enc.to_lowercase().as_str(),
                    "utf-8" | "utf8" | "us-ascii" | "ascii" | "latin-1" | "iso-8859-1"
                )
            {
                return Err(format!("Invalid encoding {enc:?} for RFC 2231 param."));
            }
            let bytes: Vec<u8> = percent_decode_str(data).collect();
            Ok(
                if matches!(enc.to_lowercase().as_str(), "latin-1" | "iso-8859-1") {
                    bytes.iter().map(|&b| b as char).collect()
                } else {
                    String::from_utf8_lossy(&bytes).into_owned()
                },
            )
        }
        _ => Ok(unquote(v)),
    }
}

const USES_RELATIVE: &[&str] = &[
    "", "ftp", "http", "gopher", "nntp", "imap", "wais", "file", "https", "shttp", "mms", "prospero",
    "rtsp", "rtspu", "sftp", "svn", "svn+ssh", "ws", "wss",
];
const USES_NETLOC: &[&str] = &[
    "", "ftp", "http", "gopher", "nntp", "telnet", "imap", "wais", "file", "mms", "https", "shttp",
    "snews", "prospero", "rtsp", "rtspu", "rsync", "svn", "svn+ssh", "sftp", "nfs", "git", "git+ssh",
    "ws", "wss",
];

/// `urllib.parse.urljoin(base, url)`.
pub fn urljoin(base: &str, url: &str) -> String {
    if base.is_empty() {
        return url.to_owned();
    }
    if url.is_empty() {
        return base.to_owned();
    }
    let (Ok(b), Ok(mut u)) = (urlsplit(base), urlsplit(url)) else {
        return url.to_owned();
    };
    if u.scheme.is_empty() {
        u.scheme = b.scheme.clone();
    }
    if u.scheme != b.scheme || !USES_RELATIVE.contains(&u.scheme.as_str()) {
        return url.to_owned();
    }
    if USES_NETLOC.contains(&u.scheme.as_str()) {
        if !u.netloc.is_empty() {
            return u.unsplit();
        }
        u.netloc = b.netloc.clone();
    }
    if u.path.is_empty() {
        u.path = b.path.clone();
        if u.query.is_empty() {
            u.query = b.query.clone();
        }
        return u.unsplit();
    }
    let mut base_parts: Vec<&str> = b.path.split('/').collect();
    if base_parts.last().is_some_and(|l| !l.is_empty()) {
        base_parts.pop();
    }
    let mut segments: Vec<&str> = if u.path.starts_with('/') {
        u.path.split('/').collect()
    } else {
        let mut s = base_parts;
        s.extend(u.path.split('/'));
        let n = s.len();
        if n > 2 {
            let mut v = vec![s[0]];
            v.extend(s[1..n - 1].iter().copied().filter(|x| !x.is_empty()));
            v.push(s[n - 1]);
            s = v;
        }
        s
    };
    let last = segments.last().copied().unwrap_or("");
    let mut resolved: Vec<&str> = Vec::new();
    for seg in segments.drain(..) {
        match seg {
            ".." => {
                resolved.pop();
            }
            "." => {}
            s => resolved.push(s),
        }
    }
    if matches!(last, "." | "..") {
        resolved.push("");
    }
    let joined = resolved.join("/");
    u.path = if joined.is_empty() { "/".into() } else { joined };
    u.unsplit()
}
