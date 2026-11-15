//! `core.handlers.base` and `core.handlers.exception`: the middleware
//! chain, view dispatch and exception-to-response conversion.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use rango_core::conf::{settings, try_settings};
use rango_core::core_signals::{GOT_REQUEST_EXCEPTION, REQUEST_FINISHED, REQUEST_STARTED};
use rango_core::exceptions::{Error, Result, SuspiciousKind};
use rango_http::{HttpRequest, HttpResponse};
use rango_urls::{RequestUrlconf, Resolver404, ResolverMatch, View, ViewArgs};
use rango_views::debug::{technical_404_response, technical_500_response};
use rango_views::defaults;

/// `get_response` as seen by a middleware: call it to run the rest of the chain.
pub type GetResponse = Arc<dyn Fn(HttpRequest) -> BoxFuture<'static, HttpResponse> + Send + Sync>;

/// A middleware. Override the hooks you need; the default `call` runs them the way
/// the `MiddlewareMixin` does. Override `call` itself for "new-style" middleware.
#[async_trait]
pub trait Middleware: Send + Sync {
    /// Runs before the view is resolved; returning a response short-circuits the view.
    async fn process_request(&self, _request: &HttpRequest) -> Result<Option<HttpResponse>> {
        Ok(None)
    }

    /// Runs just before the view; returning a response skips the view.
    async fn process_view(&self, _request: &HttpRequest, _view: &View, _args: &ViewArgs) -> Result<Option<HttpResponse>> {
        Ok(None)
    }

    /// Runs if the view raised; returning a response handles the error.
    async fn process_exception(&self, _request: &HttpRequest, _error: &Error) -> Result<Option<HttpResponse>> {
        Ok(None)
    }

    /// Runs if the view returned a deferred (template) response, before it is rendered.
    async fn process_template_response(&self, _request: &HttpRequest, response: HttpResponse) -> Result<HttpResponse> {
        Ok(response)
    }

    /// Runs on every response on the way out.
    async fn process_response(&self, _request: &HttpRequest, response: HttpResponse) -> Result<HttpResponse> {
        Ok(response)
    }

    async fn call(&self, request: HttpRequest, get_response: GetResponse) -> Result<HttpResponse> {
        let response = match self.process_request(&request).await? {
            Some(r) => r,
            None => get_response(request.clone()).await,
        };
        self.process_response(&request, response).await
    }
}

type Factory = fn() -> Result<Arc<dyn Middleware>>;

/// Link-time registration of a middleware under its `MIDDLEWARE` setting name.
pub struct MiddlewareRegistration {
    pub name: &'static str,
    pub build: Factory,
}

inventory::collect!(MiddlewareRegistration);

/// `register_middleware!("rango.middleware.common.CommonMiddleware", || Ok(Arc::new(CommonMiddleware)))`.
#[macro_export]
macro_rules! register_middleware {
    ($name:expr, $build:expr) => {
        $crate::inventory::submit! { $crate::handlers::MiddlewareRegistration { name: $name, build: $build } }
    };
}

pub type ErrorHandler = Arc<dyn Fn(&HttpRequest, &Error) -> HttpResponse + Send + Sync>;

/// `handler400/403/404/500`.
#[derive(Clone)]
pub struct ErrorHandlers {
    pub handler400: ErrorHandler,
    pub handler403: ErrorHandler,
    pub handler404: ErrorHandler,
    pub handler500: ErrorHandler,
}

impl Default for ErrorHandlers {
    fn default() -> Self {
        Self {
            handler400: Arc::new(|r, e| defaults::bad_request(r, e)),
            handler403: Arc::new(|r, e| defaults::permission_denied(r, e)),
            handler404: Arc::new(|r, e| defaults::page_not_found(r, e)),
            handler500: Arc::new(|r, _| defaults::server_error(r)),
        }
    }
}

static ERROR_HANDLERS: RwLock<Option<ErrorHandlers>> = RwLock::new(None);

/// Install custom error views (the reference implementation: `handler404 = "myapp.views.not_found"` in the root URLconf).
pub fn set_error_handlers(f: impl FnOnce(&mut ErrorHandlers)) {
    let mut g = ERROR_HANDLERS.write().expect("handlers lock");
    let mut h = g.clone().unwrap_or_default();
    f(&mut h);
    *g = Some(h);
}

fn error_handlers() -> ErrorHandlers {
    ERROR_HANDLERS.read().expect("handlers lock").clone().unwrap_or_default()
}

/// What the resolver reported for a request that did not match (kept for the debug 404 page).
#[derive(Clone, Debug)]
struct ResolveFailure(Resolver404);

/// `response_for_exception(request, exc)`.
pub async fn response_for_exception(request: &HttpRequest, error: &Error) -> HttpResponse {
    let s = settings();
    let sensitive = request.ext::<ResolverMatch>().and_then(|m| m.func.flags.sensitive_post_parameters.clone());
    match error {
        Error::Http404(msg) => {
            if s.debug {
                let failure = request.ext::<ResolveFailure>();
                technical_404_response(request, failure.as_ref().map(|f| &f.0), msg)
            } else {
                (error_handlers().handler404)(request, error)
            }
        }
        Error::PermissionDenied(_) => {
            tracing::warn!(target: "rango.request", "Forbidden (Permission denied): {}", request.path());
            (error_handlers().handler403)(request, error)
        }
        Error::BadRequest(_) => {
            tracing::warn!(target: "rango.request", "Bad request (Unable to parse request body): {}", request.path());
            if s.debug {
                technical_500_response(request, error, 400, sensitive.as_deref()).await
            } else {
                (error_handlers().handler400)(request, error)
            }
        }
        Error::Suspicious { kind, message } => {
            tracing::error!(target: "rango.security", kind = ?kind, "{message}");
            if s.debug {
                technical_500_response(request, error, 400, sensitive.as_deref()).await
            } else {
                let _ = SuspiciousKind::Operation;
                (error_handlers().handler400)(request, error)
            }
        }
        other => {
            GOT_REQUEST_EXCEPTION.send(None, other.to_string()).await;
            tracing::error!(target: "rango.request", "Internal Server Error: {}: {other}", request.path());
            if s.debug_propagate_exceptions {
                panic!("{other}");
            }
            if s.debug {
                technical_500_response(request, other, 500, sensitive.as_deref()).await
            } else {
                (error_handlers().handler500)(request, other)
            }
        }
    }
}

/// `conditional_content_removal(request, response)`: no body for HEAD, 1xx, 204 and 304.
pub fn conditional_content_removal(request: &HttpRequest, response: &mut HttpResponse) {
    if (100..200).contains(&response.status_code) || response.status_code == 204 || response.status_code == 304 {
        if response.is_streaming() {
            *response.body_mut() = rango_http::Body::Bytes(bytes::Bytes::new());
        } else {
            response.set_content(bytes::Bytes::new());
        }
    }
    if request.method() == "HEAD" {
        if response.is_streaming() {
            *response.body_mut() = rango_http::Body::Bytes(bytes::Bytes::new());
        } else {
            let len = response.content().map_or(0, |c| c.len());
            let had_cl = response.has_header("Content-Length");
            response.set_content(bytes::Bytes::new());
            if !had_cl && len > 0 {
                let _ = response.set_header("Content-Length", &len.to_string());
            }
        }
    }
}

/// `BaseHandler`: the loaded middleware chain.
pub struct Handler {
    chain: GetResponse,
}

impl Handler {
    /// `load_middleware()`: build the chain from `settings.MIDDLEWARE`.
    pub fn new() -> Result<Self> {
        let names = try_settings()?.middleware.clone();
        let mut instances: Vec<(String, Arc<dyn Middleware>)> = Vec::new();
        for name in names.iter().rev() {
            let reg = inventory::iter::<MiddlewareRegistration>
                .into_iter()
                .find(|r| r.name == name)
                .ok_or_else(|| Error::improperly_configured(format!("Middleware '{name}' is not registered. Use register_middleware!.")))?;
            match (reg.build)() {
                Ok(mw) => instances.push((name.clone(), mw)),
                Err(Error::MiddlewareNotUsed(msg)) => {
                    tracing::debug!(target: "rango.request", "MiddlewareNotUsed: {name}: {msg}");
                }
                Err(e) => return Err(e),
            }
        }
        // instances is in reverse MIDDLEWARE order (innermost first).
        let in_order: Vec<Arc<dyn Middleware>> = instances.iter().rev().map(|(_, m)| Arc::clone(m)).collect();
        let view_mw = Arc::new(in_order.clone());
        let exc_mw: Arc<Vec<Arc<dyn Middleware>>> = Arc::new(in_order.iter().rev().cloned().collect());

        let core: GetResponse = Arc::new(move |req| {
            let (view_mw, exc_mw) = (Arc::clone(&view_mw), Arc::clone(&exc_mw));
            Box::pin(async move {
                let result = get_response_inner(&req, &view_mw, &exc_mw).await;
                convert(&req, result).await
            })
        });
        let mut chain = core;
        for (_, mw) in &instances {
            let (mw, next) = (Arc::clone(mw), Arc::clone(&chain));
            chain = Arc::new(move |req: HttpRequest| {
                let (mw, next) = (Arc::clone(&mw), Arc::clone(&next));
                Box::pin(async move {
                    let result = mw.call(req.clone(), next).await;
                    convert(&req, result).await
                })
            });
        }
        Ok(Self { chain })
    }

    /// `get_response(request)`: never fails; errors become 4xx/5xx responses.
    pub async fn get_response(&self, request: HttpRequest) -> HttpResponse {
        REQUEST_STARTED.send(Some("rango.handlers"), ()).await;
        let prefix = settings().force_script_name.clone().unwrap_or_else(|| request.script_name().to_owned());
        let prefix = if prefix.is_empty() { "/".to_owned() } else { prefix };
        let req = request.clone();
        let mut response = rango_urls::with_script_prefix(&prefix, (self.chain)(request)).await;
        if response.status_code >= 400 {
            log_response(&req, &response);
        }
        REQUEST_FINISHED.send(Some("rango.handlers"), ()).await;
        conditional_content_removal(&req, &mut response);
        response
    }
}

async fn convert(request: &HttpRequest, result: Result<HttpResponse>) -> HttpResponse {
    match result {
        Ok(r) => r,
        Err(e) => response_for_exception(request, &e).await,
    }
}

fn log_response(request: &HttpRequest, response: &HttpResponse) {
    let msg = format!("{}: {}", response.reason_phrase(), request.path());
    if response.status_code >= 500 {
        tracing::error!(target: "rango.request", status = response.status_code, "{msg}");
    } else {
        tracing::warn!(target: "rango.request", status = response.status_code, "{msg}");
    }
}

async fn get_response_inner(
    request: &HttpRequest,
    view_mw: &[Arc<dyn Middleware>],
    exc_mw: &[Arc<dyn Middleware>],
) -> Result<HttpResponse> {
    // resolve_request()
    let urlconf = request.ext::<RequestUrlconf>().map(|u| u.0.clone());
    let resolved = rango_urls::resolve(request.path_info(), urlconf.as_deref())?;
    let m = match resolved {
        Ok(m) => m,
        Err(e) => {
            request.set_ext(ResolveFailure(e.clone()));
            return Err(e.into());
        }
    };
    request.set_ext(m.clone());

    let args = m.view_args();
    for mw in view_mw {
        if let Some(r) = mw.process_view(request, &m.func, &args).await? {
            return Ok(r);
        }
    }
    match m.func.call(request.clone(), args).await {
        Ok(mut r) => {
            if r.has_deferred() {
                for mw in exc_mw {
                    r = mw.process_template_response(request, r).await?;
                }
                r.render_deferred()?;
            }
            Ok(r)
        }
        Err(e) => {
            for mw in exc_mw {
                if let Some(r) = mw.process_exception(request, &e).await? {
                    return Ok(r);
                }
            }
            Err(e)
        }
    }
}
