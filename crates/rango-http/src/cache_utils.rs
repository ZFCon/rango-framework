//! Port of the header helpers in `utils.cache` (the cache-key half arrives with the
//! cache framework in M7).

use indexmap::IndexMap;
use rango_core::conf::settings;
use rango_core::utils::http::{http_date, parse_etags, parse_http_date_safe};

use crate::{HttpRequest, HttpResponse};

fn split_delim(s: &str) -> Vec<&str> {
    // cc_delim_re = r"\s*,\s*"
    s.split(',').map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// Value of a `Cache-Control` directive passed to [`patch_cache_control`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cc {
    /// Bare directive (`Python True`).
    Flag,
    Value(String),
}

#[derive(Clone, Debug)]
enum Entry {
    Plain(Cc),
    /// `no-cache` may carry several field names.
    NoCache(Vec<Cc>),
}

/// `patch_cache_control(response, **kwargs)`.
pub fn patch_cache_control(response: &mut HttpResponse, kwargs: &[(&str, Cc)]) {
    let mut cc: IndexMap<String, Entry> = IndexMap::new();
    if let Some(h) = response.header("Cache-Control").filter(|h| !h.is_empty()) {
        for field in split_delim(h) {
            let (directive, value) = match field.split_once('=') {
                Some((d, v)) => (d.to_lowercase(), Cc::Value(v.to_owned())),
                None => (field.to_lowercase(), Cc::Flag),
            };
            if directive == "no-cache" {
                match cc.entry(directive).or_insert_with(|| Entry::NoCache(Vec::new())) {
                    Entry::NoCache(v) => {
                        if !v.contains(&value) {
                            v.push(value)
                        }
                    }
                    Entry::Plain(_) => {}
                }
            } else {
                cc.insert(directive, Entry::Plain(value));
            }
        }
    }
    let mut kwargs: Vec<(String, Cc)> = kwargs.iter().map(|(k, v)| (k.replace('_', "-"), v.clone())).collect();
    if let (Some(Entry::Plain(Cc::Value(existing))), Some((_, Cc::Value(new)))) =
        (cc.get("max-age"), kwargs.iter().find(|(k, _)| k == "max-age"))
    {
        if let (Ok(a), Ok(b)) = (existing.parse::<i64>(), new.parse::<i64>()) {
            for (k, v) in kwargs.iter_mut() {
                if k == "max-age" {
                    *v = Cc::Value(a.min(b).to_string());
                }
            }
        }
    }
    if cc.contains_key("private") && kwargs.iter().any(|(k, _)| k == "public") {
        cc.shift_remove("private");
    } else if cc.contains_key("public") && kwargs.iter().any(|(k, _)| k == "private") {
        cc.shift_remove("public");
    }
    for (directive, v) in kwargs {
        if directive == "no-cache" {
            match cc.entry(directive).or_insert_with(|| Entry::NoCache(Vec::new())) {
                Entry::NoCache(list) => {
                    if !list.contains(&v) {
                        list.push(v)
                    }
                }
                Entry::Plain(_) => {}
            }
        } else {
            cc.insert(directive, Entry::Plain(v));
        }
    }
    let item = |d: &str, v: &Cc| match v {
        Cc::Flag => d.to_owned(),
        Cc::Value(v) => format!("{d}={v}"),
    };
    let mut parts = Vec::new();
    for (d, entry) in &cc {
        match entry {
            Entry::Plain(v) => parts.push(item(d, v)),
            Entry::NoCache(vs) => {
                if vs.contains(&Cc::Flag) {
                    parts.push(d.clone());
                } else {
                    parts.extend(vs.iter().map(|v| item(d, v)));
                }
            }
        }
    }
    let _ = response.set_header("Cache-Control", &parts.join(", "));
}

/// `get_max_age(response)`.
pub fn get_max_age(response: &HttpResponse) -> Option<i64> {
    let cc = response.header("Cache-Control")?;
    split_delim(cc).into_iter().find_map(|f| {
        let (d, v) = f.split_once('=')?;
        (d.eq_ignore_ascii_case("max-age")).then(|| v.parse().ok()).flatten()
    })
}

fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// `patch_response_headers(response, cache_timeout)`: Expires and Cache-Control.
pub fn patch_response_headers(response: &mut HttpResponse, cache_timeout: Option<i64>) {
    let timeout = cache_timeout.unwrap_or_else(|| settings().cache_middleware_seconds).max(0);
    if !response.has_header("Expires") {
        let _ = response.set_header("Expires", &http_date(Some(now_secs() + timeout as f64)));
    }
    patch_cache_control(response, &[("max_age", Cc::Value(timeout.to_string()))]);
}

/// `add_never_cache_headers(response)`.
pub fn add_never_cache_headers(response: &mut HttpResponse) {
    patch_response_headers(response, Some(-1));
    patch_cache_control(
        response,
        &[("no_cache", Cc::Flag), ("no_store", Cc::Flag), ("must_revalidate", Cc::Flag), ("private", Cc::Flag)],
    );
}

/// `patch_vary_headers(response, newheaders)`.
pub fn patch_vary_headers(response: &mut HttpResponse, newheaders: &[&str]) {
    let mut vary: Vec<String> = response.header("Vary").map(|v| split_delim(v).into_iter().map(str::to_owned).collect()).unwrap_or_default();
    let existing: Vec<String> = vary.iter().map(|h| h.to_lowercase()).collect();
    for h in newheaders {
        if !existing.contains(&h.to_lowercase()) {
            vary.push((*h).to_owned());
        }
    }
    let value = if vary.iter().any(|v| v == "*") { "*".to_owned() } else { vary.join(", ") };
    let _ = response.set_header("Vary", &value);
}

/// `has_vary_header(response, header_query)`.
pub fn has_vary_header(response: &HttpResponse, header_query: &str) -> bool {
    response
        .header("Vary")
        .is_some_and(|v| split_delim(v).iter().any(|h| h.trim().to_lowercase() == header_query.trim().to_lowercase()))
}

fn not_modified(response: Option<HttpResponse>) -> HttpResponse {
    let mut new = HttpResponse::not_modified();
    if let Some(r) = response {
        for h in ["Cache-Control", "Content-Location", "Date", "ETag", "Expires", "Last-Modified", "Vary"] {
            if let Some(v) = r.header(h) {
                let _ = new.set_header(h, v);
            }
        }
        new.cookies = r.cookies;
    }
    new
}

fn precondition_failed() -> HttpResponse {
    HttpResponse::with_status("", 412).expect("valid status")
}

/// `get_conditional_response(request, etag, last_modified, response)`: apply `If-*` headers.
/// Returns the (possibly replaced) response; `None` only if none was given and nothing applied.
pub fn get_conditional_response(
    request: &HttpRequest,
    etag: Option<&str>,
    last_modified: Option<i64>,
    response: Option<HttpResponse>,
) -> Option<HttpResponse> {
    if let Some(r) = &response {
        if !(200..300).contains(&r.status_code) {
            return response;
        }
    }
    let meta = |k: &str| request.meta().get(k).map(String::as_str);
    let if_match = parse_etags(meta("HTTP_IF_MATCH").unwrap_or(""));
    let if_unmodified = meta("HTTP_IF_UNMODIFIED_SINCE").and_then(parse_http_date_safe);
    let if_none_match = parse_etags(meta("HTTP_IF_NONE_MATCH").unwrap_or(""));
    let if_modified = meta("HTTP_IF_MODIFIED_SINCE").and_then(parse_http_date_safe);
    let is_get = matches!(request.method(), "GET" | "HEAD");

    if !if_match.is_empty() {
        let passes = match etag {
            None | Some("") => false,
            Some(_) if if_match == ["*"] => true,
            Some(e) if e.starts_with("W/") => false,
            Some(e) => if_match.iter().any(|x| x == e),
        };
        if !passes {
            return Some(precondition_failed());
        }
    } else if let Some(ius) = if_unmodified {
        if !last_modified.is_some_and(|lm| lm <= ius) {
            return Some(precondition_failed());
        }
    }
    if !if_none_match.is_empty() {
        let passes = match etag {
            None | Some("") => true,
            Some(_) if if_none_match == ["*"] => false,
            Some(e) => {
                let t = e.trim_matches(['W', '/']);
                !if_none_match.iter().any(|x| x.trim_matches(['W', '/']) == t)
            }
        };
        if !passes {
            return Some(if is_get { not_modified(response) } else { precondition_failed() });
        }
    } else if let Some(ims) = if_modified {
        if !last_modified.is_none_or(|lm| lm > ims) && is_get {
            return Some(not_modified(response));
        }
    }
    response
}
