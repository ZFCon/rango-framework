# Middleware

Middleware is code that wraps every request. It can inspect or answer a request before the view runs, adjust the
response on the way out, and handle errors. This page covers the `Middleware` trait and its hooks, the order in which
they run, how to write and register your own, every built-in middleware in `rango::middleware`, and CSRF protection
in detail. A one-line-per-entry table of all middleware, including the contrib ones, is in
[Middleware reference](../../ref/middleware.md).

## How a request travels

The `middleware` setting is a list of dotted names. Rango builds one chain from it when the handler starts. The first
name is the outermost layer; the view sits at the centre.

```rust
use rango::conf::Settings;

Settings {
    middleware: [
        "rango.middleware.security.SecurityMiddleware",
        "rango.middleware.common.CommonMiddleware",
        "rango.middleware.csrf.CsrfViewMiddleware",
        "rango.middleware.clickjacking.XFrameOptionsMiddleware",
    ]
    .map(String::from)
    .to_vec(),
    ..Settings::default()
}
```

A request goes down the list, the view runs, and the response comes back up the list in reverse:

```text
request  ->  Security  ->  Common  ->  Csrf  ->  XFrameOptions  ->  URL resolution  ->  view
response <-  Security  <-  Common  <-  Csrf  <-  XFrameOptions  <-  (response from the view)
```

The default value of `middleware` is empty. A project created with `rango-admin startproject` lists the CORS,
security, common, CSRF and clickjacking middleware; see the generated `settings.rs`.

Two consequences of the ordering matter in practice:

- A middleware that needs another one's work must be listed after it. The authentication middleware reads the session,
  so `rango.contrib.sessions.middleware.SessionMiddleware` must come first.
- A middleware that must see the final response (compression, caching headers) belongs near the top, because the
  response reaches it last.

## The `Middleware` trait

```rust
use rango::core::handlers::Middleware; // also: GetResponse, Handler
```

`Middleware` is an `async_trait` trait. Every hook has a default that does nothing, so you override only what you
need. Your project needs the `async-trait` crate in its own `Cargo.toml`.

| Hook | Signature (after `async_trait`) | Runs | Returning |
|---|---|---|---|
| `process_request` | `(&self, &HttpRequest) -> Result<Option<HttpResponse>>` | Before the URL is resolved. | `Some(response)` skips everything inside this layer. |
| `process_view` | `(&self, &HttpRequest, &rango::urls::View, &ViewArgs) -> Result<Option<HttpResponse>>` | After resolution, just before the view. | `Some(response)` skips the view. |
| `process_exception` | `(&self, &HttpRequest, &rango::Error) -> Result<Option<HttpResponse>>` | When the view returned an error. | `Some(response)` handles the error. |
| `process_template_response` | `(&self, &HttpRequest, HttpResponse) -> Result<HttpResponse>` | When the view returned a template response that is not rendered yet. | The (possibly changed) response. |
| `process_response` | `(&self, &HttpRequest, HttpResponse) -> Result<HttpResponse>` | On every response on the way out. | The (possibly changed) response. |
| `call` | `(&self, HttpRequest, GetResponse) -> Result<HttpResponse>` | Wraps the whole inner chain. | The response. |

`ViewArgs` is `rango::urls::ViewArgs`: `args: Vec<Arg>` for unnamed captures and `kwargs: IndexMap<String, Arg>` for
named ones. The `View` passed to `process_view` carries `name` and `flags` (`csrf_exempt`, `xframe_options_exempt`,
`login_required_exempt`, `sensitive_post_parameters`, `extra`), which is how a middleware learns that a view opted
out of its behaviour.

The default `call` is what ties the hooks together:

1. run `process_request`; if it returns a response, use it, otherwise call the rest of the chain with `get_response`;
2. run `process_response` on whichever response came out.

So `process_response` of a layer still runs when its own `process_request` short-circuited, but the layers inside it
do not run at all.

Override `call` instead of the individual hooks when you need to do something around the inner chain (time it, wrap it
in a scope, set a task-local value). The built-in `CorsMiddleware` and `LocaleMiddleware` do this. If you override
`call`, the other hooks of that middleware are not used unless you call them yourself.

### Order of hooks across layers

With layers `A, B, C` listed in that order:

| Phase | Order |
|---|---|
| `process_request` (inside the default `call`) | A, B, C |
| `process_view` | A, B, C. The first to return `Some` stops the rest and the view. |
| `process_exception` | C, B, A. The first to return `Some` handles the error; the rest are skipped. |
| `process_template_response` | C, B, A |
| `process_response` | C, B, A |

`process_view` runs only if URL resolution succeeded; an unmatched path becomes a 404 without reaching it.
`process_exception` runs only for errors the **view** returned. An error returned by `process_view`, or by another
hook, takes the path below.

### Errors inside middleware

A hook that returns `Err` does not crash the request. The error is converted to a response (404 page, 403 page, 400
page or 500 page, honouring `debug` and the handlers installed with `set_error_handlers`) at that layer, and the
response continues outward. Layers outside it see an ordinary response in their `process_response`; the failing layer's
own `process_response` and everything inside it are skipped.

The same conversion applies to the view: a view's `Err` becomes a response before it reaches `process_response`, so
`process_response` always receives a real response and never an error. The handler's final steps are: log responses
with status 400 or above, send the `request_finished` signal, and strip the body from `HEAD` replies and from 1xx,
204 and 304 responses.

## Writing a middleware

This middleware adds a header with the time the request took. It overrides `call`, because it needs to measure the
inner chain.

```rust
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use rango::core::handlers::{GetResponse, Middleware};
use rango::http::{HttpRequest, HttpResponse};
use rango::{Result, register_middleware};

pub struct Timing;

#[async_trait]
impl Middleware for Timing {
    async fn call(&self, request: HttpRequest, get_response: GetResponse) -> Result<HttpResponse> {
        let started = Instant::now();
        let mut response = get_response(request).await;
        response.set_header("X-Elapsed-Ms", &started.elapsed().as_millis().to_string())?;
        Ok(response)
    }
}

register_middleware!("myproject.middleware.Timing", || Ok(Arc::new(Timing)));
```

`get_response(request)` returns the response of the rest of the chain. It cannot fail: errors inside are already
responses.

Hook-style middleware is shorter when you only act at one point. This one rejects requests without an API key header
and records the key for later layers:

```rust
use std::sync::Arc;

use async_trait::async_trait;
use rango::core::handlers::Middleware;
use rango::http::{HttpRequest, HttpResponse};
use rango::{Result, register_middleware};

#[derive(Clone)]
pub struct ApiKey(pub String);

pub struct RequireApiKey;

#[async_trait]
impl Middleware for RequireApiKey {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        if !request.path().starts_with("/api/") {
            return Ok(None);
        }
        match request.headers().get("X-Api-Key") {
            Some(key) => {
                request.set_ext(ApiKey(key.to_owned()));
                Ok(None)
            }
            None => Ok(Some(HttpResponse::forbidden("missing key"))),
        }
    }
}

register_middleware!("myproject.middleware.RequireApiKey", || Ok(Arc::new(RequireApiKey)));
```

A view then reads the value with `request.ext::<ApiKey>()`, which returns `Option<Arc<ApiKey>>`.
`set_ext`/`ext` store one value per Rust type per request; see [Requests and responses](requests-responses.md).

### Registration

`register_middleware!(name, factory)` registers the middleware at link time under `name`. The name is just a string
that you then put in the `middleware` setting; use a prefix that belongs to your project. `factory` is a plain
function or non-capturing closure of type `fn() -> Result<Arc<dyn Middleware>>`.

Things to know:

- The factory runs once, when the handler is built (`Handler::new()`, which `runserver` and the test `Client` call).
  One instance then serves every request on many threads, so it must be `Send + Sync` and any mutable state needs a
  lock or an atomic. Read settings in the factory if you want them fixed at start-up, or per request if you want
  `override_settings` to take effect.
- A name in `middleware` that nothing registered makes `Handler::new()` fail with an improperly-configured error:
  `Middleware '<name>' is not registered. Use register_middleware!.`
- A factory can opt out by returning `Err(rango::Error::MiddlewareNotUsed("reason".into()))`. The layer is dropped from
  the chain and a debug log line is written. Any other error aborts start-up.
- The registration must be linked into the final binary. Put it in a module that your `main.rs` reaches (a plain
  `mod middleware;` is enough).

### Template responses

`process_template_response` receives responses whose body has not been rendered, so a middleware can add context
before rendering. It is only called for such responses. See
[Template engine](../templates/engine.md#templateresponse) for `deferred_mut::<TemplateResponse>()`.

## Per-view middleware

A middleware can be applied to one view instead of the whole site. `decorator_from_middleware` runs a middleware's
`process_request`, `process_view` and `process_response` around a single view:

```rust
use std::sync::Arc;
use rango::middleware::csrf::decorator_from_middleware;
use rango::urls::path;

path("report/", decorator_from_middleware(Arc::new(RequireApiKey), report));
```

Only those three hooks run; `process_exception` and `process_template_response` do not, and a middleware that
overrides `call` (such as `Timing` above) has no effect here.

## Built-in middleware

All of these live under `rango::middleware` and are registered under the dotted names shown. Each section lists the
settings it reads.

### `rango.middleware.security.SecurityMiddleware`

Transport-security behaviour in one place.

| Setting | Default | Effect |
|---|---|---|
| `secure_ssl_redirect` | `false` | Redirect (301) plain-HTTP requests to `https://` with the same path and query. |
| `secure_ssl_host` | `None` | Host to redirect to; defaults to the request host. |
| `secure_redirect_exempt` | `[]` | Regex patterns tested against the path without its leading slash; a match skips the redirect. |
| `secure_hsts_seconds` | `0` | If above zero, add `Strict-Transport-Security: max-age=N` to HTTPS responses. |
| `secure_hsts_include_subdomains` | `false` | Append `; includeSubDomains`. |
| `secure_hsts_preload` | `false` | Append `; preload`. |
| `secure_content_type_nosniff` | `true` | Add `X-Content-Type-Options: nosniff`. |
| `secure_referrer_policy` | `"same-origin"` | Add `Referrer-Policy` with this value; empty disables it. |
| `secure_cross_origin_opener_policy` | `"same-origin"` | Add `Cross-Origin-Opener-Policy`; empty disables it. |
| `secure_proxy_ssl_header` | `None` | Not read here; it decides what `request.is_secure()` returns behind a proxy. |

The redirect happens in `process_request`, the headers in `process_response`. A header the view already set is kept
(`Strict-Transport-Security` is only added when absent; the others use "set if not set"). Put this middleware first
so redirects happen before any other work.

### `rango.middleware.common.CommonMiddleware`

| Setting | Default | Effect |
|---|---|---|
| `disallowed_user_agents` | `[]` | Regex patterns searched in the `User-Agent` header; a match returns 403. An invalid pattern is an improperly-configured error. |
| `prepend_www` | `false` | Redirect (301) hosts that do not start with `www.` to the `www.` host. |
| `append_slash` | `true` | When a request would 404 but the same path plus `/` matches a URL pattern, redirect (301) to the slash form. |

It also sets `Content-Length` on non-streaming responses that lack one.

`append_slash` runs in `process_response` and only on a 404. A view can opt out by carrying the marker
`no_append_slash` in `view.flags.extra`. In debug mode, a `POST`, `PUT` or `PATCH` to a URL that would be redirected
fails with a message telling you to fix the form action, because a redirect would lose the request body.

### `rango.middleware.csrf.CsrfViewMiddleware`

Cross-site request forgery protection. It is described in full [below](#csrf-protection).

### `rango.middleware.clickjacking.XFrameOptionsMiddleware`

Adds `X-Frame-Options` with the value of `x_frame_options` (default `"DENY"`, upper-cased on output) to every
response, unless the response already has the header or the view is wrapped in
`rango::views::decorators::xframe_options_exempt`. Per-view alternatives are `xframe_options_deny` and
`xframe_options_sameorigin` in the same module.

### `rango.middleware.cors.CorsMiddleware`

Adds cross-origin response headers and answers preflight `OPTIONS` requests without calling the view. It reads the
`cors_*` settings; they are documented in [CORS](../cors.md). Place it near the top so that even error responses from
inner layers carry the headers.

### `rango.middleware.gzip.GZipMiddleware`

Compresses responses with gzip.

- Responses under 200 bytes (non-streaming) are left alone.
- `Vary: Accept-Encoding` is added to every response that is eligible, whether or not the client accepts gzip.
- Nothing happens if the request's `Accept-Encoding` lacks `gzip`, or the response already has a `Content-Encoding`.
- A non-streaming body is compressed and kept only if the result is smaller; `Content-Length` is updated and a strong
  `ETag` becomes weak (`W/"..."`).
- A streaming body is compressed chunk by chunk. Each chunk is a separate complete gzip member and `Content-Length` is
  removed.

It reads no settings. List it above `ConditionalGetMiddleware` so that the ETag is computed from the uncompressed
body and then weakened.

### `rango.middleware.http.ConditionalGetMiddleware`

Makes `GET` responses conditional. For `GET` only: unless the response has `Cache-Control: no-store`, an `ETag`, or a
streaming body, it adds an `ETag` (an MD5 digest of the body, quoted). Then it evaluates the request's `If-None-Match`,
`If-Modified-Since`, `If-Match` and `If-Unmodified-Since` headers against the response's `ETag` and `Last-Modified`
and answers `304 Not Modified` (or `412`) where they apply. The view still runs and renders; the saving is bandwidth,
not server work. For work saved on the server, use [caching](../cache.md) or the `condition` view decorator.

### `rango.middleware.locale.LocaleMiddleware`

Chooses the request language and activates it for the duration of the request.

1. If the `language_cookie_name` cookie (default `rango_language`) names a supported language, use it.
2. Otherwise walk the `Accept-Language` header by quality and use the first supported language (`en-gb` falls back to
   `en`). A `*` entry stops the search.
3. Otherwise use `language_code` (default `en-us`).

Supported means present in `languages`, equal to `language_code`, or having a translation catalog. The response gets
`Content-Language` (if not already set) and `Vary: Accept-Language`. The middleware never sets the cookie itself.
Language prefixes in URLs (`i18n_patterns`) are not supported. See [Internationalization](../i18n.md).

### Cache middleware

`rango.middleware.cache.UpdateCacheMiddleware` and `rango.middleware.cache.FetchFromCacheMiddleware` cache whole
pages; `rango.middleware.cache.CacheMiddleware` is both halves in one entry (as a single middleware it must be used
alone). Settings: `cache_middleware_alias` (`"default"`), `cache_middleware_key_prefix` (`""`) and
`cache_middleware_seconds` (`600`). These are read once, when the chain is built.

```rust
middleware: [
    "rango.middleware.cache.UpdateCacheMiddleware",   // first: sees the finished response last
    "rango.middleware.common.CommonMiddleware",
    // ...
    "rango.middleware.cache.FetchFromCacheMiddleware", // last: answers from the cache before inner work
]
```

Only `GET` and `HEAD` requests are served from the cache. Only responses with status 200 or 304 are stored, and not
streaming ones, ones with `Cache-Control: private`, or ones that set cookies and `Vary: Cookie`. The lifetime is the
response's `max-age` if it has one, otherwise `cache_middleware_seconds`; zero means do not store. Stored
responses get `Expires` and `Cache-Control: max-age`. The key includes the full URL (query string included), the
method and the request headers named in the response's `Vary`. `rango::middleware::cache::cache_page(seconds, view)`
applies the same logic to a single view. Details and limits are in [Caching](../cache.md).

## CSRF protection

`CsrfViewMiddleware` rejects state-changing requests that were not initiated from your own pages. It combines a
cookie, a per-form token and origin checks.

### Using it in forms

Include the tag in every `POST` form that targets your site:

```html
<form method="post" action="{% url 'polls:vote' question.pk %}">
    {% csrf_token %}
    ...
</form>
```

`{% csrf_token %}` writes `<input type="hidden" name="csrfmiddlewaretoken" value="...">`. It needs a `csrf_token`
variable in the template context, which the built-in context processor
`rango.template.context_processors.csrf` supplies for every template rendered with a request (it cannot be left out).
Templates rendered without a request have no token and the tag outputs nothing.

In a view that builds HTML by hand, call `rango::middleware::csrf::get_token(&request)` for the token string.

### What the middleware does

1. **Request.** `process_request` reads the cookie named by `csrf_cookie_name` (default `csrftoken`). If it is
   missing nothing happens yet. If it has the wrong length or characters, it is discarded and a new secret is made.
2. **Token creation is lazy.** A secret is generated, and the cookie set on the response, only when something calls
   `get_token`: `{% csrf_token %}`, the `csrf_token` variable, or `rotate_token`. A page that never uses a token
   sets no cookie.
3. **Check.** `process_view` checks every request whose method is not `GET`, `HEAD`, `OPTIONS` or `TRACE`, unless the
   view is marked `csrf_exempt`.
4. **Response.** If a secret was created or used, the cookie is written and `Vary: Cookie` is added.

The cookie holds a 32-character secret. The token placed in a form is 64 characters: a random 32-character mask plus
the secret combined with that mask. A new mask is drawn on every call, so tokens differ from page to page while the
secret stays the same. Either form (the 64-character token or the raw 32-character secret) is accepted on input.

The check runs in this order, and the first failure answers 403:

1. **`Origin` header present.** It must equal `scheme://host` of this request (the host is validated against
   `allowed_hosts`), or appear in `csrf_trusted_origins`, or match a wildcard entry such as `https://*.example.com`
   (same scheme, host is the domain or a subdomain). Otherwise the reason is
   `Origin checking failed - <origin> does not match any trusted origins.`
2. **No `Origin`, and the request is HTTPS.** The `Referer` header is required and must be well formed, use `https`,
   and match the request host (or the host of `csrf_cookie_domain` when that is set) or a trusted origin. The reasons
   are `Referer checking failed - no Referer.`, `... Referer is malformed.`, `... Referer is insecure while host is
   secure.` and `... <referer> does not match any trusted origins.`. Plain-HTTP requests without `Origin` skip this
   step.
3. **Cookie.** A request with no secret fails with `CSRF cookie not set.`
4. **Token.** For `POST`, the form field `csrfmiddlewaretoken` is read. If it is empty, or the method is not `POST`, the
   request header named by `csrf_header_name` is used. The default is `HTTP_X_CSRFTOKEN`, which is the header
   `X-CSRFToken`. A missing token fails with `CSRF token missing.`; a malformed one with `CSRF token has incorrect
   length.` or `CSRF token has invalid characters (...).`; a wrong one with `CSRF token incorrect.` Comparison is
   constant-time.

Because only `POST` bodies are parsed for the form field, `PUT`, `PATCH` and `DELETE` requests must send the token in
the header.

### Ajax and API clients

Read the cookie in the browser and send it back as a header. This requires `csrf_cookie_httponly` to stay `false`,
which is the default.

```javascript
const token = document.cookie.split("; ").find(c => c.startsWith("csrftoken="))?.split("=")[1];
fetch("/polls/1/vote/", {
  method: "POST",
  headers: { "X-CSRFToken": token, "Content-Type": "application/json" },
  body: JSON.stringify({ choice: 1 }),
});
```

If the page has no form, nothing creates the cookie. Wrap the view in `ensure_csrf_cookie` so it is always sent:

```rust
use rango::middleware::csrf::ensure_csrf_cookie;
use rango::urls::path;

path("app/", ensure_csrf_cookie(app_shell));
```

Token-authenticated REST endpoints that do not use cookies for authentication should be marked `csrf_exempt`.

### Settings

| Setting | Default | Meaning |
|---|---|---|
| `csrf_cookie_name` | `"csrftoken"` | Cookie name. |
| `csrf_cookie_age` | `31449600` | Cookie `max-age` in seconds (one year). |
| `csrf_cookie_domain` | `None` | Cookie domain; also used as the expected referer host. |
| `csrf_cookie_path` | `"/"` | Cookie path. |
| `csrf_cookie_secure` | `false` | Send the cookie over HTTPS only. |
| `csrf_cookie_httponly` | `false` | Hide the cookie from scripts. If `true`, Ajax code cannot read it. |
| `csrf_cookie_samesite` | `"Lax"` | `SameSite` attribute; empty omits it. |
| `csrf_header_name` | `"HTTP_X_CSRFTOKEN"` | Request-metadata key of the header carrying the token. |
| `csrf_trusted_origins` | `[]` | Extra origins allowed to submit, such as `"https://app.example.com"` or `"https://*.example.com"`. |

### Decorators

| Function | Module | Use |
|---|---|---|
| `csrf_exempt(view)` | `rango::views::decorators` | Skip the check for this view. |
| `csrf_protect(view)` | `rango::middleware::csrf` | Enforce the check on this view even when the middleware is not installed. |
| `requires_csrf_token(view)` | `rango::middleware::csrf` | Make `get_token` work in the view but never reject. |
| `ensure_csrf_cookie(view)` | `rango::middleware::csrf` | Always send the cookie. |

When `CsrfViewMiddleware` and `csrf_protect` are both active, the check runs once.

### Failures

A rejected request gets a 403 response. If a template named `403_csrf.html` exists in your template directories it is
rendered with no context; otherwise a built-in page is used, which includes the reason only when `debug` is on. Every
rejection is logged to the `rango.security.csrf` target at warning level.

`rotate_token(&request)` replaces the secret. The login function calls it, so a token captured before login cannot be
reused afterwards.

### Testing

The test `Client` does not enforce CSRF by default. Opt in with `Client::new()?.enforce_csrf_checks(true)`, fetch the
form page, read the `csrfmiddlewaretoken` value from the HTML, and post it back; the cookie jar carries `csrftoken`
automatically.

## Not supported yet

- `csrf_use_sessions = true` (storing the secret in the session). With it set, every request fails with an
  improperly-configured error.
- `csrf_failure_view`: the setting exists, but the failure page is always the built-in one or `403_csrf.html`.
- A signal or hook to enable or disable CORS per request; use `cors_urls_regex` and the origin regexes.
- Language-prefixed URLs with `LocaleMiddleware`.
- Passing configuration to a middleware from `middleware` (the list holds names only). Read your own settings in the
  factory or the hooks.

## See also

- [Middleware reference](../../ref/middleware.md)
- [URL dispatcher](urls.md)
- [Writing views](views.md)
- [Requests and responses](requests-responses.md)
- [Caching](../cache.md)
- [CORS](../cors.md)
- [Security](../security.md)
- [Settings](../settings.md)
