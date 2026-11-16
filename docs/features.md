# Feature checklist

Legend: ✅ implemented and verified against recorded golden outputs · ☑️ implemented, covered by an end-to-end test ·
🟡 partial (what is missing is listed) · ⏳ not started.

**Golden tests** (goldens live under `crates/*/tests/golden/`):
core 659 cases · l10n formats 178 · URLs 57 · ORM SQL 54 · migration DDL (SQLite) · templates 264 · forms 147 + 14 formsets · password hashes · sitemap and feed XML · CORS 19 · REST 28 requests.
Everything else is covered by end-to-end tests in `crates/*/tests/` and `crates/rango/tests/`.

## Core (`rango::core`, `rango::conf`, `rango::utils`)

| Feature | Module | Status |
|---|---|---|
| `conf` (`global_settings`, `settings`, `override_settings`) | `rango::conf` | ✅ all 163 settings (incl. `ROOT_URLCONF`, `SITE_ID`, `MESSAGE_*`), names + defaults generated |
| `core.exceptions`, `core.signing`, `core.validators`, `core.paginator` | `rango::core::*` | ✅ · 🟡 `SIGNING_BACKEND` ignored; `validate_image_file_extension` list is data-driven |
| `core.checks` + `check --deploy` security checks | `rango::core::checks`, `security_checks` | ☑️ framework ✅; security W/E messages kept in a data file (templates/urls/model checks: 🟡 only those of implemented subsystems) |
| `core.management` (`check`, `diffsettings`, `version`, `startproject`, `startapp`, `runserver`, `makemigrations`, `migrate`, `showmigrations`, `sqlmigrate`, `dumpdata`, `loaddata`, `createsuperuser`, `changepassword`, `collectstatic`, `findstatic`, `flush`, `sqlflush`, `dbshell`, `inspectdb`, `makemessages`, `compilemessages`) | `rango::core::management` | 🟡 missing: `squashmigrations`, `sqlsequencereset`, `testserver`, `shell` (not portable), `createcachetable`, `sendtestemail`, `clearsessions`; `inspectdb` does not guess relationships/unique/indexes (FKs only on SQLite); migrate lacks `--fake/--plan/--run-syncdb` |
| `core.cache` (locmem, filebased, dummy) + per-site cache middleware, `cache_page` | `rango::core::cache`, `rango::middleware::cache` | ☑️ values are JSON, not pickle · 🟡 missing: db and redis backends, per-view `vary_on_*` decorators · `{% cache %}` ✅ (in-process/file backends only) |
| `core.mail` (console, locmem, file, dummy, smtp) | `rango::core::mail` | ☑️ · 🟡 SMTP has no TLS/STARTTLS; no attachments from files helper, no `EmailMultiAlternatives` subclass (use `attach_alternative`) |
| `core.files` (`FileSystemStorage`, `ContentFile`) | `rango::core::files` | ☑️ · 🟡 `ImageField` dimension handling/`width_field`, upload handlers (uploads are buffered in memory); `FileField` columns (`#[field(file, upload_to = "dir")]`) are saved to storage by `ModelForm::save` |
| `core.serializers` (`json`, `jsonl`) | `rango::db::serializers` | ☑️ · 🟡 no `xml`/`yaml`, no natural keys, no streaming |
| `dispatch`, `apps` | `rango::dispatch`, `rango::apps` | ✅ |
| `utils.*` | `rango::utils` | ✅ for crypto, datastructures, dateparse, duration, encoding, html, http, ipv6, safestring, text, timezone, regex_helper · `formats`/`numberformat`/`translation` see the i18n/l10n section |

## HTTP, URLs, views, middleware, test client

| Feature | Module | Status |
|---|---|---|
| `http` (request, `QueryDict`, responses, cookies, uploads) | `rango::http` | ☑️ · 🟡 request bodies are buffered in memory |
| `urls` (`path`, `re_path`, `include`, `reverse`, namespaces, converters) | `rango::urls` | ✅ resolve + reverse (57 cases) |
| `views.defaults`, `debug` (technical 404/500, welcome), `static`, decorators | `rango::views` | ☑️ project `404.html`/`500.html`/`400.html`/`403.html`/`403_csrf.html` honoured |
| `views.generic` | `rango::views::generic` | ☑️ builders instead of inheritance: `View`, `TemplateView`, `RedirectView`, `ListView` (+pagination), `DetailView`, `FormView`, `CreateView`, `UpdateView`, `DeleteView`, date archives · 🟡 no `MultipleObjectMixin` hooks beyond closures, no `get_context_data` override by subclassing, week archive uses `%U` only, `date_list` computed in memory, no `LoginRequiredMixin`/`PermissionRequiredMixin` (use `login_required` wrappers) |
| `middleware` (`common`, `security`, `clickjacking`, `gzip`, `http.ConditionalGet`, `csrf`, `cache`) | `rango::middleware` | ☑️ · `locale`: see i18n |
| `shortcuts` (`render`, `redirect`, `get_object_or_404`, `get_list_or_404`) | `rango::shortcuts` | ☑️ |
| `test` (`Client`, `RequestFactory`, assertions) | `rango::test` | ☑️ · 🟡 no DB-rollback `TestCase`, no `LiveServerTestCase` |
| autoreload (`runserver`) | — | ⏳ documented divergence (use `cargo watch`) |

## ORM and migrations (`rango::db`)

| Feature | Module | Status |
|---|---|---|
| `QuerySet` (filter/exclude/annotate/aggregate/values/order_by/select_related/prefetch_related/union/get_or_create/…), lookups, `Q`/`F`/expressions, SQL compiler | `rango::db::models` | ✅ SQL matches the goldens on SQLite for 54 cases; ☑️ run on SQLite + PostgreSQL |
| Fields: Auto, Boolean, Char, Text, Integer family, Float, Decimal, Date/Time/DateTime/Duration, Email/URL/Slug/UUID, Binary, JSON, GenericIPAddress, File/Image, FK/O2O/M2M | derive(Model) | ☑️ · 🟡 File/Image columns store names only; no `GeneratedField`, `FilePathField` |
| Model inheritance (abstract/proxy/multi-table) | — | ⏳ metadata slots exist, behaviour not implemented |
| Window functions, `Subquery`/`Exists`/`OuterRef` | — | 🟡 `Exists` in filters only; no `Window` |
| Model validation (`full_clean`), constraints with conditions (`CheckConstraint`/partial unique) | — | 🟡 conditions are placeholders in DDL; form-level validation + unique checks exist in `ModelForm` |
| Signals, deletion collector, `atomic`/savepoints/`on_commit`, multi-db | `rango::db` | ☑️ · 🟡 no routers |
| Backends: SQLite, PostgreSQL | `rango::db::backends` | ✅ / ☑️ · MySQL, Oracle ⏳ |
| Migrations (state, operations, schema editor, autodetector, executor, writer; `makemigrations`/`migrate`/`showmigrations`/`sqlmigrate`) | `rango::db::migrations` | ✅ DDL matches the goldens on SQLite (9 statements); migrations are Rust source · 🟡 `squashmigrations`, `--fake/--plan`, `RunCode` reverse, rename detection prompts |
| `post_migrate` signal | `rango::db::migrations::executor::POST_MIGRATE` | ☑️ |

## Templates (`rango::template`)

| Feature | Module | Status |
|---|---|---|
| DTL: all built-in tags, ~60 filters, `date` format engine, inheritance, `include`, custom `Library` (`filter`/`tag`/`simple_tag`/`inclusion_tag`) | `rango::template` | ✅ 264 golden cases |
| Loaders (locmem, dirs, app dirs), `TEMPLATES`, context processors (`request`, `debug`, `static`, `media`, `tz`, `csrf`, `messages`, `auth`), `TemplateResponse` | `rango::template::loader` | ☑️ · 🟡 no cached-loader invalidation, `auth` processor has no `perms` object, no `i18n` processor, Jinja2 backend ⏳ |
| `{% trans %}`/`{% translate %}`, `{% blocktrans %}`/`{% blocktranslate %}` (`with`, `count`, `context`, `trimmed`, `asvar`), `{% language %}`, `get_current_language`, `get_available_languages` | `{% load i18n %}` | ☑️ |
| `{% cache %}` | `{% load cache %}` | ☑️ |
| Divergences | | related managers are not callable from templates (no async DB in render): prefetch in the view; model instances reach templates via `model_value()` |

## Forms (`rango::forms`)

| Feature | Module | Status |
|---|---|---|
| `Form`, fields (all except `FilePathField` UI details), widgets (all), `BoundField`, `ErrorList`, `Media`, renderers, built-in form templates | `rango::forms` | ✅ 147 golden cases (locale formats pinned to settings until M8) |
| `formset_factory`, management form, delete/order, `validate_min/max` | `rango::forms::FormSet` | ✅ 14 golden cases |
| `ModelForm`, `ModelChoiceField`, `modelformset`, `inlineformset` | `ModelForm`, `ModelFormSet` | ☑️ · 🟡 choices are loaded at construction (`.await`), uniqueness checked in `is_valid(&db)`, `limit_choices_to` ignored, `unique_for_date` not checked |
| `derive(Form)` | `#[derive(Form)]` | ☑️ (Rango extension) |

## contrib

| App | Status |
|---|---|
| `contenttypes` | ☑️ model, `get_for_model`, `post_migrate` creation · 🟡 generic relations, `remove_stale_contenttypes` |
| `auth` | ☑️ hashers (✅ standard formats, golden-verified for PBKDF2/PBKDF2-SHA1/scrypt/MD5; argon2 and bcrypt round-trip only), `User`/`Group`/`Permission`, `ModelBackend` + permissions, `authenticate`/`login`/`logout`, session auth hash, middleware (`Authentication`, `LoginRequired`), `login_required`/`permission_required`, forms (login, creation, set/change password), password validators, reset tokens, `LoginView`/`LogoutView`/`PasswordChangeView`, `createsuperuser`/`changepassword` · 🟡 password-reset views/forms, `RemoteUser*`, custom user model needs manual trait impl (`register_user_model`), `perms` in templates, `UserChangeForm`/`AdminPasswordChangeForm`, `auth.urls` bundle |
| `sessions` | ☑️ db, cache, cached_db, file, signed_cookies; signed JSON payloads · 🟡 `clearsessions`, `SESSION_SERIALIZER` fixed to JSON |
| `messages` | ☑️ cookie/session/fallback storage, middleware, context processor · 🟡 `MessageMixin`s, `messages.api` level helpers beyond the five |
| `staticfiles` | ☑️ finders, `collectstatic` (incl. hashed manifest names), `findstatic`, `serve` · 🟡 CSS `url()` rewriting, `--link`, `runserver` static handler wiring, `{% static %}` does not read the manifest |
| `sites`, `redirects`, `flatpages` | ☑️ |
| `humanize` | ☑️ template filters (from the template crate) |
| `sitemaps` | ✅ XML matches the goldens · 🟡 i18n alternates, `ping_google`, `GenericSitemap`/`Sitemap` item helpers (`items()` returns ready URLs) |
| `syndication` | ✅ RSS 2.0 and Atom 1.0 match the goldens byte for byte · 🟡 the `Feed` class/view (`get_object`, `item_*` hooks) — build a `Feed` and return it from a view |
| `admin` | ☑️ `AdminSite`, `ModelAdmin` (`list_display`, `list_filter`, `search_fields`, `ordering`, `list_per_page`, `fields`, `exclude`), login/logout, index, change list (search, filters, sorting, paging), add/change/delete via `ModelForm`, per-model permissions, flash messages, project-overridable templates, Rust-themed CSS from `assets/theme.css` · 🟡 missing: inlines, actions (bulk delete etc.), `LogEntry`/history, date hierarchy, autocomplete/raw-id widgets, `readonly_fields`, `list_editable`, password change/user admin, `admindocs` |
| `postgres` (ArrayField, JSONB lookups, HStore, ranges, search) | ⏳ |
| `gis` | ⏳ |

## API batteries (on by default in `startproject`)

| Feature | Module | Status |
|---|---|---|
| CORS | `rango.middleware.cors.CorsMiddleware` + the `CORS_*` settings | ✅ 19 golden cases (`tests/cors.rs`) · 🟡 `check_request_enabled` signal |
| REST framework | `rango::rest` | ✅ a CRUD API replayed against goldens: 28 requests, identical bodies, status, `Allow`, `WWW-Authenticate` (`tests/rest_parity.rs`). Covers `ModelSerializer` (field mapping, messages, unique validators), `Serializer`/fields, `ModelViewSet`/read-only/extra `action`s, `Router` + API root, `api_view`, Session/Basic/Token authentication (+ `authtoken`), permissions, throttles, PageNumber/LimitOffset pagination, search/ordering/`filterset_fields`, exception handling · 🟡 missing: browsable API and HTML forms, `HyperlinkedModelSerializer`, nested writes, `CursorPagination`, content negotiation beyond JSON (no XML/YAML renderers), model-level and object permissions, versioning, `metadata` detail on `OPTIONS`, `SerializerMethodField` on model instances needs the data map, `depth` |
| OpenAPI schema + docs | `rango::rest::schema` (`schema_view`, `swagger_view`, `redoc_view`, `spectacular` command) | ☑️ OpenAPI 3.0.3 from routers: operation ids, tags, path/query parameters, request bodies, `Patched*`/`Paginated*List` components, security schemes · 🟡 `@extend_schema` overrides, enum naming/postprocessing hooks, polymorphism, YAML output (JSON is served; it is valid YAML), `api_view` functions are not in the document |

## i18n / l10n and other backends

| Item | Status |
|---|---|
| `gettext`/`ngettext`/`pgettext`/`npgettext`, `.po` and `.mo` catalogs (plural-form expressions, contexts, fuzzy skipped), `LOCALE_PATHS` + app `locale/` dirs, per-task language (`override_language`, `activate`), `LocaleMiddleware` (cookie, `Accept-Language`, `Content-Language`), `makemessages`/`compilemessages` | ☑️ · 🟡 no `i18n_patterns`/`set_language` view, no `gettext_lazy` type (call at use), `makemessages` extracts `gettext(...)`-family literals and `{% trans %}` only, no `rango.po` header editing/obsolete entries, no bundled translations of Rango's own messages |
| l10n: `get_format` for all 84 locales (`DATE_FORMAT`, `*_INPUT_FORMATS` with ISO additions, separators, grouping) and `numberformat.format` | ✅ 178 golden cases (`tests/formats.rs`); used by forms (input formats, widget display) and the `date`/`time` template filters · 🟡 `localize`/`unlocalize` filters, locale-aware `floatformat`/`intcomma`, `FORMAT_MODULE_PATH` |
| MySQL/MariaDB backend | ⏳ not implemented |
| Oracle backend | ⏳ not implemented |
| `contrib.gis` (GEOS/GDAL, PostGIS/SpatiaLite) | ⏳ not implemented |
| `contrib.postgres` (ArrayField, JSONB, HStore, ranges, full-text search) | ⏳ not implemented |
