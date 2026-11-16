# Writing views

A view is an async function that takes a request and returns a response or an error. This page covers function
views and closures, how errors turn into status codes and error pages, view decorators, serving files during
development, and the welcome page.

## A function view

```rust
// src/polls/views.rs
use rango::Result;
use rango::http::{HttpRequest, HttpResponse};

pub async fn index(_request: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new("Hello, world. You're at the polls index."))
}

pub async fn detail(request: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    Ok(HttpResponse::new(format!("Question {question_id}, asked with {}", request.method())))
}
```

```rust
// src/polls/urls.rs
use rango::urls::path;
use rango::{register_urlconf, urlpatterns};

use super::views;

register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", views::index).name("index"),
    path("<int:question_id>/", views::detail).name("detail"),
]);
```

The shape of a view is fixed:

- The first parameter is the `HttpRequest`, taken by value. It is a cheap shared handle, so cloning it or moving it
  into another task is fine.
- Zero to six further parameters receive the values captured from the URL, **by position**. Their types must
  implement `rango::urls::FromArg` (`String`, the integer types, `uuid::Uuid`). See
  [URL dispatcher](urls.md#how-captured-values-reach-the-view).
- The return type is `rango::Result<HttpResponse>`, which is `Result<HttpResponse, rango::Error>`.
- The function must be `async` and its future must be `Send`, because requests are handled on a multi-threaded
  runtime. Do not hold a `std::sync::MutexGuard` or other non-`Send` value across an `.await`.

Views that use the database take the connection explicitly, so it is always visible which database a view talks
to:

```rust
use rango::db::Db;
use rango::shortcuts::{get_object_or_404, render};
use rango::views::generic::model_value;
use rango::{Result, context};

pub async fn detail(request: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    let db = Db::default_db();
    let question = get_object_or_404(Question::objects().filter_kw("pk", question_id), &db).await?;
    render(&request, "polls/detail.html", context! { "question" => model_value(&question) })
}
```

`model_value` turns a model instance into a template value (`{{ question.question_text }}`, `{{ question.pk }}`,
`{{ question }}`).

## Closures as views

Anything callable with the right signature is a view, including closures. Annotate the request parameter so the
signature can be inferred:

```rust
use rango::http::{HttpRequest, HttpResponse};

register_urlconf!("health.urls", || urlpatterns![
    path("ping/", |_: HttpRequest| async { Ok(HttpResponse::new("pong")) }),
    path("echo/<str:word>/", |_: HttpRequest, word: String| async move { Ok(HttpResponse::new(word)) }),
]);
```

A closure used as a view must be `Clone + Send + Sync + 'static`. To share state with it, capture an `Arc`.

`path()` also accepts a ready-made `rango::urls::View` value. That is what decorators, generic views
(`TemplateView::new("about.html").as_view()`) and `View::new(name, handler)` produce.

## Returning errors

A view reports a failure by returning `Err(rango::Error::...)`. The request handler converts the error into a
response, so `?` can be used freely.

```rust
use rango::Error;

pub async fn detail(request: HttpRequest, id: i64) -> Result<HttpResponse> {
    if id > 1000 {
        return Err(Error::Http404("No such question.".into()));
    }
    if request.method() == "DELETE" {
        return Err(Error::PermissionDenied("Questions cannot be deleted.".into()));
    }
    // ...
    Ok(HttpResponse::new("ok"))
}
```

| Error variant | Status | Notes |
|---|---|---|
| `Error::Http404(msg)` | 404 | also produced when no URL pattern matches |
| `Error::PermissionDenied(msg)` | 403 | logged as a warning to `rango.request` |
| `Error::BadRequest(msg)` | 400 | for example a malformed multipart body |
| `Error::Suspicious { kind, message }` | 400 | `SuspiciousKind`: `DisallowedHost`, `DisallowedRedirect`, `RequestDataTooBig`, `TooManyFieldsSent`, `TooManyFilesSent`, `SuspiciousFileOperation`, ...; logged to `rango.security` |
| any other variant | 500 | `ObjectDoesNotExist`, `MultipleObjectsReturned`, `Validation`, `NoReverseMatch`, `ImproperlyConfigured`, `FieldError`, `Other`, ... |

Constructors exist for the common ones: `Error::http404(msg)`, `Error::permission_denied(msg)`,
`Error::bad_request(msg)`, `Error::suspicious(kind, msg)`, `Error::improperly_configured(msg)`.

Points that are easy to miss:

- `QuerySet::get()` returns `Error::ObjectDoesNotExist` when there is no row, which is a **500**. Use
  `rango::shortcuts::get_object_or_404` to get a 404 instead.
- `Error` is `#[non_exhaustive]`; match with a wildcard arm.
- `Error::Other` wraps a `Box<dyn std::error::Error + Send + Sync>`. `?` converts a boxed error automatically, but
  not a concrete foreign error type. Convert those explicitly:

  ```rust
  let text = std::fs::read_to_string("notes.txt").map_err(|e| Error::Other(Box::new(e)))?;
  let n: i32 = raw.parse().map_err(|_| Error::BadRequest("n must be a number".into()))?;
  ```

- For every 500, the `got_request_exception` signal (`rango::core::signals::GOT_REQUEST_EXCEPTION`) is sent and
  the error is logged to `rango.request`.
- Every response with a status of 400 or above is logged to `rango.request` (warning for 4xx, error for 5xx).
- With `debug_propagate_exceptions: true`, a 500-class error panics instead of becoming a response. This is meant
  for tests.

A view can also return an error status directly, without going through the error views:

```rust
Ok(HttpResponse::not_found("<h1>No such page</h1>"))
Ok(HttpResponse::forbidden("Not yours."))
Ok(HttpResponse::with_status("teapot", 418)?)
```

Middleware can intercept view errors before they are converted; see `process_exception` in
[Middleware](middleware.md).

## Error pages

Which page is shown depends on the `debug` setting.

### With `debug: true`

- **404**: a diagnostic page with the request method and URL and the list of URL patterns that were tried, in
  order. If the 404 came from a view, the error message is shown instead of the pattern list.
- **400 and 500**: a diagnostic page with the error type and message, the error chain (each `source()` of the
  error), the request's `GET`, `POST`, `COOKIES` and `META` values, and the settings. If the client's `Accept`
  header does not allow `text/html`, a plain-text version is returned.
- **403**: the same page as in production (below).

Values whose name contains `API`, `AUTH`, `TOKEN`, `KEY`, `SECRET`, `PASS`, `SIGNATURE` or `HTTP_COOKIE` are
replaced with asterisks in the settings, cookies and `META` tables. POST values are hidden only for views that
ask for it with the `sensitive_post_parameters` decorator.

Never run with `debug: true` in production: the pages show settings and request data.

### With `debug: false`

Each status has an error view that renders a template from the project if one exists, and falls back to a minimal
built-in page otherwise. Put the templates at the top level of a template directory.

| Template | Status | Context | Rendered with context processors |
|---|---|---|---|
| `404.html` | 404 | `request_path` (URL-quoted path), `exception` (the error message) | yes |
| `403.html` | 403 | `exception` | yes |
| `400.html` | 400 | none | yes |
| `500.html` | 500 | none | no (no request is bound, so keep it self-contained) |
| `403_csrf.html` | 403 | none | yes; used when the CSRF check rejects a request |

```html
<!-- templates/404.html -->
{% extends "base.html" %}
{% block content %}
  <h1>Page not found</h1>
  <p>Nothing lives at {{ request_path }}.</p>
{% endblock %}
```

To replace an error view with your own function, use `rango::core::handlers::set_error_handlers`; see
[URL dispatcher](urls.md#when-nothing-matches-and-error-views).

## View decorators

A decorator is a function that takes a view and returns a `rango::urls::View`. Apply it where the view is routed:

```rust
use rango::views::decorators::{never_cache, require_http_methods, require_post};

register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", never_cache(views::index)).name("index"),
    path("<int:id>/vote/", require_post(views::vote)).name("vote"),
    path("<int:id>/", require_http_methods(&["GET", "POST"], views::detail)).name("detail"),
]);
```

Decorators nest: `never_cache(require_post(views::vote))`. The outermost one runs first.

### `rango::views::decorators`

| Decorator | Effect |
|---|---|
| `require_http_methods(&["GET", "POST"], view)` | 405 with an `Allow` header for any other method. Method names are upper-case. |
| `require_get(view)` | only `GET` |
| `require_post(view)` | only `POST` |
| `require_safe(view)` | only `GET` and `HEAD` |
| `csrf_exempt(view)` | the CSRF middleware skips this view |
| `xframe_options_exempt(view)` | the clickjacking middleware does not add `X-Frame-Options` |
| `xframe_options_deny(view)` | sets `X-Frame-Options: DENY` unless the view set the header |
| `xframe_options_sameorigin(view)` | sets `X-Frame-Options: SAMEORIGIN` unless the view set the header |
| `never_cache(view)` | adds `Expires` and `Cache-Control: max-age=0, no-cache, no-store, must-revalidate, private` |
| `cache_control(&[(name, Cc)], view)` | merges directives into `Cache-Control` |
| `vary_on_headers(&["User-Agent"], view)` | adds header names to `Vary` |
| `vary_on_cookie(view)` | adds `Cookie` to `Vary` |
| `sensitive_post_parameters(&["password"], view)` | hides those POST values on the debug error page; `"__ALL__"` hides every value |
| `condition(etag_fn, last_modified_fn, view)` | conditional requests (`If-None-Match`, `If-Modified-Since`, `If-Match`, `If-Unmodified-Since`) |
| `etag(etag_fn, view)` | `condition` with only an ETag function |
| `last_modified(last_modified_fn, view)` | `condition` with only a last-modified function |

`cache_control` takes `rango::http::cache_utils::Cc` values: `Cc::Flag` for a bare directive and
`Cc::Value(String)` for one with a value. Underscores in names become dashes.

```rust
use rango::http::cache_utils::Cc;
use rango::views::decorators::cache_control;

path("logo/", cache_control(&[("public", Cc::Flag), ("max_age", Cc::Value("86400".into()))], views::logo))
```

`condition`, `etag` and `last_modified` take synchronous functions of `(&HttpRequest, &ViewArgs)`. The ETag
function returns the unquoted tag; the last-modified function returns seconds since the Unix epoch. When the
request's conditional headers are satisfied, the view is not called at all and a 304 (or 412) is returned. On
`GET` and `HEAD` the computed `ETag` and `Last-Modified` headers are added to the response.

```rust
use rango::views::decorators::etag;

path("feed/", etag(|_req, _args| Some(format!("v{}", current_feed_version())), views::feed))
```

### Decorators that live with their middleware

| Decorator | Module | Effect |
|---|---|---|
| `csrf_protect(view)` | `rango::middleware::csrf` | enforce the CSRF check for this view even without the middleware |
| `requires_csrf_token(view)` | `rango::middleware::csrf` | make the token available, never reject |
| `ensure_csrf_cookie(view)` | `rango::middleware::csrf` | always send the CSRF cookie |
| `cache_page(seconds, view)` | `rango::middleware::cache` | cache this view's `GET`/`HEAD` responses |
| `decorator_from_middleware(mw, view)` | `rango::middleware::csrf` | run any middleware's hooks around one view |
| `login_required(view)` | `rango::contrib::auth::decorators` | redirect anonymous users to the login page |
| `permission_required(view, &["app.perm"], raise)` | `rango::contrib::auth::decorators` | require permissions |
| `user_passes_test(view, login_url, field, test)` | `rango::contrib::auth::decorators` | require a custom test |

There is no dedicated decorator for gzip compression; wrap a single view with the middleware instead:

```rust
use std::sync::Arc;

use rango::middleware::csrf::decorator_from_middleware;
use rango::middleware::gzip::GZipMiddleware;

path("report/", decorator_from_middleware(Arc::new(GZipMiddleware), views::report))
```

### Writing a decorator

Convert the argument with `into_view()` and wrap it. The wrapper receives the inner view, the request and the
captured URL arguments.

```rust
use rango::urls::{IntoView, View};

pub fn served_by<M>(view: impl IntoView<M>) -> View {
    view.into_view().wrap(|inner, req, args| {
        Box::pin(async move {
            let mut response = inner.call(req, args).await?;
            response.set_header("X-Served-By", "rango")?;
            Ok(response)
        })
    })
}
```

To short-circuit, return a response without calling `inner`. To mark a view for a middleware instead of wrapping
it, set a flag: `view.into_view().with_flags(|f| f.csrf_exempt = true)`. `ViewFlags` has `csrf_exempt`,
`xframe_options_exempt`, `login_required_exempt`, `sensitive_post_parameters` and a free-form `extra` map for your
own markers. Flags survive wrapping.

## Serving files during development

`rango::views::static_serve::serve` returns a file from a directory. It is meant for development: use it to serve
uploaded media (or any directory) while running the development server.

```rust
use std::path::Path;

use rango::Result;
use rango::http::{HttpRequest, HttpResponse};
use rango::views::static_serve::serve;

async fn media(request: HttpRequest, path: String) -> Result<HttpResponse> {
    serve(&request, &path, Path::new("media"), false).await
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("media/<path:path>", media),
]);
```

`serve(request, path, document_root, show_indexes)`:

- normalises the path and refuses anything that escapes `document_root`;
- returns 404 for a missing file, and for a directory unless `show_indexes` is `true`, in which case a plain
  listing is produced (names starting with `.` are left out);
- streams the file with a guessed `Content-Type`, `Content-Length` and `Last-Modified`;
- answers `If-Modified-Since` with a 304 when the file has not changed.

The route is not added for you, and the development server does not serve static files on its own. Only add such a
route when `debug` is on. In production, let the web server or a CDN serve static and media files; see
[Static files](../static-files.md) and [Deployment](../deployment.md).

`rango::contrib::staticfiles::serve(request, path)` does the same for static files, looking the path up through
the static file finders instead of a single directory.

## The welcome page

A new project has no URL patterns of its own. When `debug` is `true`, the root URLconf yields no pattern to try,
and the request is for `/`, Rango answers with a welcome page ("The install worked successfully!
Congratulations!") instead of a 404. It disappears as soon as the root URLconf contains a pattern, or when `debug`
is `false`.

The page is produced by `rango::views::debug::default_urlconf`.

## Not supported yet

- Synchronous views. Every view is `async`.
- Views with more than six URL parameters. Read the values from `ResolverMatch` or build a `View` by hand
  (see [URL dispatcher](urls.md#reading-parameters-by-name-viewargs)).
- A `gzip_page` decorator; use `decorator_from_middleware` as shown above.
- Stack traces on the debug error page. The page shows the error chain instead.

## See also

- [URL dispatcher](urls.md)
- [Requests and responses](requests-responses.md)
- [Shortcuts](shortcuts.md)
- [Generic views](generic-views.md)
- [Middleware](middleware.md)
- [File uploads](file-uploads.md)
- [Template language](../templates/language.md)
- [Testing](../testing.md)
