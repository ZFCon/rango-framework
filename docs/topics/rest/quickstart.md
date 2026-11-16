# REST quickstart

This page takes one model and turns it into a JSON CRUD API with pagination, search, filtering, an OpenAPI document
and interactive documentation. Every piece used here has its own page; the links are at the end.

The REST framework lives in `rango::rest`. It is part of the `rangoframework` crate, so there is nothing extra to add
to `Cargo.toml`.

## The model

Start from a project created with `rango-admin startproject todo_api` and an app created with
`cargo run -- startapp todos`. The model is an ordinary Rango model:

```rust
// src/todos/models.rs
use rango::db::chrono::{DateTime, Utc};
use rango::db::models::Model;

#[derive(Clone, Debug, Default, Model)]
#[model(str = "title", ordering = ["-id"])]
pub struct Todo {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 200)]
    pub title: String,
    #[field(text, blank)]
    pub notes: String,
    #[field(default = false)]
    pub done: bool,
    #[field(auto_now_add)]
    pub created: DateTime<Utc>,
}
```

`ModelViewSet` needs the model to implement `Default` (it builds a blank instance before applying validated data),
so keep `Default` in the derive list.

Create the table:

```console
$ cargo run -- makemigrations
$ cargo run -- migrate
```

## Settings

Add the app to `installed_apps`. `"rango.rest"` is the REST framework's own app entry; it has no models and no
migrations, so listing it is optional, but it documents that the project uses the framework.

```rust
// src/settings.rs (excerpt)
use rango::conf::Settings;

pub fn settings() -> Settings {
    Settings {
        installed_apps: [
            "rango.contrib.contenttypes",
            "rango.contrib.auth",
            "rango.rest",
            "todos",
        ]
        .map(String::from)
        .to_vec(),

        // REST framework defaults and the OpenAPI document header.
        rest_framework: rango::json!({}),
        spectacular_settings: rango::json!({ "TITLE": "todo_api API", "VERSION": "0.1.0" }),
        // ... the rest of the generated settings ...
        ..Settings::default()
    }
}
```

`rest_framework` holds project-wide defaults (authentication, permissions, throttles, pagination); an empty object
keeps the built-in defaults. See [Authentication, permissions and throttling](auth-permissions.md#global-defaults).

Two apps are needed only for specific features:

| App | Needed when |
| --- | --- |
| `"rango.contrib.auth"` (and its migrations) | any view authenticates users (Basic, Token or Session) |
| `"rango.rest.authtoken"` | you use token authentication; it owns the `authtoken_token` table |

## Serializer, viewset, router

Three objects make the API:

- a **serializer** validates incoming JSON and renders model instances;
- a **viewset** implements list, create, retrieve, update, partial update and destroy on top of the serializer;
- a **router** turns the viewset into URL patterns.

```rust
// src/urls.rs
use rango::rest::{ModelSerializer, ModelViewSet, Pagination, Router, schema};
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};

use crate::todos::models::Todo;

register_urlconf!("todo_api.urls", || {
    let mut router = Router::new();
    router.register(
        "todos",
        ModelViewSet::new(ModelSerializer::<Todo>::all().expect("Todo is registered"))
            .authentication(vec![])
            .pagination(Some(Pagination::page_number(20)))
            .search_fields(&["title", "notes"])
            .filterset_fields(&["done"])
            .ordering_fields(&["created", "title"])
            .description("Todo items."),
    );
    urlpatterns![
        path("api/", include(router.urls())),
        path("api/schema/", schema::schema_view()).name("schema"),
        path("api/docs/", schema::swagger_view("/api/schema/")).name("swagger-ui"),
        path("api/redoc/", schema::redoc_view("/api/schema/")).name("redoc"),
    ]
});
```

What each line does:

- `ModelSerializer::<Todo>::all()` builds one serializer field per model field. It returns a `Result` because it reads
  the model's metadata from the registry. `id` and `created` come out read-only (auto primary key, `auto_now_add`);
  `notes` and `done` are optional (blank / has a default); `title` is required and limited to 200 characters.
- `.authentication(vec![])` turns authentication off for this viewset. Without it the defaults apply (session, then
  HTTP Basic), which need the auth and session apps.
- `.pagination(Some(Pagination::page_number(20)))` wraps list responses in a `count` / `next` / `previous` / `results`
  envelope, 20 rows per page.
- `.search_fields`, `.filterset_fields` and `.ordering_fields` enable `?search=`, `?done=` and `?ordering=`.
- `router.urls()` returns the URL patterns, including an API root at `/api/`.

## The URLs you get

| Method | URL | Action | Success status |
| --- | --- | --- | --- |
| `GET` | `/api/` | API root: a map of registered prefixes to URLs | 200 |
| `GET` | `/api/todos/` | list | 200 |
| `POST` | `/api/todos/` | create | 201 |
| `GET` | `/api/todos/<id>/` | retrieve | 200 |
| `PUT` | `/api/todos/<id>/` | update (all required fields) | 200 |
| `PATCH` | `/api/todos/<id>/` | partial update | 200 |
| `DELETE` | `/api/todos/<id>/` | destroy | 204 |
| `GET` | `/api/schema/` | OpenAPI 3 document | 200 |
| `GET` | `/api/docs/`, `/api/redoc/` | Swagger UI, Redoc | 200 |

The trailing slash is part of every API URL.

## Try it

```console
$ cargo run -- runserver
```

Create a todo. Fields that are read-only or have defaults can be left out:

```console
$ curl -X POST -H 'Content-Type: application/json' \
       -d '{"title": "Buy milk"}' http://127.0.0.1:8000/api/todos/
{"id":1,"title":"Buy milk","notes":"","done":false,"created":"2026-11-16T09:30:12.345678Z"}
```

The status is `201 Created`. Responses are compact JSON with `Content-Type: application/json` and an `Allow` header
listing the methods of the URL (`GET, POST, HEAD, OPTIONS` here).

List, paginated:

```console
$ curl http://127.0.0.1:8000/api/todos/
{"count":1,"next":null,"previous":null,"results":[{"id":1,"title":"Buy milk","notes":"","done":false,"created":"2026-11-16T09:30:12.345678Z"}]}
```

Filter, search, order and page through the list with query parameters:

```console
$ curl 'http://127.0.0.1:8000/api/todos/?done=false'
$ curl 'http://127.0.0.1:8000/api/todos/?search=milk'
$ curl 'http://127.0.0.1:8000/api/todos/?ordering=-created'
$ curl 'http://127.0.0.1:8000/api/todos/?page=2'
```

Update one field with `PATCH`, or send the whole object with `PUT`:

```console
$ curl -X PATCH -H 'Content-Type: application/json' \
       -d '{"done": true}' http://127.0.0.1:8000/api/todos/1/
{"id":1,"title":"Buy milk","notes":"","done":true,"created":"2026-11-16T09:30:12.345678Z"}
```

Delete it. The response is `204 No Content` with an empty body:

```console
$ curl -i -X DELETE http://127.0.0.1:8000/api/todos/1/
HTTP/1.1 204 No Content
```

## Errors

Invalid input is answered with `400 Bad Request` and one list of messages per field:

```console
$ curl -X POST -H 'Content-Type: application/json' -d '{}' http://127.0.0.1:8000/api/todos/
{"title":["This field is required."]}

$ curl -X POST -H 'Content-Type: application/json' -d '{"title": "", "done": "maybe"}' http://127.0.0.1:8000/api/todos/
{"title":["This field may not be blank."],"done":["Must be a valid boolean."]}
```

Other errors use a `detail` key:

```console
$ curl http://127.0.0.1:8000/api/todos/99/
{"detail":"No Todo matches the given query."}

$ curl 'http://127.0.0.1:8000/api/todos/?page=9'
{"detail":"Invalid page."}
```

Both of these are `404` responses. A method the URL does not implement gets `405` with a body such as
`{"detail":"Method \"PUT\" not allowed."}`.

## Documentation pages

`/api/schema/` serves the OpenAPI 3.0.3 document generated from the router. `/api/docs/` (Swagger UI) and
`/api/redoc/` (Redoc) are HTML pages that load that document. The same document can be written to a file:

```console
$ cargo run -- spectacular --file schema.json
```

The title and version come from the `spectacular_settings` setting. See [OpenAPI schema](openapi.md).

## Calling the API from a browser front end

A front end served from another origin (for example `http://localhost:5173`) needs CORS headers. The generated
project already has `rango.middleware.cors.CorsMiddleware` first in `middleware`; list the origin:

```rust
cors_allowed_origins: vec!["http://localhost:5173".into()],
```

See [CORS](../cors.md).

## Locking it down

The quickstart API is open to everyone. To require a login for writes, keep authentication on and add a permission:

```rust
use rango::rest::{Authentication, Permission};

ModelViewSet::new(ModelSerializer::<Todo>::all()?)
    .authentication(vec![Authentication::Token])
    .permissions(vec![Permission::IsAuthenticatedOrReadOnly])
```

See [Authentication, permissions and throttling](auth-permissions.md).

## See also

- [Serializers](serializers.md)
- [Viewsets and routers](viewsets-routers.md)
- [Function-based API views](api-views.md)
- [Authentication, permissions and throttling](auth-permissions.md)
- [Pagination and filtering](pagination-filtering.md)
- [OpenAPI schema](openapi.md)
- [CORS](../cors.md)
- [Tutorial part 8: a REST API](../../tutorial/08-rest-api.md)
- [Models](../db/models.md)
