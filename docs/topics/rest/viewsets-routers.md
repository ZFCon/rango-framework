# Viewsets and routers

A viewset groups the operations on one resource; a router turns viewsets into URL patterns. This page covers
`ModelViewSet` and all of its builder methods, the URLs, methods and route names a `Router` generates, extra actions,
and how to implement the `ViewSet` trait yourself.

```rust
use rango::rest::{ModelSerializer, ModelViewSet, Router};
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};
```

## A model viewset

`ModelViewSet::new(serializer)` takes a [`ModelSerializer`](serializers.md#modelserializer) and provides six
actions. The model must implement `Default`.

```rust
register_urlconf!("blog.urls", || {
    let mut router = Router::new();
    router.register(
        "articles",
        ModelViewSet::new(ModelSerializer::<Article>::all().expect("Article is registered"))
            .search_fields(&["title", "body"])
            .ordering_fields(&["views", "title"]),
    );
    urlpatterns![path("api/", include(router.urls()))]
});
```

## Standard actions

| Action | Method and URL | What it does | Success |
| --- | --- | --- | --- |
| `list` | `GET /articles/` | Filter, search, order and paginate the queryset; render each row. | 200, an array or a pagination envelope |
| `create` | `POST /articles/` | Validate the body, save a new row. | 201, the new object |
| `retrieve` | `GET /articles/{pk}/` | Look up one row. | 200, the object |
| `update` | `PUT /articles/{pk}/` | Validate the body in full, save. | 200, the object |
| `partial_update` | `PATCH /articles/{pk}/` | Validate only the keys sent, save. | 200, the object |
| `destroy` | `DELETE /articles/{pk}/` | Delete the row. | 204, empty body |

`HEAD` is answered like `GET` without a body. `OPTIONS` returns a short description of the endpoint (see
[Function-based API views](api-views.md#options-metadata)).

Failures:

- Validation errors: `400` with one list of messages per field (see
  [error response format](serializers.md#error-response-format)).
- A row that does not exist, or a key that cannot be parsed as the lookup field's type: `404` with
  `{"detail":"No Article matches the given query."}`.
- A method the route does not implement: `405` with `{"detail":"Method \"POST\" not allowed."}`. The `Allow` response
  header always lists the methods of the route, for example `GET, PUT, PATCH, DELETE, HEAD, OPTIONS`.
- Authentication, permission and throttle failures: see [Authentication, permissions and throttling](auth-permissions.md).

Every request goes through the same cycle: authenticate, check permissions, check throttles, parse the body, run the
action, render JSON. The cycle is described in [Function-based API views](api-views.md#the-request-cycle).

## Builder methods

Each method takes the viewset by value and returns it.

| Method | Default | Effect |
| --- | --- | --- |
| `.read_only()` | off | Only `list` and `retrieve` are routed; writes answer `405`. |
| `.queryset(f)` | `M::objects()` | Base queryset, computed per request from the `&ApiRequest`. |
| `.authentication(Vec<Authentication>)` | the `rest_framework` default | Authentication schemes for this viewset. `vec![]` disables authentication. |
| `.permissions(Vec<Permission>)` | the `rest_framework` default | Permissions; all must pass. |
| `.throttles(Vec<Throttle>)` | the `rest_framework` default | Rate limits. |
| `.pagination(Option<Pagination>)` | the `rest_framework` default | `Some(p)` paginates `list`; `None` turns pagination off for this viewset even if a default is configured. |
| `.search_fields(&[...])` | none | Fields searched by `?search=`. |
| `.ordering_fields(&[...])` | none | Enables `?ordering=` and lists the fields clients may order by. |
| `.ordering(&[...])` | none | Default ordering, for example `&["-created"]`. |
| `.filterset_fields(&[...])` | none | Fields that can be filtered by exact value, for example `?published=true`. |
| `.lookup_field(&str)` | `"pk"` | Model field matched against the URL key. |
| `.using(&str)` | `"default"` | Database alias for every query of this viewset. |
| `.description(&str)` | empty | Operation description in the OpenAPI document. |
| `.action(name, detail, methods, handler)` | none | Adds an extra route (see [Extra actions](#extra-actions)). |

Search, ordering, filters and pagination are described in [Pagination and filtering](pagination-filtering.md).

### Restricting the queryset

`.queryset` receives the request and returns the queryset that `list` reads and that single-object lookups go
through. A row outside the queryset is a `404` for `retrieve`, `update`, `partial_update` and `destroy`.

```rust
ModelViewSet::new(ModelSerializer::<Article>::all()?)
    .queryset(|req| {
        let qs = Article::objects();
        if req.query("drafts") == Some("1") { qs } else { qs.filter_kw("published", true) }
    })
```

The function is synchronous. It can read the request (query string, headers, `req.user.is_authenticated()`), but it
cannot `await`; anything that needs a database round trip, such as reading the user's primary key, belongs in a
custom viewset or an [API view](api-views.md).

### Lookups

The detail routes capture one URL segment, always under the name `pk`, and match it against `lookup_field`:

```rust
ModelViewSet::new(ModelSerializer::<Article>::all()?).lookup_field("slug")   // GET /articles/first/
```

When the lookup field is an integer or auto field, the segment is parsed as an integer and a non-numeric segment is
a `404`. Otherwise it is compared as text. The segment cannot contain `/` or `.`.

### Read-only viewsets

```rust
router.register_as(
    "paged",
    ModelViewSet::new(ModelSerializer::<Article>::all()?).read_only().pagination(Some(Pagination::page_number(2))),
    "paged",
);
```

`POST /paged/` answers `405` and `Allow: GET, HEAD, OPTIONS`.

### Another database

`.using("replica")` runs the viewset's queries, including the serializer's uniqueness and related-row checks, on that
alias. Authentication always reads users and tokens from the default database.

## Routers

```rust
let mut router = Router::new();
router.register("articles", articles_viewset);
router.register_as("archive", archive_viewset, "archive");
let patterns = router.urls();   // Vec<UrlEntry>, pass it to include()
```

| Call | Meaning |
| --- | --- |
| `Router::new()` | A router with an API root view at the empty path. |
| `Router::simple()` | The same without the API root view. |
| `router.register(prefix, viewset)` | Route the viewset under `prefix`. The basename is taken from the viewset (the lower-cased model name for `ModelViewSet`). |
| `router.register_as(prefix, viewset, basename)` | The same with an explicit basename. |
| `router.urls()` | The URL patterns. |

`register` and `register_as` return `&mut Router`, so calls can be chained. Leading and trailing slashes of the
prefix are stripped.

### Generated routes

For prefix `articles` and basename `article`:

| URL pattern | Methods | Route name |
| --- | --- | --- |
| `^$` | `GET` | `api-root` (only with `Router::new()`) |
| `^articles/$` | `GET` list, `POST` create | `article-list` |
| `^articles/(?P<pk>[^/.]+)/$` | `GET` retrieve, `PUT` update, `PATCH` partial_update, `DELETE` destroy | `article-detail` |
| `^articles/<action>/$` | the methods of a list action | `article-<action>` |
| `^articles/(?P<pk>[^/.]+)/<action>/$` | the methods of a detail action | `article-<action>` |

Extra-action routes are placed before the detail route, so `articles/recent/` is not captured as a primary key.
In route names, underscores in the action name become hyphens (`mark_done` gives `article-mark-done`); the URL keeps
the action name as written. Every URL ends with a slash.

All routed views are exempt from the CSRF middleware; session-authenticated requests are checked for CSRF by the
REST framework itself (see [session authentication](auth-permissions.md#session-authentication)).

### Reversing

Route names work with `reverse`:

```rust
use rango::reverse;
use rango::urls::Reverse;

let list = reverse("article-list", Reverse::new())?;                      // "/api/articles/"
let detail = reverse("article-detail", Reverse::new().kwarg("pk", 1))?;   // "/api/articles/1/"
```

The paths above assume the router is included under `api/`.

### The API root

`Router::new()` adds a `GET` view that maps each registered prefix to its absolute list URL:

```console
$ curl http://testserver/api/
{"articles":"http://testserver/api/articles/","paged":"http://testserver/api/paged/","private":"http://testserver/api/private/"}
```

The root view uses the project defaults for authentication and permissions. Use `Router::simple()` when you do not
want to expose it.

### `register` or `register_as`

The basename must be unique per router registration: it forms the route names, and it identifies the route for the
OpenAPI generator. Registering the same model twice with `register` gives both registrations the same basename, so
`reverse` finds only one of them and the generated document describes one viewset under both prefixes. Give each
registration its own basename:

```rust
let ser = || ModelSerializer::<Article>::all().unwrap();
router.register("articles", ModelViewSet::new(ser()));
router.register_as("paged", ModelViewSet::new(ser()).read_only(), "paged");
router.register_as("private", ModelViewSet::new(ser()).read_only(), "private");
```

## Extra actions

`.action(name, detail, methods, handler)` adds a route next to the standard ones.

- `detail = false`: the route is `prefix/name/` and the handler receives `None`.
- `detail = true`: the route is `prefix/{pk}/name/`. The object is looked up first (through the viewset's queryset and
  lookup field; `404` if missing) and passed as `Some(object)`.

The handler is an async function of `(ApiRequest, Option<M>)` returning `Result<Response>`.

```rust
use rango::db::Db;
use rango::db::models::Model;
use rango::json;
use rango::rest::{ModelSerializer, ModelViewSet, Response};

let ser = ModelSerializer::<Todo>::all()?;
let for_action = ser.clone();

let viewset = ModelViewSet::new(ser)
    // POST /todos/{pk}/complete/
    .action("complete", true, &["POST"], move |_req, todo| {
        let ser = for_action.clone();
        async move {
            let db = Db::default_db();
            let mut todo = todo.expect("detail actions receive the object");
            todo.done = true;
            todo.save(&db).await?;
            Ok(Response::new(ser.represent(&db, &todo).await?))
        }
    })
    // GET /todos/open_count/
    .action("open_count", false, &["GET"], |_req, _none| async move {
        let db = Db::default_db();
        let n = Todo::objects().filter_kw("done", false).count(&db).await?;
        Ok(Response::new(json!({"open": n})))
    });
```

Things to know:

- All listed methods share one handler; branch on `req.http.method()` if they differ.
- Authentication, permissions and throttles of the viewset apply to extra actions.
- The request body is parsed into `req.data` for `POST`, `PUT` and `PATCH`, but it is not validated; run a serializer
  yourself.
- The handler does not receive the viewset. Clone what it needs (a serializer, configuration) into the closure.
- In the OpenAPI document an extra action appears with its path, parameters and security, without a request body and
  with a response described as "No response body".

## Writing your own viewset

`Router` accepts anything that implements `rango::rest::ViewSet`. Implement it when the resource is not a single
model, or when you need asynchronous logic that `ModelViewSet` has no hook for. The trait uses `async_trait`, so add
the `async-trait` crate to your `Cargo.toml`.

```rust
use async_trait::async_trait;
use rango::json;
use rango::rest::viewsets::{Route, ViewSetSchema};
use rango::rest::{ApiException, ApiRequest, Field, Response, Serializer, ViewConfig, ViewSet};
use rango::urls::ViewArgs;

pub struct StatusViewSet {
    config: ViewConfig,
}

fn status_serializer() -> Serializer {
    Serializer::new("Status").field("name", Field::char()).field("healthy", Field::boolean())
}

#[async_trait]
impl ViewSet for StatusViewSet {
    fn config(&self) -> &ViewConfig {
        &self.config
    }

    fn basename(&self) -> String {
        "status".into()
    }

    fn display_name(&self) -> String {
        "Status".into()
    }

    fn routes(&self) -> Vec<Route> {
        vec![
            Route { detail: false, url_path: None, actions: vec![("GET", "list".into())] },
            Route { detail: true, url_path: None, actions: vec![("GET", "retrieve".into())] },
        ]
    }

    async fn call(&self, action: &str, _request: ApiRequest, args: &ViewArgs) -> rango::Result<Response> {
        match action {
            "list" => Ok(Response::new(json!([{"name": "database", "healthy": true}]))),
            "retrieve" => {
                let name = args.kwargs.get("pk").map(ToString::to_string).unwrap_or_default();
                if name != "database" {
                    return Err(ApiException::not_found(None).into());
                }
                Ok(Response::new(json!({"name": name, "healthy": true})))
            }
            _ => Err(ApiException::not_found(None).into()),
        }
    }

    fn schema(&self) -> ViewSetSchema {
        ViewSetSchema {
            serializer: status_serializer(),
            pagination: None,
            search: false,
            ordering: false,
            filterset_fields: vec![],
            lookup_is_integer: false,
            description: "Service status.".into(),
        }
    }
}
```

Register it like any other viewset: `router.register("status", StatusViewSet { config: ViewConfig::default() })`.

| Trait method | Purpose |
| --- | --- |
| `config()` | Authentication, permissions and throttles (`ViewConfig`; `None` members fall back to the project defaults). |
| `routes()` | The routes to generate. Each `Route` has `detail` (list or detail URL), `url_path` (`None` for the standard routes, `Some(segment)` for an extra one) and `actions`, a list of `(HTTP method, action name)` pairs. |
| `basename()` | Default basename for `Router::register`. |
| `display_name()` | Name used in `OPTIONS` responses (`"Status List"`, `"Status Instance"`, `"Status Action"`). |
| `call(action, request, args)` | Run one action. The URL key is `args.kwargs["pk"]`. |
| `schema()` | What the OpenAPI generator needs: the serializer, pagination, whether search and ordering are enabled, the filter fields, whether the key is an integer, and a description. |

The router passes `call` the action name paired with the request method in the matching route. Errors returned from
`call` are turned into responses the same way as everywhere else in the REST framework.

## Not supported yet

- Per-action configuration on `ModelViewSet` (a different serializer, permission or queryset for one action).
- Asynchronous `queryset` hooks and `perform_create`-style hooks; write a custom viewset or an API view instead.
- Object-level permissions.
- Nested routers and URL keys with a name other than `pk`.
- A `Location` header on `201` responses.

## See also

- [REST quickstart](quickstart.md)
- [Serializers](serializers.md)
- [Function-based API views](api-views.md)
- [Authentication, permissions and throttling](auth-permissions.md)
- [Pagination and filtering](pagination-filtering.md)
- [OpenAPI schema](openapi.md)
- [URL dispatcher](../http/urls.md)
- [Making queries](../db/queries.md)
