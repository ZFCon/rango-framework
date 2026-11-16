# Settings

A Rango project is configured by one value of type `rango::conf::Settings`, a plain Rust struct with one typed
field per setting. This page explains how to build it, install it, read it, override it in tests, and how to keep
development and production configurations apart. The meaning of each field is in the
[settings reference](../ref/settings.md).

## The basics

Settings are ordinary Rust code. There is no settings file format and no lookup by environment variable or module
name: your project builds a `Settings` value and hands it to `conf::configure` once, at the start of `main`.

```rust
use rango::conf::{self, Settings};

#[tokio::main]
async fn main() {
    conf::configure(Settings {
        debug: true,
        secret_key: "change-me".into(),
        installed_apps: vec!["polls".into()],
        ..Settings::default()
    })
    .expect("settings are already configured");

    rango::setup().await.expect("failed to set up Rango");
    std::process::exit(rango::core::management::execute_from_command_line(std::env::args()).await);
}
```

Three things are worth noticing:

- **Setting names are the snake_case field names**: `debug`, `secret_key`, `installed_apps`, `session_cookie_age`.
  A misspelled name or a value of the wrong type is a compile error.
- **`..Settings::default()` fills in everything you do not list.** There are 163 settings; a typical project sets
  between ten and twenty.
- **Explicit beats implicit.** Nothing is read until you call `configure`, and nothing can change the settings
  afterwards except a scoped override (see [Overriding settings in tests](#overriding-settings-in-tests)).

Projects created by `rango-admin startproject` keep the literal in `src/settings.rs` as
`pub fn settings() -> Settings`, and `main` calls `rango::conf::configure(settings::settings())`.

## The `rango::conf` API

| Item | Signature | Purpose |
|---|---|---|
| `Settings` | `struct` | Every setting with its default (`Settings::default()`). `Clone`, `Debug`, serde `Serialize`/`Deserialize`. |
| `DatabaseConfig` | `struct` | One entry of `Settings::databases`. |
| `DatabaseTestConfig` | `struct` | The `test` field of `DatabaseConfig`. |
| `Json` | `type Json = serde_json::Value` | Type of the free-form settings. |
| `configure` | `fn configure(settings: Settings) -> Result<()>` | Install the process-wide settings. Fails with "Settings already configured." on a second call. |
| `configured` | `fn configured() -> bool` | Whether settings are available (configured, or inside an override scope). |
| `settings` | `fn settings() -> Arc<Settings>` | The current settings. Panics if `configure` has not been called. |
| `try_settings` | `fn try_settings() -> Result<Arc<Settings>>` | The current settings, or an `ImproperlyConfigured` error if `configure` has not been called. |
| `override_settings` | `async fn override_settings<F: Future>(modify: impl FnOnce(&mut Settings), fut: F) -> F::Output` | Run a future with modified settings. |
| `override_settings_sync` | `fn override_settings_sync<R>(modify: impl FnOnce(&mut Settings), body: impl FnOnce() -> R) -> R` | Run a closure with modified settings. |

Methods on `Settings`:

| Method | Purpose |
|---|---|
| `secret_key(&self) -> Result<&str>` | The secret key, or an `ImproperlyConfigured` error ("The SECRET_KEY setting must not be empty.") when it is empty. Signing uses this, so a missing key is reported the first time something is signed. |
| `is_overridden(&self, name: &str) -> bool` | Whether the setting named `name` (UPPER_CASE) differs from its default. |
| `to_map(&self) -> BTreeMap<String, Json>` | Every setting as `{ "UPPER_CASE_NAME": value }`. |
| `Settings::default_of(name: &str) -> Option<&'static Json>` | The default of one setting by UPPER_CASE name; `None` for an unknown name. |

## Reading settings

Call `settings()` wherever you need a value. It returns an `Arc<Settings>` snapshot, so reading is cheap and the
value you hold stays consistent even if an override scope ends while you still hold it.

```rust
use rango::conf::settings;
use rango::http::{HttpRequest, HttpResponse};

pub async fn about(_request: HttpRequest) -> rango::Result<HttpResponse> {
    let s = settings();
    let mode = if s.debug { "development" } else { "production" };
    Ok(HttpResponse::new(format!("Running in {mode} mode, time zone {}", s.time_zone)))
}
```

Use `try_settings()` in code that may run before `configure` (library code, early start-up helpers); it returns an
error instead of panicking:

```rust
use rango::conf::try_settings;

fn language() -> String {
    try_settings().map_or_else(|_| "en-us".to_owned(), |s| s.language_code.clone())
}
```

Do not copy a setting into a `static` at start-up. Read it where it is used, so that scoped overrides in tests
are honoured.

## Defaults

`Settings::default()` is a complete, safe-by-default configuration: `debug` is `false`, `allowed_hosts`,
`installed_apps`, `middleware`, `templates` and `databases` are empty, and `secret_key` is the empty string.
A project therefore has to set at least:

| Setting | Why |
|---|---|
| `secret_key` | Needed by signing (sessions, CSRF tokens, password-reset tokens, signed cookies). |
| `installed_apps` | The apps that take part in the project. |
| `middleware` | The request/response pipeline, outermost first. |
| `root_urlconf` | The name of the URLconf registered with `register_urlconf!`. |
| `templates` | The template engine configuration; an empty list means no engine. |
| `databases` | At least a `"default"` entry if the project uses the ORM. |
| `allowed_hosts` | Required as soon as `debug` is `false`. |

To see what your project changed, run [`diffsettings`](#inspecting-settings-diffsettings).

## Secrets and environment variables

Because settings are Rust code, reading the environment is a function call. Keep secrets out of the source tree
and fail early when one is missing in production:

```rust
use std::env;
use rango::conf::Settings;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("environment variable {name} is not set"))
}

pub fn settings() -> Settings {
    let debug = env::var("MYSITE_DEBUG").is_ok_and(|v| v == "1");
    Settings {
        debug,
        secret_key: if debug {
            env::var("MYSITE_SECRET_KEY").unwrap_or_else(|_| "rango-insecure-dev-only".into())
        } else {
            required("MYSITE_SECRET_KEY")
        },
        allowed_hosts: env::var("MYSITE_ALLOWED_HOSTS")
            .map(|v| v.split(',').map(|h| h.trim().to_owned()).collect())
            .unwrap_or_default(),
        ..Settings::default()
    }
}
```

Rango itself reads only a handful of environment variables, none of them for settings: `NO_COLOR` (command
output), `RANGO_SUPERUSER_USERNAME`, `RANGO_SUPERUSER_EMAIL` and `RANGO_SUPERUSER_PASSWORD` (the
`createsuperuser` command), and `RANGO_EMAIL_FILE_PATH` (the file-based email backend).

The key generated by `startproject` starts with `rango-insecure-`. `check --deploy` warns (`security.W009`) about
any key with that prefix, shorter than 50 characters, or with fewer than 5 distinct characters.

### Rotating the secret key

Put the new key in `secret_key` and move the old one to `secret_key_fallbacks`. Signatures are always created with
`secret_key`; verification also accepts the fallbacks, so existing sessions and tokens keep working. Remove the old
key from the fallbacks once they have expired.

```rust
Settings {
    secret_key: required("MYSITE_SECRET_KEY"),
    secret_key_fallbacks: env::var("MYSITE_OLD_SECRET_KEY").into_iter().collect(),
    ..Settings::default()
}
```

## Databases

`databases` maps an alias to a `DatabaseConfig`. The alias `"default"` is what `Db::default_db()` and every
command's `--database` option use unless told otherwise.

```rust
use std::collections::BTreeMap;
use rango::conf::{DatabaseConfig, Settings};

Settings {
    databases: BTreeMap::from([
        (
            "default".to_owned(),
            DatabaseConfig {
                engine: "rango.db.backends.postgresql".into(),
                name: "mysite".into(),
                user: "mysite".into(),
                password: std::env::var("DB_PASSWORD").unwrap_or_default(),
                host: "db.internal".into(),
                port: "5432".into(),
                options: rango::json!({ "max_connections": 20 }),
                ..DatabaseConfig::default()
            },
        ),
        (
            "archive".to_owned(),
            DatabaseConfig {
                engine: "rango.db.backends.sqlite3".into(),
                name: "archive.sqlite3".into(),
                ..DatabaseConfig::default()
            },
        ),
    ]),
    ..Settings::default()
}
```

`DatabaseConfig` fields:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `engine` | `String` | `"rango.db.backends.dummy"` | `"rango.db.backends.sqlite3"` or `"rango.db.backends.postgresql"`. Any other value (including the default) fails with an `ImproperlyConfigured` error when the alias is first used. |
| `name` | `String` | `""` | SQLite: file path; empty or `":memory:"` opens an in-memory database. PostgreSQL: database name. |
| `user` | `String` | `""` | PostgreSQL user; omitted from the connection when empty. |
| `password` | `String` | `""` | PostgreSQL password; omitted when empty. |
| `host` | `String` | `""` | PostgreSQL host; omitted when empty. |
| `port` | `String` | `""` | PostgreSQL port; omitted when empty. |
| `options` | `Json` | `{}` | PostgreSQL reads `max_connections` (pool size, default 10). Other keys are ignored. |
| `atomic_requests` | `bool` | `false` | Accepted but not used yet. |
| `autocommit` | `bool` | `true` | Accepted but not used yet. |
| `conn_max_age` | `Option<u64>` | `Some(0)` | Accepted but not used yet. |
| `conn_health_checks` | `bool` | `false` | Accepted but not used yet. |
| `disable_server_side_cursors` | `bool` | `false` | Accepted but not used yet. |
| `time_zone` | `Option<String>` | `None` | Accepted but not used yet. |
| `test` | `DatabaseTestConfig` | see below | Accepted but not used yet. |

`DatabaseTestConfig` has the fields `charset`, `collation`, `mirror`, `name` (all `Option<String>`, default `None`)
and `migrate` (`bool`, default `true`). Nothing reads them yet.

A connection pool is opened the first time an alias is used and kept for the life of the process. SQLite pools
hold up to 8 connections for a file database and exactly one for an in-memory database. See
[Databases](db/databases.md).

## JSON-valued settings

Most settings have a precise Rust type. A few describe open-ended structures (a list of engines, a map of
backends with backend-specific options) and are typed `Json` (`serde_json::Value`). Build them with the
`rango::json!` macro, which is `serde_json::json!` re-exported. Their inner keys are UPPER_CASE strings.

| Setting | Type | Shape |
|---|---|---|
| `templates` | `Vec<Json>` | One object per engine with `BACKEND`, `DIRS`, `APP_DIRS`, `OPTIONS`. Only the first entry is used. |
| `caches` | `Json` | `{ alias: { "BACKEND": ..., "LOCATION": ..., "TIMEOUT": ..., "KEY_PREFIX": ..., "VERSION": ..., "OPTIONS": ... } }` |
| `storages` | `Json` | `{ "default": { "BACKEND": ... }, "staticfiles": { "BACKEND": ... } }` |
| `auth_password_validators` | `Vec<Json>` | `[{ "NAME": ..., "OPTIONS": { ... } }]` |
| `rest_framework` | `Json` | `{ "DEFAULT_PERMISSION_CLASSES": [...], "PAGE_SIZE": 20, ... }` |
| `spectacular_settings` | `Json` | `{ "TITLE": ..., "VERSION": ..., "DESCRIPTION": ..., "SERVERS": [...] }` |
| `message_tags` | `Option<Json>` | `{ "<level number>": "<tag>" }` |
| `number_grouping` | `Json` | A number, or a list of group sizes. |
| `absolute_url_overrides`, `logging`, `migration_modules` | `Json` | Accepted but not used yet. |

```rust
use rango::conf::Settings;

Settings {
    templates: vec![rango::json!({
        "BACKEND": "rango.template.backends.rango.RangoTemplates",
        "DIRS": ["templates"],
        "APP_DIRS": true,
        "OPTIONS": {
            "context_processors": [
                "rango.template.context_processors.debug",
                "rango.template.context_processors.request",
                "rango.contrib.auth.context_processors.auth",
                "rango.contrib.messages.context_processors.messages",
            ],
        },
    })],
    caches: rango::json!({
        "default": {
            "BACKEND": "rango.core.cache.backends.filebased.FileBasedCache",
            "LOCATION": "/var/tmp/mysite-cache",
            "TIMEOUT": 600,
        },
    }),
    auth_password_validators: vec![
        rango::json!({ "NAME": "rango.contrib.auth.password_validation.MinimumLengthValidator", "OPTIONS": { "min_length": 10 } }),
        rango::json!({ "NAME": "rango.contrib.auth.password_validation.CommonPasswordValidator" }),
    ],
    rest_framework: rango::json!({
        "DEFAULT_PERMISSION_CLASSES": ["rango.rest.permissions.IsAuthenticated"],
        "DEFAULT_PAGINATION_CLASS": "rango.rest.pagination.PageNumberPagination",
        "PAGE_SIZE": 20,
    }),
    ..Settings::default()
}
```

The compiler cannot check the inside of a `Json` value. A misspelled key is ignored and the default behaviour
applies, so copy key names from the [reference](../ref/settings.md).

### Dotted names are registry keys

Values such as `"rango.middleware.csrf.CsrfViewMiddleware"`, `"rango.contrib.sessions.backends.db"` or
`"rango.core.cache.backends.locmem.LocMemCache"` look like paths, but they are names under which a component was
registered at link time. Rust cannot load code by name at run time, so a dotted name only works if the code that
registered it is part of your binary. Your own middleware, URLconfs, cache backends and context processors get
their names from the corresponding `register_*!` macro; you choose the string. See
[Applications](apps.md#link-time-registration) for the mechanism.

## What `rango::setup()` does

`rango::setup()` must be awaited once, after `configure` and before anything uses apps, models, commands or the
request handler.

```rust
rango::conf::configure(settings::settings())?;
rango::setup().await?;
```

It does two things:

1. It makes sure the bundled crates (middleware and the development server, the CLI commands, the contrib apps,
   the REST toolkit, the migration and fixture commands) are linked into your binary, so that everything they
   register — apps, commands, middleware, checks — is visible.
2. It populates the app registry from `installed_apps`, in order, and then awaits each app's `ready` hook.

It returns an error when:

- settings are not configured;
- an entry of `installed_apps` was never registered: `Cannot find app 'polls'. Register it with
  rango::register_app! and link its crate.`;
- two installed apps share a label or a name: `Application labels aren't unique, duplicates: ...`;
- a `ready` hook returns an error.

Calling `setup()` a second time is a no-op: the registry is populated once per process and is not rebuilt, even
if `installed_apps` is overridden later.

## Overriding settings in tests

Tests often need one setting changed for a few lines. `override_settings` clones the current settings, applies
your closure to the clone, and runs a future that sees the modified copy. When the future completes, the previous
settings are back.

```rust
use rango::conf::{self, override_settings};
use rango::test::Client;

#[tokio::test]
async fn not_found_page_depends_on_debug() {
    // configure(...) and rango::setup().await were called earlier in this test binary

    override_settings(|s| s.debug = false, async {
        let client = Client::new().unwrap();
        let response = client.get("/missing/").await.unwrap();
        assert_eq!(response.status_code, 404);
        assert!(!conf::settings().debug);
    })
    .await;

    // outside the scope, the original value applies again
}
```

The closure receives `&mut Settings`, so it can change several fields or replace the whole value
(`|s| *s = other`).

Rules to keep in mind:

- **The scope follows the task, not the thread.** The override is visible to the future you pass and to
  everything it awaits. A task started with `tokio::spawn` inside the scope does **not** see it; it sees the
  process-wide settings.
- **Overrides nest.** An inner `override_settings` starts from the outer override's values; leaving it restores
  the outer scope.

  ```rust
  override_settings(|s| s.debug = false, async {
      override_settings(|s| s.silenced_system_checks = vec!["demo.E001".into()], async {
          // debug is false AND demo.E001 is silenced here
      })
      .await;
      // debug is still false, nothing is silenced
  })
  .await;
  ```

- **Settings must already be configured.** The override clones the current settings, so calling it before
  `configure` panics.
- **Use `override_settings_sync` for code that is not async:**

  ```rust
  use rango::conf::{override_settings_sync, settings};

  let age = override_settings_sync(|s| s.session_cookie_age = 60, || settings().session_cookie_age);
  assert_eq!(age, 60);
  ```

- **Some things are built once and do not follow later overrides:**
  - the app registry (`installed_apps`) is populated by the first `rango::setup()`;
  - a database pool is opened on first use of its alias and reused, so changing `databases` afterwards has no
    effect on aliases already in use;
  - the middleware chain is built when a handler is created (`Client::new()`, `runserver`), so override
    `middleware` *around* the creation of the client, as in the example above.

  Template engines and caches are rebuilt automatically when `templates`, `debug` or `caches` differ from the
  values they were built with.

`configure` can be called once per process. Each file under `tests/` is its own process, so configure at the
start of the test there; inside one test binary that has several `#[tokio::test]` functions, guard the call:

```rust
use std::sync::Once;
use rango::conf::{self, Settings};

static INIT: Once = Once::new();

async fn init() {
    INIT.call_once(|| {
        conf::configure(Settings { secret_key: "test".into(), ..Settings::default() }).unwrap();
    });
    rango::setup().await.unwrap();
}
```

## Inspecting settings: `diffsettings`

`diffsettings` prints every setting whose value differs from the default, one per line, as
`UPPER_CASE_NAME = <JSON value>`:

```console
$ cargo run -- diffsettings
DEBUG = true
INSTALLED_APPS = ["polls"]
ROOT_URLCONF = "mysite.urls"
SECRET_KEY = "rango-insecure-..."
...
```

With `--all` it prints all 163 settings; the ones still at their default are prefixed with `###`:

```console
$ cargo run -- diffsettings --all
### ABSOLUTE_URL_OVERRIDES = {}
### ADMINS = []
ALLOWED_HOSTS = ["example.com"]
### APPEND_SLASH = true
...
```

The output contains your secret key and database passwords. Do not paste it into bug reports unredacted.

### UPPER_CASE names

Wherever a setting is referred to by a string rather than by a field — `diffsettings` output, `is_overridden`,
`default_of`, `to_map`, serde serialization, and the texts of check messages — the name is the field name in
upper case: `secret_key` is `"SECRET_KEY"`.

### `is_overridden` semantics

`Settings` is a plain struct and cannot remember which fields you assigned. `is_overridden(name)` therefore
compares the current value with the default:

```rust
use rango::conf::Settings;

let s = Settings { debug: true, append_slash: true, ..Settings::default() };
assert!(s.is_overridden("DEBUG"));          // differs from the default (false)
assert!(!s.is_overridden("APPEND_SLASH"));  // set explicitly, but to its default value
assert!(!s.is_overridden("NO_SUCH_NAME"));  // unknown names are never overridden
```

`diffsettings` uses the same comparison, which is why a setting you wrote out with its default value does not
appear in its output.

## Development and production

Keep one `settings()` function and derive the differences from the environment, so that both modes are exercised
by the same code and cannot drift apart silently.

```rust
use std::collections::BTreeMap;
use std::env;

use rango::conf::{DatabaseConfig, Settings};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("environment variable {name} is not set"))
}

fn base() -> Settings {
    Settings {
        installed_apps: [
            "rango.contrib.contenttypes",
            "rango.contrib.auth",
            "rango.contrib.sessions",
            "rango.contrib.messages",
            "rango.contrib.staticfiles",
            "polls",
        ]
        .map(String::from)
        .to_vec(),
        middleware: [
            "rango.middleware.security.SecurityMiddleware",
            "rango.middleware.common.CommonMiddleware",
            "rango.contrib.sessions.middleware.SessionMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
            "rango.contrib.auth.middleware.AuthenticationMiddleware",
            "rango.contrib.messages.middleware.MessageMiddleware",
            "rango.middleware.clickjacking.XFrameOptionsMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        root_urlconf: Some("mysite.urls".into()),
        templates: vec![rango::json!({
            "BACKEND": "rango.template.backends.rango.RangoTemplates",
            "DIRS": ["templates"],
            "APP_DIRS": true,
            "OPTIONS": { "context_processors": [
                "rango.template.context_processors.request",
                "rango.contrib.auth.context_processors.auth",
                "rango.contrib.messages.context_processors.messages",
            ] },
        })],
        time_zone: "UTC".into(),
        static_url: Some("static/".into()),
        ..Settings::default()
    }
}

fn development() -> Settings {
    Settings {
        debug: true,
        secret_key: "rango-insecure-development-key".into(),
        databases: BTreeMap::from([(
            "default".to_owned(),
            DatabaseConfig {
                engine: "rango.db.backends.sqlite3".into(),
                name: "db.sqlite3".into(),
                ..DatabaseConfig::default()
            },
        )]),
        email_backend: "rango.core.mail.backends.console.EmailBackend".into(),
        ..base()
    }
}

fn production() -> Settings {
    Settings {
        debug: false,
        secret_key: required("MYSITE_SECRET_KEY"),
        allowed_hosts: required("MYSITE_ALLOWED_HOSTS").split(',').map(|h| h.trim().to_owned()).collect(),
        databases: BTreeMap::from([(
            "default".to_owned(),
            DatabaseConfig {
                engine: "rango.db.backends.postgresql".into(),
                name: required("MYSITE_DB_NAME"),
                user: required("MYSITE_DB_USER"),
                password: required("MYSITE_DB_PASSWORD"),
                host: required("MYSITE_DB_HOST"),
                ..DatabaseConfig::default()
            },
        )]),
        static_root: Some("/srv/mysite/static".into()),
        // HTTPS
        secure_ssl_redirect: true,
        secure_hsts_seconds: 31_536_000,
        secure_hsts_include_subdomains: true,
        secure_hsts_preload: true,
        session_cookie_secure: true,
        csrf_cookie_secure: true,
        // behind a reverse proxy that sets this header on HTTPS requests
        secure_proxy_ssl_header: Some(("HTTP_X_FORWARDED_PROTO".into(), "https".into())),
        ..base()
    }
}

pub fn settings() -> Settings {
    match env::var("MYSITE_ENV").as_deref() {
        Ok("production") => production(),
        _ => development(),
    }
}
```

What changes between the two, and why:

| Setting | Development | Production |
|---|---|---|
| `debug` | `true`: detailed error pages, template caching off, `localhost` hosts accepted with an empty `allowed_hosts` | `false`: errors go to the regular 400/403/404/500 handlers |
| `allowed_hosts` | may stay empty | must list your host names; with an empty list `runserver` refuses to start and `request.get_host()` rejects every host |
| `secret_key` | any constant | a long random value from the environment |
| `databases` | a SQLite file | usually PostgreSQL |
| `secure_*`, `*_cookie_secure` | defaults | enabled once the site is served over HTTPS |
| `static_root` | unset | where `collectstatic` gathers files for the web server |

Before deploying, run the deployment checks against the production configuration; they inspect exactly these
settings:

```console
$ MYSITE_ENV=production cargo run --release -- check --deploy
```

`secure_proxy_ssl_header` takes the header in its request-metadata form: `HTTP_` followed by the header name in
upper case with dashes replaced by underscores. Set it only when a proxy you control always sets or strips that
header, otherwise clients can claim to be on HTTPS.

## Not supported yet

- **Project-specific settings.** `Settings` has a fixed set of fields and rejects unknown keys when
  deserialized. Keep your own configuration in your own struct or `static`.
- **Loading settings from a file.** `Settings` implements serde's `Deserialize` under the UPPER_CASE names, but
  every non-optional setting must be present in the input, so it is not a convenient configuration-file format.
  Read individual values from the environment or from your own file instead.
- **A signal when a setting changes.** Overrides are scoped values; nothing is notified.
- Settings marked "accepted but not used yet" in the [reference](../ref/settings.md) exist so that a complete
  configuration type-checks, but no code reads them.

## See also

- [Settings reference](../ref/settings.md) — every setting, its type and default
- [Applications](apps.md) — `installed_apps` and `rango::setup()`
- [System checks](checks.md) — `check --deploy` and `silenced_system_checks`
- [Management command reference](../ref/commands.md) — `diffsettings`
- [Databases](db/databases.md), [Cache](cache.md), [Templates: the engine](templates/engine.md)
- [Security](security.md) and [Deployment](deployment.md)
- [Testing](testing.md)
