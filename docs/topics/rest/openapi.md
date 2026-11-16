# OpenAPI schema

Rango can describe your REST API as an OpenAPI 3.0.3 document, serve it from a URL, show it in Swagger UI or Redoc, and
write it to a file. This page covers the three views, the `spectacular` command, the settings that are honoured, the
contents of the document and its limits.

```rust
use rango::rest::schema;
use rango::urls::path;
```

## Serving the document and the docs pages

Add the views to your URLconf next to the router:

```rust
register_urlconf!("todo_api.urls", || {
    let mut router = Router::new();
    // ... router.register(...)
    urlpatterns![
        path("api/", include(router.urls())),
        path("api/schema/", schema::schema_view()).name("schema"),
        path("api/docs/", schema::swagger_view("/api/schema/")).name("swagger-ui"),
        path("api/redoc/", schema::redoc_view("/api/schema/")).name("redoc"),
    ]
});
```

| Function | Returns |
| --- | --- |
| `schema::schema_view()` | The OpenAPI document as JSON, with content type `application/vnd.oai.openapi+json`. |
| `schema::swagger_view(schema_url: &str)` | An HTML page with Swagger UI that loads the document from `schema_url`. |
| `schema::redoc_view(schema_url: &str)` | An HTML page with Redoc that loads the document from `schema_url`. |

Things to know:

- `schema_url` is written into the page as given. Pass the path (or full URL) where `schema_view()` is mounted.
- The page title is the `TITLE` setting.
- The two pages load Swagger UI and Redoc from a public CDN, so the browser needs internet access to render them. The document itself does not.
- The document is regenerated on every request from the current URLconf. It is not cached.
- The views perform no authentication or permission checks. Anyone who can reach the URL can read the document. Mount them only where that is acceptable, or leave them out of production URLconfs.
- The document is always JSON (which is also valid YAML). There is no format negotiation.

## The `spectacular` command

The command writes the same document without starting a server. It is provided by the `rango.rest` app, so that app
must be in `installed_apps`.

```console
$ cargo run -- spectacular --file schema.json
$ cargo run -- spectacular > schema.json
```

With `--file PATH` the pretty-printed JSON is written to `PATH`. Without it, the document is printed to standard output.
Generation errors, for example a URLconf that fails to load, end the command with an error message.

The command is useful in CI: generate the file, then give it to a client generator or a diff check.

## Settings

The `spectacular_settings` setting is a JSON object. These keys are read; anything else in the object is ignored.

```rust
use rango::conf::Settings;

Settings {
    spectacular_settings: rango::json!({
        "TITLE": "Blog API",
        "VERSION": "1.4.0",
        "DESCRIPTION": "Articles and comments.",
        "SERVERS": [{ "url": "https://api.example.com" }],
    }),
    ..Settings::default()
}
```

| Key | Default | Used for |
| --- | --- | --- |
| `TITLE` | `""` | `info.title`, and the `<title>` of the Swagger and Redoc pages. |
| `VERSION` | `"0.0.0"` | `info.version`. |
| `DESCRIPTION` | omitted | `info.description`, when it is a non-empty string. |
| `SERVERS` | omitted | The top-level `servers` array, when it is a non-empty array. Copied as given. |

The default is an empty object, which gives an empty title and version `0.0.0`.

## What the document contains

The generator walks the URLconf, finds every URL that came from a `Router`, and describes it from the viewset's
serializer, pagination, filters and authentication.

### Paths and operations

- One path per route, with `{id}` for the key when the lookup is an integer primary key and `{pk}` otherwise. Nested
  `include()` prefixes are included.
- `operationId` is the path segments after the common prefix of all API paths, joined with underscores, plus the action
  (`articles_list`, `articles_partial_update`). For an extra action, the suffix depends on its HTTP method
  (`list`, `retrieve`, `create`, `update`, `partial_update`, `destroy`).
- The first segment after the common prefix is the operation's tag (`articles`). With `api/` as the only shared prefix
  every resource gets its own tag.
- The viewset's `.description(...)` becomes the operation `description`.

### Parameters

| Parameter | Where | When |
| --- | --- | --- |
| each `filterset_fields` entry | query | on `list`; type taken from the serializer field (string if unknown) |
| `ordering` (or `ORDERING_PARAM`) | query | on `list`, when `.ordering_fields` was called |
| pagination parameters (`page`, `page_size`; or `limit`, `offset`) | query | on `list`, when paginated |
| `search` (or `SEARCH_PARAM`) | query | on `list`, when `.search_fields` was called |
| the key | path | on every detail route; integer with a description for integer keys |

### Bodies and responses

| Action | Request body | Response |
| --- | --- | --- |
| `list` | none | `200`: an array of the serializer schema, or `Paginated<Name>List` when paginated |
| `create` | the serializer schema | `201` with the serializer schema |
| `retrieve` | none | `200` with the serializer schema |
| `update` | the serializer schema | `200` with the serializer schema |
| `partial_update` | `Patched<Name>` (no required fields) | `200` with the serializer schema |
| `destroy` | none | `204`, "No response body" |
| extra action | none | `200`, "No response body" |

Request bodies are offered as `application/json`, `application/x-www-form-urlencoded` and `multipart/form-data`.
`Paginated<Name>List` is an object with `count`, `next`, `previous` and `results`.

### Component schemas

Each serializer becomes `components.schemas.<SerializerName>`. A field maps to a type like this:

| Serializer field | Schema |
| --- | --- |
| boolean | `boolean` |
| char | `string`, with `minLength`, `maxLength`, `format` (for email and similar) or a pattern (slug) |
| integer | `integer`, with `minimum`, `maximum`; `format: int64` above the 32-bit range |
| float | `number` with `format: double` |
| decimal | `string` with `format: decimal` and a pattern from the digits |
| date-time, date, time | `string` with `format` `date-time`, `date`, `time` |
| duration | `string` |
| choice | `enum` with the values, `integer` or `string` |
| list | `array` of the child schema |
| related key | `integer`; an array of integers for many-valued relations |
| nested serializer | a `$ref` to its component (an array of refs when many) |
| JSON, method fields | no type |

Flags add `readOnly`, `writeOnly`, `nullable`, `default` and `description` (from help text). `required` lists the fields
a client must send, plus every read-only field. Nested serializers are added as their own components.

### Security

Every operation lists the authentication schemes of its viewset under `security`, and the schemes are defined in
`components.securitySchemes`:

| Authentication | Scheme name | Definition |
| --- | --- | --- |
| session | `cookieAuth` | API key in the cookie named by `session_cookie_name` |
| basic | `basicAuth` | HTTP basic |
| token | `tokenAuth` | API key in the `Authorization` header, with the `Token` prefix noted |
| custom | whatever `Authenticator::openapi()` returns | none by default |

When the viewset's permissions allow anonymous access for that method, an empty requirement `{}` is added, which marks
the operation as usable without credentials.

## Checking it

```console
$ curl -s http://localhost:8000/api/schema/ | head -c 300
$ cargo run -- spectacular | grep operationId
```

## Limits

- Only routes created through `Router` are described. Functions registered with `api_view`, plain views and everything else in the URLconf do not appear.
- Error responses (`400`, `401`, `403`, `404`) are not described; only the success status of each action.
- Extra actions have no request body and no typed response.
- Two registrations of the same model need distinct basenames, or the document describes one of them twice (see [register or register_as](viewsets-routers.md#register-or-register_as)).
- Viewsets written by hand supply their own description through `ViewSet::schema()`.
- Only the four `spectacular_settings` keys above are honoured. There are no post-processing hooks, component splitting options or per-view overrides.
- The output is OpenAPI 3.0.3, not 3.1.

## See also

- [Viewsets and routers](viewsets-routers.md)
- [Serializers](serializers.md)
- [Pagination and filtering](pagination-filtering.md)
- [Authentication, permissions and throttling](auth-permissions.md)
- [REST quickstart](quickstart.md)
- [Management commands](../../ref/commands.md)
- [Settings](../../ref/settings.md)
