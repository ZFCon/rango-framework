//! `views.decorators.*`.
//!
//! A decorator is a function from view to view: `let v = require_get(index);`.
//! Markers the reference implementation stores as function attributes live in [`ViewFlags`](rango_urls::ViewFlags).

use std::sync::Arc;

use rango_core::exceptions::Result;
use rango_core::utils::http::http_date;
use rango_http::cache_utils::{Cc, add_never_cache_headers, get_conditional_response, patch_cache_control, patch_vary_headers};
use rango_http::{HttpRequest, HttpResponse};
use rango_urls::{IntoView, View, ViewArgs};

/// `require_http_methods(methods)`: 405 for other methods.
pub fn require_http_methods<M>(methods: &[&str], view: impl IntoView<M>) -> View {
    let allowed: Vec<String> = methods.iter().map(|m| (*m).to_owned()).collect();
    view.into_view().wrap(move |inner, req, args| {
        let allowed = allowed.clone();
        Box::pin(async move {
            if !allowed.iter().any(|m| m == req.method()) {
                let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();
                return Ok(HttpResponse::not_allowed(&refs, ""));
            }
            inner.call(req, args).await
        })
    })
}

pub fn require_get<M>(view: impl IntoView<M>) -> View {
    require_http_methods(&["GET"], view)
}
pub fn require_post<M>(view: impl IntoView<M>) -> View {
    require_http_methods(&["POST"], view)
}
pub fn require_safe<M>(view: impl IntoView<M>) -> View {
    require_http_methods(&["GET", "HEAD"], view)
}

/// `csrf_exempt`.
pub fn csrf_exempt<M>(view: impl IntoView<M>) -> View {
    view.into_view().with_flags(|f| f.csrf_exempt = true)
}

/// `xframe_options_exempt`.
pub fn xframe_options_exempt<M>(view: impl IntoView<M>) -> View {
    view.into_view().wrap(|inner, req, args| {
        Box::pin(async move {
            let mut r = inner.call(req, args).await?;
            r.flags.insert("xframe_options_exempt".into());
            Ok(r)
        })
    })
}

fn set_default_header<M>(name: &'static str, value: &'static str, view: impl IntoView<M>) -> View {
    view.into_view().wrap(move |inner, req, args| {
        Box::pin(async move {
            let mut r = inner.call(req, args).await?;
            if r.header(name).is_none() {
                r.set_header(name, value)?;
            }
            Ok(r)
        })
    })
}

/// `xframe_options_deny`.
pub fn xframe_options_deny<M>(view: impl IntoView<M>) -> View {
    set_default_header("X-Frame-Options", "DENY", view)
}

/// `xframe_options_sameorigin`.
pub fn xframe_options_sameorigin<M>(view: impl IntoView<M>) -> View {
    set_default_header("X-Frame-Options", "SAMEORIGIN", view)
}

/// `never_cache`.
pub fn never_cache<M>(view: impl IntoView<M>) -> View {
    view.into_view().wrap(|inner, req, args| {
        Box::pin(async move {
            let mut r = inner.call(req, args).await?;
            add_never_cache_headers(&mut r);
            Ok(r)
        })
    })
}

/// `cache_control(**kwargs)`.
pub fn cache_control<M>(directives: &[(&str, Cc)], view: impl IntoView<M>) -> View {
    let directives: Vec<(String, Cc)> = directives.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect();
    view.into_view().wrap(move |inner, req, args| {
        let directives = directives.clone();
        Box::pin(async move {
            let mut r = inner.call(req, args).await?;
            let refs: Vec<(&str, Cc)> = directives.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            patch_cache_control(&mut r, &refs);
            Ok(r)
        })
    })
}

/// `vary_on_headers(*headers)`.
pub fn vary_on_headers<M>(headers: &[&str], view: impl IntoView<M>) -> View {
    let headers: Vec<String> = headers.iter().map(|h| (*h).to_owned()).collect();
    view.into_view().wrap(move |inner, req, args| {
        let headers = headers.clone();
        Box::pin(async move {
            let mut r = inner.call(req, args).await?;
            let refs: Vec<&str> = headers.iter().map(String::as_str).collect();
            patch_vary_headers(&mut r, &refs);
            Ok(r)
        })
    })
}

/// `vary_on_cookie`.
pub fn vary_on_cookie<M>(view: impl IntoView<M>) -> View {
    vary_on_headers(&["Cookie"], view)
}

/// `sensitive_post_parameters(*parameters)`: names are hidden from error reports.
pub fn sensitive_post_parameters<M>(parameters: &[&str], view: impl IntoView<M>) -> View {
    let params: Vec<String> = parameters.iter().map(|p| (*p).to_owned()).collect();
    view.into_view().with_flags(move |f| f.sensitive_post_parameters = Some(params))
}

type EtagFn = Arc<dyn Fn(&HttpRequest, &ViewArgs) -> Option<String> + Send + Sync>;
type LastModFn = Arc<dyn Fn(&HttpRequest, &ViewArgs) -> Option<i64> + Send + Sync>;

/// `condition(etag_func, last_modified_func)`: conditional GET/PUT handling.
pub fn condition<M>(etag_func: Option<EtagFn>, last_modified_func: Option<LastModFn>, view: impl IntoView<M>) -> View {
    view.into_view().wrap(move |inner, req, args| {
        let (etag_func, last_func) = (etag_func.clone(), last_modified_func.clone());
        Box::pin(async move {
            let etag = etag_func.as_ref().and_then(|f| f(&req, &args)).map(|e| rango_core::utils::http::quote_etag(&e));
            let last = last_func.as_ref().and_then(|f| f(&req, &args));
            let response: Result<HttpResponse> = match get_conditional_response(&req, etag.as_deref(), last, None) {
                Some(r) => Ok(r),
                None => inner.call(req.clone(), args).await,
            };
            let mut response = response?;
            if matches!(req.method(), "GET" | "HEAD") {
                if let Some(lm) = last {
                    if !response.has_header("Last-Modified") {
                        response.set_header("Last-Modified", &http_date(Some(lm as f64)))?;
                    }
                }
                if let Some(e) = &etag {
                    if !response.has_header("ETag") {
                        response.set_header("ETag", e)?;
                    }
                }
            }
            Ok(response)
        })
    })
}

/// `etag(etag_func)`.
pub fn etag<M>(f: impl Fn(&HttpRequest, &ViewArgs) -> Option<String> + Send + Sync + 'static, view: impl IntoView<M>) -> View {
    condition(Some(Arc::new(f)), None, view)
}

/// `last_modified(last_modified_func)`.
pub fn last_modified<M>(f: impl Fn(&HttpRequest, &ViewArgs) -> Option<i64> + Send + Sync + 'static, view: impl IntoView<M>) -> View {
    condition(None, Some(Arc::new(f)), view)
}
