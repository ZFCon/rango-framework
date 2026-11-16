# Deploying a Rango project

How to take a project from `cargo run` on your machine to a service on a server: building a release binary, what
the built-in server does and does not do, running behind a reverse proxy, collecting static files, switching to
PostgreSQL, reading settings from the environment, and a checklist for turning `debug` off. The systemd unit and
Dockerfile near the end are examples to adapt, not files Rango generates.

A Rango project is an ordinary Rust binary. Its `main.rs` configures settings, calls `rango::setup()`, and hands the
command line to `execute_from_command_line`. Deploying it means building that binary, putting it next to the files
it reads at run time (templates, static files, and the database), and running its commands.

## Build a release binary

```console
$ cargo build --release
$ ls target/release/mysite
target/release/mysite
```

The binary is named after your package. It takes the same commands as `cargo run --`:

```console
$ ./target/release/mysite check --deploy
$ ./target/release/mysite migrate
$ ./target/release/mysite collectstatic --noinput
$ ./target/release/mysite runserver 127.0.0.1:8000
```

Files the binary reads at run time are not compiled in. Template directories (`DIRS` in the `templates` setting),
`staticfiles_dirs`, `static_root`, `media_root`, locale directories and a SQLite database file are paths. Relative
paths are resolved against the process's working directory, which `cargo run` sets to the project root. In
production, either start the process in the project directory or use absolute paths. A service that starts in `/`
will not find `templates/` or `db.sqlite3`.

## What the built-in server is

`runserver` is the only server in the framework. Its behaviour, from `rango::core::servers`:

- It listens on one address and speaks plain HTTP, with the connection protocol (HTTP/1 or HTTP/2 prior knowledge) detected automatically. There is no HTTPS support, so
  something in front of it must terminate TLS.
- It is asynchronous on the Tokio runtime and serves connections concurrently. `--nothreading` and `--noreload`
  are accepted and ignored.
- The default address is `127.0.0.1:8000`. Give `addr:port`, a bare port, or `-6` for IPv6 (default `[::1]`);
  `0` as the address means `0.0.0.0`.
- It reads each request body completely into memory before the view runs. The body limit comes from your settings
  (`data_upload_max_memory_size`, `data_upload_max_number_fields`), not from the server. There are no connection,
  header or request timeouts, so slow clients are held open until they finish or disconnect.
- It prints one line per request (`[date] "METHOD /path HTTP/1.1" status size`) to standard error, always. You cannot
  turn that off from settings.
- It stops on Ctrl-C (SIGINT). It does not handle SIGTERM specially, and it does not wait for in-flight requests: the
  process just ends. Under a service manager that sends SIGTERM, the default process termination is what you get.
- It runs the system checks first (skip with `--skip-checks`) and refuses to start when `debug` is `false` and
  `allowed_hosts` is empty.
- It does not serve static or media files. See [Static files](#static-files).
- It does not reload on change. Use `cargo watch -x run` in development.
- The client address it reports to your code is the TCP peer, so behind a proxy it is the proxy's address.
  `X-Forwarded-For` is not rewritten into it.

These are the limits of a minimal server, not security features. Treat `runserver` as an application server you
put behind a hardened reverse proxy, and let the proxy handle TLS, buffering, timeouts, request size limits and
static files.

If you need a different embedding, `rango::core::handlers::Handler::new()` builds the request handler and
`rango::core::servers::serve(handler, listener, log_requests)` runs it on a `tokio::net::TcpListener` until
Ctrl-C; `request_from_hyper` and `response_to_hyper` convert between hyper and Rango types.

## Behind a reverse proxy

Run Rango on a loopback port and let nginx, Caddy or a cloud load balancer face the internet. Tell Rango how to
recognise HTTPS and which host the client asked for.

```rust
use rango::conf::Settings;

Settings {
    debug: false,
    allowed_hosts: vec!["example.com".into(), "www.example.com".into()],

    // The proxy sets X-Forwarded-Proto to "https" for TLS connections and strips it from client requests.
    secure_proxy_ssl_header: Some(("HTTP_X_FORWARDED_PROTO".into(), "https".into())),

    // Only if the proxy rewrites Host and sets X-Forwarded-Host itself.
    use_x_forwarded_host: false,
    ..Settings::default()
}
```

- `secure_proxy_ssl_header` is a pair `(header in request-metadata form, expected value)`. Without it Rango sees
  every request as plain HTTP, which breaks `secure_ssl_redirect`, secure cookies' assumptions, CSRF Referer
  checks for HTTPS pages, and absolute URLs built with `build_absolute_uri`.
- Do not enable `use_x_forwarded_host` or `use_x_forwarded_port` unless the proxy overwrites those headers.
- The simplest redirect-to-HTTPS arrangement is to let the proxy do it. If you use `secure_ssl_redirect: true` in
  Rango as well, add health-check paths to `secure_redirect_exempt` so the proxy's plain-HTTP probe is not
  redirected.
- Set `csrf_trusted_origins` to your public origins (`"https://example.com"`) if the browser's `Origin` can differ
  from the host Rango sees.

See [Security](security.md) for the full list of HTTPS and cookie settings.

An nginx server block, as an example only:

```nginx
server {
    listen 443 ssl;
    server_name example.com;
    client_max_body_size 10m;

    location /static/ { alias /srv/mysite/static/; expires 30d; }
    location /media/  { alias /srv/mysite/media/; }

    location / {
        proxy_pass http://127.0.0.1:8000;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
}
```

## Static files

`runserver` does not serve static files by itself, and the helper `rango::contrib::staticfiles::serve` is meant for
development. In production:

1. Set `static_url` and `static_root`:

   ```rust
   Settings {
       static_url: Some("/static/".into()),
       static_root: Some("/srv/mysite/static".into()),
       ..Settings::default()
   }
   ```

2. Copy all static files into `static_root` as part of every release:

   ```console
   $ ./mysite collectstatic --noinput
   ```

   `collectstatic` fails with an "improperly configured" error when `static_root` is unset. `--clear` empties the
   directory first, `--dry-run` reports what would be copied, and `--ignore PATTERN` skips files.

3. Let the web server (or a CDN) serve that directory at `static_url`.

For cache-friendly hashed file names, set the `staticfiles` storage to the manifest backend; `collectstatic` then
writes `staticfiles.json` and the `static` template tag resolves names through it. See
[Static files](static-files.md). User uploads are separate: serve `media_root` from the web server as well, see
[Managing files](files.md).

## PostgreSQL

SQLite is the template's default and works for a single machine with modest write load. For anything larger, use
PostgreSQL:

```rust
use std::collections::BTreeMap;
use rango::conf::{DatabaseConfig, Settings};

Settings {
    databases: BTreeMap::from([(
        "default".to_owned(),
        DatabaseConfig {
            engine: "rango.db.backends.postgresql".into(),
            name: "mysite".into(),
            user: "mysite".into(),
            password: std::env::var("MYSITE_DB_PASSWORD").expect("MYSITE_DB_PASSWORD"),
            host: "127.0.0.1".into(),
            port: "5432".into(),
            options: rango::json!({ "max_connections": 20 }),
            ..DatabaseConfig::default()
        },
    )]),
    ..Settings::default()
}
```

Points that matter in production:

- The pool opens at most `options.max_connections` connections per process (default 10) and waits for a free one
  when all are in use. Keep the total across processes below the server's `max_connections`.
- The PostgreSQL connection is made without TLS. Connect over a loopback or private network, or through a tunnel
  or sidecar that encrypts; do not point Rango at a database across the public internet.
- The connection string is assembled from `name`, `host`, `port`, `user` and `password` without quoting, so avoid
  spaces in those values.
- Run `migrate` once per release, before the new code serves traffic. See [Migrations](db/migrations.md).

## Settings from the environment

Settings are a Rust value, so reading the environment is ordinary code. Keep one function that builds production
settings and choose between it and the development settings at start-up:

```rust
use std::env;
use rango::conf::Settings;

fn require(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("environment variable {name} is not set"))
}

pub fn settings() -> Settings {
    let debug = env::var("MYSITE_DEBUG").is_ok_and(|v| v == "1");
    Settings {
        debug,
        secret_key: require("MYSITE_SECRET_KEY"),
        allowed_hosts: require("MYSITE_ALLOWED_HOSTS").split(',').map(str::to_owned).collect(),
        static_root: Some(require("MYSITE_STATIC_ROOT")),
        // ... middleware, installed_apps, databases as in your development settings
        ..Settings::default()
    }
}
```

Do not give `secret_key` a fallback in production code: a missing key should stop the process. More patterns are in
[Settings](settings.md).

## Checklist for `debug: false`

`Settings::default()` already has `debug: false`; the project template turns it on for development. Before going
live:

- `debug` is `false`. With `true`, error pages show source locations, settings and request data.
- `allowed_hosts` lists every host name you serve.
- `secret_key` comes from the environment, is long and random (50 or more characters), and is not committed.
- `middleware` still includes the security, common, CSRF and clickjacking middleware.
- Cookies: `session_cookie_secure` and `csrf_cookie_secure` are `true`.
- HTTPS: `secure_proxy_ssl_header` is set if a proxy terminates TLS; start HSTS with a small
  `secure_hsts_seconds`.
- Static files are collected and served by the web server; media files are served by the web server.
- `migrate` has been run against the production database.
- You can see errors: with `debug` off, users get the plain 404/500 pages, and the server's request log on
  standard error is the only record unless you add your own logging. Capture standard error (systemd's journal
  does this).
- `check --deploy` passes:

  ```console
  $ ./mysite check --deploy --fail-level WARNING
  ```

  It audits the security settings listed in [Security](security.md#auditing-check-deploy) and exits non-zero on
  any message at or above the level.
- Custom 404 and 500 templates (`404.html`, `500.html`) are in your templates directory if you want a branded
  page.

## Example: systemd unit

An example for a Linux server. Adjust paths and the user.

```ini
# /etc/systemd/system/mysite.service
[Unit]
Description=mysite (Rango)
After=network.target postgresql.service

[Service]
User=mysite
WorkingDirectory=/srv/mysite
EnvironmentFile=/etc/mysite.env
ExecStartPre=/srv/mysite/mysite migrate --noinput
ExecStart=/srv/mysite/mysite runserver 127.0.0.1:8000 --skip-checks
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

`/etc/mysite.env` holds `MYSITE_SECRET_KEY=...`, `MYSITE_ALLOWED_HOSTS=example.com`, and so on, readable only by
root. `WorkingDirectory` is the directory containing `templates/`. `--skip-checks` avoids running the checks
on every restart; run `check --deploy` in your release pipeline instead.

## Example: Dockerfile

A two-stage build. The final image contains the binary, the templates and the collected static files.

```dockerfile
# Example only.
FROM rust:1 AS build
WORKDIR /src
COPY . .
RUN cargo build --release

FROM debian:stable-slim
WORKDIR /srv/mysite
COPY --from=build /src/target/release/mysite /srv/mysite/mysite
COPY templates ./templates
COPY static ./static
ENV MYSITE_STATIC_ROOT=/srv/mysite/staticroot
RUN mkdir -p /srv/mysite/staticroot
EXPOSE 8000
CMD ["/srv/mysite/mysite", "runserver", "0.0.0.0:8000", "--skip-checks"]
```

Run the commands that change state as separate steps of your deployment, not in the image build:

```console
$ docker run --rm --env-file prod.env mysite migrate --noinput
$ docker run --rm --env-file prod.env -v static:/srv/mysite/staticroot mysite collectstatic --noinput
```

Share the `static` volume with the proxy container so it can serve `/static/`. Because the `runserver` address
`0.0.0.0` makes the port reachable from outside the container, publish it only to the proxy's network.

## Not supported yet

- The built-in server has no TLS, graceful shutdown, worker management or request timeouts; use a proxy and a
  process supervisor for those.
- PostgreSQL connections cannot use TLS.
- `runserver` has no static-file serving of its own.

## See also

- [Security](security.md)
- [Settings](settings.md)
- [Static files](static-files.md)
- [Managing files](files.md)
- [Migrations](db/migrations.md)
- [System checks](checks.md)
- [Management commands](commands.md)
