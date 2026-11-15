# Rango guide

Everything is re-exported by the `rango` crate: `rango::core::signing`, `rango::utils::text`,
`rango::conf::settings()`, and so on.

## Starting a project

```rust
use rango::conf::{self, Settings};

conf::configure(Settings {
    debug: true,
    secret_key: "change-me".into(),
    installed_apps: vec!["polls".into()],
    ..Settings::default()            // every other setting keeps its default
})?;
rango::setup().await?;               // load apps, models, commands
std::process::exit(rango::core::management::execute_from_command_line(std::env::args()).await);
```

`cargo run -p rangoframework --example manage -- help` shows a working management entry point.

## The module map

| Module | What is in it |
|---|---|
| `rango::conf` | `Settings`, `configure`, `settings()`, `override_settings` |
| `rango::apps` | `register_app!` |
| `rango::db::models` | `#[derive(Model)]`, `QuerySet`, `Q`, `F`, `filter!` |
| `rango::db::migrations` | migrations are Rust files; `makemigrations`/`migrate` |
| `rango::db::serializers` | JSON/JSONL fixtures, `dumpdata`/`loaddata` |
| `rango::http` | `HttpRequest`, `HttpResponse`, `QueryDict`, `JsonResponse` |
| `rango::urls` | `path`, `re_path`, `include`, `reverse!`, `register_urlconf!` |
| `rango::views`, `rango::views::generic` | error views, decorators, generic views (`ListView::<M>::new().paginate_by(10).as_view()`) |
| `rango::shortcuts` | `render`, `redirect`, `get_object_or_404` |
| `rango::template` | `Engine`, `Context`/`context!`, `Library`, `TemplateResponse` |
| `rango::forms` | `Form`, `Field`, `Widget`, `FormSet`, `ModelForm`, `#[derive(Form)]` |
| `rango::middleware` | built-in middleware, `register_middleware!`, CORS (`rango.middleware.cors.CorsMiddleware`, `CORS_*` settings) |
| `rango::core::*` | `cache`, `mail`, `files`, `signing`, `paginator`, `validators`, `checks`, `management` |
| `rango::contrib::*` | `auth`, `sessions`, `messages`, `contenttypes`, `staticfiles`, `sites`, `redirects`, `flatpages`, `sitemaps`, `syndication`, `admin` |
| `rango::rest`, `rango::rest::schema` | serializers, viewsets, routers, OpenAPI schema, Swagger UI and Redoc |
| `rango::test` | `Client`, `RequestFactory`, assertions |
| `rango::utils::translation` | `gettext` and friends |

## A model, a form, a view, a URL

```rust
// models.rs
#[derive(Clone, Default, Model)]
#[model(app_label = "polls", str = "question_text")]
pub struct Question {
    #[field(primary_key, auto)] pub id: Option<i64>,
    #[field(max_length = 200)]  pub question_text: String,
    #[field(auto_now_add)]      pub pub_date: DateTime<Utc>,
}

// views.rs
pub async fn detail(req: HttpRequest, id: i64) -> Result<HttpResponse> {
    let q = get_object_or_404(Question::objects().filter(filter!(id = id)), &Db::default_db()).await?;
    render(&req, "polls/detail.html", context! { "question" => model_value(&q) })
}

// urls.rs
register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", ListView::<Question>::new().paginate_by(5).as_view()).name("index"),
    path("<int:id>/", detail).name("detail"),
]);
```

## A REST API

```rust
let mut router = Router::new();                                   // routes + API root
router.register("articles", ModelViewSet::new(ModelSerializer::<Article>::all()?)
    .permissions(vec![Permission::IsAuthenticatedOrReadOnly])
    .pagination(Some(Pagination::page_number(20)))
    .search_fields(&["title", "body"]).ordering_fields(&["created"]));
urlpatterns![
    path("api/", include(router.urls())),
    path("api/schema/", schema::schema_view()),                   // OpenAPI JSON
    path("api/docs/", schema::swagger_view("/api/schema/")),      // Swagger UI
]
```

The REST layer is configured with the `rest_framework`, `spectacular_settings` and `cors_*` settings.

## Rules of thumb

- **Everything that touches the database is `async` and takes the connection explicitly** (`qs.fetch(&db).await?`).
  There is no global connection; use `Db::default_db()` or `Db::using("alias")`.
- **Lazy loading is explicit.** Related rows are fetched with a query on the other model (`Choice::objects().filter(...)`);
  inside templates use `select_related`/`prefetch_related` in the view and pass the data in.
- **Class-based views are builders**, not subclasses: configure with methods and closures, or implement
  `rango::views::generic::View` yourself.
- **Forms are values**: build a `Form` once, call `.bound(data, files)` per request. Anything that needs the
  database (`ModelForm` choices, uniqueness) is `async` (`ModelForm::new(&db, ..).await`, `is_valid(&db).await`).
- **URL parameters bind by position** in view arguments (`async fn detail(req, id: i64)`), generic views read them by name.

## Design notes

| Topic | How Rango does it | Why |
|---|---|---|
| settings | `conf::configure(Settings { .. })` | settings are a typed struct |
| `settings.is_overridden(X)` | "differs from default" | a struct cannot remember assignment |
| `INSTALLED_APPS` | apps self-register with `register_app!`; the setting selects by name | link-time registration via `inventory` |
| management commands | `register_command!(app, name, Type)` | same reason |
| system checks | `register_check!(tags = [..], fn)` | same reason |
| signals | explicit `disconnect(id)`; senders are label strings | ownership instead of weak references |
| `override_settings`, timezone override | async scope functions (`override_settings(f, fut).await`) | task-local; tasks spawned inside don't inherit |
| `TZ` env var | not modified | process-global mutation is unsound in Rust |
| translations | `gettext()` is evaluated when the message is produced | no lazy proxy objects |
| `signing`, cache, sessions | JSON payloads only | portable and safe to load |
| `SIGNING_BACKEND` | ignored (always `TimestampSigner`) | needs a component registry |
| model display text | `#[model(str = "field")]` | used by form choices, the admin and `model_value` |
| managers | `Model::objects()` returns a `QuerySet` | no manager classes |
| foreign keys | `ForeignKey<Question>` holds the id (and a cached instance after `select_related`) | no implicit queries |
| user and session | `req.user()` (`RequestUserExt`), `req.session()` (`RequestSessionExt`) | typed extension traits |
| form fields | `CharField::new().max_length(10)` | builders |
| model forms | `ModelFormOptions` | `fields`, `exclude`, `widgets`, `labels`, `help_texts`, `error_messages` |
| admin | `AdminSite::new("/admin/")` + `site.register::<M>(ModelAdmin::default()...)` | an explicit site value mounted with `include(site.urls())` |

## Data formats

Password hashes use the standard PBKDF2, PBKDF2-SHA1, scrypt, argon2 and bcrypt encodings. Signed values, cookies and
sessions are HMAC-signed JSON keyed by `SECRET_KEY` with `rango.*` salts.
