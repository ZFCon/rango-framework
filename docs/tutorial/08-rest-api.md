# Part 8: a REST API

This part exposes the polls data as a JSON API with a serializer, a viewset and a router, protects writes with
permissions, publishes interactive OpenAPI documentation, and allows a browser front end on another origin to call
it. It continues from [Part 7](07-admin.md). The same pattern is used end to end in the small `todo_api` demo
project (`/home/ziad/Desktop/Work/todo_api/`), a todo list API whose settings, models, migration and URL
configuration this part borrows from.

## Goal

| Request | Result |
| --- | --- |
| `GET /api/questions/` | a page of questions |
| `POST /api/questions/` | create a question (signed-in users) |
| `GET/PUT/PATCH/DELETE /api/questions/<id>/` | read, replace, change or delete one |
| `GET /api/choices/?question=1` | the choices of question 1 |
| `GET /api/docs/` | Swagger UI for all of the above |

## Turn on the REST app

Add `rango.rest` to `installed_apps` in `src/settings.rs`, next to the apps from Part 7:

```rust
installed_apps: [
    "rango.contrib.contenttypes",
    "rango.contrib.auth",
    "rango.contrib.sessions",
    "rango.contrib.messages",
    "rango.contrib.admin",
    "rango.contrib.staticfiles",
    "rango.rest",
    "polls",
]
.map(String::from)
.to_vec(),
```

The generated settings already contain the two REST-related entries:

```rust
// REST framework defaults (empty: built-in defaults apply).
rest_framework: rango::json!({}),
// Title and version of the OpenAPI document.
spectacular_settings: rango::json!({ "TITLE": "mysite API", "VERSION": "0.1.0" }),
```

`rest_framework` holds project-wide API defaults, for example `DEFAULT_PERMISSION_CLASSES`,
`DEFAULT_AUTHENTICATION_CLASSES`, `DEFAULT_PAGINATION_CLASS` with `PAGE_SIZE`, `SEARCH_PARAM` and `ORDERING_PARAM`.
Left empty, an API accepts session and basic-auth logins, allows everyone, and does not paginate unless a viewset
says so. The second entry sets the `title`, `version` and optional `DESCRIPTION` shown in the OpenAPI document.

## Serializer, viewset and router

A serializer converts model instances to JSON and validates incoming JSON. A viewset provides the list, create,
retrieve, update and delete operations for one model. A router turns viewsets into URL patterns.

For plain CRUD over a model, one expression covers all three: `ModelViewSet::new(ModelSerializer::<Model>::all()?)`.
Register the viewsets on a router in the project URL configuration.

### `src/urls.rs`

```rust
use rango::contrib::admin::AdminSite;
use rango::rest::{Authentication, ModelSerializer, ModelViewSet, Pagination, Permission, Router, schema};
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};

use crate::polls::{self, models::{Choice, Question}};

register_urlconf!("mysite.urls", || {
    let mut router = Router::new();
    router.register(
        "questions",
        ModelViewSet::new(ModelSerializer::<Question>::all().expect("Question is registered"))
            .authentication(vec![Authentication::Session, Authentication::Basic])
            .permissions(vec![Permission::IsAuthenticatedOrReadOnly])
            .pagination(Some(Pagination::page_number(20)))
            .search_fields(&["question_text"])
            .ordering_fields(&["pub_date"])
            .description("Poll questions."),
    );
    router.register(
        "choices",
        ModelViewSet::new(ModelSerializer::<Choice>::all().expect("Choice is registered"))
            .authentication(vec![Authentication::Session, Authentication::Basic])
            .permissions(vec![Permission::IsAuthenticatedOrReadOnly])
            .filterset_fields(&["question"])
            .ordering_fields(&["votes"])
            .description("Answers to a question."),
    );

    let site = AdminSite::new("/admin/");
    polls::admin::register(&site);

    urlpatterns![
        path("admin/", include(site.urls())),
        path("polls/", include("polls.urls")),
        path("api/", include(router.urls())),
        path("api/schema/", schema::schema_view()).name("schema"),
        path("api/docs/", schema::swagger_view("/api/schema/")).name("swagger-ui"),
        path("api/redoc/", schema::redoc_view("/api/schema/")).name("redoc"),
        // ... the static route from Part 6
    ]
});
```

What this gives you:

- `ModelSerializer::<Question>::all()` serializes every field of the model: `id`, `question_text` and `pub_date`.
  The primary key and the server-filled `pub_date` are read-only. `all()` fails if the model is not registered, hence
  the `expect`. When you need only some fields, use `ModelSerializer::<Question>::fields(&["id", "question_text"])`,
  and refine with `.exclude(...)` and `.read_only_fields(...)`.
- A `ForeignKey` such as `Choice::question` appears in the JSON as the id of the related question.
- `Router::register("questions", viewset)` creates `/api/questions/` (list and create) and `/api/questions/<id>/`
  (retrieve, update, partial update, delete). The router also adds an API root view at `/api/`.
- `search_fields` enables `?search=text` over the listed fields; `ordering_fields` enables `?ordering=-pub_date`;
  `filterset_fields` enables exact-match filters such as `?question=1`.
- `pagination(Some(Pagination::page_number(20)))` wraps list responses in
  `{"count": ..., "next": ..., "previous": ..., "results": [...]}` and reads `?page=`. `Pagination::limit_offset(n)`
  is the alternative. Without it, the list is a plain array.
- `.read_only()` on a viewset removes the write operations altogether.

Try it:

```console
$ cargo run -- runserver
$ curl http://127.0.0.1:8000/api/questions/
$ curl -u admin:choose-a-password -X POST -H 'Content-Type: application/json' \
       -d '{"question_text": "Favourite colour?"}' http://127.0.0.1:8000/api/questions/
$ curl 'http://127.0.0.1:8000/api/choices/?question=1&ordering=-votes'
```

## Permissions and authentication

Each viewset has a list of authentication methods and a list of permissions; every request must pass the
permissions.

| `Authentication` | Reads credentials from |
| --- | --- |
| `Session` | the login cookie (CSRF is enforced on unsafe methods) |
| `Basic` | an `Authorization: Basic ...` header |
| `Token` | an `Authorization: Token <key>` header |
| `Custom(...)` | your own authenticator |

| `Permission` | Allows |
| --- | --- |
| `AllowAny` | everyone |
| `IsAuthenticated` | signed-in users |
| `IsAuthenticatedOrReadOnly` | everyone to read; only signed-in users to write |
| `IsAdminUser` | staff users |
| `Custom(...)` | whatever your closure over the `ApiRequest` decides |

With `IsAuthenticatedOrReadOnly` as above, anonymous clients can `GET` the polls but a `POST`, `PUT`, `PATCH` or
`DELETE` is rejected. To require a login for everything, use `IsAuthenticated`; to restrict to staff, `IsAdminUser`.
Set project-wide defaults in `rest_framework`:

```rust
rest_framework: rango::json!({
    "DEFAULT_PERMISSION_CLASSES": ["IsAuthenticatedOrReadOnly"],
}),
```

(only the last dot-separated segment of each name is read, so a dotted path works too). A viewset's own `.permissions(...)` and `.authentication(...)`
override the defaults. The demo `todo_api` project passes `.authentication(vec![])` and so serves its API without
any login at all; do that only for data that is meant to be public.

## Interactive documentation

The three schema routes you already have in `src/urls.rs` document the API automatically from your viewsets:

- `/api/schema/` is the OpenAPI document (JSON);
- `/api/docs/` is Swagger UI, loading that document;
- `/api/redoc/` is ReDoc.

Open `http://127.0.0.1:8000/api/docs/`, expand `POST /api/questions/`, and try it out. The document lists each
operation with an id such as `questions_list`, the query parameters (`search`, `ordering`, `page`, filters), the
request and response schemas derived from the serializer (maximum lengths, required fields, read-only fields), and
the security scheme for the authentication you chose. `.description("...")` on a viewset becomes the description of
its operations. The title and version come from `spectacular_settings`.

## CORS: allowing a browser front end

A page served from `http://localhost:3000` that calls `http://127.0.0.1:8000/api/` is a cross-origin request, which
browsers block unless the API opts in. `CorsMiddleware` is already first in the generated middleware list; it does
nothing until you list origins in `src/settings.rs`:

```rust
cors_allowed_origins: vec!["http://localhost:3000".into()],
```

Then the API answers browser pre-flight requests and adds the `Access-Control-Allow-Origin` header for that origin.
Related settings: `cors_allow_credentials` (cookies and auth headers), `cors_allow_headers`, `cors_allow_methods`,
`cors_allowed_origin_regexes`, `cors_expose_headers`, `cors_preflight_max_age` and `cors_urls_regex` (limit CORS to
some paths, for example `^/api/`).

For local development only, `cors_allow_all_origins: true` allows any origin; the `todo_api` demo sets it with a
"development only" comment. Do not ship it for an API that uses cookies or sensitive data.

## Test the API

The test client works for the API too. Add a check to the test from Part 5 (the `Client` and helpers are already
imported there):

```rust
let r = client.get("/api/questions/").await.unwrap();
assert_eq!(r.status_code, 200);
assert_contains(&r, "Favourite colour?", 200);
assert_eq!(client.get("/api/schema/").await.unwrap().status_code, 200);
```

`Client::post_json(path, &value)` sends a JSON body, and `TestResponse::json::<T>()` parses a JSON response.

## Not supported yet

Check [features](../features.md) for the current status. Limits worth knowing now: write-heavy customization such as nested writable serializers
needs a custom serializer or an `api_view`; see [API views](../topics/rest/api-views.md).

## Recap

- `ModelSerializer`, `ModelViewSet` and `Router` give a full CRUD API for a model in a few lines.
- Authentication and permissions are lists on the viewset, with defaults in the `rest_framework` setting.
- The OpenAPI document and Swagger UI come from the same viewsets, at `/api/schema/` and `/api/docs/`.
- CORS is opt-in through `cors_allowed_origins`.

You have now built a complete application: project and app, models and migrations, views and templates, a voting
form, tests, static files, an admin and an API.

## Where to go next

- Models and queries: [Models](../topics/db/models.md), [Queries](../topics/db/queries.md),
  [Relations](../topics/db/relations.md), [Migrations](../topics/db/migrations.md)
- Requests and views: [URLs](../topics/http/urls.md), [Views](../topics/http/views.md),
  [Middleware](../topics/http/middleware.md), [Generic views](../topics/http/generic-views.md)
- Templates and forms: [Template language](../topics/templates/language.md), [Forms](../topics/forms/forms.md),
  [Model forms](../topics/forms/modelforms.md)
- Users: [Authentication](../topics/auth/overview.md), [Sessions](../topics/sessions.md),
  [Admin](../topics/admin.md)
- REST: [Quickstart](../topics/rest/quickstart.md), [Serializers](../topics/rest/serializers.md),
  [Viewsets and routers](../topics/rest/viewsets-routers.md), [OpenAPI](../topics/rest/openapi.md),
  [CORS](../topics/cors.md)
- Shipping: [Settings](../topics/settings.md), [Security](../topics/security.md),
  [Deployment](../topics/deployment.md), [Testing](../topics/testing.md)
- The full documentation index: [Rango documentation](../index.md)

## See also

- [REST quickstart](../topics/rest/quickstart.md)
- [Authentication and permissions for APIs](../topics/rest/auth-permissions.md)
- [Documentation index](../index.md)
