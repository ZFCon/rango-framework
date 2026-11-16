# Applications

An application ("app") is a unit of a project: a set of models, views, templates, static files, migrations and
commands that belong together. This page explains how apps are registered, how a project selects the ones it uses,
what an `AppConfig` carries, and how the app registry is queried.

## Registering an app

Rust cannot import a module from a string, so an app announces itself at link time with the `register_app!` macro.
Registration only makes the app available; the project turns it on by listing its name in `installed_apps`.

```rust
// src/polls/mod.rs
use rango::apps::AppConfig;
use rango::register_app;

pub mod models;
pub mod views;
pub mod urls;
pub mod migrations;

register_app!(AppConfig::new("polls").verbose_name("Polls"));
```

`rango-admin startapp polls` generates this for you and adds `mod polls;` to `src/main.rs`. The `mod` line matters:
if the module is never compiled into the binary the macro never runs, and the app cannot be found.

```rust
// src/settings.rs
use rango::conf::Settings;

pub fn settings() -> Settings {
    Settings {
        installed_apps: vec!["polls".into()],
        ..Settings::default()
    }
}
```

`register_app!` takes one expression of type `AppConfig`. The expression is evaluated lazily, when the registry is
populated, so it may call any `const`-friendly or ordinary function, including `env!` and `concat!`.

An app can live in its own crate. Depend on that crate and reference it from `main.rs` so that the linker keeps it
(for example with `use my_polls_crate as _;`); its `register_app!` then runs like any other.

## Populating the registry

`rango::setup()` reads `installed_apps` from the configured settings, builds the registry and runs every app's
`ready` hook. Call it once, after `conf::configure` and before running commands or serving requests:

```rust
use rango::conf;

#[tokio::main]
async fn main() {
    conf::configure(settings::settings()).expect("settings are already configured");
    rango::setup().await.expect("failed to set up Rango");
    std::process::exit(rango::core::management::execute_from_command_line(std::env::args()).await);
}
```

Calling `setup` again after a successful run does nothing. During population Rango:

1. Looks up each entry of `installed_apps` among the registered apps by `name`, keeping the order of the list. An
   entry that nothing registered fails with `ImproperlyConfigured`: `Cannot find app '<name>'. Register it with
   rango::register_app! and link its crate.`
2. Rejects duplicate labels or names: `Application labels aren't unique, duplicates: ...`.
3. Publishes the registry, then calls each app's `ready` hook in `installed_apps` order, stopping at the first error.

The order of `installed_apps` therefore decides the order of `get_app_configs()`, of `ready` calls, of
[template](templates/engine.md) lookup across apps' `templates` directories and of static-file lookup. Put the app that
should win a name clash first.

## AppConfig

`rango::apps::AppConfig` describes one app. Create it with `AppConfig::new(name)` and refine it with the builder
methods.

| Item | Type | Meaning |
|---|---|---|
| `name` | `String` | Full name, as written in `installed_apps`: `"polls"`, `"rango.contrib.auth"`. |
| `label` | `String` | Short, unique identifier. Defaults to the last dot-separated part of `name` (`"auth"` for `rango.contrib.auth`). Used in model labels (`polls.Question`), migration names and command arguments. |
| `verbose_name` | `String` | Human-readable name. Defaults to the title-cased label. |
| `path` | `Option<PathBuf>` | The app's directory on disk. `None` by default. |
| `default_auto_field` | `Option<String>` | Present for forward compatibility; nothing reads it yet. |
| `ready` | `Option<fn() -> BoxFuture<'static, Result<()>>>` | Start-up hook. |

| Method | Effect |
|---|---|
| `AppConfig::new(name: &str)` | New config; label and verbose name derived from `name`. |
| `.label(label: &str)` | Set the label (and reset the verbose name to its title-cased form, so call `.verbose_name` after it). |
| `.verbose_name(name: &str)` | Set the human-readable name. |
| `.path(path: impl Into<PathBuf>)` | Set the app's directory. |
| `.ready(f: fn() -> BoxFuture<'static, Result<()>>)` | Set the start-up hook. |

Give two apps with the same last name different labels:

```rust
use rango::apps::AppConfig;
use rango::register_app;

register_app!(AppConfig::new("billing.admin").label("billing_admin").verbose_name("Billing administration"));
```

### The app directory

Several features look for files below an app's `path`:

| Directory | Used by |
|---|---|
| `<path>/templates` | the template engine, when `APP_DIRS` is `true` in the `templates` setting |
| `<path>/static` | `collectstatic`, `findstatic` |
| `<path>/locale` | translation catalogs (`rango.mo`) |
| `<path>/fixtures` | `loaddata` |

Without a `path` the app contributes none of these. The skeleton made by `startapp` does not set one, so add it when
the app has templates, static files, translations or fixtures. Point it at the module's directory:

```rust
use rango::apps::AppConfig;
use rango::register_app;

register_app!(
    AppConfig::new("polls").path(concat!(env!("CARGO_MANIFEST_DIR"), "/src/polls"))
);
```

For an app published as its own crate, `env!("CARGO_MANIFEST_DIR")` alone is the crate root, so use that and keep
`templates/` and `static/` at the crate root.

Migrations are found differently: `makemigrations` looks for `src/<label>/migrations` or `./<label>/migrations`
relative to the working directory, and the compiled migration modules register themselves.

### The ready hook

`ready` runs once, after every installed app is registered. It is the place to connect [signals](signals.md) or
validate configuration. The hook is a plain function pointer returning a boxed future, so a non-capturing closure
works:

```rust
use rango::apps::AppConfig;
use rango::dispatch::ConnectOptions;
use rango::db::models::signals::POST_SAVE;
use rango::register_app;

register_app!(
    AppConfig::new("polls").ready(|| Box::pin(async {
        POST_SAVE.connect(
            |event| async move {
                println!("saved {} #{:?}", event.args.label, event.args.pk);
            },
            ConnectOptions::sender("polls.Question").uid("polls.log_question_save"),
        );
        Ok(())
    }))
);
```

A hook that returns `Err` aborts `setup`. It must not depend on the database being migrated.

## Querying the registry

`rango::apps::apps()` returns the populated registry as `Result<Arc<Apps>>`. Before `setup` has run it fails with
`AppRegistryNotReady` ("Apps aren't loaded yet.").

| Method on `Apps` | Returns |
|---|---|
| `get_app_configs()` | Iterator over `&Arc<AppConfig>` in `installed_apps` order. |
| `get_app_config(label: &str)` | `Result<&Arc<AppConfig>>`. An unknown label is a `LookupError` ("No installed app with label 'x'."), with a "Did you mean" hint when the label equals the last part of an installed app's name. |
| `is_installed(name: &str)` | `bool`; matches the full `name`, not the label. |

```rust
use rango::apps::apps;

fn describe_apps() -> rango::Result<()> {
    let registry = apps()?;
    for config in registry.get_app_configs() {
        println!("{} ({}) at {:?}", config.verbose_name, config.label, config.path);
    }
    if registry.is_installed("rango.contrib.sessions") {
        println!("sessions are on");
    }
    Ok(())
}
```

## Built-in apps

These are registered by Rango and can be named in `installed_apps`. Commands of a `rango.*` app are available
regardless of whether the app is installed; installing an app is what activates its models, migrations, templates and
signal hooks.

| Name | Label |
|---|---|
| `rango.contrib.admin` | `admin` |
| `rango.contrib.auth` | `auth` |
| `rango.contrib.contenttypes` | `contenttypes` |
| `rango.contrib.sessions` | `sessions` |
| `rango.contrib.messages` | `messages` |
| `rango.contrib.staticfiles` | `staticfiles` |
| `rango.contrib.sites` | `sites` |
| `rango.contrib.redirects` | `redirects` |
| `rango.contrib.flatpages` | `flatpages` |
| `rango.contrib.sitemaps` | `sitemaps` |
| `rango.contrib.syndication` | `syndication` |
| `rango.rest` | `rest` |
| `rango.rest.authtoken` | `authtoken` |

## Apps and commands

Commands are tied to an app by the first argument of `register_command!`. A command registered under an app that is
not installed is hidden and cannot be run; that is how an app's commands stay inactive until you opt in. See
[Writing management commands](commands.md).

## Not supported yet

- Importing an app by module path: a name that nothing registered is an error.
- Per-app `default_auto_field`: the field exists on `AppConfig` but is not read.
- Reloading the registry in a running process.

## See also

- [Settings](settings.md) and the [settings reference](../ref/settings.md) (`installed_apps`)
- [Writing management commands](commands.md)
- [Signals](signals.md)
- [Migrations](db/migrations.md)
- [Static files](static-files.md)
