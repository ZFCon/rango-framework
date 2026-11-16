# CORS

Browsers refuse to let a page read responses from another origin unless the server opts in with CORS headers.
`CorsMiddleware` adds those headers and answers preflight requests. This page covers how to enable it, every `cors_*`
setting, how requests are matched, and common configurations.

```rust
use rango::conf::Settings;
```

## Enabling the middleware

Add the dotted name to `middleware` and list the origins that may call you:

```rust
Settings {
    middleware: [
        "rango.middleware.cors.CorsMiddleware",
        "rango.middleware.security.SecurityMiddleware",
        "rango.middleware.common.CommonMiddleware",
        "rango.contrib.sessions.middleware.SessionMiddleware",
        "rango.middleware.csrf.CsrfViewMiddleware",
        "rango.contrib.auth.middleware.AuthenticationMiddleware",
    ]
    .map(String::from)
    .to_vec(),
    cors_allowed_origins: vec!["http://localhost:5173".into(), "https://app.example.com".into()],
    ..Settings::default()
}
```

Put it first, or at least before any middleware that can answer a request itself or reject it (security redirects,
CSRF, authentication). The middleware wraps everything after it, so with it first, error responses and redirects also
carry the CORS headers and a preflight is answered before the CSRF or authentication checks run. A preflight that
reached CSRF or authentication first would be rejected, and the browser would report a CORS failure.

Projects created with `rango-admin` already list it first. With no origins configured, the middleware adds no CORS
headers except `Vary: Origin`.

## Settings

| Setting | Type | Default | Meaning |
| --- | --- | --- | --- |
| `cors_allowed_origins` | `Vec<String>` | empty | Origins allowed to make cross-origin requests, as `scheme://host[:port]`. The scheme and host (with port) must match exactly. The string `"null"` allows the opaque origin. |
| `cors_allowed_origin_regexes` | `Vec<String>` | empty | Regular expressions an origin may match instead. Anchored at the start only; end the pattern with `$` to anchor the end. |
| `cors_allow_all_origins` | `bool` | `false` | Allow any origin. |
| `cors_allow_credentials` | `bool` | `false` | Allow cookies and other credentials in cross-origin requests. |
| `cors_allow_headers` | `Vec<String>` | `accept`, `authorization`, `content-type`, `user-agent`, `x-csrftoken`, `x-requested-with` | Request headers a page may send. Sent as `Access-Control-Allow-Headers`. |
| `cors_allow_methods` | `Vec<String>` | `DELETE`, `GET`, `OPTIONS`, `PATCH`, `POST`, `PUT` | Methods a page may use. Sent as `Access-Control-Allow-Methods`. |
| `cors_expose_headers` | `Vec<String>` | empty | Response headers that scripts may read besides the safe ones. Sent as `Access-Control-Expose-Headers`, omitted when empty. |
| `cors_preflight_max_age` | `i64` seconds | `86400` | How long browsers may cache a preflight answer. `0` omits the header. |
| `cors_allow_private_network` | `bool` | `false` | Answer private-network preflights (see [below](#private-network-access)). |
| `cors_urls_regex` | `String` | `"^.*$"` | Only request paths matching this expression get CORS handling. |

## How a request is handled

For each request the middleware does the following.

1. If the path (`path_info`, without the host or query) does not match `cors_urls_regex`, the request passes through
   untouched, with no CORS headers and no `Vary`.
2. Otherwise `Vary: Origin` is added to the response, even when the request has no `Origin` header or the origin is
   refused. Caches then keep separate copies per origin.
3. Without an `Origin` header, or with one that cannot be parsed, nothing else is added.
4. The origin is checked. It is allowed when `cors_allow_all_origins` is true, or when its scheme and host match an
   entry of `cors_allowed_origins`, or when it matches one of `cors_allowed_origin_regexes`. A refused origin gets no
   further headers; the browser then blocks the response.
5. For an allowed origin the response gets `Access-Control-Allow-Origin`: `*` when all origins are allowed and
   credentials are off, and the request's own origin in every other case.
6. With `cors_allow_credentials`, `Access-Control-Allow-Credentials: true` is added.
7. With a non-empty `cors_expose_headers`, `Access-Control-Expose-Headers` is added (comma-separated).
8. If the request method is `OPTIONS`, the allow-headers, allow-methods and (unless `0`) max-age headers are added too.
9. `Access-Control-Allow-Private-Network: true` is added when enabled and requested.

The allow-headers list is always the configured list; it does not echo the browser's `Access-Control-Request-Headers`.
A request header you use must therefore be listed in `cors_allow_headers`.

### Preflight requests

Before a cross-origin request with a custom header, a JSON body or a method other than `GET`, `HEAD` or `POST`, the
browser sends a preflight: `OPTIONS` with `Origin` and `Access-Control-Request-Method`. `CorsMiddleware` answers it
itself and the view is never called: status `200`, an empty body, `Content-Length: 0`, plus the headers above.

```console
$ curl -i -X OPTIONS http://localhost:8000/api/todos/ \
    -H "Origin: http://localhost:5173" \
    -H "Access-Control-Request-Method: POST"
HTTP/1.1 200 OK
Vary: Origin
Access-Control-Allow-Origin: http://localhost:5173
Access-Control-Allow-Headers: accept, authorization, content-type, user-agent, x-csrftoken, x-requested-with
Access-Control-Allow-Methods: DELETE, GET, OPTIONS, PATCH, POST, PUT
Access-Control-Max-Age: 86400
Content-Length: 0
```

A preflight from a refused origin also gets `200` with an empty body, but without any `Access-Control-*` header.

An `OPTIONS` request without `Access-Control-Request-Method` is not a preflight. It reaches the view (for the REST
framework, that returns the endpoint description, see [Function-based API views](rest/api-views.md#options-metadata)),
and the response still receives the CORS headers, including the `OPTIONS` ones.

### Credentials

The CORS rules forbid `Access-Control-Allow-Origin: *` on requests with credentials. When both
`cors_allow_all_origins` and `cors_allow_credentials` are true, the middleware therefore echoes the request's origin
instead of `*`, which allows credentialed requests from every site. Do not use that combination in production. List
the origins explicitly.

For the browser to send cookies at all, the front end must also request it (for example `credentials: "include"` in
`fetch`). Session and CSRF cookies are covered in [Security](security.md) and
[Authentication, permissions and throttling](rest/auth-permissions.md#session-authentication).

### Private network access

Browsers that send `Access-Control-Request-Private-Network: true` (a public page contacting a private address) need
`Access-Control-Allow-Private-Network: true` in the reply. Set `cors_allow_private_network: true` to answer it. It
applies to any allowed request that carries the request header.

### Vary

`Vary: Origin` is appended to any existing `Vary` value on every response to a path covered by `cors_urls_regex`. Keep
it; a shared cache would otherwise serve a response generated for one origin to another.

## Recipes

### A single-page app in development

```rust
Settings {
    cors_allowed_origins: vec!["http://localhost:5173".into()],
    ..Settings::default()
}
```

For throwaway local work, `cors_allow_all_origins: true` is accepted but should not ship.

### Every subdomain of one site

```rust
Settings {
    cors_allowed_origin_regexes: vec![r"^https://[a-z0-9-]+\.example\.com$".into()],
    ..Settings::default()
}
```

Without the leading `^` and trailing `$`, `https://example.com.evil.net` would match too. The expression is tried
against the full origin string, from the start; the trailing `$` is up to you.

### Cookie sessions from a separate front end

```rust
Settings {
    cors_allowed_origins: vec!["https://app.example.com".into()],
    cors_allow_credentials: true,
    ..Settings::default()
}
```

### Extra request or response headers

Extend the defaults rather than replacing them:

```rust
Settings {
    cors_allow_headers: {
        let mut h = Settings::default().cors_allow_headers;
        h.push("x-api-key".into());
        h
    },
    cors_expose_headers: vec!["x-request-id".into()],
    ..Settings::default()
}
```

### Only the API

```rust
Settings {
    cors_urls_regex: r"^/api/".into(),
    cors_allowed_origins: vec!["https://app.example.com".into()],
    ..Settings::default()
}
```

Pages outside `/api/` then get no CORS headers and no `Vary: Origin`.

## Not supported yet

- A hook to decide per request whether CORS applies (use `cors_urls_regex` and the origin lists).
- Echoing `Access-Control-Request-Headers`; the header list is fixed by `cors_allow_headers`.
- Per-path origin lists. All paths matched by `cors_urls_regex` share one policy.

## See also

- [Middleware](http/middleware.md)
- [REST quickstart](rest/quickstart.md)
- [Authentication, permissions and throttling](rest/auth-permissions.md)
- [Security](security.md)
- [Settings](settings.md)
- [Deployment](deployment.md)
