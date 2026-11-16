# Sessions

Sessions keep per-visitor data on the server between requests and identify the visitor with one cookie.
`rango::contrib::sessions` provides the middleware, the `Session` handle, five storage backends and the settings that
control the cookie. Authentication ([auth overview](auth/overview.md)) is built on top of it.

## Enabling sessions

Add the app and the middleware. The `sessions` app owns the `rango_session` table used by the default database
backend, so run `migrate` after adding it.

```rust
use rango::conf::Settings;

pub fn settings() -> Settings {
    Settings {
        installed_apps: ["rango.contrib.contenttypes", "rango.contrib.sessions"]
            .map(String::from)
            .to_vec(),
        middleware: [
            "rango.middleware.security.SecurityMiddleware",
            "rango.contrib.sessions.middleware.SessionMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        ..Settings::default()
    }
}
```

Place `SessionMiddleware` before any middleware that reads the session (authentication, messages with session
storage). Only the `db` and `cached_db` engines need the migration; the other engines need no table.

## Using the session in a view

Import the `RequestSessionExt` trait to get `request.session()`. The returned `Session` is a cheap, shared handle:
clones refer to the same data.

```rust
use rango::contrib::sessions::RequestSessionExt;
use rango::http::{HttpRequest, HttpResponse};
use rango::Result;

pub async fn counter(req: HttpRequest) -> Result<HttpResponse> {
    let n: i64 = req.session().get_as("n").unwrap_or(0) + 1;
    req.session().set("n", n);
    Ok(HttpResponse::new(n.to_string()))
}
```

`session()` panics when `SessionMiddleware` is not installed. Use `try_session()` to get an `Option<Session>`
instead.

Values are stored as JSON. `set` accepts anything that implements `serde::Serialize`; a value that cannot be
serialized is stored as `null`.

| Method | Meaning |
| --- | --- |
| `get(key) -> Option<serde_json::Value>` | Read a value. |
| `get_as::<T>(key) -> Option<T>` | Read and deserialize; `None` if missing or the wrong shape. |
| `set(key, value)` | Store a value and mark the session modified. |
| `pop(key) -> Option<Value>` | Remove and return a value. |
| `contains(key) -> bool` | Whether the key exists. |
| `keys() -> Vec<String>` | All keys, in insertion order. |
| `setdefault(key, value) -> Value` | Return the existing value or store and return `value`. |
| `is_empty() -> bool` | True when there is no key and no data. |
| `session_key() -> Option<String>` | The current session key (the cookie value; see signed cookies below). |
| `accessed()`, `modified()` | Whether the session was read or changed during this request. |
| `mark_modified()` | Force a save, for example after you changed a nested value you read out. |
| `save().await`, `flush().await`, `cycle_key().await`, `exists(key).await` | Persistence calls, described below. |

Reading any value marks the session as accessed, which adds `Vary: Cookie` to the response so caches do not share
the page between visitors.

### Persistence calls

The middleware saves the session for you at the end of a request. You normally call these only for special cases:

- `flush()` deletes the stored data and the key and clears the in-memory data. The middleware then deletes the cookie.
  `auth::logout` uses it.
- `cycle_key()` saves the data under a new key and deletes the old entry. Call it when privilege changes, to prevent
  session fixation. `auth::login` does this.
- `save()` writes now. With no key yet, the backend creates one.

### Test cookie

To check that a browser accepts cookies, call `set_test_cookie()` on one request, `test_cookie_worked()` on a later
one, then `delete_test_cookie()`.

## Expiry

By default a session lasts `session_cookie_age` seconds (two weeks). Override it per session with `set_expiry`:

```rust
use rango::contrib::sessions::{Expiry, RequestSessionExt};

fn remember_me(req: &rango::http::HttpRequest, remember: bool) {
    let s = req.session();
    if remember {
        s.set_expiry(Expiry::Age(60 * 60 * 24 * 90)); // 90 days
    } else {
        s.set_expiry(Expiry::BrowserClose);
    }
}
```

| `Expiry` variant | Effect |
| --- | --- |
| `Age(seconds)` | Expire that many seconds from now. |
| `At(DateTime<Utc>)` | Expire at an absolute moment. |
| `BrowserClose` | Cookie has no `Max-Age`/`Expires`; it ends with the browser session. |
| `Default` | Remove the override and use the settings again. |

The override is stored in the session data under the key `_session_expiry`. `get_expiry_age()`, `get_expiry_date()`
and `get_expire_at_browser_close()` return the effective values. `BrowserClose` still keeps the server-side entry for
`session_cookie_age` seconds, so a stolen cookie is not valid forever.

## Backends

Choose the backend with the `session_engine` setting. Any other value makes the first request fail with an
`ImproperlyConfigured` error ("Unknown SESSION_ENGINE").

| `session_engine` value | Storage |
| --- | --- |
| `rango.contrib.sessions.backends.db` (default) | One row per session in `rango_session`. |
| `rango.contrib.sessions.backends.cache` | Only in the cache named by `session_cache_alias`. Lost on eviction or restart. |
| `rango.contrib.sessions.backends.cached_db` | Written to both the cache and the database; reads try the cache first and refill it from the database. |
| `rango.contrib.sessions.backends.file` | One file per session, named `sessionid<key>`, in `session_file_path`. |
| `rango.contrib.sessions.backends.signed_cookies` | No server storage; the cookie holds the signed data. |

Notes per backend:

- **db** loads only rows whose `expire_date` is in the future. Expired rows are not deleted automatically, and there is
  no `clearsessions` command yet; delete old rows yourself if the table grows.
- **cache** and **cached_db** use the cache from `caches` (see [cache](cache.md)). The default in-memory cache
  (`LocMemCache`) is per process, so it does not share sessions across several worker processes; use the
  file-based cache or a registered shared backend there.
- **file** uses `session_file_path`, or the operating system's temporary directory when it is `None`. Each file
  starts with the expiry timestamp. Keys with characters other than ASCII letters and digits are rejected as a
  suspicious session key.
- **signed_cookies** has no server-side state, so `flush()` cannot invalidate a stolen cookie, and the whole payload
  travels with every request. Browsers limit a cookie to about 4 KB; keep the data small. Never store secrets in it:
  the data is signed, not encrypted.

Keys for the server-side backends are 32 random characters from `a-z0-9`.

## Payload format

Stored session data is a JSON object, compressed when that makes it smaller, then signed with `secret_key` using the
signing framework (see [signing](signing.md)). Each backend uses a salt, so a payload from one backend is not
accepted by another. A payload that fails to verify loads as an empty session. `rango::contrib::sessions::encode` and
`decode` expose the format.

The `session_serializer` setting exists but is not consulted; the format is always JSON.

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `session_engine` | `rango.contrib.sessions.backends.db` | Backend, see above. |
| `session_cookie_name` | `"sessionid"` | Cookie name. |
| `session_cookie_age` | `1209600` (14 days) | Lifetime in seconds. |
| `session_expire_at_browser_close` | `false` | Use a cookie without `Max-Age`/`Expires` by default. |
| `session_save_every_request` | `false` | Save and re-send the cookie on every request, which slides the expiry. |
| `session_cookie_path` | `"/"` | Cookie `Path`. |
| `session_cookie_domain` | `None` | Cookie `Domain`; `None` means the exact host only. |
| `session_cookie_secure` | `false` | Send the cookie only over HTTPS. |
| `session_cookie_httponly` | `true` | Hide the cookie from scripts. |
| `session_cookie_samesite` | `"Lax"` | `SameSite` value; an empty string omits the attribute. |
| `session_cache_alias` | `"default"` | Cache used by `cache` and `cached_db`. |
| `session_file_path` | `None` | Directory for the `file` backend. |
| `session_serializer` | JSON serializer name | Present but unused. |

## Cookie security

- Set `session_cookie_secure: true` in production so the cookie never travels over plain HTTP. `check --deploy`
  reports when it is off (see [checks](checks.md) and [deployment](deployment.md)).
- Leave `session_cookie_httponly` on unless client-side script must read the cookie.
- Keep `session_cookie_samesite` at `Lax` or `Strict` to reduce cross-site request risk; the CSRF middleware is still
  needed for forms (see [security](security.md)).
- Set `session_cookie_domain` only when several subdomains must share the session.

## When the cookie is written

The middleware's response phase follows these rules:

1. If the request had a session cookie and the session is now empty, the cookie is deleted.
2. If the session was read, `Vary: Cookie` is added.
3. If the session was modified (or `session_save_every_request` is on) and is not empty, the session is saved and
   the cookie is set. Nothing is saved when the response status is 500.
4. If the save fails, for example because the session was deleted during the request, the response is replaced by a
   400 response and a warning is logged to `rango.security.SessionInterrupted`.

An anonymous visitor who never writes to the session never receives a cookie. A cookie naming an unknown or expired
key is treated as no session and replaced on the next write.

## Not supported yet

- No `clearsessions` command.
- `session_serializer` cannot select another serializer.
- Custom backends cannot be selected by `session_engine`; the five names above are the only accepted values.
  The `SessionBackend` trait and the backend structs are public for direct use.

## See also

- [Authentication overview](auth/overview.md)
- [Messages](messages.md)
- [Cache](cache.md)
- [Signing](signing.md)
- [Settings](settings.md)
