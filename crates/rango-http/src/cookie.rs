//! Cookie parsing (`parse_cookie`) and `Set-Cookie` serialization (`SimpleCookie` morsels).

use indexmap::IndexMap;
use rango_core::utils::http::http_date;

/// `http.cookie.parse_cookie`: lenient, never fails.
pub fn parse_cookie(header: &str) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    for chunk in header.split(';') {
        let (key, val) = match chunk.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            // Assume an empty name per https://bugzilla.mozilla.org/show_bug.cgi?id=169091
            None => ("", chunk.trim()),
        };
        if !key.is_empty() || !val.is_empty() {
            out.insert(key.to_owned(), unquote_cookie(val));
        }
    }
    out
}

/// `http.cookies._unquote`.
fn unquote_cookie(v: &str) -> String {
    let v = match v.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        Some(inner) if v.len() >= 2 => inner,
        _ => return v.to_owned(),
    };
    let mut out = String::new();
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let d: String = chars.clone().take(3).collect();
            if d.len() == 3 && d.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                if let Ok(n) = u8::from_str_radix(&d, 8) {
                    out.push(n as char);
                    chars.nth(2);
                    continue;
                }
            }
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A cookie to send (`Morsel`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Morsel {
    pub value: String,
    pub expires: Option<String>,
    pub path: String,
    pub comment: String,
    pub domain: String,
    pub max_age: Option<i64>,
    pub secure: bool,
    pub httponly: bool,
    pub samesite: String,
}

/// Options of `HttpResponse.set_cookie`.
#[derive(Debug, Clone)]
pub struct CookieOptions {
    pub max_age: Option<i64>,
    /// Absolute expiry as seconds since the epoch.
    pub expires: Option<i64>,
    pub path: String,
    pub domain: Option<String>,
    pub secure: bool,
    pub httponly: bool,
    pub samesite: Option<String>,
}

impl Default for CookieOptions {
    fn default() -> Self {
        Self { max_age: None, expires: None, path: "/".into(), domain: None, secure: false, httponly: false, samesite: None }
    }
}

impl Morsel {
    /// Build per `HttpResponseBase.set_cookie`; `Err` for an invalid `samesite`.
    pub fn new(value: &str, o: &CookieOptions) -> Result<Self, String> {
        let mut m = Morsel { value: value.to_owned(), path: o.path.clone(), ..Morsel::default() };
        if let Some(ts) = o.expires {
            m.expires = Some(http_date(Some(ts as f64)));
            if let Some(age) = o.max_age {
                m.max_age = Some(age);
            }
        } else if let Some(age) = o.max_age {
            m.max_age = Some(age);
            m.expires = Some(http_date(Some(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()) + age as f64,
            )));
        }
        m.domain = o.domain.clone().unwrap_or_default();
        m.secure = o.secure;
        m.httponly = o.httponly;
        if let Some(s) = &o.samesite {
            if !["lax", "none", "strict"].contains(&s.to_lowercase().as_str()) {
                return Err("samesite must be \"lax\", \"none\", or \"strict\".".into());
            }
            m.samesite = s.clone();
        }
        Ok(m)
    }

    /// `output(header="Set-Cookie")` value for cookie `key`.
    pub fn to_header(&self, key: &str) -> String {
        let mut parts = vec![format!("{key}={}", quote_cookie_value(&self.value))];
        if let Some(e) = &self.expires {
            parts.push(format!("expires={e}"));
        }
        if !self.path.is_empty() {
            parts.push(format!("Path={}", self.path));
        }
        if !self.domain.is_empty() {
            parts.push(format!("Domain={}", self.domain));
        }
        if let Some(a) = self.max_age {
            parts.push(format!("Max-Age={a}"));
        }
        if self.secure {
            parts.push("Secure".into());
        }
        if self.httponly {
            parts.push("HttpOnly".into());
        }
        if !self.samesite.is_empty() {
            parts.push(format!("SameSite={}", self.samesite));
        }
        parts.join("; ")
    }
}

/// `http.cookies._quote`: quote values with characters outside the legal cookie set.
pub fn quote_cookie_value(v: &str) -> String {
    const LEGAL: &str = "!#$%&'*+-.^_`|~:";
    if v.chars().all(|c| c.is_ascii_alphanumeric() || LEGAL.contains(c)) {
        return v.to_owned();
    }
    let mut out = String::from("\"");
    for b in v.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e if b != b',' && b != b';' => out.push(b as char),
            b => out.push_str(&format!("\\{b:03o}")),
        }
    }
    out.push('"');
    out
}
