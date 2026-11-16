# Testing

`rango::test` runs requests through your real middleware stack and URLconf in-process, with no socket. It provides `Client`, `RequestFactory`, `TestResponse` and three assertion functions.

```rust
use rango::test::{assert_contains, assert_not_contains, assert_redirects, Client, FilePart, RequestFactory, TestResponse};
```

## Configure settings for a test

Settings are process-wide and `rango::conf::configure` can be called only once per process: a second call returns `ImproperlyConfigured("Settings already configured.")`. Cargo compiles every file in `tests/` into its own binary, so the rule is: one `configure` call per test file, in a single `#[tokio::test]` or in a shared helper guarded by a `Once`/`OnceLock`. To vary a setting inside a test, use `override_settings`, which applies to one future:

```rust
use rango::conf::override_settings;

override_settings(|s| s.debug = true, async {
    let r = Client::new().unwrap().get("/nope/").await.unwrap();
    assert_contains(&r, "Page not found", 404);
})
.await;
```

An override is visible to everything awaited on the same task. A task started with `tokio::spawn` does not inherit it. `override_settings_sync` does the same for synchronous code.

Typical test settings:

| Setting | Value | Why |
|---|---|---|
| `secret_key` | any non-empty string | Required by sessions, CSRF and signing. |
| `allowed_hosts` | `["testserver"]` | The client sends requests with host `testserver`. |
| `root_urlconf` | the name given to `register_urlconf!` | Which URLs the client resolves. |
| `databases` | in-memory SQLite (below) | Fast, isolated, nothing to clean up. |
| `password_hashers` | `["rango.contrib.auth.hashers.MD5PasswordHasher"]` | Fast hashing when you create users. |
| `email_backend` | `"rango.core.mail.backends.locmem.EmailBackend"` | Captures mail in memory. |
| `use_tz` | `true` | Consistent datetime handling. |

## In-memory database from code

Apply migrations yourself, after `rango::setup()`:

```rust
use rango::conf::{self, DatabaseConfig, Settings};
use rango::db::migrations::{executor, MigrationLoader, Target};
use rango::db::Db;

conf::configure(Settings {
    secret_key: "k".into(),
    databases: [("default".to_owned(), DatabaseConfig {
        engine: "rango.db.backends.sqlite3".into(),
        name: ":memory:".into(),
        ..DatabaseConfig::default()
    })].into(),
    ..Settings::default()
})
.unwrap();
rango::setup().await.unwrap();                     // builds the app registry, runs ready() hooks

let db = Db::default_db();
let labels: Vec<String> = ["contenttypes", "auth", "sessions"].map(String::from).to_vec();
let loader = MigrationLoader::load(Some(&labels)).unwrap();
executor::migrate(&db, &loader, &Target::All).await.unwrap();
```

`MigrationLoader::load(Some(&labels))` restricts migration to the listed app labels; `None` loads every registered migration. Migrations of the listed apps must have their dependencies in the list too (flat pages and redirects need `sites`). Running `migrate` fires `post_migrate`, so contenttypes and the default site are created as in a real project.

## Client

`Client::new()` builds a handler from the current `middleware` and URLconf settings, so configure settings first. It returns `Result`. The client keeps a cookie jar, so sessions and login persist across calls on the same client; make a second `Client` for a second user.

Requests (all `async`, all return `Result<TestResponse>`):

| Method | Sends |
|---|---|
| `get(path)` | GET. |
| `get_with(path, &[("k","v")])` | GET with the pairs as the query string. |
| `head(path)`, `options(path)`, `delete(path)` | The named method. |
| `post(path, &[("k","v")])` | POST as `multipart/form-data`. |
| `post_form(path, &[...])` | POST as `application/x-www-form-urlencoded`. |
| `post_json(path, &value)` | POST a `serde::Serialize` value as `application/json`. |
| `post_multipart(path, data, &[FilePart])` | POST with file parts. `FilePart { field, filename, content_type, data }`. |
| `put(path, body, content_type)`, `patch(path, body, content_type)` | PUT or PATCH with an explicit body. |
| `generic(method, path, body, content_type)` | Any method and body. |

Builder options (consume and return the client):

- `enforce_csrf_checks(true)`: CSRF is skipped by default; turn it on to test the real check.
- `follow_redirects(true)`: follow 301, 302, 303, 307 and 308 up to 20 hops; 307 and 308 keep method and body, the others switch to GET. The hops are recorded in `redirect_chain`.
- `default_header(name, value)`: a header sent with every request.

Cookies: `cookie(name) -> Option<String>` and `set_cookie(name, value)`. Cookies the server expires (`Max-Age=0` or a 1970 expiry) are removed from the jar.

## TestResponse

| Member | Meaning |
|---|---|
| `status_code: u16` | Status. |
| `headers: Vec<(String, String)>` and `header(name) -> Option<&str>` | Headers; lookup is case-insensitive. |
| `content: Bytes` and `text() -> String` | Fully read body (streams are collected). |
| `json::<T>() -> Result<T>` | Deserialize the body. |
| `url() -> Option<&str>` | The `Location` header. |
| `cookies` | Cookies set by this response. |
| `request: HttpRequest` | The request that produced it. |
| `redirect_chain: Vec<(String, u16)>` | `(location, status)` for each followed redirect. |

## Assertions

- `assert_contains(&response, text, status_code)`: status equals `status_code` and the body contains `text`.
- `assert_not_contains(&response, text, status_code)`: status equals and the body lacks `text`.
- `assert_redirects(&response, expected_url, status_code)`: status equals and `Location` equals `expected_url` exactly. Use it on a client that does not follow redirects; the check is a string comparison, not a fetch of the target.

All panic with the response text on failure. For anything else use plain `assert_eq!` on `status_code`, `header(...)` and `json()`.

## RequestFactory

`RequestFactory` builds an `HttpRequest` without any middleware, so you can call a view function directly.

```rust
let rf = RequestFactory;
let req = rf.get("/polls/1/")?;
let req = rf.post("/vote/", &[("choice", "1")])?;                       // urlencoded
let req = rf.generic("PUT", "/x/", br#"{"a":1}"#.to_vec(), "application/json")?;
let resp = my_view(req).await?;
```

Because no sessions, auth or CSRF middleware ran, `request.session()` or `request.user()` will not be set. Use `Client` when the view depends on them.

## Mail

With the in-memory backend, sent messages are collected in a process-wide outbox. Tests in one binary run in parallel and share it, so assert on content rather than counts, or use `override_settings` with one test. See [Mail](mail.md) for `outbox()` and `clear_outbox()`.

## A complete example

Adapted from `crates/rango/tests/auth_e2e.rs`: sessions, login, messages and redirects through the real middleware, against in-memory SQLite.

```rust
use rango::conf::{self, DatabaseConfig, Settings};
use rango::contrib::auth::{self, decorators::login_required, Credentials, RequestUserExt};
use rango::contrib::messages;
use rango::db::migrations::{executor, MigrationLoader, Target};
use rango::db::Db;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::redirect;
use rango::test::{assert_contains, assert_redirects, Client};
use rango::urls::path;
use rango::{register_urlconf, urlpatterns, Result};

async fn login_view(req: HttpRequest) -> Result<HttpResponse> {
    let post = req.post().await?;
    let creds = Credentials::new(post.get("username").unwrap_or(""), post.get("password").unwrap_or(""));
    let db = Db::default_db();
    match auth::authenticate(&db, &creds).await? {
        Some((user, backend)) => {
            auth::login(&req, &db, user, Some(&backend)).await?;
            messages::success(&req, "Welcome back")?;
            redirect("/secret/")
        }
        None => Ok(HttpResponse::new("bad credentials")),
    }
}

async fn secret(req: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new(format!("hello {}", req.user().username().await)))
}

register_urlconf!("authsite.urls", || urlpatterns![
    path("login/", login_view),
    path("secret/", login_required(secret)),
]);

#[tokio::test]
async fn login_flow() {
    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("authsite.urls".into()),
        login_url: "/login/".into(),
        use_tz: true,
        installed_apps: ["contenttypes", "auth", "sessions", "messages"].map(|a| format!("rango.contrib.{a}")).to_vec(),
        middleware: ["sessions.middleware.SessionMiddleware", "auth.middleware.AuthenticationMiddleware", "messages.middleware.MessageMiddleware"]
            .map(|m| format!("rango.contrib.{m}")).to_vec(),
        password_hashers: vec!["rango.contrib.auth.hashers.MD5PasswordHasher".into()],
        databases: [("default".to_owned(), DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })].into(),
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();

    let db = Db::default_db();
    let labels: Vec<String> = ["contenttypes", "auth", "sessions"].map(String::from).to_vec();
    executor::migrate(&db, &MigrationLoader::load(Some(&labels)).unwrap(), &Target::All).await.unwrap();
    auth::get_user_model().unwrap().create_user(&db, "ann", "ann@example.com", Some("s3cret")).await.unwrap();

    let c = Client::new().unwrap();
    // anonymous: sent to the login page with ?next=
    assert_redirects(&c.get("/secret/").await.unwrap(), "/login/?next=/secret/", 302);
    // wrong password: no session cookie
    assert_contains(&c.post("/login/", &[("username", "ann"), ("password", "wrong")]).await.unwrap(), "bad credentials", 200);
    assert!(c.cookie("sessionid").is_none());
    // right password: redirected, cookie set, protected page now works
    assert_redirects(&c.post("/login/", &[("username", "ann"), ("password", "s3cret")]).await.unwrap(), "/secret/", 302);
    assert!(c.cookie("sessionid").is_some());
    assert_contains(&c.get("/secret/").await.unwrap(), "hello ann", 200);
}
```

The test binary has exactly one `configure` call. If you need several tests in one file, move the setup into a function protected by `std::sync::OnceLock` or a `tokio::sync::OnceCell`, and make each test create its own `Client`.

## Not supported yet

- No `TestCase` base type, no automatic per-test transaction rollback, and no test database creation: you apply migrations to a database you choose.
- No test runner command; use `cargo test`.
- No helpers for form or formset assertions, template-used assertions, or `assertJSONEqual`.
- No `Client::login` shortcut; call your login view or `auth::login` as in the example.

## See also

- [Tutorial part 5: testing](../tutorial/05-testing.md)
- [Settings](settings.md)
- [Mail](mail.md)
- [Fixtures and serialization](serialization.md)
- [Middleware](http/middleware.md)
