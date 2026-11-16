# Installing Rango

This page covers what you need on your machine, how to install the `rango-admin` tool, how to generate and run a
project, and how to add Rango to a crate you already have.

## Requirements

- **A stable Rust toolchain.** The workspace's `rust-toolchain.toml` tracks the `stable` channel (with `rustfmt` and
  `clippy`). Rango and the projects it generates use the 2024 edition, including `if let` chains, so use Rust 1.88 or
  newer. Install or update with [rustup](https://rustup.rs):

  ```console
  $ rustup update stable
  $ rustc --version
  ```

- **A C compiler.** The SQLite backend compiles SQLite from source as part of the build, so `cc`/`gcc`/`clang` (or
  MSVC on Windows) must be available. No system SQLite library is needed.
- **A database server only if you choose PostgreSQL.** SQLite needs nothing installed. See
  [SQLite or PostgreSQL](#sqlite-or-postgresql) below.

Rango runs on the [Tokio](https://tokio.rs) runtime. Projects depend on `tokio` directly because `main` is an
`async fn` annotated with `#[tokio::main]`.

## Install the command-line tool

`rango-admin` creates projects and apps. It is published as `rangoframework-cli`:

```console
$ cargo install rangoframework-cli
$ rango-admin version
0.1.0
```

`rango-admin` only knows the commands that work without a project: `startproject`, `startapp`, `version` and `help`.
Everything else (`migrate`, `runserver`, `check`, ...) is run through your own project's binary, as shown below.

## Create a project

```console
$ rango-admin startproject mysite
$ cd mysite
```

`startproject` writes this tree:

```text
mysite/
├── Cargo.toml          # package "mysite", depends on rangoframework and tokio
├── .gitignore          # /target and *.sqlite3
├── src/
│   ├── main.rs         # the entry point: configure settings, set up, run a command
│   ├── settings.rs     # fn settings() -> rango::conf::Settings
│   └── urls.rs         # the root URLconf, registered as "mysite.urls"
├── templates/          # project-wide templates (empty)
└── static/             # project-wide static files (empty)
```

What each generated file contains:

| File | Contents |
|---|---|
| `Cargo.toml` | `rangoframework = "<version of rango-admin>"` and `tokio = { version = "1", features = ["rt-multi-thread", "macros"] }`, edition 2024 |
| `src/main.rs` | declares `mod settings; mod urls;`, calls `rango::conf::configure(settings::settings())`, `rango::setup().await`, then `rango::core::management::execute_from_command_line(std::env::args())` |
| `src/settings.rs` | a `Settings { .. }` literal: a freshly generated `secret_key` (prefixed `rango-insecure-`), `debug: true`, empty `allowed_hosts` and `installed_apps`, five middleware (CORS, security, common, CSRF, clickjacking), `root_urlconf: Some("mysite.urls")`, one template engine with `DIRS: ["templates"]` and `APP_DIRS: true`, a SQLite database `db.sqlite3`, `time_zone: "UTC"`, `static_url: Some("static/")`, and empty REST/OpenAPI settings |
| `src/urls.rs` | `register_urlconf!("mysite.urls", ...)` with three routes: `api/schema/` (the OpenAPI document), `api/docs/` (Swagger UI) and `api/redoc/` (Redoc) |

### `startproject` options

```console
$ rango-admin startproject <name> [directory] [--rango-path <path>]
```

| Argument | Meaning |
|---|---|
| `name` | Project name. It becomes the Cargo package name and the URLconf prefix, so it must be a valid Rust identifier: letters, digits and underscores, not starting with a digit, and not a Rust keyword or one of `rango`, `std`, `core`, `alloc`, `test`, `proc_macro`, `tokio`. |
| `directory` | Where to write the project. Defaults to `./<name>`. The directory must not exist, or must be empty; otherwise the command fails with `'<directory>' already exists`. |
| `--rango-path <path>` | Depend on a local checkout of the Rango repository instead of the published crate. The generated `Cargo.toml` then contains `rangoframework = { path = "<path>/crates/rango" }`. Use this when working on Rango itself or when you have no network access to crates.io. |

## Create an app

An app is a module of your crate that groups models, views, URLs and migrations (see
[Applications](../topics/apps.md)). From the project directory:

```console
$ rango-admin startapp polls
```

Because `src/main.rs` exists in the current directory, the app is created in `src/polls/` and `mod polls;` is added
to `src/main.rs` after the last existing `mod` line:

```text
src/polls/
├── mod.rs              # declares the submodules and calls rango::register_app!
├── models.rs
├── views.rs            # an `index` view returning "Hello, world. You're at the polls index."
├── urls.rs             # registers the URLconf "polls.urls" with app_name = "polls"
├── admin.rs
├── tests.rs
└── migrations/
    └── mod.rs          # filled in by makemigrations
```

`startapp` does not edit your settings or your root URLconf. Do these two steps by hand:

```rust
// src/settings.rs
installed_apps: vec!["polls".into()],
```

```rust
// src/urls.rs
use rango::urls::{include, path};

register_urlconf!("mysite.urls", || urlpatterns![
    path("polls/", include("polls.urls")),
    // ...
]);
```

`startapp` takes the same `name` rules as `startproject` and an optional `directory`. Outside a project (no
`src/main.rs` in the current directory) it writes to `./<name>` and edits nothing else. It fails if the target
directory already exists.

The same command is also available from the project binary: `cargo run -- startapp polls`.

## Run the project

Every management command is a subcommand of your project's binary. With Cargo, arguments after `--` go to the
binary:

```console
$ cargo run -- migrate
$ cargo run -- runserver
Performing system checks...
System check identified no issues (0 silenced).
November 16, 2026 - 10:12:03
Rango version 0.1.0
Starting development server at http://127.0.0.1:8000/
Quit the server with CONTROL-C.
```

Open `http://127.0.0.1:8000/api/docs/` for the generated project's Swagger UI, or `http://127.0.0.1:8000/polls/`
once the app above is wired in.

Other commands you will use early on:

```console
$ cargo run -- help                    # list every available command, grouped by app
$ cargo run -- help migrate            # help for one command
$ cargo run -- check                   # run the system checks
$ cargo run -- makemigrations          # write migrations for model changes
$ cargo run -- runserver 0.0.0.0:8080  # another address and port
```

The development server does not reload on source changes; Rust code has to be recompiled. Use
`cargo watch -x run` (from the `cargo-watch` crate) if you want a rebuild on save. After `makemigrations` writes a
new migration file you also need to rebuild before `migrate` can see it, which `cargo run` does for you.

The full list is in the [management command reference](../ref/commands.md).

## Add Rango to an existing crate

You do not need `rango-admin`. Add the dependency:

```toml
[dependencies]
rangoframework = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

The package is called `rangoframework` on crates.io but its library is named `rango`, so in code you always write
`use rango::...`.

A minimal entry point configures settings, populates the app registry and hands the command line to Rango:

```rust
use rango::apps::AppConfig;
use rango::conf::{self, Settings};
use rango::core::management::execute_from_command_line;
use rango::register_app;

register_app!(AppConfig::new("demo").verbose_name("Demo"));

#[tokio::main]
async fn main() {
    conf::configure(Settings {
        debug: true,
        secret_key: "insecure-demo-key".into(),
        installed_apps: vec!["demo".into()],
        ..Settings::default()
    })
    .expect("configure settings");
    rango::setup().await.expect("setup");
    std::process::exit(execute_from_command_line(std::env::args()).await);
}
```

The three calls must happen in this order:

1. `conf::configure` installs the process-wide settings. It can be called once; a second call returns an error.
2. `rango::setup()` builds the app registry from `installed_apps` and runs each app's `ready` hook.
3. `execute_from_command_line` parses the arguments, runs one command and returns the process exit code.

The repository ships this as a runnable example, including a custom `greet` command:
`cargo run -p rangoframework --example manage -- help`.

To serve requests you also need `root_urlconf`, `middleware`, `templates` and a database in the settings; copy
them from the generated `src/settings.rs` shown in [Settings](../topics/settings.md).

## The crates.io packages

On crates.io every package is named `rangoframework*`; in code the library is always `rango`.

| crates.io package | Imported as | What it is |
|---|---|---|
| `rangoframework` | `rango` | The whole framework. This is the only dependency most projects need. |
| `rangoframework-cli` | — | The `rango-admin` binary (`startproject`, `startapp`). |
| `rangoframework-core` | `rango_core` | Settings, app registry, checks, signals, management commands, cache, mail, signing, utilities. |
| `rangoframework-http` | `rango_http` | `HttpRequest`, `HttpResponse`, `QueryDict`, cookies, uploads. |
| `rangoframework-urls` | `rango_urls` | URL patterns, resolving and reversing. |
| `rangoframework-views` | `rango_views` | Error views, debug pages, decorators, generic views, shortcuts. |
| `rangoframework-middleware` | `rango_middleware` | The request handler, the built-in middleware, the development server. |
| `rangoframework-db` | `rango_db` | Models, querysets, migrations, the SQLite and PostgreSQL backends. |
| `rangoframework-macros` | `rango_macros` | Derive and helper macros (`#[derive(Model)]`, `filter!`, `q!`). |
| `rangoframework-template` | `rango_template` | The template engine. |
| `rangoframework-forms` | `rango_forms` | Forms, fields and widgets. |
| `rangoframework-contrib` | `rango_contrib` | Auth, sessions, messages, admin, static files, sites and the other bundled apps. |
| `rangoframework-rest` | `rango_rest` | The REST toolkit and OpenAPI schema generation. |
| `rangoframework-test` | `rango_test` | The test client and assertions. |

The `rangoframework` facade re-exports the others under `rango::conf`, `rango::db`, `rango::http`, `rango::urls`,
`rango::views`, `rango::template`, `rango::forms`, `rango::middleware`, `rango::core::*`, `rango::contrib::*`,
`rango::rest` and `rango::test`. Depend on an individual part only if you are building something that does not
need the rest.

## SQLite or PostgreSQL

Both backends are always compiled in; the `engine` of each entry in the `databases` setting picks one.

| | SQLite | PostgreSQL |
|---|---|---|
| `engine` | `"rango.db.backends.sqlite3"` | `"rango.db.backends.postgresql"` |
| `name` | path of the database file; empty or `":memory:"` for an in-memory database | database name |
| `user`, `password`, `host`, `port` | ignored | used when not empty |
| What you install | nothing (SQLite is built into the binary) | a PostgreSQL server you can reach |
| `dbshell` needs | the `sqlite3` client on `PATH` | the `psql` client on `PATH` |

```rust
use std::collections::BTreeMap;
use rango::conf::{DatabaseConfig, Settings};

let databases = BTreeMap::from([(
    "default".to_owned(),
    DatabaseConfig {
        engine: "rango.db.backends.postgresql".into(),
        name: "mysite".into(),
        user: "mysite".into(),
        password: std::env::var("DB_PASSWORD").unwrap_or_default(),
        host: "127.0.0.1".into(),
        port: "5432".into(),
        ..DatabaseConfig::default()
    },
)]);
```

SQLite is the right default for development and for the tutorial: the generated project uses a `db.sqlite3` file
in the project directory. The PostgreSQL backend connects without TLS, so keep the server on a trusted network or
behind a local proxy. Any other `engine` value fails with an "improperly configured" error the first time the
database is used. See [Databases](../topics/db/databases.md) for the details.

## See also

- [Tutorial, part 1: a project and an app](../tutorial/01-project.md)
- [Settings](../topics/settings.md) and the [settings reference](../ref/settings.md)
- [Applications](../topics/apps.md)
- [Management command reference](../ref/commands.md)
- [Databases](../topics/db/databases.md)
- [Deployment](../topics/deployment.md)
