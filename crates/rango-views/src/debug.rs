//! `views.debug`: the technical 500/404 pages and the welcome page.
//!
//! Rust has no tracebacks to show; the error chain (`source()`) is shown instead. Sensitive
//! values are hidden by [`cleanse`] exactly as `SafeExceptionReporterFilter` does.

use std::error::Error as StdError;
use std::sync::LazyLock;

use rango_core::conf::{settings, try_settings};
use rango_core::exceptions::Error;
use rango_core::utils::html::escape;
use rango_http::{HttpRequest, HttpResponse};
use rango_urls::Resolver404;
use regex::Regex;
use serde_json::Value;

/// The shared colour theme (also used by the admin).
pub const THEME_CSS: &str = include_str!("../assets/theme.css");
const LOGO_SVG: &str = include_str!("../assets/rango-logo.svg");
const CLEANSED: &str = "********************";

static HIDDEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)API|AUTH|TOKEN|KEY|SECRET|PASS|SIGNATURE|HTTP_COOKIE").expect("valid regex"));

/// `cleanse_setting(key, value)`: hide values whose name looks sensitive (recursing into objects).
pub fn cleanse(key: &str, value: &Value) -> Value {
    if HIDDEN.is_match(key) {
        return Value::String(CLEANSED.into());
    }
    match value {
        Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), cleanse(k, v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(|v| cleanse(key, v)).collect()),
        v => v.clone(),
    }
}

/// keyword-style exception class name for an [`Error`].
pub fn exception_type(error: &Error) -> String {
    let name = match error {
        Error::AppRegistryNotReady(_) => "rango.core.exceptions.AppRegistryNotReady",
        Error::ObjectDoesNotExist { .. } => "rango.core.exceptions.ObjectDoesNotExist",
        Error::MultipleObjectsReturned { .. } => "rango.core.exceptions.MultipleObjectsReturned",
        Error::Suspicious { .. } => "rango.core.exceptions.SuspiciousOperation",
        Error::PermissionDenied(_) => "rango.core.exceptions.PermissionDenied",
        Error::ViewDoesNotExist(_) => "rango.core.exceptions.ViewDoesNotExist",
        Error::MiddlewareNotUsed(_) => "rango.core.exceptions.MiddlewareNotUsed",
        Error::ImproperlyConfigured(_) => "rango.core.exceptions.ImproperlyConfigured",
        Error::FieldError(_) => "rango.core.exceptions.FieldError",
        Error::FieldDoesNotExist(_) => "rango.core.exceptions.FieldDoesNotExist",
        Error::BadRequest(_) => "rango.core.exceptions.BadRequest",
        Error::RequestAborted(_) => "rango.core.exceptions.RequestAborted",
        Error::Http404(_) => "rango.http.response.Http404",
        Error::Validation(_) => "rango.core.exceptions.ValidationError",
        Error::NoReverseMatch(_) => "rango.urls.exceptions.NoReverseMatch",
        Error::LookupError(_) => "LookupError",
        Error::EmptyResultSet => "rango.core.exceptions.EmptyResultSet",
        Error::FullResultSet => "rango.core.exceptions.FullResultSet",
        _ => "Error",
    };
    name.replacen("rango.", "rango.", 1)
}

fn table(title: &str, rows: &[(String, String)]) -> String {
    if rows.is_empty() {
        return format!("<h3 id=\"{0}-info\">{0}</h3>\n<p>No data</p>\n", escape(title));
    }
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let body: String = rows
        .iter()
        .map(|(k, v)| format!("<tr><td>{}</td><td class=\"code\"><pre>{}</pre></td></tr>\n", escape(k), escape(v)))
        .collect();
    format!("<h3 id=\"{0}-info\">{0}</h3>\n<table class=\"req\"><thead><tr><th>Variable</th><th>Value</th></tr></thead><tbody>\n{body}</tbody></table>\n", escape(title))
}

fn page(title: &str, status_line: &str, summary: &str, sections: &str, footer: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <meta http-equiv=\"content-type\" content=\"text/html; charset=utf-8\">\n  <meta name=\"robots\" content=\"NONE,NOARCHIVE\">\n  <title>{title}</title>\n  <style>\n{THEME_CSS}\n    html * {{ padding: 0; margin: 0; }}\n    body {{ font: 0.875rem/1.5 var(--font-family-primary, system-ui, sans-serif); background: var(--body-bg); color: var(--body-fg); }}\n    body * {{ padding: 10px 20px; }} body * * {{ padding: 0; }}\n    h1 {{ font-weight: normal; }} h2 {{ margin-bottom: .8em; }} h3 {{ margin: 1em 0 .5em; }}\n    code, pre {{ font-family: var(--font-family-monospace, monospace); }}\n    #summary {{ background: var(--rango-rust-ink); color: #fff; }} #summary h1, #summary h2 {{ color: #fff; }}\n    #summary table {{ border: none; background: transparent; }}\n    #summary th {{ text-align: left; color: var(--accent); padding-right: 1em; }} #summary td, #summary th {{ vertical-align: top; }}\n    #info {{ background: var(--darkened-bg); }}\n    table.req {{ border: 1px solid var(--border-color); border-collapse: collapse; width: 100%; }}\n    table.req td, table.req th {{ padding: 4px 10px; border-bottom: 1px solid var(--hairline-color); vertical-align: top; text-align: left; }}\n    a {{ color: var(--link-fg); }} #explanation {{ background: var(--darkened-bg); border-top: 1px solid var(--border-color); }}\n    .exception_value {{ font-size: 1.3em; }} .logo svg {{ width: 64px; height: 64px; float: right; }}\n  </style>\n</head>\n<body>\n<div id=\"summary\">\n  <span class=\"logo\">{LOGO_SVG}</span>\n  <h1>{status_line}</h1>\n{summary}\n</div>\n{sections}\n<div id=\"explanation\">\n  <p>{footer}</p>\n</div>\n</body>\n</html>\n"
    )
}

/// `technical_500_response(request, exc)`: the debug page for an unhandled error.
pub async fn technical_500_response(request: &HttpRequest, error: &Error, status: u16, sensitive_post: Option<&[String]>) -> HttpResponse {
    let wants_html = request.accepts("text/html");
    let chain = error_chain(error);
    let etype = exception_type(error);
    let message = error.to_string();
    let uri = request.build_absolute_uri(None).unwrap_or_else(|_| request.get_full_path());
    let time = rango_core::utils::http::http_date(None);

    if !wants_html {
        let mut t = format!("{etype} at {}\n{message}\n\nRequest Method: {}\nRequest URL: {uri}\nRango Version: {}\nServer time: {time}\n\nError chain:\n", request.path(), request.method(), rango_core::management::VERSION);
        for (i, c) in chain.iter().enumerate() {
            t.push_str(&format!("  {i}: {c}\n"));
        }
        let mut r = HttpResponse::new(t).content_type("text/plain; charset=utf-8");
        r.status_code = status;
        return r;
    }

    let summary = format!(
        "  <h1>{etype} at {path}</h1>\n  <pre class=\"exception_value\">{msg}</pre>\n  <table class=\"meta\">\n    <tr><th>Request Method:</th><td>{method}</td></tr>\n    <tr><th>Request URL:</th><td>{uri}</td></tr>\n    <tr><th>Rango Version:</th><td>{ver}</td></tr>\n    <tr><th>Exception Type:</th><td>{etype}</td></tr>\n    <tr><th>Exception Value:</th><td><pre>{msg}</pre></td></tr>\n    <tr><th>Server time:</th><td>{time}</td></tr>\n  </table>",
        path = escape(request.path()),
        msg = escape(&message),
        method = escape(request.method()),
        uri = escape(&uri),
        ver = rango_core::management::VERSION,
        etype = escape(&etype),
    );

    let mut sections = String::from("<div id=\"info\">\n<h2>Error chain</h2>\n<ol>\n");
    for c in &chain {
        sections.push_str(&format!("<li><pre>{}</pre></li>\n", escape(c)));
    }
    sections.push_str("</ol>\n<h2>Request information</h2>\n");
    let kv = |it: Vec<(&str, &str)>| it.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect::<Vec<_>>();
    sections.push_str(&table("GET", &kv(request.query().items().collect())));
    let post = match request.post().await {
        Ok(p) => {
            let hide_all = sensitive_post.is_some_and(|s| s.iter().any(|x| x == "__ALL__"));
            p.items()
                .map(|(k, v)| {
                    let hidden = hide_all || sensitive_post.is_some_and(|s| s.iter().any(|x| x == k)) || HIDDEN.is_match(k) && sensitive_post.is_none() && false;
                    (k.to_owned(), if hidden { CLEANSED.to_owned() } else { v.to_owned() })
                })
                .collect()
        }
        Err(_) => Vec::new(),
    };
    sections.push_str(&table("POST", &post));
    let cookies: Vec<(String, String)> = request.cookies().iter().map(|(k, v)| (k.clone(), if HIDDEN.is_match(k) { CLEANSED.to_owned() } else { v.clone() })).collect();
    sections.push_str(&table("COOKIES", &cookies));
    let meta: Vec<(String, String)> = request.meta().iter().map(|(k, v)| (k.clone(), if HIDDEN.is_match(k) { CLEANSED.to_owned() } else { v.clone() })).collect();
    sections.push_str(&table("META", &meta));
    if let Ok(s) = try_settings() {
        let rows: Vec<(String, String)> = s.to_map().iter().map(|(k, v)| (k.clone(), cleanse(k, v).to_string())).collect();
        sections.push_str(&table("Settings", &[("Using settings".to_owned(), "rango::conf::settings()".to_owned())]));
        sections.push_str(&table("Settings values", &rows));
    }
    sections.push_str("</div>\n");

    let footer = "You’re seeing this error because you have <code>debug = true</code> in your Rango settings. Change that to <code>false</code>, and Rango will display a standard page generated by the handler for this status code.";
    let mut r = HttpResponse::new(page(&format!("{etype} at {}", escape(request.path())), &format!("{} at {}", escape(&etype), escape(request.path())), &summary, &sections, footer));
    r.status_code = status;
    r
}

fn error_chain(error: &Error) -> Vec<String> {
    let mut out = vec![error.to_string()];
    let mut cur: Option<&dyn StdError> = error.source();
    while let Some(e) = cur {
        out.push(e.to_string());
        cur = e.source();
    }
    out
}

/// `technical_404_response(request, exception)`: lists the URL patterns that were tried.
pub fn technical_404_response(request: &HttpRequest, resolver: Option<&Resolver404>, message: &str) -> HttpResponse {
    let tried: &[Vec<String>] = resolver.map_or(&[], |r| r.tried.as_slice());
    if tried.is_empty() && request.path() == "/" {
        return default_urlconf(request);
    }
    let error_url = resolver.map_or_else(|| request.path_info().trim_start_matches('/').to_owned(), |r| r.path.trim_start_matches('/').to_owned());
    let urlconf = rango_urls::get_urlconf().or_else(|| settings().root_urlconf.clone()).unwrap_or_else(|| "ROOT_URLCONF".into());
    let mut body = String::new();
    if tried.is_empty() {
        body.push_str(&format!("<p>{}</p>", escape(message)));
    } else {
        body.push_str(&format!(
            "<p>Using the URLconf defined in <code>{}</code>, Rango tried these URL patterns, in this order:</p>\n<ol>\n",
            escape(&urlconf)
        ));
        for t in tried {
            body.push_str(&format!("<li><code>{}</code></li>\n", escape(&t.join(" "))));
        }
        body.push_str(&format!("</ol>\n<p>The current path, <code>{}</code>, didn’t match any of these.</p>", escape(&error_url)));
    }
    let summary = format!(
        "  <table class=\"meta\">\n    <tr><th>Request Method:</th><td>{}</td></tr>\n    <tr><th>Request URL:</th><td>{}</td></tr>\n  </table>",
        escape(request.method()),
        escape(&request.build_absolute_uri(None).unwrap_or_else(|_| request.get_full_path())),
    );
    let footer = "You’re seeing this error because you have <code>debug = true</code> in your Rango settings file. Change that to <code>false</code>, and Rango will display a standard 404 page.";
    let sections = format!("<div id=\"info\">\n{body}\n</div>");
    let mut r = HttpResponse::new(page("Page not found at ", "Page not found <span>(404)</span>", &summary, &sections, footer));
    r.status_code = 404;
    r
}

/// `default_urlconf(request)`: the "It worked!" welcome page shown for an empty project.
pub fn default_urlconf(_request: &HttpRequest) -> HttpResponse {
    let body = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>The install worked successfully! Congratulations!</title>\n<style>\n{THEME_CSS}\nbody {{ margin: 0; font-family: var(--font-family-primary, system-ui, sans-serif); color: var(--rango-rust-ink); background: var(--body-bg); }}\nmain {{ max-width: 40rem; margin: 0 auto; padding: 4rem 1.5rem; text-align: center; }}\nsvg {{ width: 160px; height: 160px; }}\nh1 {{ font-weight: 600; margin: 1.5rem 0 .5rem; }} p {{ color: var(--rango-ink-muted); }}\n.ok {{ color: var(--rango-lizard-deep); font-weight: 700; }}\na {{ color: var(--link-fg); }}\n</style></head>\n<body><main>\n{LOGO_SVG}\n<h1><span class=\"ok\">✓</span> The install worked successfully! Congratulations!</h1>\n<p>You are seeing this page because <code>debug = true</code> is in your settings and you have not configured any URLs.</p>\n<p>Version {ver}. Next: add a URLconf with <code>register_urlconf!</code> and set <code>ROOT_URLCONF</code>.</p>\n</main></body></html>\n",
        ver = rango_core::management::VERSION
    );
    HttpResponse::new(body)
}
