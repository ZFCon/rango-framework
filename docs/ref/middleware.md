# Middleware reference

Every middleware that ships with Rango: its dotted name for the `middleware` setting, what it does, the settings it
reads and where it belongs in the list. For how middleware works, how to write your own and how the CSRF check
behaves, see [Middleware](../topics/http/middleware.md).

The list is ordered: the first entry is outermost. Requests pass through the entries from top to bottom; responses pass
back from bottom to top. "Position" below is advice about where an entry goes relative to the others.

## Core middleware

| Dotted name | Purpose | Settings read | Position |
|---|---|---|---|
| `rango.middleware.cors.CorsMiddleware` | Adds `Access-Control-*` headers to responses and answers preflight `OPTIONS` requests itself. | `cors_allowed_origins`, `cors_allowed_origin_regexes`, `cors_allow_all_origins`, `cors_allow_credentials`, `cors_allow_headers`, `cors_allow_methods`, `cors_allow_private_network`, `cors_expose_headers`, `cors_preflight_max_age`, `cors_urls_regex` | Near the top, above anything that can answer a request early (security redirects, cache, common). |
| `rango.middleware.security.SecurityMiddleware` | HTTPS redirect, `Strict-Transport-Security`, `X-Content-Type-Options`, `Referrer-Policy`, `Cross-Origin-Opener-Policy`. | `secure_ssl_redirect`, `secure_ssl_host`, `secure_redirect_exempt`, `secure_hsts_seconds`, `secure_hsts_include_subdomains`, `secure_hsts_preload`, `secure_content_type_nosniff`, `secure_referrer_policy`, `secure_cross_origin_opener_policy` | First or right after CORS. |
| `rango.middleware.cache.UpdateCacheMiddleware` | Stores finished `GET`/`HEAD` responses in the cache. | `cache_middleware_alias`, `cache_middleware_key_prefix`, `cache_middleware_seconds` | Near the top (it must see the response last). Pair with `FetchFromCacheMiddleware`. |
| `rango.middleware.gzip.GZipMiddleware` | Compresses responses of 200 bytes or more for clients that accept gzip; adds `Vary: Accept-Encoding`. | none | Above `ConditionalGetMiddleware`, and below the cache's update half if you want compressed pages cached. |
| `rango.middleware.common.CommonMiddleware` | Blocks user agents, `www.` redirect, trailing-slash redirect on 404, sets `Content-Length`. | `disallowed_user_agents`, `prepend_www`, `append_slash` | After `SecurityMiddleware`, before anything that needs the final URL. |
| `rango.middleware.locale.LocaleMiddleware` | Picks the request language from the cookie or `Accept-Language`, activates it, sets `Content-Language` and `Vary: Accept-Language`. | `language_cookie_name`, `languages`, `language_code` | Before `CommonMiddleware`. It reads only the cookie and the header, so it does not depend on sessions. |
| `rango.middleware.csrf.CsrfViewMiddleware` | Rejects state-changing requests without a valid CSRF token or trusted origin; sets the token cookie. | `csrf_cookie_name`, `csrf_cookie_age`, `csrf_cookie_domain`, `csrf_cookie_path`, `csrf_cookie_secure`, `csrf_cookie_httponly`, `csrf_cookie_samesite`, `csrf_header_name`, `csrf_trusted_origins`, `debug` (error detail) | After sessions, before the authentication middleware and any view-level code that submits forms. |
| `rango.middleware.http.ConditionalGetMiddleware` | Adds an `ETag` to `GET` responses and answers `304`/`412` for conditional requests. | none | Below `GZipMiddleware`; near the bottom otherwise, so it sees the response after others have changed it. |
| `rango.middleware.clickjacking.XFrameOptionsMiddleware` | Adds `X-Frame-Options` unless the response has one or the view is exempt. | `x_frame_options` | Near the bottom. |
| `rango.middleware.cache.FetchFromCacheMiddleware` | Answers `GET`/`HEAD` requests from the cache before the view runs. | `cache_middleware_alias`, `cache_middleware_key_prefix` | Last in the list, so every other middleware has run. |
| `rango.middleware.cache.CacheMiddleware` | Both halves of the page cache in one entry. Use it instead of the pair, never together with them. | `cache_middleware_alias`, `cache_middleware_key_prefix`, `cache_middleware_seconds` | Alone; its place decides what is cached and what runs on a cache hit. |

The table is in the order a typical project lists them, not alphabetical. Settings listed for the cache middleware are
read once, when the handler is built. The other middleware read their settings on every request.

A typical order:

```rust
middleware: [
    "rango.middleware.cache.UpdateCacheMiddleware",
    "rango.middleware.cors.CorsMiddleware",
    "rango.middleware.security.SecurityMiddleware",
    "rango.contrib.sessions.middleware.SessionMiddleware",
    "rango.middleware.gzip.GZipMiddleware",
    "rango.middleware.locale.LocaleMiddleware",
    "rango.middleware.common.CommonMiddleware",
    "rango.middleware.csrf.CsrfViewMiddleware",
    "rango.contrib.auth.middleware.AuthenticationMiddleware",
    "rango.contrib.messages.middleware.MessageMiddleware",
    "rango.middleware.http.ConditionalGetMiddleware",
    "rango.middleware.clickjacking.XFrameOptionsMiddleware",
    "rango.middleware.cache.FetchFromCacheMiddleware",
]
```

## Contrib middleware

These ship with the contrib apps. They are available when the app crate is linked, which the `rango` facade does for
you.

| Dotted name | Purpose | Settings read | Position |
|---|---|---|---|
| `rango.contrib.sessions.middleware.SessionMiddleware` | Loads the session named by the session cookie into the request and saves it, sets or clears the cookie, adds `Vary: Cookie` when the session was read. | `session_cookie_name`, `session_cookie_path`, `session_cookie_domain`, `session_cookie_secure`, `session_cookie_httponly`, `session_cookie_samesite`, `session_cookie_age`, `session_expire_at_browser_close`, `session_save_every_request`, `session_engine`, `session_file_path` | Before `AuthenticationMiddleware`, `MessageMiddleware` and anything using sessions; usually before CSRF. |
| `rango.contrib.auth.middleware.AuthenticationMiddleware` | Attaches the current user to the request (`request.user()`); anonymous when nobody is logged in. Fails with an improperly-configured error when the session middleware is missing. | `auth_user_model`, `authentication_backends` | After `SessionMiddleware`. |
| `rango.contrib.auth.middleware.LoginRequiredMiddleware` | Redirects anonymous users to the login page for every view not marked `login_required_exempt` in its view flags. | `login_url` | After `AuthenticationMiddleware`. |
| `rango.contrib.messages.middleware.MessageMiddleware` | Makes the one-time message queue available to the request and stores unread messages in the cookie or session when the response is built. | `message_storage`, `message_level`, `message_tags`, `session_cookie_*` (cookie storage attributes), `secret_key` | After `SessionMiddleware` when the session storage is used. |
| `rango.contrib.redirects.middleware.RedirectFallbackMiddleware` | When a response is 404, looks the path up in the redirects table for the current site: 301 to the new path, or 410 when the new path is empty. | `append_slash`, `site_id` | Near the bottom, where it sees 404 responses from everything inside. |
| `rango.contrib.flatpages.middleware.FlatpageFallbackMiddleware` | When a response is 404, looks for a flat page at the path (and with a trailing slash if `append_slash` is on) and renders it. | `append_slash`, `site_id` | Near the bottom, so it sees 404 responses from everything inside. |

See [Sessions](../topics/sessions.md), [Authentication](../topics/auth/overview.md), [Messages](../topics/messages.md)
and [Sites, flat pages and redirects](../topics/sites-flatpages-redirects.md).

## View decorators related to middleware

| Function | Module | Pairs with |
|---|---|---|
| `csrf_exempt(view)` | `rango::views::decorators` | `CsrfViewMiddleware` |
| `csrf_protect(view)`, `requires_csrf_token(view)`, `ensure_csrf_cookie(view)` | `rango::middleware::csrf` | `CsrfViewMiddleware` |
| `xframe_options_exempt(view)`, `xframe_options_deny(view)`, `xframe_options_sameorigin(view)` | `rango::views::decorators` | `XFrameOptionsMiddleware` |
| `cache_page(seconds, view)` | `rango::middleware::cache` | the cache middleware |
| `never_cache(view)`, `cache_control(directives, view)`, `vary_on_headers(headers, view)`, `vary_on_cookie(view)` | `rango::views::decorators` | any caching layer |
| `condition(etag_fn, last_modified_fn, view)` | `rango::views::decorators` | `ConditionalGetMiddleware` |
| `decorator_from_middleware(middleware, view)` | `rango::middleware::csrf` | turns any `Middleware` into a per-view wrapper |

## The `Middleware` trait

Defined in `rango::core::handlers` (also exported there: `GetResponse`, `Handler`, `set_error_handlers`). Hooks, all
optional:

| Hook | Runs | Short-circuits with |
|---|---|---|
| `process_request(&self, &HttpRequest) -> Result<Option<HttpResponse>>` | Before URL resolution, in list order. | `Some(response)` |
| `process_view(&self, &HttpRequest, &View, &ViewArgs) -> Result<Option<HttpResponse>>` | Before the view, in list order. | `Some(response)` |
| `process_exception(&self, &HttpRequest, &Error) -> Result<Option<HttpResponse>>` | When the view fails, in reverse list order. | `Some(response)` |
| `process_template_response(&self, &HttpRequest, HttpResponse) -> Result<HttpResponse>` | For unrendered template responses, in reverse list order. | |
| `process_response(&self, &HttpRequest, HttpResponse) -> Result<HttpResponse>` | On every response, in reverse list order. | |
| `call(&self, HttpRequest, GetResponse) -> Result<HttpResponse>` | Wraps the whole inner chain; the default runs `process_request`, the inner chain and `process_response`. | |

Registration: `rango::register_middleware!("my.dotted.Name", || Ok(Arc::new(MyMiddleware)))`. A factory may return
`Err(rango::Error::MiddlewareNotUsed(reason))` to leave itself out of the chain.

## See also

- [Middleware](../topics/http/middleware.md)
- [Settings reference](settings.md)
- [CORS](../topics/cors.md)
- [Caching](../topics/cache.md)
- [Security](../topics/security.md)
- [Sessions](../topics/sessions.md)
