# Settings reference

Every field of `rango::conf::Settings`, grouped by area, with its Rust type, its default and what it does. For how to
build, install, read and override settings, see [Settings](../topics/settings.md).

Conventions used on this page:

- The name in the first column is the Rust field name. The same setting appears as the `UPPER_CASE` name in
  `diffsettings` output, in `Settings::to_map()` and in [system check](../topics/checks.md) messages.
- `Json` is `rango::conf::Json` (`serde_json::Value`), used for free-form settings. Build values with `rango::json!`.
- Dotted strings such as `"rango.middleware.csrf.CsrfViewMiddleware"` are registry names, not module paths; they are
  matched against what Rango registers, not imported.
- **Accepted but not used yet** means the field exists, is part of `Settings` and is shown by `diffsettings`, but no
  code in the workspace reads it, so changing it has no effect today.
- "Read by" names the area of the code that reads the setting. A setting without that note is read where its
  description says.

There are 163 settings.

## Core

| Field | Type | Default | Meaning |
|---|---|---|---|
| `debug` | `bool` | `false` | Development mode: detailed error pages, template debug info, relaxed `ALLOWED_HOSTS` handling, `runserver` without allowed hosts. Never enable in production (`security.W018`). |
| `debug_propagate_exceptions` | `bool` | `false` | After logging an unhandled error, the handler panics instead of building a 500 response. Useful in tests. |
| `secret_key` | `String` | `""` | Key for signing: sessions, CSRF, password-reset tokens, signed cookies, messages. Empty is an `ImproperlyConfigured` error on first use (`Settings::secret_key()`). |
| `secret_key_fallbacks` | `Vec<String>` | `[]` | Old keys still accepted when verifying signatures, so the key can be rotated. Each is also checked by `security.W025`. |
| `allowed_hosts` | `Vec<String>` | `[]` | Host names the site may serve. Must be non-empty when `debug` is `false`; `runserver` refuses to start otherwise. |
| `internal_ips` | `Vec<String>` | `[]` | Client addresses for which the `debug` template context processor exposes `debug` to templates while `debug` is `true`. |
| `admins` | `Vec<(String, String)>` | `[]` | `(name, email)` pairs notified by `mail_admins`. |
| `managers` | `Vec<(String, String)>` | `[]` | `(name, email)` pairs notified by `mail_managers`. |
| `installed_apps` | `Vec<String>` | `[]` | Names of the registered apps that take part in the project, in order. See [Applications](../topics/apps.md). An unknown name fails start-up. |
| `middleware` | `Vec<String>` | `[]` | Registry names of the middleware, outermost first. See the [middleware reference](middleware.md). |
| `root_urlconf` | `Option<String>` | `None` | Name of the URLconf registered with `register_urlconf!` that resolves request paths. Resolving without one is an `ImproperlyConfigured` error. |
| `append_slash` | `bool` | `true` | `CommonMiddleware` redirects a path without a trailing slash when the slashed path resolves. Also read by the flatpages and redirects fallbacks. |
| `prepend_www` | `bool` | `false` | `CommonMiddleware` redirects `example.com` to `www.example.com`. |
| `disallowed_user_agents` | `Vec<String>` | `[]` | Regular expressions; requests whose `User-Agent` matches get a 403 from `CommonMiddleware`. |
| `ignorable_404_urls` | `Vec<String>` | `[]` | Accepted but not used yet. |
| `force_script_name` | `Option<String>` | `None` | Use this value as the script prefix instead of the one the server reports. |
| `use_x_forwarded_host` | `bool` | `false` | Take the host from `X-Forwarded-Host` instead of `Host`. Enable only behind a proxy you control. |
| `use_x_forwarded_port` | `bool` | `false` | Take the port from `X-Forwarded-Port`. |
| `secure_proxy_ssl_header` | `Option<(String, String)>` | `None` | `(header, value)` pair; when the request carries that header with that value it is treated as secure (HTTPS). |
| `default_charset` | `String` | `"utf-8"` | Charset of responses and of decoding request data. |
| `wsgi_application` | `Option<String>` | `None` | Accepted but not used yet. Rango has no WSGI layer. |
| `asgi_application` | `Option<String>` | `None` | Accepted but not used yet. |
| `default_exception_reporter` | `String` | `"rango.views.debug.ExceptionReporter"` | Accepted but not used yet. |
| `default_exception_reporter_filter` | `String` | `"rango.views.debug.SafeExceptionReporterFilter"` | Accepted but not used yet. |
| `absolute_url_overrides` | `Json` | `{}` | Accepted but not used yet. |
| `silenced_system_checks` | `Vec<String>` | `[]` | Check ids (such as `"security.W004"`) that are not reported and never fail a command. See [System checks](../topics/checks.md). |
| `logging` | `Json` | `{}` | Accepted but not used yet. |
| `logging_config` | `String` | `"rango.utils.log.configure_logging"` | Accepted but not used yet. |
| `test_runner` | `String` | `"rango.test.runner.DiscoverRunner"` | Accepted but not used yet. Tests run through `cargo test`. |
| `test_non_serialized_apps` | `Vec<String>` | `[]` | Accepted but not used yet. |
| `fixture_dirs` | `Vec<String>` | `[]` | Extra directories searched by `loaddata` for fixture files. |

## Databases

| Field | Type | Default | Meaning |
|---|---|---|---|
| `databases` | `BTreeMap<String, DatabaseConfig>` | empty | Connections by alias. `"default"` is used when no alias is given. |
| `database_routers` | `Vec<String>` | `[]` | Accepted but not used yet. |
| `default_auto_field` | `String` | `"rango.db.models.AutoField"` | Accepted but not used yet. (`AppConfig::default_auto_field` also exists but is likewise unread.) |
| `default_tablespace` | `String` | `""` | Accepted but not used yet. |
| `default_index_tablespace` | `String` | `""` | Accepted but not used yet. |
| `migration_modules` | `Json` | `{}` | Accepted but not used yet. Migrations are found through each app's `migrations` module. |

`DatabaseConfig` (the value type of `databases`) has these fields. Its `Default` is a dummy engine, so always set
`engine`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `engine` | `String` | `"rango.db.backends.dummy"` | `"rango.db.backends.sqlite3"` or `"rango.db.backends.postgresql"`. Any other value is an `ImproperlyConfigured` error when the connection is first used. |
| `name` | `String` | `""` | SQLite: file path. PostgreSQL: database name. |
| `user`, `password`, `host`, `port` | `String` | `""` | PostgreSQL connection details. `dbshell` also reads them. |
| `options` | `Json` | `{}` | Backend options. PostgreSQL reads `max_connections` (default 10) for the pool size. |
| `atomic_requests` | `bool` | `false` | Accepted but not used yet. |
| `autocommit` | `bool` | `true` | Accepted but not used yet. |
| `conn_max_age` | `Option<u64>` | `Some(0)` | Accepted but not used yet. |
| `conn_health_checks` | `bool` | `false` | Accepted but not used yet. |
| `disable_server_side_cursors` | `bool` | `false` | Accepted but not used yet. |
| `time_zone` | `Option<String>` | `None` | Accepted but not used yet. |
| `test` | `DatabaseTestConfig` | see below | Test-database options. |

`DatabaseTestConfig`: `charset: Option<String>` (`None`), `collation: Option<String>` (`None`), `migrate: bool`
(`true`), `mirror: Option<String>` (`None`), `name: Option<String>` (`None`). None of them is read yet.

```rust
use std::collections::BTreeMap;
use rango::conf::{DatabaseConfig, Settings};

let settings = Settings {
    databases: BTreeMap::from([(
        "default".to_owned(),
        DatabaseConfig {
            engine: "rango.db.backends.sqlite3".into(),
            name: "db.sqlite3".into(),
            ..DatabaseConfig::default()
        },
    )]),
    ..Settings::default()
};
```

## Templates and forms

| Field | Type | Default | Meaning |
|---|---|---|---|
| `templates` | `Vec<Json>` | `[]` | Template engine configurations (`BACKEND`, `DIRS`, `APP_DIRS`, `OPTIONS`). An empty list means no engine. See [engine](../topics/templates/engine.md). |
| `form_renderer` | `String` | `"rango.forms.renderers.RangoTemplates"` | Renderer used to turn forms and widgets into HTML. |
| `forms_urlfield_assume_https` | `bool` | `false` | A URL form field without a scheme gets `https://` instead of `http://`. |

## Internationalisation and formatting

| Field | Type | Default | Meaning |
|---|---|---|---|
| `language_code` | `String` | `"en-us"` | Default language. |
| `languages` | `Vec<(String, String)>` | the built-in list of 100+ languages (`"en"`, `"fr"`, ...) | Languages the site offers, as `(code, English name)`. Language negotiation only picks from this list. |
| `languages_bidi` | `Vec<String>` | `["he", "ar", "ar-dz", "ckb", "fa", "ug", "ur"]` | Accepted but not used yet. |
| `use_i18n` | `bool` | `true` | Accepted but not used yet. Translation is always available. |
| `locale_paths` | `Vec<String>` | `[]` | Extra directories searched for `locale/<lang>/LC_MESSAGES/rango.mo` catalogs. |
| `language_cookie_name` | `String` | `"rango_language"` | Cookie that `LocaleMiddleware` reads to pick the language. |
| `language_cookie_age` | `Option<i64>` | `None` | Accepted but not used yet. |
| `language_cookie_domain` | `Option<String>` | `None` | Accepted but not used yet. |
| `language_cookie_httponly` | `bool` | `false` | Accepted but not used yet. |
| `language_cookie_path` | `String` | `"/"` | Accepted but not used yet. |
| `language_cookie_samesite` | `Option<String>` | `None` | Accepted but not used yet. |
| `language_cookie_secure` | `bool` | `false` | Accepted but not used yet. |
| `time_zone` | `String` | `"America/Chicago"` | Default time zone name. |
| `use_tz` | `bool` | `true` | Store and handle timezone-aware date-times (UTC in the database). |
| `use_thousand_separator` | `bool` | `false` | Group digits when formatting numbers. |
| `thousand_separator` | `String` | `","` | Group separator. |
| `decimal_separator` | `String` | `"."` | Decimal separator. |
| `number_grouping` | `Json` | `0` | Digits per group (a number, or a list for uneven groups); `0` means no grouping. |
| `first_day_of_week` | `u8` | `0` | First day of the week for calendars (0 is Sunday). |
| `format_module_path` | `Option<String>` | `None` | Accepted but not used yet. |
| `date_format` | `String` | `"N j, Y"` | Default display format for dates (template date-format syntax). |
| `datetime_format` | `String` | `"N j, Y, P"` | Display format for date-times. |
| `time_format` | `String` | `"P"` | Display format for times. |
| `year_month_format` | `String` | `"F Y"` | Display format for year and month. |
| `month_day_format` | `String` | `"F j"` | Display format for month and day. |
| `short_date_format` | `String` | `"m/d/Y"` | Short date display format. |
| `short_datetime_format` | `String` | `"m/d/Y P"` | Short date-time display format. |
| `date_input_formats` | `Vec<String>` | `["%Y-%m-%d", "%m/%d/%Y", "%m/%d/%y", "%b %d %Y", ...]` (11 patterns) | Accepted input formats of date form fields, tried in order. |
| `datetime_input_formats` | `Vec<String>` | `["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M:%S.%f", "%Y-%m-%d %H:%M", "%m/%d/%Y %H:%M:%S", ...]` (9 patterns) | Accepted input formats of date-time form fields. |
| `time_input_formats` | `Vec<String>` | `["%H:%M:%S", "%H:%M:%S.%f", "%H:%M"]` | Accepted input formats of time form fields. |

## HTTP requests and uploads

| Field | Type | Default | Meaning |
|---|---|---|---|
| `data_upload_max_memory_size` | `i64` | `2621440` (2.5 MiB) | Largest request body read into memory, excluding file uploads. Larger bodies are rejected. |
| `data_upload_max_number_fields` | `i64` | `1000` | Most form fields (GET or POST) in one request. |
| `data_upload_max_number_files` | `i64` | `100` | Most files in one multipart request. |
| `file_upload_max_memory_size` | `i64` | `2621440` | Size above which an uploaded file is spooled to a temporary file instead of kept in memory. |
| `file_upload_temp_dir` | `Option<String>` | `None` | Directory for those temporary files; `None` uses the system temporary directory. |
| `file_upload_handlers` | `Vec<String>` | memory and temporary-file handlers | Accepted but not used yet. |
| `file_upload_permissions` | `Option<u32>` | `Some(0o644)` | Accepted but not used yet. |
| `file_upload_directory_permissions` | `Option<u32>` | `None` | Accepted but not used yet. |
| `media_root` | `String` | `""` | Directory where `FileSystemStorage` keeps uploaded files. |
| `media_url` | `String` | `""` | URL prefix for those files. |

## Static files

| Field | Type | Default | Meaning |
|---|---|---|---|
| `static_url` | `Option<String>` | `None` | URL prefix of static files (for example `"static/"`); used by `{% static %}` and widgets. |
| `static_root` | `Option<String>` | `None` | Directory `collectstatic` copies into. Required by that command. |
| `staticfiles_dirs` | `Vec<String>` | `[]` | Extra directories searched for static files, besides each app's `static` directory. |
| `staticfiles_finders` | `Vec<String>` | file-system and app-directories finders | Accepted but not used yet. The two default finders are always used. |
| `storages` | `Json` | `{"default": {"BACKEND": "rango.core.files.storage.FileSystemStorage"}, "staticfiles": {"BACKEND": "rango.contrib.staticfiles.storage.StaticFilesStorage"}}` | Storage backends by alias. A `staticfiles` backend whose name ends in `ManifestStaticFilesStorage` makes `collectstatic` write content-hashed file names and a `staticfiles.json` manifest. |

## Email

| Field | Type | Default | Meaning |
|---|---|---|---|
| `email_backend` | `String` | `"rango.core.mail.backends.smtp.EmailBackend"` | Backend: a name containing `smtp`, `console`, `locmem`, `dummy` or `filebased` followed by `.EmailBackend`. Anything else is an `ImproperlyConfigured` error. The file-based backend writes to the directory in the `RANGO_EMAIL_FILE_PATH` environment variable. |
| `email_host` | `String` | `"localhost"` | SMTP host. |
| `email_port` | `u16` | `25` | SMTP port. |
| `email_host_user` | `String` | `""` | SMTP user. |
| `email_host_password` | `String` | `""` | SMTP password. |
| `email_use_tls` | `bool` | `false` | Use STARTTLS. |
| `email_use_ssl` | `bool` | `false` | Use implicit TLS. |
| `email_ssl_certfile` | `Option<String>` | `None` | Accepted but not used yet. |
| `email_ssl_keyfile` | `Option<String>` | `None` | Accepted but not used yet. |
| `email_timeout` | `Option<f64>` | `None` | Accepted but not used yet. |
| `email_use_localtime` | `bool` | `false` | Accepted but not used yet. |
| `email_subject_prefix` | `String` | `"[Rango] "` | Prefix of `mail_admins` / `mail_managers` subjects. |
| `default_from_email` | `String` | `"webmaster@localhost"` | Sender when none is given. |
| `server_email` | `String` | `"root@localhost"` | Sender of error mail to admins and managers. |

## Cache

| Field | Type | Default | Meaning |
|---|---|---|---|
| `caches` | `Json` | `{"default": {"BACKEND": "rango.core.cache.backends.locmem.LocMemCache"}}` | Cache configurations by alias. Each may set `BACKEND` (`locmem.LocMemCache`, `dummy.DummyCache`, `filebased.FileBasedCache`, `redis.RedisCache` under `rango.core.cache.backends`), `LOCATION`, `TIMEOUT`, `KEY_PREFIX` and `VERSION`. See [Cache](../topics/cache.md). |
| `cache_middleware_alias` | `String` | `"default"` | Cache alias used by the cache middleware. |
| `cache_middleware_key_prefix` | `String` | `""` | Key prefix used by the cache middleware. |
| `cache_middleware_seconds` | `i64` | `600` | Default lifetime of pages cached by the middleware. |

## Sessions

| Field | Type | Default | Meaning |
|---|---|---|---|
| `session_engine` | `String` | `"rango.contrib.sessions.backends.db"` | One of `rango.contrib.sessions.backends.` followed by `db`, `cache`, `cached_db`, `file` or `signed_cookies`. Anything else is an `ImproperlyConfigured` error. |
| `session_cache_alias` | `String` | `"default"` | Cache alias for the `cache` and `cached_db` engines. |
| `session_file_path` | `Option<String>` | `None` | Directory for the `file` engine. |
| `session_cookie_name` | `String` | `"sessionid"` | Session cookie name (also read by the REST session authentication). |
| `session_cookie_age` | `i64` | `1209600` (two weeks) | Session lifetime in seconds. |
| `session_cookie_domain` | `Option<String>` | `None` | Cookie domain. |
| `session_cookie_path` | `String` | `"/"` | Cookie path. |
| `session_cookie_secure` | `bool` | `false` | Send the cookie over HTTPS only (`security.W010`-`W012`). |
| `session_cookie_httponly` | `bool` | `true` | Hide the cookie from scripts (`security.W013`-`W015`). |
| `session_cookie_samesite` | `String` | `"Lax"` | `SameSite` attribute. |
| `session_expire_at_browser_close` | `bool` | `false` | Use a browser-session cookie with no fixed expiry. |
| `session_save_every_request` | `bool` | `false` | Save the session and refresh the cookie on every request, not only when modified. |
| `session_serializer` | `String` | `"rango.contrib.sessions.serializers.JSONSerializer"` | Accepted but not used yet. Sessions are always JSON. |

## Messages

| Field | Type | Default | Meaning |
|---|---|---|---|
| `message_storage` | `String` | `"rango.contrib.messages.storage.fallback.FallbackStorage"` | Where messages live between requests: a name ending in `SessionStorage` uses the session only, `CookieStorage` the cookie only, anything else (the default) the cookie with a session fallback. |
| `message_level` | `Option<i64>` | `None` | Minimum level recorded; `None` means `INFO` (20). |
| `message_tags` | `Option<Json>` | `None` | Map from level number to CSS tag, overriding the built-in tags. |

## CSRF

| Field | Type | Default | Meaning |
|---|---|---|---|
| `csrf_cookie_name` | `String` | `"csrftoken"` | Cookie name. |
| `csrf_cookie_age` | `i64` | `31449600` (one year) | Cookie lifetime in seconds. |
| `csrf_cookie_domain` | `Option<String>` | `None` | Cookie domain. |
| `csrf_cookie_path` | `String` | `"/"` | Cookie path. |
| `csrf_cookie_secure` | `bool` | `false` | HTTPS-only cookie (`security.W016`). |
| `csrf_cookie_httponly` | `bool` | `false` | Hide the cookie from scripts. |
| `csrf_cookie_samesite` | `String` | `"Lax"` | `SameSite` attribute. |
| `csrf_header_name` | `String` | `"HTTP_X_CSRFTOKEN"` | Request header carrying the token, in the server-variable spelling (`HTTP_` plus the upper-cased header with dashes as underscores). |
| `csrf_trusted_origins` | `Vec<String>` | `[]` | Origins (with scheme, may start with `*.`) trusted for unsafe cross-origin requests over HTTPS. |
| `csrf_use_sessions` | `bool` | `false` | Keep the secret in the session instead of a cookie. |
| `csrf_failure_view` | `String` | `"rango.views.csrf.csrf_failure"` | Accepted but not used yet. The middleware always renders the built-in 403 page. |

## CORS

Read by `rango.middleware.cors.CorsMiddleware`; see [CORS](../topics/cors.md).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `cors_allowed_origins` | `Vec<String>` | `[]` | Exact origins allowed. |
| `cors_allowed_origin_regexes` | `Vec<String>` | `[]` | Regular expressions matched against the origin. |
| `cors_allow_all_origins` | `bool` | `false` | Allow any origin. |
| `cors_allow_credentials` | `bool` | `false` | Send `Access-Control-Allow-Credentials`. |
| `cors_allow_headers` | `Vec<String>` | `accept, authorization, content-type, user-agent, x-csrftoken, x-requested-with` | Allowed request headers. |
| `cors_allow_methods` | `Vec<String>` | `DELETE, GET, OPTIONS, PATCH, POST, PUT` | Allowed methods. |
| `cors_expose_headers` | `Vec<String>` | `[]` | Response headers visible to scripts. |
| `cors_preflight_max_age` | `i64` | `86400` | Seconds a preflight answer may be cached. |
| `cors_allow_private_network` | `bool` | `false` | Answer private-network preflight requests. |
| `cors_urls_regex` | `String` | `"^.*$"` | Only paths matching this expression get CORS headers. |

## Security headers and redirects

Read by `rango.middleware.security.SecurityMiddleware` and `rango.middleware.clickjacking.XFrameOptionsMiddleware`;
also inspected by `check --deploy`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `secure_ssl_redirect` | `bool` | `false` | Redirect HTTP requests to HTTPS. |
| `secure_ssl_host` | `Option<String>` | `None` | Host used in that redirect; `None` keeps the request host. |
| `secure_redirect_exempt` | `Vec<String>` | `[]` | Path regular expressions not redirected. |
| `secure_hsts_seconds` | `i64` | `0` | `Strict-Transport-Security` max-age; `0` sends no header. |
| `secure_hsts_include_subdomains` | `bool` | `false` | Add `includeSubDomains`. |
| `secure_hsts_preload` | `bool` | `false` | Add `preload`. |
| `secure_content_type_nosniff` | `bool` | `true` | Send `X-Content-Type-Options: nosniff`. |
| `secure_referrer_policy` | `String` | `"same-origin"` | `Referrer-Policy` value (comma-separated policies allowed). |
| `secure_cross_origin_opener_policy` | `String` | `"same-origin"` | `Cross-Origin-Opener-Policy` value. |
| `x_frame_options` | `String` | `"DENY"` | `X-Frame-Options` value. |
| `signed_cookie_legacy_salt_fallback` | `bool` | `true` | Also accept signed cookies made with the older salt. |
| `signing_backend` | `String` | `"rango.core.signing.TimestampSigner"` | Accepted but not used yet. |

## Authentication

| Field | Type | Default | Meaning |
|---|---|---|---|
| `auth_user_model` | `String` | `"auth.User"` | `app_label.ModelName` of the user model. |
| `authentication_backends` | `Vec<String>` | `["rango.contrib.auth.backends.ModelBackend"]` | Backends tried in order by `authenticate`. |
| `auth_password_validators` | `Vec<Json>` | `[]` | Password validators (each `{"NAME": ..., "OPTIONS": ...}`) run by password validation. |
| `password_hashers` | `Vec<String>` | PBKDF2 (SHA-256), PBKDF2 (SHA-1), Argon2, BCrypt (SHA-256), Scrypt | Hashers in order; the first is used for new passwords. Recognised last components: `PBKDF2PasswordHasher`, `PBKDF2SHA1PasswordHasher`, `Argon2PasswordHasher`, `BCryptSHA256PasswordHasher`, `BCryptPasswordHasher`, `ScryptPasswordHasher`, `MD5PasswordHasher`. Others are an `ImproperlyConfigured` error. |
| `password_reset_timeout` | `i64` | `259200` (three days) | Lifetime in seconds of password-reset tokens. |
| `login_url` | `String` | `"/accounts/login/"` | Where `login_required` and friends redirect. |
| `login_redirect_url` | `String` | `"/accounts/profile/"` | Where to go after a successful login without a `next` value. |
| `logout_redirect_url` | `Option<String>` | `None` | Where to go after logout; `None` renders the logged-out page. |
| `site_id` | `Option<i64>` | `None` | Id of the current `Site` for the sites app. |

## REST and OpenAPI

| Field | Type | Default | Meaning |
|---|---|---|---|
| `rest_framework` | `Json` | `{}` | Global options of `rango::rest` (default authentication, permissions, pagination, ...). See [REST quickstart](../topics/rest/quickstart.md). |
| `spectacular_settings` | `Json` | `{}` | Options of the OpenAPI generator (`TITLE`, `VERSION`, ...). See [OpenAPI](../topics/rest/openapi.md). |

## See also

- [Settings](../topics/settings.md)
- [System checks](../topics/checks.md)
- [Management commands reference](commands.md)
- [Middleware reference](middleware.md)
