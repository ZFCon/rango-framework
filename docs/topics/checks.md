# System checks

System checks inspect a project for configuration mistakes before they bite: a missing secret key, an insecure
cookie setting, a custom rule of your own. This page covers the `check` command, deployment checks, silencing, writing
and registering your own checks, and the built-in security checks.

## Running the checks

```console
$ cargo run -- check
System check identified no issues (0 silenced).
```

When a check reports something, the report lists it under its severity and ends with a count:

```console
$ cargo run -- check --deploy
System check identified some issues:

WARNINGS:
?: (security.W004) You have not set a value for the SECURE_HSTS_SECONDS setting. ...
?: (security.W018) You should not have DEBUG set to True in deployment.

System check identified 2 issues (0 silenced).
```

The leading `?` is the object a message is about; it appears when the check did not name one. Messages with a hint
add a `HINT:` line below. Severities, from least to most serious, are `DEBUG` (10), `INFO` (20), `WARNING` (30),
`ERROR` (40) and `CRITICAL` (50); the constants live in `rango::core::checks`.

By default the command fails (exit status 1, report on stderr) when any non-silenced message is at `ERROR` or above.
Use `--fail-level` to move the threshold, for example `--fail-level WARNING` in a deployment pipeline. Every option of
the command, including `--tag`, `--list-tags` and `--database`, is in the [commands reference](../ref/commands.md#check).

Most other commands run the (non-deployment) checks automatically before they start and refuse to run if one fails.
Pass `--skip-checks` to bypass that for a single run; `runserver` prints `Performing system checks...` and does the
same.

## Deployment checks

A check can be marked as a deployment check. Those run only with `--deploy`, because they flag things that are fine
in development (`debug = true`, no HTTPS) but wrong in production. Run it against your production settings:

```console
$ cargo run -- check --deploy --fail-level WARNING
```

All of Rango's built-in checks are deployment checks; see [Built-in security checks](#built-in-security-checks).

## Silencing a check

List the id of a message in `silenced_system_checks`. A silenced message is not printed, does not fail the command and
is counted in the footer (`1 issue (1 silenced)`).

```rust
use rango::conf::Settings;

let settings = Settings {
    silenced_system_checks: vec!["security.W021".into()],
    ..Settings::default()
};
```

Only messages that have an id can be silenced.

## Writing a check

A check is an `async fn` that takes a `&CheckContext` and returns a `Vec<CheckMessage>`; an empty vector means all is
well. Register it with `register_check!`.

```rust
use rango::conf;
use rango::core::checks::{self, CheckContext, CheckMessage};
use rango::register_check;

async fn site_url_check(_: &CheckContext) -> Vec<CheckMessage> {
    let s = conf::settings();
    if !s.debug && s.allowed_hosts.is_empty() {
        vec![
            CheckMessage::error("ALLOWED_HOSTS must not be empty in production.")
                .with_id("shop.E001")
                .with_obj("settings")
                .with_hint("List the host names the site is served on."),
        ]
    } else {
        vec![]
    }
}

register_check!(tags = [checks::tags::SECURITY], site_url_check);
```

(The same check, as a test, is in `crates/rango/tests/manage_cli.rs`.)

`register_check!` has two forms:

| Form | Meaning |
|---|---|
| `register_check!(tags = [t1, t2], f)` | A normal check; runs on every `check` and before commands. |
| `register_check!(tags = [t1], deploy = true, f)` | A deployment check; runs only with `--deploy`. |

`tags` may be empty and takes string expressions; `f` is a path to the function. Like `register_app!`, the macro runs
at link time, so the module containing it must be compiled into the binary.

### CheckMessage

| Item | Meaning |
|---|---|
| `CheckMessage::new(level, msg)` | A message of any level. |
| `CheckMessage::debug / info / warning / error / critical(msg)` | Shortcuts for the five levels. |
| `.with_hint(text)` | Advice on how to fix it. |
| `.with_obj(text)` | What the message concerns (a setting name, a model label). Shown first in the report. |
| `.with_id(text)` | Identifier such as `"shop.E001"`, used for silencing. Use your own prefix. |
| `is_serious(level) -> bool` | Whether the message is at least that level. |
| `is_silenced() -> bool` | Whether its id is in `silenced_system_checks`. |

The fields `level`, `msg`, `hint`, `obj` and `id` are public. By convention ids look like `app.E001` for errors and
`app.W001` for warnings.

### CheckContext

Every check receives a `CheckContext` with two optional fields from the command line:

| Field | Meaning |
|---|---|
| `app_labels: Option<Vec<String>>` | The app labels given to `check` (`None` means all). Use it to limit model-related checks. |
| `databases: Option<Vec<String>>` | The aliases given with `--database` (`None` means none). Only touch databases listed here. |

### Tags

Tags group checks so they can be selected with `check --tag <tag>`. The constants in `rango::core::checks::tags` are
`ADMIN`, `ASYNC_SUPPORT`, `CACHES`, `COMPATIBILITY`, `DATABASE`, `FILES`, `MODELS`, `SECURITY`, `SIGNALS`, `SITES`,
`STATICFILES`, `TEMPLATES`, `TRANSLATION` and `URLS`; any string is allowed. At present only the `security` tag has
registered checks from Rango itself, so `check --list-tags` prints nothing until you add checks, and `security` with
`--deploy`.

### Running checks from code

| Function | Purpose |
|---|---|
| `checks::run_checks(&CheckContext, tags: Option<&[String]>, include_deployment_checks: bool) -> Result<Vec<CheckMessage>>` | Run the selected checks in turn and collect their messages. |
| `checks::tags_available(deployment_checks: bool) -> Vec<&'static str>` | Sorted, de-duplicated tags of the registered checks. |
| `management::run_system_checks(&CommandContext, &CheckOptions) -> Result<(), CommandError>` | Run the checks, print the report, and fail on serious messages, as the commands do. |

The order in which different checks run is not specified.

## Built-in security checks

`check --deploy` runs one function that evaluates the rules below against the settings, tagged `security`. Messages
carry no object, so they appear as `?: (security.Wnnn) ...`. The middleware names are the dotted registry names in
the `middleware` setting.

| Id | Level | Raised when |
|---|---|---|
| `security.W001` | WARNING | `rango.middleware.security.SecurityMiddleware` is not in `middleware`, so the HSTS, nosniff, referrer, opener-policy and SSL-redirect settings have no effect. |
| `security.W002` | WARNING | `rango.middleware.clickjacking.XFrameOptionsMiddleware` is not in `middleware`. |
| `security.W003` | WARNING | `rango.middleware.csrf.CsrfViewMiddleware` is not in `middleware`. |
| `security.W004` | WARNING | `SecurityMiddleware` is installed and `secure_hsts_seconds` is `0`. |
| `security.W005` | WARNING | `SecurityMiddleware` is installed, `secure_hsts_seconds` is non-zero and `secure_hsts_include_subdomains` is `false`. |
| `security.W006` | WARNING | `SecurityMiddleware` is installed and `secure_content_type_nosniff` is `false`. |
| `security.W008` | WARNING | `SecurityMiddleware` is installed and `secure_ssl_redirect` is `false`. |
| `security.W009` | WARNING | `secret_key` is shorter than 50 characters, has fewer than 5 distinct characters, or starts with `rango-insecure-`. |
| `security.W010` | WARNING | `rango.contrib.sessions` is in `installed_apps` and `session_cookie_secure` is `false`. |
| `security.W011` | WARNING | `rango.contrib.sessions.middleware.SessionMiddleware` is in `middleware` and `session_cookie_secure` is `false`. |
| `security.W012` | WARNING | Both W010 and W011 apply; reported instead of the two. |
| `security.W013` | WARNING | `rango.contrib.sessions` is installed and `session_cookie_httponly` is `false`. |
| `security.W014` | WARNING | The session middleware is installed and `session_cookie_httponly` is `false`. |
| `security.W015` | WARNING | Both W013 and W014 apply; reported instead of the two. |
| `security.W016` | WARNING | `CsrfViewMiddleware` is installed, `csrf_use_sessions` is `false` and `csrf_cookie_secure` is `false`. |
| `security.W018` | WARNING | `debug` is `true`. |
| `security.W019` | WARNING | `XFrameOptionsMiddleware` is installed and `x_frame_options` is not `"DENY"`. |
| `security.W020` | WARNING | `debug` is `false` and `allowed_hosts` is empty. |
| `security.W021` | WARNING | `SecurityMiddleware` is installed, `secure_hsts_seconds` is non-zero and `secure_hsts_preload` is `false`. |
| `security.W022` | WARNING | `SecurityMiddleware` is installed and `secure_referrer_policy` is empty. |
| `security.E023` | ERROR | `secure_referrer_policy` is non-empty and a comma-separated part is not one of `no-referrer`, `no-referrer-when-downgrade`, `origin`, `origin-when-cross-origin`, `same-origin`, `strict-origin`, `strict-origin-when-cross-origin`, `unsafe-url`. |
| `security.E024` | ERROR | `SecurityMiddleware` is installed and `secure_cross_origin_opener_policy` is not `same-origin`, `same-origin-allow-popups` or `unsafe-none`. |
| `security.W025` | WARNING | Any entry of `secret_key_fallbacks` fails the same test as W009. |

There are no ids `security.W007` or `security.W017`: those rules do not exist. The full message and hint texts are
stored in `crates/rango-core/data/security_checks.json`.

A starting point for production settings that clears the warnings above, given a long random key:

```rust
use rango::conf::Settings;

pub fn production() -> Settings {
    Settings {
        debug: false,
        secret_key: std::env::var("MYSITE_SECRET_KEY").expect("MYSITE_SECRET_KEY"),
        allowed_hosts: vec!["example.com".into()],
        secure_ssl_redirect: true,
        secure_hsts_seconds: 31_536_000,
        secure_hsts_include_subdomains: true,
        secure_hsts_preload: true,
        session_cookie_secure: true,
        csrf_cookie_secure: true,
        middleware: [
            "rango.middleware.security.SecurityMiddleware",
            "rango.contrib.sessions.middleware.SessionMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
            "rango.middleware.clickjacking.XFrameOptionsMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        ..Settings::default()
    }
}
```

Read the HSTS documentation of your browser vendor before enabling `secure_hsts_seconds` with preload: it is hard to
undo.

## Not supported yet

- Built-in checks for models, URL configuration, templates, caches, databases, static files and translations: the
  tags exist, but no checks are registered under them. Mistakes there surface when the code runs.
- Checks that need to be re-run per request, or health-check endpoints: checks run only from the command line and
  before commands.

## See also

- [Management commands reference](../ref/commands.md#check)
- [Writing management commands](commands.md)
- [Settings reference](../ref/settings.md)
- [Security](security.md) and [Deployment](deployment.md)
