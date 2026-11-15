//! `middleware.csrf`: cross-site request forgery protection.
//!
//! Per-request CSRF state (the reference implementation keeps it in `request.META`) lives in a [`CsrfState`]
//! attached to the request. `CSRF_USE_SESSIONS` arrives with the sessions app.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rango_core::conf::{settings, try_settings};
use rango_core::exceptions::{Error, Result};
use rango_core::utils::crypto::{constant_time_compare, get_random_string};
use rango_core::utils::http::{is_same_domain, urlsplit};
use rango_http::cache_utils::patch_vary_headers;
use rango_http::{CookieOptions, HttpRequest, HttpResponse};
use rango_urls::{IntoView, View, ViewArgs};
use rango_views::defaults::{csrf_failure, reasons};

use crate::handlers::Middleware;

pub const CSRF_SECRET_LENGTH: usize = 32;
pub const CSRF_TOKEN_LENGTH: usize = 2 * CSRF_SECRET_LENGTH;
const CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const REASON_CSRF_TOKEN_INCORRECT: &str = "CSRF token incorrect.";
const REASON_INCORRECT_LENGTH: &str = "has incorrect length";
const REASON_INVALID_CHARACTERS: &str = "has invalid characters";

#[derive(Default)]
struct Inner {
    secret: Option<String>,
    needs_update: bool,
    processing_done: bool,
    dont_enforce: bool,
}

/// Per-request CSRF state (`request.META["CSRF_COOKIE"]`, `csrf_processing_done`, ...).
#[derive(Default)]
pub struct CsrfState(Mutex<Inner>);

impl CsrfState {
    /// Mechanism to turn off CSRF checks (the test client's `enforce_csrf_checks=False`).
    pub fn dont_enforce_checks(&self) {
        self.0.lock().expect("csrf lock").dont_enforce = true;
    }
}

/// The request's CSRF state, created on first use.
pub fn csrf_state(request: &HttpRequest) -> Arc<CsrfState> {
    if let Some(s) = request.ext::<CsrfState>() {
        return s;
    }
    request.set_ext(CsrfState::default());
    request.ext::<CsrfState>().expect("just set")
}

fn new_csrf_string() -> String {
    get_random_string(CSRF_SECRET_LENGTH, CHARS)
}

fn idx(c: char) -> usize {
    CHARS.find(c).expect("validated csrf character")
}

fn mask_cipher_secret(secret: &str) -> String {
    let mask = new_csrf_string();
    let n = CHARS.len();
    let cipher: String =
        secret.chars().zip(mask.chars()).map(|(x, y)| CHARS.as_bytes()[(idx(x) + idx(y)) % n] as char).collect();
    format!("{mask}{cipher}")
}

fn unmask_cipher_token(token: &str) -> String {
    let (mask, token) = token.split_at(CSRF_SECRET_LENGTH);
    let n = CHARS.len() as i64;
    token
        .chars()
        .zip(mask.chars())
        .map(|(x, y)| CHARS.as_bytes()[(idx(x) as i64 - idx(y) as i64).rem_euclid(n) as usize] as char)
        .collect()
}

fn check_token_format(token: &str) -> std::result::Result<(), String> {
    if token.len() != CSRF_TOKEN_LENGTH && token.len() != CSRF_SECRET_LENGTH {
        return Err(REASON_INCORRECT_LENGTH.into());
    }
    if let Some(bad) = token.chars().find(|c| !CHARS.contains(*c)) {
        return Err(format!("{REASON_INVALID_CHARACTERS} ({bad:?})"));
    }
    Ok(())
}

fn does_token_match(request_token: &str, secret: &str) -> bool {
    let t = if request_token.len() == CSRF_TOKEN_LENGTH { unmask_cipher_token(request_token) } else { request_token.to_owned() };
    constant_time_compare(t, secret)
}

fn add_new_csrf_cookie(request: &HttpRequest) -> String {
    let secret = new_csrf_string();
    let st = csrf_state(request);
    let mut i = st.0.lock().expect("csrf lock");
    i.secret = Some(secret.clone());
    i.needs_update = true;
    secret
}

/// `get_token(request)`: a fresh masked token to embed in a form (what `{% csrf_token %}` uses).
pub fn get_token(request: &HttpRequest) -> String {
    let st = csrf_state(request);
    let existing = {
        let mut i = st.0.lock().expect("csrf lock");
        if i.secret.is_some() {
            i.needs_update = true; // renew the cookie expiry
        }
        i.secret.clone()
    };
    mask_cipher_secret(&existing.unwrap_or_else(|| add_new_csrf_cookie(request)))
}

/// `rotate_token(request)`: change the token on login so a pre-login token cannot be reused.
pub fn rotate_token(request: &HttpRequest) {
    add_new_csrf_cookie(request);
}

#[derive(Default)]
pub struct CsrfViewMiddleware {
    /// Decorator variants that only create the token / never reject.
    pub never_reject: bool,
}

struct Origins {
    exact: Vec<String>,
    subdomains: Vec<(String, String)>, // (scheme, host)
    hosts: Vec<String>,
}

fn origins() -> Origins {
    let s = settings();
    let mut o = Origins { exact: vec![], subdomains: vec![], hosts: vec![] };
    for origin in &s.csrf_trusted_origins {
        let netloc = urlsplit(origin).map(|u| u.netloc).unwrap_or_default();
        o.hosts.push(netloc.trim_start_matches('*').to_owned());
        if origin.contains('*') {
            if let Ok(u) = urlsplit(origin) {
                o.subdomains.push((u.scheme, u.netloc.trim_start_matches('*').to_owned()));
            }
        } else {
            o.exact.push(origin.clone());
        }
    }
    o
}

fn origin_verified(request: &HttpRequest, o: &Origins) -> bool {
    let request_origin = request.meta().get("HTTP_ORIGIN").cloned().unwrap_or_default();
    if let Ok(good_host) = request.get_host() {
        let good_origin = format!("{}://{good_host}", if request.is_secure() { "https" } else { "http" });
        if request_origin == good_origin {
            return true;
        }
    }
    if o.exact.contains(&request_origin) {
        return true;
    }
    let Ok(parsed) = urlsplit(&request_origin) else { return false };
    o.subdomains.iter().any(|(scheme, host)| *scheme == parsed.scheme && is_same_domain(&parsed.netloc, host))
}

fn check_referer(request: &HttpRequest, o: &Origins) -> std::result::Result<(), String> {
    let Some(referer) = request.meta().get("HTTP_REFERER") else { return Err(reasons::NO_REFERER.into()) };
    let r = urlsplit(referer).map_err(|_| reasons::MALFORMED_REFERER.to_owned())?;
    if r.scheme.is_empty() || r.netloc.is_empty() {
        return Err(reasons::MALFORMED_REFERER.into());
    }
    if r.scheme != "https" {
        return Err(reasons::INSECURE_REFERER.into());
    }
    if o.subdomains.iter().any(|(s, h)| s == "https" && is_same_domain(&r.netloc, h)) {
        return Ok(());
    }
    let s = settings();
    let good_referer = match &s.csrf_cookie_domain {
        None => request.get_host().map_err(|_| reasons::BAD_REFERER.replace("%s", &r.unsplit()))?,
        Some(d) => {
            let port = request.get_port();
            if port != "443" && port != "80" { format!("{d}:{port}") } else { d.clone() }
        }
    };
    let mut good_hosts = o.hosts.clone();
    good_hosts.push(good_referer);
    if !good_hosts.iter().any(|h| is_same_domain(&r.netloc, h)) {
        return Err(reasons::BAD_REFERER.replace("%s", &r.unsplit()));
    }
    Ok(())
}

impl CsrfViewMiddleware {
    fn secret_from_cookie(request: &HttpRequest) -> Result<Option<String>> {
        let s = settings();
        if s.csrf_use_sessions {
            return Err(Error::improperly_configured("CSRF_USE_SESSIONS requires the sessions app (not enabled in this build)."));
        }
        let Some(secret) = request.cookies().get(&s.csrf_cookie_name) else { return Ok(None) };
        if check_token_format(secret).is_err() {
            return Err(Error::BadRequest("invalid csrf cookie".into()));
        }
        Ok(Some(if secret.len() == CSRF_TOKEN_LENGTH { unmask_cipher_token(secret) } else { secret.clone() }))
    }

    fn check_token(request: &HttpRequest, post_token: Option<String>) -> std::result::Result<(), String> {
        let st = csrf_state(request);
        let secret = st.0.lock().expect("csrf lock").secret.clone();
        let Some(secret) = secret else { return Err(reasons::NO_CSRF_COOKIE.into()) };
        let mut token = post_token.unwrap_or_default();
        if token.is_empty() {
            let header = settings().csrf_header_name.clone();
            token = request.meta().get(&header).cloned().unwrap_or_default();
        }
        if let Err(why) = check_token_format(&token) {
            return Err(if token.is_empty() { reasons::CSRF_TOKEN_MISSING.to_owned() } else { format!("CSRF token {why}.") });
        }
        if !does_token_match(&token, &secret) {
            return Err(REASON_CSRF_TOKEN_INCORRECT.into());
        }
        Ok(())
    }

    fn reject(&self, request: &HttpRequest, reason: &str) -> Option<HttpResponse> {
        if self.never_reject {
            return None;
        }
        let response = csrf_failure(request, reason);
        tracing::warn!(target: "rango.security.csrf", "Forbidden ({reason}): {}", request.path());
        Some(response)
    }

    async fn view_check(&self, request: &HttpRequest, view: &View) -> Result<Option<HttpResponse>> {
        let st = csrf_state(request);
        if st.0.lock().expect("csrf lock").processing_done || view.flags.csrf_exempt {
            return Ok(None);
        }
        let accept = |st: &CsrfState| {
            st.0.lock().expect("csrf lock").processing_done = true;
        };
        if matches!(request.method(), "GET" | "HEAD" | "OPTIONS" | "TRACE") {
            accept(&st);
            return Ok(None);
        }
        if st.0.lock().expect("csrf lock").dont_enforce {
            accept(&st);
            return Ok(None);
        }
        let o = origins();
        if let Some(origin) = request.meta().get("HTTP_ORIGIN") {
            if !origin_verified(request, &o) {
                return Ok(self.reject(request, &reasons::BAD_ORIGIN.replace("%s", origin)));
            }
        } else if request.is_secure() {
            if let Err(reason) = check_referer(request, &o) {
                return Ok(self.reject(request, &reason));
            }
        }
        let post_token = if request.method() == "POST" {
            match request.post().await {
                Ok(p) => p.get("csrfmiddlewaretoken").map(str::to_owned),
                Err(_) => return Ok(self.reject(request, reasons::CSRF_TOKEN_MISSING)),
            }
        } else {
            None
        };
        match Self::check_token(request, post_token) {
            Ok(()) => {
                accept(&st);
                Ok(None)
            }
            Err(reason) => Ok(self.reject(request, &reason)),
        }
    }

    fn set_cookie(request: &HttpRequest, response: &mut HttpResponse) -> Result<()> {
        let s = try_settings()?;
        let st = csrf_state(request);
        let secret = st.0.lock().expect("csrf lock").secret.clone();
        if let Some(secret) = secret {
            response.set_cookie(
                &s.csrf_cookie_name,
                &secret,
                &CookieOptions {
                    max_age: Some(s.csrf_cookie_age),
                    path: s.csrf_cookie_path.clone(),
                    domain: s.csrf_cookie_domain.clone(),
                    secure: s.csrf_cookie_secure,
                    httponly: s.csrf_cookie_httponly,
                    samesite: Some(s.csrf_cookie_samesite.clone()).filter(|v| !v.is_empty()),
                    expires: None,
                },
            )?;
            patch_vary_headers(response, &["Cookie"]);
        }
        Ok(())
    }
}

#[async_trait]
impl Middleware for CsrfViewMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        match Self::secret_from_cookie(request) {
            Ok(Some(secret)) => csrf_state(request).0.lock().expect("csrf lock").secret = Some(secret),
            Ok(None) => {}
            Err(Error::BadRequest(_)) => {
                add_new_csrf_cookie(request);
            }
            Err(e) => return Err(e),
        }
        Ok(None)
    }

    async fn process_view(&self, request: &HttpRequest, view: &View, _args: &ViewArgs) -> Result<Option<HttpResponse>> {
        self.view_check(request, view).await
    }

    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        let st = csrf_state(request);
        let needs = std::mem::take(&mut st.0.lock().expect("csrf lock").needs_update);
        if needs {
            Self::set_cookie(request, &mut response)?;
        }
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.csrf.CsrfViewMiddleware", || Ok(Arc::new(CsrfViewMiddleware::default())));

/// `decorator_from_middleware`: run a middleware's request/view/response hooks around one view.
pub fn decorator_from_middleware<M>(mw: Arc<dyn Middleware>, view: impl IntoView<M>) -> View {
    view.into_view().wrap(move |inner, req, args| {
        let mw = Arc::clone(&mw);
        Box::pin(async move {
            let response = match mw.process_request(&req).await? {
                Some(r) => r,
                None => match mw.process_view(&req, &inner, &args).await? {
                    Some(r) => r,
                    None => inner.call(req.clone(), args).await?,
                },
            };
            mw.process_response(&req, response).await
        })
    })
}

/// `csrf_protect`: enforce CSRF checks for this view only.
pub fn csrf_protect<M>(view: impl IntoView<M>) -> View {
    decorator_from_middleware(Arc::new(CsrfViewMiddleware::default()), view)
}

/// `requires_csrf_token`: make `get_token()` work, never reject.
pub fn requires_csrf_token<M>(view: impl IntoView<M>) -> View {
    decorator_from_middleware(Arc::new(CsrfViewMiddleware { never_reject: true }), view)
}

/// `ensure_csrf_cookie`: always send the CSRF cookie.
pub fn ensure_csrf_cookie<M>(view: impl IntoView<M>) -> View {
    requires_csrf_token(view).wrap(|inner, req, args| {
        Box::pin(async move {
            get_token(&req);
            inner.call(req, args).await
        })
    })
}

/// `template.context_processors.csrf`: a lazy `csrf_token` (the cookie is only set if it is used).
fn csrf_context_processor(request: &HttpRequest) -> indexmap::IndexMap<String, rango_template::Value> {
    let req = request.clone();
    let token: rango_template::value::Callable = Arc::new(move || rango_template::Value::Str(get_token(&req)));
    [("csrf_token".to_owned(), rango_template::Value::Callable(token))].into_iter().collect()
}

rango_template::register_context_processor!("rango.template.context_processors.csrf", csrf_context_processor);
