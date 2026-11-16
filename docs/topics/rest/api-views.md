# Function-based API views

Not every endpoint is CRUD on a model. `api_view` wraps an async function in the same request cycle that viewsets
use: authentication, permissions, throttling, body parsing, error handling and JSON rendering. This page covers
`api_view`, `ApiRequest`, `Response`, `ViewConfig`, `ApiException` and the status constants.

```rust
use rango::json;
use rango::rest::{ApiException, ApiRequest, Response, ViewConfig, api_view, status};
use rango::urls::{ViewArgs, path};
use rango::{register_urlconf, urlpatterns};
```

## A first view

`api_view(methods, config, handler)` returns a `rango::urls::View` that goes straight into `path(...)`.

```rust
async fn ping(req: ApiRequest, _args: ViewArgs) -> rango::Result<Response> {
    Ok(Response::new(json!({
        "pong": true,
        "name": req.query("name").unwrap_or("stranger"),
    })))
}

async fn echo(req: ApiRequest, _args: ViewArgs) -> rango::Result<Response> {
    match req.http.method() {
        "POST" => Ok(Response::created(req.data.clone())),
        _ => Ok(Response::new(json!({"send": "POST me some JSON"}))),
    }
}

register_urlconf!("api.urls", || urlpatterns![
    path("api/ping/", api_view(&["GET"], ViewConfig::default(), ping)).name("ping"),
    path("api/echo/", api_view(&["GET", "POST"], ViewConfig::default(), echo)).name("echo"),
]);
```

```console
$ curl 'http://127.0.0.1:8000/api/ping/?name=ann'
{"pong":true,"name":"ann"}

$ curl -X POST -H 'Content-Type: application/json' -d '{"a": 1}' http://127.0.0.1:8000/api/echo/
{"a":1}
```

- `methods` lists the HTTP methods the handler implements. `HEAD` is added when `GET` is listed, and `OPTIONS` is
  always available. Anything else is answered with `405`.
- The handler takes `(ApiRequest, ViewArgs)` and returns `rango::Result<Response>`. It must be `Clone`, which plain
  `async fn` items and closures that capture only cloneable values are.
- The view is exempt from the CSRF middleware (see [session authentication](auth-permissions.md#session-authentication)
  for how session logins are still protected).

### URL parameters

Captured path parameters arrive in `ViewArgs`. `kwargs` is a map from parameter name to `rango::urls::Arg`
(`Str`, `Int` or `Uuid`), which implements `Display`.

```rust
use rango::urls::Arg;

async fn square(_req: ApiRequest, args: ViewArgs) -> rango::Result<Response> {
    let Some(Arg::Int(n)) = args.kwargs.get("n") else {
        return Err(ApiException::not_found(None).into());
    };
    Ok(Response::new(json!({"n": n, "square": n * n})))
}

// path("api/square/<int:n>/", api_view(&["GET"], ViewConfig::default(), square))
```

## The request cycle

For every request to an API view or a routed viewset, in this order:

1. **Authenticate.** Each configured scheme is tried in turn; the first that recognises the request sets the user.
   A scheme that recognises the request but rejects the credentials ends the request with an error.
2. **Check permissions.** If one fails, the response is `401` for an anonymous request and `403` for an
   authenticated one.
3. **Check throttles.** Over the limit is a `429`.
4. **`OPTIONS`** is answered with metadata; the handler is not called.
5. **Check the method.** Not in the allowed list is a `405`.
6. **Parse the body** for `POST`, `PUT` and `PATCH`.
7. **Call the handler.**
8. **Render.** An `Ok(Response)` is serialised as JSON; an `Err` is converted as described under
   [Errors](#errors).

Permissions run before the method check, so a client without access sees `401`/`403` rather than `405`.

Every rendered response carries `Allow` (the view's methods) and `Vary: Accept`. Responses with data have
`Content-Type: application/json`. A `HEAD` response has the headers of the `GET` response and no body.

## ApiRequest

| Member | Type | Meaning |
| --- | --- | --- |
| `http` | `rango::http::HttpRequest` | The underlying request: method, path, headers, cookies, query string. |
| `data` | `serde_json::Value` | The parsed body for `POST`, `PUT` and `PATCH`; `null` for other methods. |
| `user` | `rango::contrib::auth::CurrentUser` | The authenticated user, or the anonymous user. |
| `auth` | `Option<String>` | The credential that authenticated the request: the token key for token authentication, `None` otherwise. |
| `query(name)` | `Option<&str>` | One query-string parameter. Use `http.query()` for repeated parameters. |

Useful methods on `user`: `is_authenticated()`, `is_anonymous()`, and the async `username().await`,
`pk().await`, `is_staff().await`, `is_superuser().await`.

```rust
async fn whoami(req: ApiRequest, _args: ViewArgs) -> rango::Result<Response> {
    Ok(Response::new(json!({
        "authenticated": req.user.is_authenticated(),
        "username": req.user.username().await,
        "token_used": req.auth.is_some(),
    })))
}
```

## Body parsing

The body is parsed according to the request's `Content-Type`:

| Content type | `data` |
| --- | --- |
| `application/json` | The decoded JSON value. An empty body is `{}`. |
| `application/x-www-form-urlencoded` | An object of strings. A key that appears several times becomes an array of strings. |
| `multipart/form-data` | The text parts, in the same shape as a form. |
| anything else, empty body | `{}` |
| anything else, non-empty body | `415` |

Details and limits:

- Malformed JSON is a `400` whose `detail` starts with `JSON parse error - `.
- Form values are always strings (`"7"`, `"true"`). Serializer fields accept those forms, so an integer or boolean
  field validates them without extra work.
- Uploaded files are not part of `data`. Read them with `req.http.files().await?`
  (see [File uploads](../http/file-uploads.md)).
- Form and multipart bodies are decoded for `POST` requests only. On `PUT` and `PATCH` they produce an empty object;
  send JSON for those methods.
- Bodies of `GET` and `DELETE` requests are not parsed.
- The raw bytes are always available through `req.http.body()?`.

## Response

A `Response` is JSON data, a status and extra headers.

| Constructor / method | Result |
| --- | --- |
| `Response::new(data)` | `200` with `data` as the body. |
| `Response::created(data)` | `201` with `data` as the body. |
| `Response::no_content()` | `204`, empty body, no `Content-Type`. |
| `.status(code)` | Set the status code. |
| `.header(name, value)` | Add a response header. May be called several times. |

```rust
let accepted = Response::new(json!({"job": 42}))
    .status(status::HTTP_202_ACCEPTED)
    .header("Location", "/api/jobs/42/");
```

The members `data: Option<Value>`, `status: u16` and `headers: Vec<(String, String)>` are public. Output is compact
UTF-8 JSON; keys keep the order in which you built the object. JSON is the only output format.

## ViewConfig

`ViewConfig` carries the per-view policy. Each member is an `Option`; `None` means "use the project default from the
`rest_framework` setting".

```rust
use rango::rest::{Authentication, Permission, Throttle};

let config = ViewConfig {
    authentication: Some(vec![Authentication::Token]),
    permissions: Some(vec![Permission::IsAuthenticated]),
    throttles: Some(vec![Throttle::User(Some("60/min".into()))]),
};
let view = api_view(&["GET"], config, whoami);
```

| Member | Type | `None` means |
| --- | --- | --- |
| `authentication` | `Option<Vec<Authentication>>` | Session, then Basic, unless configured otherwise |
| `permissions` | `Option<Vec<Permission>>` | `AllowAny` unless configured otherwise |
| `throttles` | `Option<Vec<Throttle>>` | No throttling unless configured otherwise |

`ViewConfig::default()` leaves all three as `None`. `Some(vec![])` is different from `None`: it means "none at all"
(no authentication, no permission checks, no throttles). See
[Authentication, permissions and throttling](auth-permissions.md).

## Errors

### ApiException

Return an `ApiException` (converted with `.into()` or `?`) to end the request with a specific response.

| Constructor | Status | Body |
| --- | --- | --- |
| `ApiException::validation(errors)` | 400 | `errors`, as given |
| `ApiException::parse_error(msg)` | 400 | `{"detail": msg}` |
| `ApiException::authentication_failed(msg)` | 401 | `{"detail": msg}` |
| `ApiException::not_authenticated()` | 401 | `{"detail":"Authentication credentials were not provided."}` |
| `ApiException::permission_denied(None)` | 403 | `{"detail":"You do not have permission to perform this action."}` |
| `ApiException::permission_denied(Some(msg))` | 403 | `{"detail": msg}` |
| `ApiException::not_found(None)` | 404 | `{"detail":"Not found."}` |
| `ApiException::not_found(Some(msg))` | 404 | `{"detail": msg}` |
| `ApiException::method_not_allowed(method)` | 405 | `{"detail":"Method \"DELETE\" not allowed."}` |
| `ApiException::not_acceptable()` | 406 | `{"detail":"Could not satisfy the request Accept header."}` |
| `ApiException::unsupported_media_type(media_type)` | 415 | `{"detail":"Unsupported media type \"text/plain\" in request."}` |
| `ApiException::throttled(Some(seconds))` | 429 | `{"detail":"Request was throttled. Expected available in 30 seconds."}` plus `Retry-After: 30` |
| `ApiException::throttled(None)` | 429 | `{"detail":"Request was throttled."}` |

```rust
async fn transfer(req: ApiRequest, _args: ViewArgs) -> rango::Result<Response> {
    let Some(amount) = req.data.get("amount").and_then(|v| v.as_i64()) else {
        return Err(ApiException::validation(json!({"amount": ["A valid integer is required."]})).into());
    };
    if amount > 1000 {
        return Err(ApiException::permission_denied(Some("Transfers above 1000 need approval.")).into());
    }
    Ok(Response::new(json!({"transferred": amount})))
}
```

`ApiException` is a plain struct with public members, for statuses that have no constructor:

```rust
let conflict = ApiException {
    status: status::HTTP_409_CONFLICT,
    body: json!({"detail": "Already exists."}),
    code: "conflict",
    headers: vec![],
};
```

`code` is a short machine name kept on the value; it is not sent to the client. `headers` are added to the response.

A `401` is special: it is sent with a `WWW-Authenticate` header taken from the view's first authentication scheme,
and it is downgraded to `403` when that scheme has no challenge to offer. See
[401 or 403](auth-permissions.md#401-or-403).

### Other errors

A handler returns `rango::Result`, so `?` works on database calls and everything else. Errors that are not
`ApiException` are mapped like this:

| Error | Response |
| --- | --- |
| `rango::Error::Http404(msg)` | 404, `{"detail": msg}` (`"Not found."` when the message is empty) |
| `rango::Error::ObjectDoesNotExist { .. }` (for example from `QuerySet::get`) | 404, `{"detail":"Not found."}` |
| `rango::Error::PermissionDenied(msg)` | 403, `{"detail": msg}` (the default message when empty) |
| anything else | Handled by Rango's normal error machinery: the debug page when `debug` is on, otherwise the project's 400/500 handler. This response is not JSON. |

So `rango::shortcuts::get_object_or_404` and a bare `.get(&db).await?` both produce a JSON 404 inside an API view.

## OPTIONS metadata

An `OPTIONS` request that passes authentication, permissions and throttles gets a fixed description of the endpoint:

```json
{"name":"Api","description":"","renders":["application/json"],"parses":["application/json","application/x-www-form-urlencoded","multipart/form-data"]}
```

`name` is `"Api"` for `api_view` functions, `"Api Root"` for a router's root view, and `"<Model> List"`,
`"<Model> Instance"` or `"<Model> Action"` for viewset routes. The `Allow` header lists the methods. The response
does not describe the fields the endpoint accepts.

If the CORS middleware is installed, browser preflight requests (an `OPTIONS` request with an
`Access-Control-Request-Method` header) are answered by the middleware and never reach the view. See
[CORS](../cors.md#preflight-requests).

## Status constants

`rango::rest::status` holds named `u16` constants and three helpers.

| Constant | Value |
| --- | --- |
| `HTTP_200_OK` | 200 |
| `HTTP_201_CREATED` | 201 |
| `HTTP_202_ACCEPTED` | 202 |
| `HTTP_204_NO_CONTENT` | 204 |
| `HTTP_400_BAD_REQUEST` | 400 |
| `HTTP_401_UNAUTHORIZED` | 401 |
| `HTTP_403_FORBIDDEN` | 403 |
| `HTTP_404_NOT_FOUND` | 404 |
| `HTTP_405_METHOD_NOT_ALLOWED` | 405 |
| `HTTP_409_CONFLICT` | 409 |
| `HTTP_415_UNSUPPORTED_MEDIA_TYPE` | 415 |
| `HTTP_429_TOO_MANY_REQUESTS` | 429 |
| `HTTP_500_INTERNAL_SERVER_ERROR` | 500 |

`status::is_success(code)`, `status::is_client_error(code)` and `status::is_server_error(code)` test for the 2xx,
4xx and 5xx ranges. Any other status is written as a number: `Response::new(data).status(418)`.

## Using the cycle directly

`api_view` is a thin wrapper around `rango::rest::views::dispatch(&config, http, allowed, name, handler)`, which
takes an `HttpRequest` and returns an `HttpResponse`. Call it from an ordinary Rango view when you need the cycle
inside a view you build yourself; `name` is the string reported by `OPTIONS`. `rango::rest::views::parse_body` and
`rango::rest::views::render` are the body parser and the JSON renderer on their own.

## Not supported yet

- API views are not included in the generated OpenAPI document; only routes registered through a `Router` are.
- Output formats other than JSON, and content negotiation on the `Accept` header.
- A custom exception handler; the mapping above is fixed.
- Field-level metadata in `OPTIONS` responses.

## See also

- [REST quickstart](quickstart.md)
- [Serializers](serializers.md)
- [Viewsets and routers](viewsets-routers.md)
- [Authentication, permissions and throttling](auth-permissions.md)
- [OpenAPI schema](openapi.md)
- [Requests and responses](../http/requests-responses.md)
- [Views](../http/views.md)
- [File uploads](../http/file-uploads.md)
