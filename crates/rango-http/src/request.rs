//! `http.request`: `HttpRequest`.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use bytes::Bytes;
use indexmap::IndexMap;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result, SuspiciousKind};
use rango_core::signing;
use rango_core::utils::encoding::{escape_uri_path, iri_to_uri};
use rango_core::utils::http::{is_same_domain, parse_header_parameters, unquote, urljoin, urlsplit};
use regex::Regex;
use tokio::sync::OnceCell;

use crate::cookie::parse_cookie;
use crate::headers::Headers;
use crate::querydict::QueryDict;
use crate::uploads::{Files, parse_multipart};

/// Typemap for things middleware attaches to a request (`request.user`, `request.session`, ...).
#[derive(Default)]
struct Extensions(HashMap<TypeId, Arc<dyn Any + Send + Sync>>);

pub(crate) struct Inner {
    method: String,
    path: String,
    path_info: String,
    script_name: String,
    meta: IndexMap<String, String>,
    headers: Headers,
    body: Bytes,
    query: QueryDict,
    cookies: IndexMap<String, String>,
    content_type: String,
    content_params: Vec<(String, String)>,
    encoding: RwLock<Option<String>>,
    post_files: OnceCell<(QueryDict, Files)>,
    ext: RwLock<Extensions>,
}

/// `HttpRequest`. Cheap to clone (a shared handle); attach per-request state with
/// [`set_ext`](Self::set_ext) instead of mutating attributes.
#[derive(Clone)]
pub struct HttpRequest(pub(crate) Arc<Inner>);

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<HttpRequest: {} {:?}>", self.0.method, self.get_full_path())
    }
}

/// Builder for requests (used by servers, the test client and `RequestFactory`).
#[derive(Debug, Clone)]
pub struct RequestBuilder {
    method: String,
    path_info: String,
    query_string: String,
    script_name: String,
    headers: Vec<(String, String)>,
    body: Bytes,
    scheme: String,
    server_name: String,
    server_port: u16,
    remote_addr: String,
}

impl Default for RequestBuilder {
    fn default() -> Self {
        Self {
            method: "GET".into(),
            path_info: "/".into(),
            query_string: String::new(),
            script_name: String::new(),
            headers: Vec::new(),
            body: Bytes::new(),
            scheme: "http".into(),
            server_name: "testserver".into(),
            server_port: 80,
            remote_addr: "127.0.0.1".into(),
        }
    }
}

impl RequestBuilder {
    pub fn new(method: &str, uri: &str) -> Self {
        let mut b = Self::default();
        b.method = method.to_uppercase();
        b.uri(uri);
        b
    }

    /// Set path and query string from a request target such as `/polls/?page=2`.
    pub fn uri(&mut self, uri: &str) -> &mut Self {
        let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
        self.path_info = unquote(path);
        self.query_string = query.to_owned();
        self
    }

    pub fn script_name(&mut self, s: &str) -> &mut Self {
        self.script_name = s.to_owned();
        self
    }

    pub fn header(&mut self, name: &str, value: &str) -> &mut Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn body(&mut self, body: impl Into<Bytes>) -> &mut Self {
        self.body = body.into();
        self
    }

    pub fn scheme(&mut self, scheme: &str) -> &mut Self {
        self.scheme = scheme.to_owned();
        self
    }

    pub fn server(&mut self, name: &str, port: u16) -> &mut Self {
        self.server_name = name.to_owned();
        self.server_port = port;
        self
    }

    pub fn remote_addr(&mut self, addr: &str) -> &mut Self {
        self.remote_addr = addr.to_owned();
        self
    }

    pub fn build(&self) -> Result<HttpRequest> {
        let mut meta: IndexMap<String, String> = IndexMap::new();
        meta.insert("REQUEST_METHOD".into(), self.method.clone());
        meta.insert("SCRIPT_NAME".into(), self.script_name.clone());
        meta.insert("PATH_INFO".into(), self.path_info.clone());
        meta.insert("QUERY_STRING".into(), self.query_string.clone());
        meta.insert("REMOTE_ADDR".into(), self.remote_addr.clone());
        meta.insert("SERVER_NAME".into(), self.server_name.clone());
        meta.insert("SERVER_PORT".into(), self.server_port.to_string());
        meta.insert("wsgi.url_scheme".into(), self.scheme.clone());
        meta.insert("CONTENT_LENGTH".into(), self.body.len().to_string());
        for (k, v) in &self.headers {
            let up = k.to_uppercase().replace('-', "_");
            let key = if up == "CONTENT_TYPE" || up == "CONTENT_LENGTH" { up } else { format!("HTTP_{up}") };
            // Repeated headers are comma-joined, as WSGI servers do.
            match meta.get_mut(&key) {
                Some(existing) if key.starts_with("HTTP_") => {
                    existing.push(',');
                    existing.push_str(v);
                }
                _ => {
                    meta.insert(key, v.clone());
                }
            }
        }
        HttpRequest::from_meta(meta, self.body.clone())
    }
}

impl HttpRequest {
    /// Build from WSGI-style `META` (must contain `REQUEST_METHOD`, `PATH_INFO`, `SCRIPT_NAME`).
    pub fn from_meta(meta: IndexMap<String, String>, body: Bytes) -> Result<Self> {
        let s = settings();
        let script_name = s.force_script_name.clone().unwrap_or_else(|| meta.get("SCRIPT_NAME").cloned().unwrap_or_default());
        let mut path_info = meta.get("PATH_INFO").cloned().unwrap_or_else(|| "/".into());
        if path_info.is_empty() {
            path_info = "/".into();
        }
        let path = format!("{}/{}", script_name.trim_end_matches('/'), path_info.replacen('/', "", 1));
        let headers = Headers::from_meta(&meta);
        let (content_type, params) = parse_header_parameters(meta.get("CONTENT_TYPE").map_or("", String::as_str)).map_err(Error::BadRequest)?;
        let query = QueryDict::parse(meta.get("QUERY_STRING").map_or("", String::as_str), false)?;
        let cookies = parse_cookie(meta.get("HTTP_COOKIE").map_or("", String::as_str));
        let encoding = params.iter().find(|(k, _)| k == "charset").map(|(_, v)| v.clone());
        Ok(Self(Arc::new(Inner {
            method: meta.get("REQUEST_METHOD").map_or_else(|| "GET".into(), |m| m.to_uppercase()),
            path,
            path_info,
            script_name,
            meta,
            headers,
            body,
            query,
            cookies,
            content_type,
            content_params: params,
            encoding: RwLock::new(encoding),
            post_files: OnceCell::new(),
            ext: RwLock::new(Extensions::default()),
        })))
    }

    pub fn builder(method: &str, uri: &str) -> RequestBuilder {
        RequestBuilder::new(method, uri)
    }

    pub fn method(&self) -> &str {
        &self.0.method
    }
    /// `request.path`: script name + path info.
    pub fn path(&self) -> &str {
        &self.0.path
    }
    pub fn path_info(&self) -> &str {
        &self.0.path_info
    }
    pub fn script_name(&self) -> &str {
        &self.0.script_name
    }
    /// `request.META`.
    pub fn meta(&self) -> &IndexMap<String, String> {
        &self.0.meta
    }
    pub fn headers(&self) -> &Headers {
        &self.0.headers
    }
    /// `request.GET`.
    pub fn query(&self) -> &QueryDict {
        &self.0.query
    }
    /// `request.COOKIES`.
    pub fn cookies(&self) -> &IndexMap<String, String> {
        &self.0.cookies
    }
    /// `request.content_type` (media type, lowercased).
    pub fn content_type(&self) -> &str {
        &self.0.content_type
    }
    pub fn content_params(&self) -> &[(String, String)] {
        &self.0.content_params
    }
    /// `request.encoding` (charset of the body, default `DEFAULT_CHARSET`).
    pub fn encoding(&self) -> String {
        self.0.encoding.read().expect("lock").clone().unwrap_or_else(|| settings().default_charset.clone())
    }
    pub fn set_encoding(&self, encoding: &str) {
        *self.0.encoding.write().expect("lock") = Some(encoding.to_owned());
    }

    /// `request.body`; `RequestDataTooBig` when above `DATA_UPLOAD_MAX_MEMORY_SIZE`
    /// (multipart bodies are exempt, their parts are limited individually).
    pub fn body(&self) -> Result<&Bytes> {
        let max = settings().data_upload_max_memory_size;
        if max > 0 && self.0.body.len() as i64 > max && self.content_type() != "multipart/form-data" {
            return Err(Error::suspicious(SuspiciousKind::RequestDataTooBig, "Request body exceeded settings.DATA_UPLOAD_MAX_MEMORY_SIZE."));
        }
        Ok(&self.0.body)
    }

    async fn post_files(&self) -> Result<&(QueryDict, Files)> {
        self.0
            .post_files
            .get_or_try_init(|| async {
                if self.method() != "POST" {
                    return Ok((QueryDict::new(), Files::new()));
                }
                match self.content_type() {
                    "multipart/form-data" => {
                        let boundary = self
                            .content_params()
                            .iter()
                            .find(|(k, _)| k == "boundary")
                            .map(|(_, v)| v.clone())
                            .ok_or_else(|| Error::BadRequest("Invalid boundary in multipart: None".into()))?;
                        parse_multipart(self.0.body.clone(), &boundary, &self.encoding()).await
                    }
                    "application/x-www-form-urlencoded" => {
                        let text = String::from_utf8_lossy(self.body()?).into_owned();
                        Ok((QueryDict::parse(&text, false)?, Files::new()))
                    }
                    _ => Ok((QueryDict::new(), Files::new())),
                }
            })
            .await
    }

    /// `request.POST` (parsed on first use).
    pub async fn post(&self) -> Result<&QueryDict> {
        Ok(&self.post_files().await?.0)
    }

    /// `request.FILES`.
    pub async fn files(&self) -> Result<&Files> {
        Ok(&self.post_files().await?.1)
    }

    /// Attach per-request state (what the reference implementation does with `request.user = ...`).
    pub fn set_ext<T: Any + Send + Sync>(&self, value: T) {
        self.0.ext.write().expect("lock").0.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// Read state attached with [`set_ext`](Self::set_ext).
    pub fn ext<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.0.ext.read().expect("lock").0.get(&TypeId::of::<T>()).and_then(|v| Arc::clone(v).downcast::<T>().ok())
    }

    fn raw_host(&self) -> String {
        let s = settings();
        if s.use_x_forwarded_host {
            if let Some(h) = self.meta().get("HTTP_X_FORWARDED_HOST") {
                return h.clone();
            }
        }
        if let Some(h) = self.meta().get("HTTP_HOST") {
            return h.clone();
        }
        let name = self.meta().get("SERVER_NAME").cloned().unwrap_or_default();
        let port = self.meta().get("SERVER_PORT").cloned().unwrap_or_default();
        if port != if self.is_secure() { "443" } else { "80" } { format!("{name}:{port}") } else { name }
    }

    /// `get_host()`: the validated host (against `ALLOWED_HOSTS`), `DisallowedHost` otherwise.
    pub fn get_host(&self) -> Result<String> {
        let host = self.raw_host();
        let s = settings();
        let mut allowed = s.allowed_hosts.clone();
        if s.debug && allowed.is_empty() {
            allowed = vec![".localhost".into(), "127.0.0.1".into(), "[::1]".into()];
        }
        let (domain, _) = split_domain_port(&host);
        if !domain.is_empty() && validate_host(&domain, &allowed) {
            return Ok(host);
        }
        let mut msg = format!("Invalid HTTP_HOST header: {host:?}.");
        if domain.is_empty() {
            msg.push_str(" The domain name provided is not valid according to RFC 1034/1035.");
        } else {
            msg.push_str(&format!(" You may need to add {domain:?} to ALLOWED_HOSTS."));
        }
        Err(Error::suspicious(SuspiciousKind::DisallowedHost, msg))
    }

    /// `get_port()`.
    pub fn get_port(&self) -> String {
        if settings().use_x_forwarded_port {
            if let Some(p) = self.meta().get("HTTP_X_FORWARDED_PORT") {
                return p.clone();
            }
        }
        self.meta().get("SERVER_PORT").cloned().unwrap_or_default()
    }

    fn full_path(&self, path: &str, force_append_slash: bool) -> String {
        let qs = self.meta().get("QUERY_STRING").map_or("", String::as_str);
        format!(
            "{}{}{}",
            escape_uri_path(path),
            if force_append_slash && !path.ends_with('/') { "/" } else { "" },
            if qs.is_empty() { String::new() } else { format!("?{}", iri_to_uri(qs)) }
        )
    }

    /// `get_full_path()`.
    pub fn get_full_path(&self) -> String {
        self.full_path(self.path(), false)
    }

    pub fn get_full_path_with_slash(&self) -> String {
        self.full_path(self.path(), true)
    }

    pub fn get_full_path_info(&self) -> String {
        self.full_path(self.path_info(), false)
    }

    /// `request.scheme` (honours `SECURE_PROXY_SSL_HEADER`).
    pub fn scheme(&self) -> String {
        if let Some((header, value)) = &settings().secure_proxy_ssl_header {
            let hdr = self.meta().get(header);
            return if hdr.is_some_and(|h| h == value) { "https".into() } else { "http".into() };
        }
        self.meta().get("wsgi.url_scheme").cloned().unwrap_or_else(|| "http".into())
    }

    pub fn is_secure(&self) -> bool {
        self.scheme() == "https"
    }

    /// `build_absolute_uri(location=None)`.
    pub fn build_absolute_uri(&self, location: Option<&str>) -> Result<String> {
        let current = format!("{}://{}", self.scheme(), self.get_host()?);
        let mut location = match location {
            None => format!("//{}", self.get_full_path()),
            Some(l) => l.to_owned(),
        };
        let bits = urlsplit(&location).map_err(Error::BadRequest)?;
        if bits.scheme.is_empty() || bits.netloc.is_empty() {
            if bits.path.starts_with('/') && bits.scheme.is_empty() && bits.netloc.is_empty() && !bits.path.contains("/./") && !bits.path.contains("/../") {
                location = format!("{current}{}", location.strip_prefix("//").unwrap_or(&location));
            } else {
                location = urljoin(&format!("{current}{}", self.path()), &location);
            }
        }
        Ok(iri_to_uri(&location))
    }

    /// `get_signed_cookie()`.
    pub fn get_signed_cookie(&self, key: &str, salt: &str, max_age: Option<std::time::Duration>) -> Result<Option<String>> {
        let Some(raw) = self.cookies().get(key) else { return Ok(None) };
        Ok(signing::unsign_cookie(raw, key, salt, max_age)?.ok())
    }

    /// `accepts(media_type)`: does the `Accept` header allow this media type?
    pub fn accepts(&self, media_type: &str) -> bool {
        accepted_types(self.headers().get("Accept").unwrap_or("*/*")).iter().any(|a| a.matches(media_type))
    }

    /// `request.accepted_types`.
    pub fn accepted_types(&self) -> Vec<MediaType> {
        accepted_types(self.headers().get("Accept").unwrap_or("*/*"))
    }
}

fn accepted_types(header: &str) -> Vec<MediaType> {
    header.split(',').map(|p| MediaType::new(p.trim())).collect()
}

/// `MediaType`: one entry of an `Accept` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaType {
    pub main_type: String,
    pub sub_type: String,
    pub params: Vec<(String, String)>,
}

impl MediaType {
    pub fn new(spec: &str) -> Self {
        let (full, params) = parse_header_parameters(spec).unwrap_or_default();
        let (main, sub) = full.split_once('/').unwrap_or((full.as_str(), ""));
        Self { main_type: main.to_owned(), sub_type: sub.to_owned(), params }
    }

    /// `MediaType.match(other)`: wildcards and parameters considered.
    pub fn matches(&self, other: &str) -> bool {
        let other = MediaType::new(other);
        if self.main_type == "*" && other.main_type.is_empty() {
            return true;
        }
        if self.params.iter().any(|(k, v)| other.params.iter().find(|(ok, _)| ok == k).is_none_or(|(_, ov)| ov != v) && k != "q") {
            return false;
        }
        let main_ok = self.main_type == "*" || other.main_type == "*" || self.main_type == other.main_type;
        let sub_ok = self.sub_type == "*" || other.sub_type == "*" || self.sub_type == other.sub_type;
        main_ok && sub_ok
    }

    pub fn quality(&self) -> f64 {
        self.params.iter().find(|(k, _)| k == "q").and_then(|(_, v)| v.parse().ok()).unwrap_or(1.0)
    }
}

static HOST_VALIDATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-z0-9.-]+|\[[a-f0-9]*:[a-f0-9\.:]+\])(?::([0-9]+))?$").expect("valid regex"));

/// `split_domain_port()`: `("", "")` when the host is not valid.
pub fn split_domain_port(host: &str) -> (String, String) {
    let host = host.to_lowercase();
    let Some(m) = HOST_VALIDATION.captures(&host) else { return (String::new(), String::new()) };
    let domain = m[1].strip_suffix('.').unwrap_or(&m[1]).to_owned();
    (domain, m.get(2).map_or(String::new(), |p| p.as_str().to_owned()))
}

/// `validate_host()`.
pub fn validate_host(host: &str, allowed_hosts: &[String]) -> bool {
    allowed_hosts.iter().any(|p| p == "*" || is_same_domain(host, p))
}
