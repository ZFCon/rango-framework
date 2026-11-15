//! `views.defaults` and the CSRF failure view.
//!
//! A project's `404.html`/`500.html`/`400.html`/`403.html` take precedence; otherwise the
//! built-in fallback markup is used.

use rango_core::exceptions::Error;
use rango_core::utils::http::quote;
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, TemplateError, loader};

fn html(status: u16, body: String) -> HttpResponse {
    let mut r = HttpResponse::new(body);
    r.status_code = status;
    r
}

/// `loader.get_template(name).render(context, request)`; `None` when the project has no such template.
fn project_template(name: &str, ctx: Context, request: Option<&HttpRequest>) -> Option<String> {
    let engine = loader::engine().ok()?;
    let template = match engine.get_template(name) {
        Ok(t) => t,
        Err(TemplateError::DoesNotExist(_)) => return None,
        Err(e) => return Some(format!("{e}")),
    };
    let mut ctx = ctx;
    if let Some(r) = request {
        loader::bind_request(&mut ctx, r, &engine).ok()?;
    }
    Some(template.render(&engine, &mut ctx).unwrap_or_else(|e| e.to_string()))
}

/// `page_not_found(request, exception)`.
pub fn page_not_found(request: &HttpRequest, exception: &Error) -> HttpResponse {
    let ctx = rango_template::context! { "request_path" => quote(request.path(), "/"), "exception" => exception.to_string() };
    if let Some(body) = project_template("404.html", ctx, Some(request)) {
        return html(404, body);
    }
    html(
        404,
        "<!doctype html><html lang=\"en\"><head><title>Not Found</title></head><body><h1>Not Found</h1><p>The requested resource was not found on this server.</p></body></html>".into(),
    )
}

/// `server_error(request)`.
pub fn server_error(_request: &HttpRequest) -> HttpResponse {
    if let Some(body) = project_template("500.html", Context::new(), None) {
        return html(500, body);
    }
    html(500, "<!doctype html><html lang=\"en\"><head><title>Server Error (500)</title></head><body><h1>Server Error (500)</h1></body></html>".into())
}

/// `bad_request(request, exception)`.
pub fn bad_request(request: &HttpRequest, _exception: &Error) -> HttpResponse {
    if let Some(body) = project_template("400.html", Context::new(), Some(request)) {
        return html(400, body);
    }
    html(400, "<!doctype html><html lang=\"en\"><head><title>Bad Request (400)</title></head><body><h1>Bad Request (400)</h1></body></html>".into())
}

/// `permission_denied(request, exception)`.
pub fn permission_denied(request: &HttpRequest, exception: &Error) -> HttpResponse {
    let ctx = rango_template::context! { "exception" => exception.to_string() };
    if let Some(body) = project_template("403.html", ctx, Some(request)) {
        return html(403, body);
    }
    html(403, "<!doctype html><html lang=\"en\"><head><title>403 Forbidden</title></head><body><h1>403 Forbidden</h1></body></html>".into())
}

/// Why a CSRF check failed (`middleware.csrf.REASON_*`).
pub mod reasons {
    pub const NO_REFERER: &str = "Referer checking failed - no Referer.";
    pub const BAD_REFERER: &str = "Referer checking failed - %s does not match any trusted origins.";
    pub const NO_CSRF_COOKIE: &str = "CSRF cookie not set.";
    pub const CSRF_TOKEN_MISSING: &str = "CSRF token missing.";
    pub const MALFORMED_REFERER: &str = "Referer checking failed - Referer is malformed.";
    pub const INSECURE_REFERER: &str = "Referer checking failed - Referer is insecure while host is secure.";
    pub const BAD_ORIGIN: &str = "Origin checking failed - %s does not match any trusted origins.";
}

fn esc(s: &str) -> String {
    rango_core::utils::html::escape(s).into_string()
}

/// `csrf_failure(request, reason)`: the 403 page of the CSRF middleware.
pub fn csrf_failure(request: &HttpRequest, reason: &str) -> HttpResponse {
    let debug = rango_core::conf::settings().debug;
    let no_referer = reason == reasons::NO_REFERER;
    let no_cookie = reason == reasons::NO_CSRF_COOKIE || reason == reasons::CSRF_TOKEN_MISSING;
    if let Some(body) = project_template("403_csrf.html", Context::new(), Some(request)) {
        return html(403, body);
    }
    let mut body = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta http-equiv=\"content-type\" content=\"text/html; charset=utf-8\">\n  <meta name=\"robots\" content=\"NONE,NOARCHIVE\">\n  <title>403 Forbidden</title>\n</head>\n<body>\n  <div id=\"summary\">\n    <h1>Forbidden <span>(403)</span></h1>\n    <p>CSRF verification failed. Request aborted.</p>\n",
    );
    if no_referer {
        body.push_str("    <p>You are seeing this message because this HTTPS site requires a “Referer header” to be sent by your web browser, but none was sent. This header is required for security reasons, to ensure that your browser is not being hijacked by third parties.</p>\n    <p>If you have configured your browser to disable “Referer” headers, please re-enable them, at least for this site, or for HTTPS connections, or for “same-origin” requests.</p>\n");
    }
    if no_cookie {
        body.push_str("    <p>You are seeing this message because this site requires a CSRF cookie when submitting forms. This cookie is required for security reasons, to ensure that your browser is not being hijacked by third parties.</p>\n    <p>If you have configured your browser to disable cookies, please re-enable them, at least for this site, or for “same-origin” requests.</p>\n");
    }
    body.push_str("  </div>\n");
    if debug && !reason.is_empty() {
        body.push_str(&format!("  <div id=\"info\">\n    <h2>Help</h2>\n    <p>Reason given for failure:</p>\n    <pre>\n    {}\n    </pre>\n  </div>\n", esc(reason)));
    }
    body.push_str("</body>\n</html>\n");
    html(403, body)
}
