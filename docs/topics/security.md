# Security in Rango

What Rango does for you against common web attacks, which settings control it, and what remains your job. The
sections cover template escaping, CSRF protection, clickjacking, SQL injection, host header validation, HTTPS and
cookie settings, password storage, the secret key, and the `check --deploy` command that audits all of it.

Safe defaults are on only where the middleware is listed in `settings.middleware`. The project template enables
`SecurityMiddleware`, `CommonMiddleware`, `CsrfViewMiddleware` and `XFrameOptionsMiddleware`; a settings struct
built from `Settings::default()` alone has an empty `middleware` list and therefore none of them.

## Cross-site scripting: escaping

Templates escape every variable by default (`<`, `>`, `&`, `"` and `'` become entities). Do not switch this off for
data a user can influence. The details, `safe`, and `{% autoescape %}` are in the
[template language](templates/language.md#automatic-html-escaping).

When you build HTML in Rust, use the helpers in `rango::utils`:

```rust
use rango::format_html;
use rango::utils::html::{escape, conditional_escape, json_script};
use rango::utils::safestring::mark_safe;

let name = "<script>alert(1)</script>";
let link = format_html!("<a href=\"/u/{}/\">{}</a>", "ann", name); // arguments are escaped
let data = json_script(&serde_json::json!({"a": "<x>"}), Some("page-data"))?; // safe inside a script block
```

- `escape(&str) -> SafeString` always escapes. `conditional_escape` escapes unless the value is already a
  `SafeString`.
- `format_html!` escapes every positional or named argument. Values written inline as `{name}` in the format
  string are not escaped, so pass them as arguments.
- `mark_safe(s)` is a promise that `s` needs no escaping. Only call it on text you built from escaped parts.
- Put data for JavaScript in `json_script` or escape it with `escapejs`; never write it directly into a script.
- `strip_tags` returns an error when its input is nested too deeply to process, instead of returning
  partially-stripped text.

See [Utilities](utils.md) for these functions.

## Cross-site request forgery

`rango.middleware.csrf.CsrfViewMiddleware` rejects unsafe requests that do not prove they came from your own
pages.

```rust
Settings {
    middleware: vec![
        "rango.middleware.csrf.CsrfViewMiddleware".into(),
        // ...
    ],
    ..Settings::default()
}
```

### How it works

- The middleware keeps a random secret in the `csrftoken` cookie. A token placed in a form is the secret masked
  with a fresh random pad each time, so the token in the page differs on every render (this blocks attacks that
  read the compressed page size).
- `GET`, `HEAD`, `OPTIONS` and `TRACE` are never checked.
- For other methods the middleware first checks the `Origin` header if present. The origin must equal the
  request's own scheme and host, or match an entry of `csrf_trusted_origins`. If there is no `Origin` header and
  the request is secure (see [HTTPS behind a proxy](#https-and-proxies)), the `Referer` header must be present,
  well formed, `https`, and match the host or a trusted origin.
- Then the token is compared in constant time with the cookie's secret. The token is read from the
  `csrfmiddlewaretoken` form field of a `POST`, or else from the request header named by `csrf_header_name`.
- A failed check returns a 403 page and logs a warning to the `rango.security.csrf` target. If the project has a
  template named `403_csrf.html` it is used for the page.
- A malformed `csrftoken` cookie is replaced with a new one instead of failing the request.

### In templates

Add the tag inside every form that posts to your site:

```html
<form method="post" action="/polls/vote/">
  {% csrf_token %}
  ...
</form>
```

This needs the `rango.template.context_processors.csrf` context processor, which supplies `csrf_token`. The cookie
is only set on responses whose page actually used the token.

### From JavaScript

Send the token in the header named by `csrf_header_name`. The setting holds the header in request-metadata form;
the default `"HTTP_X_CSRFTOKEN"` means the request header is `X-CSRFToken`.

```javascript
fetch("/api/vote/", {
  method: "POST",
  headers: { "X-CSRFToken": csrfToken },   // read from {{ csrf_token }} or a cookie
  body: JSON.stringify({ choice: 3 }),
  credentials: "same-origin",
});
```

If your script reads the cookie, set `csrf_cookie_httponly: false` (the default). If you embed the token in the
page instead, you may set it to `true`.

### Per-view control

```rust
use rango::views::decorators::csrf_exempt;
use rango::middleware::csrf::{csrf_protect, requires_csrf_token, ensure_csrf_cookie, get_token, rotate_token};

let webhook = csrf_exempt(receive_webhook); // skip the check for this view only
let form    = csrf_protect(my_view);        // enforce the check even without the middleware
```

| Item | Meaning |
|---|---|
| `csrf_exempt(view)` | The middleware does not check this view. Use it only for endpoints that authenticate some other way, such as a signed webhook. |
| `csrf_protect(view)` | Run the check for this view alone. |
| `requires_csrf_token(view)` | Make `get_token` work in the view but never reject. |
| `ensure_csrf_cookie(view)` | Always send the cookie, for pages whose JavaScript needs it but that contain no form. |
| `get_token(&request) -> String` | A fresh masked token for the request. |
| `rotate_token(&request)` | Replace the secret. Call it when the user's privilege changes (the auth login does this). |

### Settings

| Setting | Default | Meaning |
|---|---|---|
| `csrf_cookie_name` | `"csrftoken"` | Cookie name. |
| `csrf_cookie_age` | `31449600` (one year) | Cookie lifetime in seconds. |
| `csrf_cookie_domain` | `None` | Set to `".example.com"` for subdomain sharing. When set it also becomes the trusted host for the Referer check. |
| `csrf_cookie_path` | `"/"` | Cookie path. |
| `csrf_cookie_secure` | `false` | Send the cookie only over HTTPS. Turn on in production. |
| `csrf_cookie_httponly` | `false` | Hide the cookie from scripts. |
| `csrf_cookie_samesite` | `"Lax"` | `SameSite` attribute; an empty string omits it. |
| `csrf_header_name` | `"HTTP_X_CSRFTOKEN"` | Request header carrying the token. |
| `csrf_trusted_origins` | `[]` | Extra origins allowed for unsafe requests, such as `"https://app.example.com"`. A leading wildcard (`"https://*.example.com"`) allows subdomains. Entries need the scheme. |

### Not supported yet

- `csrf_use_sessions: true` makes the middleware fail with an "improperly configured" error, so the token cannot
  yet be stored in the session.
- `csrf_failure_view` exists as a setting but is not read: the middleware always renders its built-in 403 page
  (or your `403_csrf.html` template).

## Clickjacking

`rango.middleware.clickjacking.XFrameOptionsMiddleware` adds `X-Frame-Options` to every response that does not
already have one, so other sites cannot embed your pages in a frame. The value comes from `x_frame_options`
(default `"DENY"`; `"SAMEORIGIN"` allows your own site to frame pages).

To let a single view be framed, wrap it:

```rust
use rango::views::decorators::xframe_options_exempt;

let widget = xframe_options_exempt(embeddable_widget);
```

## SQL injection

The query builder never splices values into SQL text. Every value in `filter(...)`, `exclude(...)`, `create`
and the other model methods is passed to the database driver as a bound parameter, and identifiers come from your
model definitions, not from strings you pass at run time.

For raw SQL, pass values through the parameter list, never through `format!`:

```rust
// Safe: the value is a bound parameter.
use rango::db::models::QuerySet;
let rows = QuerySet::<Question>::raw(&db, "SELECT * FROM polls_question WHERE slug = %s", &[Value::Str(slug.into())]).await?;

// Unsafe: the user controls the SQL text.
// let sql = format!("SELECT * FROM polls_question WHERE slug = '{slug}'");
```

The placeholder is `%s` for every database. See [Queries](db/queries.md) for `raw`, `query` and `execute`.

## Host header validation

`allowed_hosts` lists the host names your site answers to. Every `request.get_host()` call checks the `Host`
header against it and fails with a `DisallowedHost` error (answered with a 400 response by the default handler) when it does not match. This stops an
attacker from making your site build links or password-reset emails that point at a host they control.

```rust
Settings {
    debug: false,
    allowed_hosts: vec!["example.com".into(), ".example.com".into()],
    ..Settings::default()
}
```

- `"example.com"` matches exactly; `".example.com"` matches the domain and all of its subdomains; `"*"` matches
  anything (do not use it in production).
- When `debug` is `true` and `allowed_hosts` is empty, only `localhost`, its subdomains, `127.0.0.1` and `[::1]`
  are accepted.
- When `debug` is `false` and the list is empty every request that needs the host fails, and `runserver` refuses
  to start ("You must set settings.ALLOWED_HOSTS if DEBUG is False.").
- `use_x_forwarded_host: true` makes Rango read the host from `X-Forwarded-Host` instead of `Host`. Enable it
  only behind a proxy that overwrites that header; otherwise any client can choose the host. The value is still
  checked against `allowed_hosts`. `use_x_forwarded_port` does the same for the port.

## HTTPS and proxies

`rango.middleware.security.SecurityMiddleware` handles the HTTPS-related headers and redirects:

| Setting | Default | Effect |
|---|---|---|
| `secure_ssl_redirect` | `false` | Redirect every plain HTTP request to the same URL over HTTPS (permanent redirect). |
| `secure_ssl_host` | `None` | Host to redirect to instead of the request's host. |
| `secure_redirect_exempt` | `[]` | Regular expressions matched against the request path without its leading slash; a match skips the redirect (for example a health-check at `"healthz"`). |
| `secure_hsts_seconds` | `0` | When above zero, secure responses carry `Strict-Transport-Security: max-age=<n>`. |
| `secure_hsts_include_subdomains` | `false` | Adds `includeSubDomains`. |
| `secure_hsts_preload` | `false` | Adds `preload`. |
| `secure_content_type_nosniff` | `true` | Adds `X-Content-Type-Options: nosniff`. |
| `secure_referrer_policy` | `"same-origin"` | Value of `Referrer-Policy`; an empty string omits the header. |
| `secure_cross_origin_opener_policy` | `"same-origin"` | Value of `Cross-Origin-Opener-Policy`; an empty string omits the header. |

The middleware only sets a header the view did not set itself. `Strict-Transport-Security` is sent only on secure
requests.

Enable HSTS gradually. A browser that has seen the header refuses plain HTTP for the whole period, and a
`preload` listing is very hard to undo. Start with a small `secure_hsts_seconds` such as `3600`, confirm that the
whole site works over HTTPS, then raise it.

### Proxies

`runserver` speaks plain HTTP only. When a reverse proxy terminates TLS, Rango sees plain HTTP unless you tell it
which header the proxy sets:

```rust
Settings {
    secure_proxy_ssl_header: Some(("HTTP_X_FORWARDED_PROTO".into(), "https".into())),
    secure_ssl_redirect: true,
    ..Settings::default()
}
```

The first element is the header in request-metadata form (`X-Forwarded-Proto` becomes `HTTP_X_FORWARDED_PROTO`);
the request counts as secure only when the header equals the second element. The proxy must remove any value the
client sent for this header, or clients can fake HTTPS. When this setting is `None`, `request.is_secure()` is
always `false` under the built-in server, so `secure_ssl_redirect` would redirect forever; set the header setting
first.

## Cookies

| Setting | Default | Production advice |
|---|---|---|
| `session_cookie_secure` | `false` | `true` |
| `session_cookie_httponly` | `true` | keep `true` |
| `session_cookie_samesite` | `"Lax"` | `"Lax"` or `"Strict"` |
| `csrf_cookie_secure` | `false` | `true` |
| `language_cookie_secure` | `false` | `true` if you use the language cookie |

`secure` stops the browser from sending the cookie over plain HTTP, `httponly` hides it from scripts, and
`samesite` limits cross-site sending. Cookies named with the `__Secure-` or `__Host-` prefix are always marked
`Secure` by `set_cookie`.

For your own cookies, set the same flags through `CookieOptions` (`secure`, `httponly`, `samesite`, `max_age`,
`path`, `domain`). To stop users editing a cookie value, use [signed cookies](signing.md#signed-cookies).

## Passwords

Passwords are never stored in clear text. The first hasher in `password_hashers` is used for new passwords; the
default order is PBKDF2-SHA256 (one million iterations), PBKDF2-SHA1, Argon2, BCrypt-SHA256 and Scrypt. Existing
hashes made with any listed hasher keep working and are upgraded on the next successful login. Details are in
[Passwords](auth/passwords.md).

## The secret key

`secret_key` seeds signing ([signing](signing.md)) and other secrets. Rules:

- Keep it out of source control; read it from the environment or a secrets store.
- Use at least 50 characters with at least 5 distinct characters, and do not start it with `rango-insecure-`
  (the prefix the project template gives generated keys).
- Rotate it with `secret_key_fallbacks` so existing signed values stay valid during the change.
- An empty `secret_key` makes anything that signs fail with an "improperly configured" error.

## Auditing: `check --deploy`

```console
$ cargo run -- check --deploy
$ cargo run -- check --deploy --fail-level WARNING
```

`--deploy` adds the security checks below to the normal checks. The command exits non-zero when a message at or
above `--fail-level` (default `ERROR`) is found, so `--fail-level WARNING` makes the audit usable in CI.

| Id | Raised when |
|---|---|
| `security.W001` | `SecurityMiddleware` is not in `middleware`. |
| `security.W002` | `XFrameOptionsMiddleware` is not in `middleware`. |
| `security.W003` | `CsrfViewMiddleware` is not in `middleware`. |
| `security.W004` | `SecurityMiddleware` is on and `secure_hsts_seconds` is `0`. |
| `security.W005` | HSTS is on but `secure_hsts_include_subdomains` is `false`. |
| `security.W006` | `SecurityMiddleware` is on and `secure_content_type_nosniff` is `false`. |
| `security.W008` | `SecurityMiddleware` is on and `secure_ssl_redirect` is `false`. |
| `security.W009` | `secret_key` is shorter than 50 characters, has fewer than 5 distinct characters, or starts with `rango-insecure-`. |
| `security.W010`, `W011`, `W012` | `session_cookie_secure` is `false` while the sessions app (W010), the session middleware (W011) or both (W012) are in use. |
| `security.W013`, `W014`, `W015` | The same, for `session_cookie_httponly`. |
| `security.W016` | CSRF middleware is on, `csrf_use_sessions` is `false` and `csrf_cookie_secure` is `false`. |
| `security.W018` | `debug` is `true`. |
| `security.W019` | `XFrameOptionsMiddleware` is on but `x_frame_options` is not `"DENY"`. |
| `security.W020` | `debug` is `false` and `allowed_hosts` is empty. |
| `security.W021` | HSTS is on but `secure_hsts_preload` is `false`. |
| `security.W022` | `SecurityMiddleware` is on and `secure_referrer_policy` is empty. |
| `security.E023` | `secure_referrer_policy` has a value that is not a valid policy (comma-separated lists are checked per item). |
| `security.E024` | `secure_cross_origin_opener_policy` is not `same-origin`, `same-origin-allow-popups` or `unsafe-none`. |
| `security.W025` | A key in `secret_key_fallbacks` fails the same strength test as W009. |

W-ids are warnings; E-ids are errors. The check only inspects settings, so it can run on a build machine with no
database. Some warnings are choices rather than defects (for example W021 if you do not want to be on the
preload list); decide per project and keep the output reviewed rather than silenced.

A production-leaning settings example:

```rust
use std::env;
use rango::conf::Settings;

pub fn production() -> Settings {
    Settings {
        debug: false,
        secret_key: env::var("MYSITE_SECRET_KEY").expect("MYSITE_SECRET_KEY"),
        allowed_hosts: vec!["example.com".into(), "www.example.com".into()],
        secure_proxy_ssl_header: Some(("HTTP_X_FORWARDED_PROTO".into(), "https".into())),
        secure_ssl_redirect: true,
        secure_hsts_seconds: 3600,
        session_cookie_secure: true,
        csrf_cookie_secure: true,
        csrf_trusted_origins: vec!["https://example.com".into()],
        ..Settings::default()
    }
}
```

## See also

- [Cryptographic signing](signing.md)
- [Deployment](deployment.md)
- [Settings](settings.md)
- [Middleware](http/middleware.md)
- [System checks](checks.md)
- [Passwords](auth/passwords.md)
- [Utilities](utils.md)
