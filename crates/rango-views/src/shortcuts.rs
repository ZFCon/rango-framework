//! `shortcuts` (`get_object_or_404`/`get_list_or_404` live in the `rango` facade: they need the ORM).

use rango_core::exceptions::{Error, Result};
use rango_http::{HttpRequest, HttpResponse};
use rango_template::Context;
use rango_urls::{Reverse, reverse};

/// `resolve_url(to, ...)`: a URL as is, or a view/pattern name reversed.
pub fn resolve_url(to: &str, args: Reverse) -> Result<String> {
    if to.starts_with("./") || to.starts_with("../") {
        return Ok(to.to_owned());
    }
    match reverse(to, args) {
        Ok(u) => Ok(u),
        Err(Error::NoReverseMatch(m)) => {
            if !to.contains('/') && !to.contains('.') {
                Err(Error::NoReverseMatch(m))
            } else {
                Ok(to.to_owned())
            }
        }
        Err(e) => Err(e),
    }
}

/// `redirect(to)`: 302 to a URL or a pattern name.
pub fn redirect(to: &str) -> Result<HttpResponse> {
    redirect_with(to, Reverse::new(), false)
}

/// `redirect(to, *args, **kwargs, permanent=...)`.
pub fn redirect_with(to: &str, args: Reverse, permanent: bool) -> Result<HttpResponse> {
    let url = resolve_url(to, args)?;
    if permanent { HttpResponse::permanent_redirect(&url) } else { HttpResponse::redirect(&url) }
}

/// `render(request, template_name, context)`: render with a `RequestContext` into a 200 response.
pub fn render(request: &HttpRequest, names: impl rango_template::loader::TemplateNames, context: Context) -> Result<HttpResponse> {
    render_with(request, names, context, None, None)
}

/// `render(..., content_type=, status=)`.
pub fn render_with(
    request: &HttpRequest,
    names: impl rango_template::loader::TemplateNames,
    context: Context,
    content_type: Option<&str>,
    status: Option<u16>,
) -> Result<HttpResponse> {
    let body = rango_template::render_to_string(names, context, Some(request))?;
    let mut r = HttpResponse::new(body);
    if let Some(ct) = content_type {
        r = r.content_type(ct);
    }
    if let Some(s) = status {
        r.set_status_code(s)?;
    }
    Ok(r)
}
