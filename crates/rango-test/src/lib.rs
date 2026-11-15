//! Rango test utilities (`test`): an in-process [`Client`], a [`RequestFactory`] and
//! response assertions.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::StreamExt;
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_core::utils::http::{urlencode, urljoin, urlsplit};
use rango_http::{Body, HttpRequest, HttpResponse, Morsel, RequestBuilder};
use rango_middleware::Handler;
use rango_middleware::middleware::csrf::csrf_state;
use serde::de::DeserializeOwned;

/// A response as seen by a test: the body is fully read.
#[derive(Debug)]
pub struct TestResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub content: Bytes,
    pub cookies: IndexMap<String, Morsel>,
    /// The request that produced this response.
    pub request: HttpRequest,
    /// `(url, status)` for every redirect followed (`follow=true`).
    pub redirect_chain: Vec<(String, u16)>,
}

impl TestResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.content).into_owned()
    }

    /// `response.json()`.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.content).map_err(|e| Error::Other(Box::new(e)))
    }

    /// `response.url` for redirects.
    pub fn url(&self) -> Option<&str> {
        self.header("Location")
    }
}

async fn collect(request: HttpRequest, response: HttpResponse) -> TestResponse {
    let (status_code, _reason, headers, cookies, body, closers) = response.into_body();
    let content = match body {
        Body::Bytes(b) => b,
        Body::Stream(mut s) => {
            let mut buf = Vec::new();
            while let Some(Ok(chunk)) = s.next().await {
                buf.extend_from_slice(&chunk);
            }
            Bytes::from(buf)
        }
    };
    for c in closers {
        c();
    }
    TestResponse {
        status_code,
        headers: headers.iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
        content,
        cookies,
        request,
        redirect_chain: Vec::new(),
    }
}

/// `RequestFactory`: build requests without going through the middleware chain.
#[derive(Default, Clone)]
pub struct RequestFactory;

impl RequestFactory {
    pub fn get(&self, path: &str) -> Result<HttpRequest> {
        HttpRequest::builder("GET", path).build()
    }

    pub fn post(&self, path: &str, data: &[(&str, &str)]) -> Result<HttpRequest> {
        let mut b = HttpRequest::builder("POST", path);
        b.header("Content-Type", "application/x-www-form-urlencoded").body(urlencode(data.iter().copied()));
        b.build()
    }

    pub fn generic(&self, method: &str, path: &str, body: impl Into<Bytes>, content_type: &str) -> Result<HttpRequest> {
        let mut b = HttpRequest::builder(method, path);
        b.header("Content-Type", content_type).body(body);
        b.build()
    }
}

/// A file part for [`Client::post_multipart`].
pub struct FilePart<'a> {
    pub field: &'a str,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub data: &'a [u8],
}

/// `test.Client`.
pub struct Client {
    handler: Arc<Handler>,
    cookies: Mutex<IndexMap<String, String>>,
    enforce_csrf_checks: bool,
    follow: bool,
    default_headers: Vec<(String, String)>,
}

impl Client {
    /// A client over the current `MIDDLEWARE`/URLconf settings.
    pub fn new() -> Result<Self> {
        Ok(Self {
            handler: Arc::new(Handler::new()?),
            cookies: Mutex::new(IndexMap::new()),
            enforce_csrf_checks: false,
            follow: false,
            default_headers: Vec::new(),
        })
    }

    /// `Client(enforce_csrf_checks=True)`.
    pub fn enforce_csrf_checks(mut self, enforce: bool) -> Self {
        self.enforce_csrf_checks = enforce;
        self
    }

    /// Follow redirects (`follow=True`).
    pub fn follow_redirects(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }

    /// Header sent with every request (`Client(headers=...)`).
    pub fn default_header(mut self, name: &str, value: &str) -> Self {
        self.default_headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// `client.cookies`.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.lock().expect("cookies").get(name).cloned()
    }

    pub fn set_cookie(&self, name: &str, value: &str) {
        self.cookies.lock().expect("cookies").insert(name.to_owned(), value.to_owned());
    }

    async fn send(&self, method: &str, uri: &str, body: Bytes, content_type: Option<&str>, extra: &[(&str, &str)]) -> Result<TestResponse> {
        let mut b: RequestBuilder = RequestBuilder::new(method, uri);
        b.body(body);
        if let Some(ct) = content_type {
            b.header("Content-Type", ct);
        }
        for (k, v) in &self.default_headers {
            b.header(k, v);
        }
        for (k, v) in extra {
            b.header(k, v);
        }
        let cookie_header = {
            let jar = self.cookies.lock().expect("cookies");
            jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
        };
        if !cookie_header.is_empty() {
            b.header("Cookie", &cookie_header);
        }
        let request = b.build()?;
        if !self.enforce_csrf_checks {
            csrf_state(&request).dont_enforce_checks();
        }
        let response = self.handler.get_response(request.clone()).await;
        let collected = collect(request, response).await;
        let mut jar = self.cookies.lock().expect("cookies");
        for (k, m) in &collected.cookies {
            let expired = m.max_age == Some(0) || m.expires.as_deref().is_some_and(|e| e.contains("1970"));
            if expired { jar.shift_remove(k); } else { jar.insert(k.clone(), m.value.clone()); }
        }
        Ok(collected)
    }

    async fn request(&self, method: &str, uri: &str, body: Bytes, content_type: Option<&str>, extra: &[(&str, &str)]) -> Result<TestResponse> {
        let mut resp = self.send(method, uri, body.clone(), content_type, extra).await?;
        if !self.follow {
            return Ok(resp);
        }
        let mut chain = Vec::new();
        let mut current = uri.to_owned();
        let (mut m, mut b, mut ct) = (method.to_owned(), body, content_type.map(str::to_owned));
        while matches!(resp.status_code, 301 | 302 | 303 | 307 | 308) && chain.len() < 20 {
            let loc = resp.url().unwrap_or("/").to_owned();
            let abs = urljoin(&format!("http://testserver{current}"), &loc);
            let parts = urlsplit(&abs).map_err(Error::BadRequest)?;
            let next = if parts.query.is_empty() { parts.path.clone() } else { format!("{}?{}", parts.path, parts.query) };
            chain.push((loc, resp.status_code));
            if !matches!(resp.status_code, 307 | 308) {
                m = if m == "HEAD" { "HEAD".into() } else { "GET".into() };
                b = Bytes::new();
                ct = None;
            }
            resp = self.send(&m, &next, b.clone(), ct.as_deref(), extra).await?;
            current = next;
        }
        resp.redirect_chain = chain;
        Ok(resp)
    }

    pub async fn get(&self, path: &str) -> Result<TestResponse> {
        self.request("GET", path, Bytes::new(), None, &[]).await
    }

    /// `client.get(path, data)`: `data` becomes the query string.
    pub async fn get_with(&self, path: &str, data: &[(&str, &str)]) -> Result<TestResponse> {
        let sep = if path.contains('?') { '&' } else { '?' };
        self.request("GET", &format!("{path}{sep}{}", urlencode(data.iter().copied())), Bytes::new(), None, &[]).await
    }

    pub async fn head(&self, path: &str) -> Result<TestResponse> {
        self.request("HEAD", path, Bytes::new(), None, &[]).await
    }

    pub async fn options(&self, path: &str) -> Result<TestResponse> {
        self.request("OPTIONS", path, Bytes::new(), None, &[]).await
    }

    pub async fn delete(&self, path: &str) -> Result<TestResponse> {
        self.request("DELETE", path, Bytes::new(), None, &[]).await
    }

    /// `client.post(path, data)`: multipart/form-data's default.
    pub async fn post(&self, path: &str, data: &[(&str, &str)]) -> Result<TestResponse> {
        self.post_multipart(path, data, &[]).await
    }

    /// POST with `application/x-www-form-urlencoded`.
    pub async fn post_form(&self, path: &str, data: &[(&str, &str)]) -> Result<TestResponse> {
        self.request("POST", path, Bytes::from(urlencode(data.iter().copied())), Some("application/x-www-form-urlencoded"), &[]).await
    }

    /// POST with an explicit content type and body (`content_type=`, JSON, ...).
    pub async fn generic(&self, method: &str, path: &str, body: impl Into<Bytes>, content_type: &str) -> Result<TestResponse> {
        self.request(method, path, body.into(), Some(content_type), &[]).await
    }

    pub async fn post_json<T: serde::Serialize>(&self, path: &str, data: &T) -> Result<TestResponse> {
        let body = serde_json::to_vec(data).map_err(|e| Error::Other(Box::new(e)))?;
        self.generic("POST", path, body, "application/json").await
    }

    pub async fn put(&self, path: &str, body: impl Into<Bytes>, content_type: &str) -> Result<TestResponse> {
        self.generic("PUT", path, body, content_type).await
    }

    pub async fn patch(&self, path: &str, body: impl Into<Bytes>, content_type: &str) -> Result<TestResponse> {
        self.generic("PATCH", path, body, content_type).await
    }

    /// POST `multipart/form-data` with optional file parts.
    pub async fn post_multipart(&self, path: &str, data: &[(&str, &str)], files: &[FilePart<'_>]) -> Result<TestResponse> {
        const BOUNDARY: &str = "BoUnDaRyStRiNg";
        let mut body: Vec<u8> = Vec::new();
        for (k, v) in data {
            body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
        }
        for f in files {
            body.extend_from_slice(
                format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n", f.field, f.filename, f.content_type)
                    .as_bytes(),
            );
            body.extend_from_slice(f.data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        self.request("POST", path, Bytes::from(body), Some(&format!("multipart/form-data; boundary={BOUNDARY}")), &[]).await
    }
}

/// `assertContains(response, text, status_code=200)`.
pub fn assert_contains(response: &TestResponse, text: &str, status_code: u16) {
    assert_eq!(response.status_code, status_code, "Couldn't retrieve content: Response code was {}", response.status_code);
    assert!(response.text().contains(text), "Couldn't find {text:?} in response:\n{}", response.text());
}

/// `assertNotContains(response, text, status_code=200)`.
pub fn assert_not_contains(response: &TestResponse, text: &str, status_code: u16) {
    assert_eq!(response.status_code, status_code, "Couldn't retrieve content: Response code was {}", response.status_code);
    assert!(!response.text().contains(text), "Response should not contain {text:?}");
}

/// `assertRedirects(response, expected_url, status_code=302)` (response must not have followed).
pub fn assert_redirects(response: &TestResponse, expected_url: &str, status_code: u16) {
    assert_eq!(response.status_code, status_code, "Response didn't redirect as expected: Response code was {}", response.status_code);
    let url = response.url().unwrap_or("");
    assert_eq!(url, expected_url, "Response redirected to '{url}', expected '{expected_url}'");
}
