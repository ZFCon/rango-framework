//! `http.response`: `HttpResponse`, `StreamingHttpResponse`, `FileResponse`,
//! `JsonResponse` and the status-specific subclasses (as constructors).

use std::io;
use std::path::Path;

use bytes::Bytes;
use futures_util::stream::{self, BoxStream, StreamExt};
use indexmap::IndexMap;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result, SuspiciousKind};
use rango_core::signing;
use rango_core::utils::encoding::iri_to_uri;
use rango_core::utils::http::{content_disposition_header, urlsplit};
use serde::Serialize;

use crate::cookie::{CookieOptions, Morsel};
use crate::headers::Headers;

/// Response payload.
pub enum Body {
    Bytes(Bytes),
    /// `StreamingHttpResponse`: chunks produced lazily; `Content-Length` is not known.
    Stream(BoxStream<'static, io::Result<Bytes>>),
}

pub struct HttpResponse {
    pub status_code: u16,
    reason_phrase: Option<String>,
    pub headers: Headers,
    pub cookies: IndexMap<String, Morsel>,
    body: Body,
    charset: Option<String>,
    pub closed: bool,
    /// Free-form markers set by decorators (the reference implementation sets attributes such as `xframe_options_exempt`).
    pub flags: std::collections::HashSet<String>,
    closers: Vec<Box<dyn FnOnce() + Send>>,
    /// `SimpleTemplateResponse`: content produced later by [`DeferredRender::render`].
    deferred: Option<Box<dyn DeferredRender>>,
}

/// Lazily rendered response body (`template.response.SimpleTemplateResponse`).
/// Middleware may inspect/modify it (via [`HttpResponse::deferred_mut`]) before the handler renders it.
pub trait DeferredRender: Send + Sync {
    /// Produce the content and set it on `response`.
    fn render(&mut self, response: &mut HttpResponse) -> Result<()>;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<HttpResponse status_code={}, {:?}>",
            self.status_code,
            self.headers.get("Content-Type").unwrap_or("")
        )
    }
}

fn default_content_type() -> String {
    format!("text/html; charset={}", settings().default_charset)
}

impl HttpResponse {
    /// `HttpResponse(content)`: 200, `text/html; charset=DEFAULT_CHARSET`.
    pub fn new(content: impl Into<Bytes>) -> Self {
        let mut headers = Headers::new();
        let _ = headers.set("Content-Type", &default_content_type());
        Self {
            status_code: 200,
            reason_phrase: None,
            headers,
            cookies: IndexMap::new(),
            body: Body::Bytes(content.into()),
            charset: None,
            closed: false,
            flags: std::collections::HashSet::new(),
            closers: Vec::new(),
            deferred: None,
        }
    }

    /// Attach a deferred renderer; `content()` fails until [`render_deferred`](Self::render_deferred) ran.
    pub fn with_deferred(mut self, d: impl DeferredRender + 'static) -> Self {
        self.deferred = Some(Box::new(d));
        self
    }

    /// `hasattr(response, "render")` and not yet rendered.
    pub fn has_deferred(&self) -> bool {
        self.deferred.is_some()
    }

    /// The pending deferred renderer downcast to `T` (e.g. `TemplateResponse`).
    pub fn deferred_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.deferred.as_mut()?.as_any_mut().downcast_mut::<T>()
    }

    /// `response.render()`: run the deferred renderer once.
    pub fn render_deferred(&mut self) -> Result<()> {
        if let Some(mut d) = self.deferred.take() {
            d.render(self)?;
        }
        Ok(())
    }

    pub fn with_status(content: impl Into<Bytes>, status: u16) -> Result<Self> {
        let mut r = Self::new(content);
        r.set_status_code(status)?;
        Ok(r)
    }

    /// `StreamingHttpResponse(stream)`.
    pub fn streaming(stream: impl futures_util::Stream<Item = io::Result<Bytes>> + Send + 'static) -> Self {
        let mut r = Self::new(Bytes::new());
        r.body = Body::Stream(stream.boxed());
        r
    }

    /// Set `Content-Type` (builder style).
    pub fn content_type(mut self, content_type: &str) -> Self {
        let _ = self.headers.set("Content-Type", content_type);
        self
    }

    pub fn status(mut self, status: u16) -> Result<Self> {
        self.set_status_code(status)?;
        Ok(self)
    }

    pub fn set_status_code(&mut self, status: u16) -> Result<()> {
        if !(100..=599).contains(&status) {
            return Err(Error::Other("HTTP status code must be an integer from 100 to 599.".into()));
        }
        self.status_code = status;
        Ok(())
    }

    pub fn set_reason_phrase(&mut self, phrase: &str) {
        self.reason_phrase = Some(phrase.to_owned());
    }

    /// `reason_phrase`: custom, else the standard phrase, else "Unknown Status Code".
    pub fn reason_phrase(&self) -> String {
        self.reason_phrase.clone().unwrap_or_else(|| {
            http::StatusCode::from_u16(self.status_code)
                .ok()
                .and_then(|s| s.canonical_reason())
                .unwrap_or("Unknown Status Code")
                .to_owned()
        })
    }

    /// `charset`: explicit, else from `Content-Type`, else `DEFAULT_CHARSET`.
    pub fn charset(&self) -> String {
        if let Some(c) = &self.charset {
            return c.clone();
        }
        if let Some(ct) = self.headers.get("Content-Type") {
            if let Some(idx) = ct.to_lowercase().find("charset=") {
                let v = ct[idx + 8..].split(';').next().unwrap_or("").trim().trim_matches(['"', '\'']);
                if !v.is_empty() {
                    return v.to_owned();
                }
            }
        }
        settings().default_charset.clone()
    }

    pub fn set_charset(&mut self, charset: &str) {
        self.charset = Some(charset.to_owned());
    }

    pub fn is_streaming(&self) -> bool {
        matches!(self.body, Body::Stream(_))
    }

    /// `response.content` (not available on streaming responses).
    pub fn content(&self) -> Result<&Bytes> {
        if self.deferred.is_some() {
            return Err(Error::Other("The response content must be rendered before it can be accessed.".into()));
        }
        match &self.body {
            Body::Bytes(b) => Ok(b),
            Body::Stream(_) => Err(Error::Other("This StreamingHttpResponse instance has no `content` attribute. Use `streaming_content` instead.".into())),
        }
    }

    /// Content decoded with [`charset`](Self::charset) (lossy).
    pub fn text(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.content()?).into_owned())
    }

    pub fn set_content(&mut self, content: impl Into<Bytes>) {
        self.body = Body::Bytes(content.into());
    }

    /// `write()`: append to the content.
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        match &mut self.body {
            Body::Bytes(b) => {
                let mut v = b.to_vec();
                v.extend_from_slice(data);
                *b = Bytes::from(v);
                Ok(())
            }
            Body::Stream(_) => Err(Error::Other("This StreamingHttpResponse instance is not writable".into())),
        }
    }

    /// Take the body (to hand to a server).
    pub fn into_body(self) -> (u16, String, Headers, IndexMap<String, Morsel>, Body, Vec<Box<dyn FnOnce() + Send>>) {
        let reason = self.reason_phrase();
        (self.status_code, reason, self.headers, self.cookies, self.body, self.closers)
    }

    pub fn body_mut(&mut self) -> &mut Body {
        &mut self.body
    }

    /// Register something to run on `close()` (`_resource_closers`).
    pub fn on_close(&mut self, f: impl FnOnce() + Send + 'static) {
        self.closers.push(Box::new(f));
    }

    /// `close()`.
    pub fn close(&mut self) {
        for f in self.closers.drain(..) {
            f();
        }
        self.closed = true;
    }

    pub fn has_header(&self, name: &str) -> bool {
        self.headers.contains(name)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)
    }

    /// `response[name] = value`.
    pub fn set_header(&mut self, name: &str, value: &str) -> Result<()> {
        self.headers.set(name, value).map_err(|e| Error::Other(Box::new(e)))
    }

    /// `serialize_headers()`.
    pub fn serialize_headers(&self) -> Vec<u8> {
        let mut out = String::new();
        for (k, v) in self.headers.iter() {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        for (k, m) in &self.cookies {
            out.push_str(&format!("Set-Cookie: {}\r\n", m.to_header(k)));
        }
        out.into_bytes()
    }

    /// `set_cookie()`.
    pub fn set_cookie(&mut self, key: &str, value: &str, options: &CookieOptions) -> Result<()> {
        let mut opts = options.clone();
        if key.starts_with("__Secure-") || key.starts_with("__Host-") {
            opts.secure = true;
        }
        let m = Morsel::new(value, &opts).map_err(|e| Error::Other(e.into()))?;
        self.cookies.insert(key.to_owned(), m);
        Ok(())
    }

    /// `set_signed_cookie()`.
    pub fn set_signed_cookie(&mut self, key: &str, value: &str, salt: &str, options: &CookieOptions) -> Result<()> {
        let signed = signing::sign_cookie(value, key, salt)?;
        self.set_cookie(key, &signed, options)
    }

    /// `delete_cookie()`: expire the cookie in the browser.
    pub fn delete_cookie(&mut self, key: &str, path: &str, domain: Option<&str>, samesite: Option<&str>) -> Result<()> {
        let secure = key.starts_with("__Secure-") || key.starts_with("__Host-");
        self.set_cookie(
            key,
            "",
            &CookieOptions {
                max_age: Some(0),
                expires: Some(0),
                path: path.to_owned(),
                domain: domain.map(str::to_owned),
                secure,
                httponly: false,
                samesite: samesite.map(str::to_owned),
            },
        )
    }

    // ---- status-specific responses (HttpResponseRedirect, HttpResponseNotFound, ...) ----

    fn redirect_with(url: &str, status: u16) -> Result<Self> {
        let scheme = urlsplit(url).map(|u| u.scheme).unwrap_or_default();
        if !scheme.is_empty() && !["http", "https", "ftp"].contains(&scheme.as_str()) {
            return Err(Error::suspicious(SuspiciousKind::DisallowedRedirect, format!("Unsafe redirect to URL with protocol '{scheme}'")));
        }
        let mut r = Self::with_status(Bytes::new(), status)?;
        r.set_header("Location", &iri_to_uri(url))?;
        Ok(r)
    }

    /// `HttpResponseRedirect` (302).
    pub fn redirect(url: &str) -> Result<Self> {
        Self::redirect_with(url, 302)
    }

    /// `HttpResponsePermanentRedirect` (301).
    pub fn permanent_redirect(url: &str) -> Result<Self> {
        Self::redirect_with(url, 301)
    }

    /// `HttpResponseNotModified` (304, no Content-Type).
    pub fn not_modified() -> Self {
        let mut r = Self::new(Bytes::new());
        r.status_code = 304;
        r.headers.remove("Content-Type");
        r
    }

    fn status_only(status: u16, content: impl Into<Bytes>) -> Self {
        let mut r = Self::new(content);
        r.status_code = status;
        r
    }

    pub fn bad_request(content: impl Into<Bytes>) -> Self {
        Self::status_only(400, content)
    }
    pub fn forbidden(content: impl Into<Bytes>) -> Self {
        Self::status_only(403, content)
    }
    pub fn not_found(content: impl Into<Bytes>) -> Self {
        Self::status_only(404, content)
    }
    pub fn gone(content: impl Into<Bytes>) -> Self {
        Self::status_only(410, content)
    }
    pub fn server_error(content: impl Into<Bytes>) -> Self {
        Self::status_only(500, content)
    }

    /// `HttpResponseNotAllowed(permitted_methods)` (405 with `Allow`).
    pub fn not_allowed(permitted_methods: &[&str], content: impl Into<Bytes>) -> Self {
        let mut r = Self::status_only(405, content);
        let _ = r.headers.set("Allow", &permitted_methods.join(", "));
        r
    }

    /// `HttpResponseRedirectBase.url`.
    pub fn url(&self) -> Option<&str> {
        self.headers.get("Location")
    }
}

/// `JsonResponse`.
pub struct JsonResponse;

impl JsonResponse {
    /// `JsonResponse(data, safe=True)`: non-dict top-level values need `safe = false`.
    pub fn new<T: Serialize + ?Sized>(data: &T, safe: bool) -> Result<HttpResponse> {
        let value = serde_json::to_value(data).map_err(|e| Error::Other(Box::new(e)))?;
        if safe && !value.is_object() {
            return Err(Error::Other("In order to allow non-dict objects to be serialized set the safe parameter to False.".into()));
        }
        let body = rango_core::utils::json::dumps(&value, false).map_err(|e| Error::Other(Box::new(e)))?;
        Ok(HttpResponse::new(body).content_type("application/json"))
    }
}

/// `FileResponse`.
pub struct FileResponse;

impl FileResponse {
    const BLOCK_SIZE: usize = 64 * 1024;

    /// Stream a file; sets Content-Length, Content-Type (guessed), Content-Encoding for
    /// compressed extensions, and Content-Disposition when `as_attachment` or `filename` is given.
    pub async fn open(path: impl AsRef<Path>, as_attachment: bool, filename: Option<&str>) -> io::Result<HttpResponse> {
        let path = path.as_ref();
        let file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        let name = filename.map(str::to_owned).or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();

        let stream = stream::unfold(file, |mut f| async move {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; Self::BLOCK_SIZE];
            match f.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    Some((Ok(Bytes::from(buf)), f))
                }
                Err(e) => Some((Err(e), f)),
            }
        });
        let mut r = HttpResponse::streaming(stream);

        let lower = name.to_lowercase();
        let (guess_name, encoding) = [("gz", "gzip"), ("br", "br"), ("bz2", "bzip2"), ("xz", "xz"), ("Z", "compress")]
            .iter()
            .find_map(|(ext, enc)| lower.strip_suffix(&format!(".{ext}")).map(|stem| (stem.to_owned(), Some(*enc))))
            .unwrap_or((lower.clone(), None));
        let content_type = match encoding {
            _ if lower.ends_with(".tar.gz") => "application/gzip".to_owned(),
            _ => mime_guess::from_path(&guess_name).first().map_or_else(|| "application/octet-stream".to_owned(), |m| m.essence_str().to_owned()),
        };
        let _ = r.headers.set("Content-Length", &size.to_string());
        let ct = if lower.ends_with(".tar.gz") { content_type } else { content_type };
        let _ = r.headers.set("Content-Type", &ct);
        if let (Some(enc), false) = (encoding, lower.ends_with(".tar.gz")) {
            let _ = r.headers.set("Content-Encoding", enc);
        }
        if as_attachment || filename.is_some() {
            if let Some(cd) = content_disposition_header(as_attachment, Some(&name)) {
                let _ = r.headers.set("Content-Disposition", &cd);
            }
        }
        Ok(r)
    }
}
