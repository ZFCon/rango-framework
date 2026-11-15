//! `views.generic.base`: `View`, `TemplateView`, `RedirectView`.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, TemplateResponse, Value as T};
use rango_urls::{Reverse, ViewArgs, reverse};

use super::util::url_kwargs_context;

/// Hook adjusting the template context (`get_context_data`).
pub type ContextHook = Arc<dyn Fn(&HttpRequest, &ViewArgs, &mut Context) + Send + Sync>;

pub const HTTP_METHOD_NAMES: [&str; 8] = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];

/// `views.generic.View`: dispatch on the request method.
#[async_trait]
pub trait View: Send + Sync + 'static {
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.http_method_not_allowed(&req, args)
    }
    async fn post(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.http_method_not_allowed(&req, args)
    }
    async fn put(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.http_method_not_allowed(&req, args)
    }
    async fn patch(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.http_method_not_allowed(&req, args)
    }
    async fn delete(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.http_method_not_allowed(&req, args)
    }
    async fn trace(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.http_method_not_allowed(&req, args)
    }

    /// `head()` defaults to `get()` (the `setup()` does that when `get` exists).
    async fn head(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.get(req, args).await
    }

    /// The methods this view implements (for `OPTIONS` and 405 `Allow`).
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "OPTIONS"]
    }

    fn http_method_not_allowed(&self, req: &HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        tracing::warn!(target: "rango.request", "Method Not Allowed ({}): {}", req.method(), req.path());
        Ok(HttpResponse::not_allowed(&self.allowed_methods(), ""))
    }

    async fn options(&self, _req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        let mut r = HttpResponse::new("");
        r.set_header("Allow", &self.allowed_methods().join(", "))?;
        r.set_header("Content-Length", "0")?;
        Ok(r)
    }

    async fn dispatch(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        match req.method().to_lowercase().as_str() {
            "get" => self.get(req, args).await,
            "post" => self.post(req, args).await,
            "put" => self.put(req, args).await,
            "patch" => self.patch(req, args).await,
            "delete" => self.delete(req, args).await,
            "head" => self.head(req, args).await,
            "options" => self.options(req, args).await,
            "trace" => self.trace(req, args).await,
            _ => self.http_method_not_allowed(&req, args),
        }
    }

    /// `View.as_view()`: the value to hand to `path()`.
    fn as_view(self) -> rango_urls::View
    where
        Self: Sized,
    {
        let this = Arc::new(self);
        let name = std::any::type_name::<Self>();
        rango_urls::View::new(name, move |req, args| {
            let this = Arc::clone(&this);
            Box::pin(async move { this.dispatch(req, &args).await })
        })
    }
}

/// `TemplateView`: render a template with the URL kwargs and `extra_context`.
pub struct TemplateView {
    pub template_name: String,
    pub extra_context: IndexMap<String, T>,
    pub content_type: Option<String>,
    context_hook: Option<ContextHook>,
}

impl TemplateView {
    pub fn new(template_name: &str) -> Self {
        Self { template_name: template_name.to_owned(), extra_context: IndexMap::new(), content_type: None, context_hook: None }
    }
    pub fn extra(mut self, key: &str, value: impl Into<T>) -> Self {
        self.extra_context.insert(key.to_owned(), value.into());
        self
    }
    pub fn content_type(mut self, ct: &str) -> Self {
        self.content_type = Some(ct.to_owned());
        self
    }
    /// `get_context_data()` override.
    pub fn context(mut self, f: impl Fn(&HttpRequest, &ViewArgs, &mut Context) + Send + Sync + 'static) -> Self {
        self.context_hook = Some(Arc::new(f));
        self
    }
}

#[async_trait]
impl View for TemplateView {
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let mut ctx = Context::new();
        for (k, v) in url_kwargs_context(args) {
            ctx.set(&k, v);
        }
        for (k, v) in &self.extra_context {
            ctx.set(k, v.clone());
        }
        if let Some(h) = &self.context_hook {
            h(&req, args, &mut ctx);
        }
        let mut r = TemplateResponse::new(&req, self.template_name.as_str(), ctx);
        if let Some(ct) = &self.content_type {
            r = r.content_type(ct);
        }
        Ok(r)
    }
}

/// `RedirectView`: redirect to a fixed `url` (`%(kwarg)s` formatted) or a named pattern.
pub struct RedirectView {
    pub url: Option<String>,
    pub pattern_name: Option<String>,
    pub permanent: bool,
    pub query_string: bool,
}

impl RedirectView {
    pub fn to_url(url: &str) -> Self {
        Self { url: Some(url.to_owned()), pattern_name: None, permanent: false, query_string: false }
    }
    pub fn to_pattern(name: &str) -> Self {
        Self { url: None, pattern_name: Some(name.to_owned()), permanent: false, query_string: false }
    }
    pub fn permanent(mut self, p: bool) -> Self {
        self.permanent = p;
        self
    }
    pub fn query_string(mut self, q: bool) -> Self {
        self.query_string = q;
        self
    }

    /// `get_redirect_url()`.
    pub fn get_redirect_url(&self, req: &HttpRequest, args: &ViewArgs) -> Result<Option<String>> {
        let mut url = if let Some(u) = &self.url {
            let mut out = u.clone();
            for (k, v) in &args.kwargs {
                out = out.replace(&format!("%({k})s"), &v.to_string());
            }
            out
        } else if let Some(name) = &self.pattern_name {
            let mut rev = Reverse::new();
            for (k, v) in &args.kwargs {
                rev = rev.kwarg(k, v.to_string());
            }
            if args.kwargs.is_empty() {
                for a in &args.args {
                    rev = rev.args([a.to_string()]);
                }
            }
            reverse(name, rev)?
        } else {
            return Ok(None);
        };
        let qs = req.meta().get("QUERY_STRING").cloned().unwrap_or_default();
        if self.query_string && !qs.is_empty() {
            url = format!("{url}{}{qs}", if url.contains('?') { "&" } else { "?" });
        }
        Ok(Some(url))
    }

    async fn respond(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        match self.get_redirect_url(&req, args)? {
            Some(url) if self.permanent => HttpResponse::permanent_redirect(&url),
            Some(url) => HttpResponse::redirect(&url),
            None => {
                tracing::warn!(target: "rango.request", "Gone: {}", req.path());
                Ok(HttpResponse::gone(""))
            }
        }
    }
}

#[async_trait]
impl View for RedirectView {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "OPTIONS", "DELETE", "PUT", "PATCH"]
    }
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.respond(req, args).await
    }
    async fn post(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.respond(req, args).await
    }
    async fn put(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.respond(req, args).await
    }
    async fn patch(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.respond(req, args).await
    }
    async fn delete(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.respond(req, args).await
    }
}

pub(crate) fn improperly_configured(msg: impl Into<String>) -> Error {
    Error::improperly_configured(msg)
}
