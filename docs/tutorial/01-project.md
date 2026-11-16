# Part 1: project, app, first view

This eight-part tutorial builds a small polling application: visitors read a list of questions, vote on one, and see
the results; staff manage questions in the admin; the same data is exposed as a JSON API. Part 1 installs the
command-line tool, creates a project and an app, and serves a first page.

You need a stable Rust toolchain and a C compiler (SQLite is compiled from source). [Installing Rango](../intro/install.md)
has the details.

## Goal

By the end of this part `cargo run -- runserver` serves "Hello, world. You're at the polls index." at
`http://127.0.0.1:8000/polls/`.

## Install the command-line tool

```console
$ cargo install rangoframework-cli
```

This installs `rango-admin`. It only knows the commands that work without a project (`startproject`, `startapp`,
`version`, `help`). Everything else runs through your own project's binary with `cargo run -- <command>`.

## Create the project

```console
$ rango-admin startproject mysite
$ cd mysite
```

`startproject` writes this tree (names come from `crates/rango-cli/templates/project/`):

```text
mysite/
├── Cargo.toml
├── .gitignore
├── src/
│   ├── main.rs
│   ├── settings.rs
│   └── urls.rs
├── templates/
│   └── .gitkeep
└── static/
    └── .gitkeep
```

The project name must be a valid Rust identifier (letters, digits, underscores, not starting with a digit) and
must not clash with a reserved name such as `rango`, `std` or `test`.

### `Cargo.toml`

```toml
[package]
name = "mysite"
version = "0.1.0"
edition = "2024"

[dependencies]
rangoframework = "0.1.0"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

The crate is published as `rangoframework`; in code it is called `rango` (`use rango::...`). Rango runs on Tokio, so
the project depends on it directly.

### `src/main.rs`

```rust
mod settings;
mod urls;

#[tokio::main]
async fn main() {
    rango::conf::configure(settings::settings()).expect("settings are already configured");
    rango::setup().await.expect("failed to set up Rango");
    std::process::exit(rango::core::management::execute_from_command_line(std::env::args()).await);
}
```

The binary is both your application and its management tool. It installs the settings (once per process), calls
`rango::setup()` to build the app registry, then runs whatever command was given on the command line:
`cargo run -- migrate`, `cargo run -- runserver`, and so on.

### `src/settings.rs`

```rust
use std::collections::BTreeMap;

use rango::conf::{DatabaseConfig, Settings};

pub fn settings() -> Settings {
    Settings {
        secret_key: "rango-insecure-...".into(),   // generated for you
        debug: true,
        allowed_hosts: vec![],

        installed_apps: vec![],
        middleware: [
            "rango.middleware.cors.CorsMiddleware",
            "rango.middleware.security.SecurityMiddleware",
            "rango.middleware.common.CommonMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
            "rango.middleware.clickjacking.XFrameOptionsMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        root_urlconf: Some("mysite.urls".into()),

        templates: vec![rango::json!({
            "BACKEND": "rango.template.backends.rango.RangoTemplates",
            "DIRS": ["templates"],
            "APP_DIRS": true,
            "OPTIONS": {
                "context_processors": [
                    "rango.template.context_processors.debug",
                    "rango.template.context_processors.request",
                ],
            },
        })],

        databases: BTreeMap::from([(
            "default".to_owned(),
            DatabaseConfig {
                engine: "rango.db.backends.sqlite3".into(),
                name: "db.sqlite3".into(),
                ..DatabaseConfig::default()
            },
        )]),

        language_code: "en-us".into(),
        time_zone: "UTC".into(),
        use_i18n: true,
        use_tz: true,
        static_url: Some("static/".into()),
        // ... CORS and REST settings
        ..Settings::default()
    }
}
```

Settings are a typed struct, so a misspelled setting is a compile error. Every setting you leave out keeps its
default. Dotted names such as `"rango.middleware.csrf.CsrfViewMiddleware"` refer to components registered with the
framework; they are looked up by name at start-up. The database is a SQLite file, `db.sqlite3`, in the directory you
run the command from.

### `src/urls.rs`

```rust
use rango::rest::schema;
use rango::urls::path;
use rango::{register_urlconf, urlpatterns};

register_urlconf!("mysite.urls", || urlpatterns![
    // path("polls/", rango::urls::include("polls.urls")),
    path("api/schema/", schema::schema_view()).name("schema"),
    path("api/docs/", schema::swagger_view("/api/schema/")).name("swagger-ui"),
    path("api/redoc/", schema::redoc_view("/api/schema/")).name("redoc"),
]);
```

`register_urlconf!` registers a list of URL patterns under a name; `root_urlconf` in the settings points at the
name `"mysite.urls"`. The generated file already serves the OpenAPI document and its viewers, which part 8 fills in.

## Run the server

```console
$ cargo run -- runserver
```

The server listens on `127.0.0.1:8000` by default. The first build downloads and compiles dependencies, which takes
a while. Stop it with Ctrl-C. Nothing is served at `/` yet, and the OpenAPI pages are empty until viewsets exist.

## Create the polls app

```console
$ rango-admin startapp polls
```

(`cargo run -- startapp polls` does the same.) Inside a project, the app is created as `src/polls/`, and a
`mod polls;` line is added to `src/main.rs` so the app is linked into the binary:

```text
src/polls/
├── mod.rs
├── models.rs
├── views.rs
├── urls.rs
├── admin.rs
├── tests.rs
└── migrations/
    └── mod.rs
```

### `src/polls/mod.rs`

```rust
pub mod admin;
pub mod migrations;
pub mod models;
#[cfg(test)]
mod tests;
pub mod urls;
pub mod views;

rango::register_app!(rango::apps::AppConfig::new("polls"));
```

`register_app!` announces the app to the framework when the binary is linked. An app is only active if its name is
also listed in the `installed_apps` setting, so edit `src/settings.rs`:

```rust
installed_apps: vec!["polls".into()],
```

Apps that carry templates or static files must also say where they live on disk. The generated line does not, so
change it now, because parts 3 and 6 need it:

```rust
rango::register_app!(
    rango::apps::AppConfig::new("polls").path(concat!(env!("CARGO_MANIFEST_DIR"), "/src/polls"))
);
```

`APP_DIRS: true` in the template settings then finds `src/polls/templates/`, and static files are found in
`src/polls/static/`. `CARGO_MANIFEST_DIR` is fixed at compile time, so these directories are found where the source
tree exists at the build path.

## Write the first view

### `src/polls/views.rs`

```rust
use rango::Result;
use rango::http::{HttpRequest, HttpResponse};

pub async fn index(_request: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new("Hello, world. You're at the polls index."))
}
```

A view is an `async fn` that takes the request and returns `Result<HttpResponse>`. Returning an `Err` produces the
matching error page (a 404, for example), so views can use `?`.

### `src/polls/urls.rs`

```rust
use rango::urls::path;
use rango::{register_urlconf, urlpatterns};

use super::views;

register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", views::index).name("index"),
]);
```

`app_name = "polls"` makes the pattern names reachable as `polls:index`. Naming patterns lets templates and views
build URLs with `reverse!` and `{% url %}` instead of hard-coding paths.

### Wire it into the project

In `src/urls.rs`, uncomment the include and import it:

```rust
use rango::rest::schema;
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};

register_urlconf!("mysite.urls", || urlpatterns![
    path("polls/", include("polls.urls")),
    path("api/schema/", schema::schema_view()).name("schema"),
    path("api/docs/", schema::swagger_view("/api/schema/")).name("swagger-ui"),
    path("api/redoc/", schema::redoc_view("/api/schema/")).name("redoc"),
]);
```

`include("polls.urls")` mounts the app's patterns below the `polls/` prefix. Run the server again and open
`http://127.0.0.1:8000/polls/`.

## Recap

- `rango-admin startproject` and `startapp` generate the files; `cargo run -- <command>` runs everything else.
- Settings are a Rust struct in `src/settings.rs`; apps are enabled by name in `installed_apps`.
- A view is an `async fn(HttpRequest) -> Result<HttpResponse>`; URL patterns are registered with
  `register_urlconf!` and mounted with `include`.

Next: [Part 2: models and the database](02-models.md).

## See also

- [Installing Rango](../intro/install.md)
- [Settings](../topics/settings.md)
- [URL dispatcher](../topics/http/urls.md)
- [Writing views](../topics/http/views.md)
