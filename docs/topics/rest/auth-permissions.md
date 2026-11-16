# Authentication, permissions and throttling

Three policies decide who may call an API endpoint. **Authentication** works out who is making the request,
**permissions** decide whether that user may proceed, and **throttles** limit how often. This page covers the built-in
schemes, token authentication, custom schemes and checks, and the `rest_framework` setting that provides project-wide
defaults.

```rust
use rango::rest::{Authentication, Permission, Throttle};
```

## Setting the policy

Policies are set per viewset or per API view. Anything left unset falls back to the project default.

```rust
use rango::rest::{ModelSerializer, ModelViewSet, ViewConfig, api_view};

// A viewset
let articles = ModelViewSet::new(ModelSerializer::<Article>::all()?)
    .authentication(vec![Authentication::Token, Authentication::Session])
    .permissions(vec![Permission::IsAuthenticatedOrReadOnly])
    .throttles(vec![Throttle::Anon(Some("100/day".into())), Throttle::User(Some("1000/day".into()))]);

// A function-based view
let config = ViewConfig {
    authentication: Some(vec![Authentication::Basic]),
    permissions: Some(vec![Permission::IsAuthenticated]),
    throttles: None, // project default
};
```

An explicitly empty list is not the same as "unset": `.authentication(vec![])` means no authentication at all, so
every request is anonymous.

## How authentication runs

The schemes are tried in the order listed. Each one has three possible outcomes:

- **Not attempted**: the request carries nothing for this scheme (no session login, a different `Authorization`
  scheme). The next scheme is tried.
- **Success**: the user is set on `ApiRequest::user` and no further scheme is tried.
- **Failure**: the request carries credentials for this scheme and they are wrong. The request ends with an error
  response, even if the endpoint allows anonymous access.

If no scheme succeeds, the request continues as anonymous and the permissions decide what happens.

Users are loaded through `rango::contrib::auth`, so `"rango.contrib.auth"` must be installed and migrated. Users and
tokens are always read from the default database.

### 401 or 403

When a request is refused for lack of valid credentials, the status depends on the **first** scheme in the list:

- If that scheme has a challenge, the response is `401 Unauthorized` with a `WWW-Authenticate` header.
- If it has none (session authentication, or an empty list), the response is `403 Forbidden` with the same body.

| First scheme | `WWW-Authenticate` | Status for missing or bad credentials |
| --- | --- | --- |
| `Authentication::Basic` | `Basic realm="api"` | 401 |
| `Authentication::Token` | `Token` | 401 |
| `Authentication::Session` | none | 403 |
| `Authentication::Custom(..)` | what `authenticate_header()` returns | 401 if it returns a value, else 403 |

A request that is authenticated but fails a permission is always `403`.

Put the scheme your API clients use first when you want them to receive `401`. With the default order (session
first), an anonymous request to a protected endpoint is answered with `403`.

Exchanges against a read-only viewset with `authentication(vec![Authentication::Basic])` and
`permissions(vec![Permission::IsAuthenticated])`:

```console
$ curl -i http://testserver/api/private/
HTTP/1.1 401 Unauthorized
WWW-Authenticate: Basic realm="api"

{"detail":"Authentication credentials were not provided."}

$ curl -i -u ann:nope http://testserver/api/private/
HTTP/1.1 401 Unauthorized
WWW-Authenticate: Basic realm="api"

{"detail":"Invalid username/password."}

$ curl -u ann:pw http://testserver/api/private/2/
{"id":2,"title":"Second","slug":"second","body":"","published":true,"views":70,"rating":null}
```

## Session authentication

`Authentication::Session` accepts the user that the session and authentication middleware attached to the request.
It needs `rango.contrib.sessions.middleware.SessionMiddleware` and
`rango.contrib.auth.middleware.AuthenticationMiddleware` in `middleware`, in that order. It suits browser clients on
the same site that logged in through the regular login view.

- An anonymous or inactive session user is "not attempted"; the next scheme is tried.
- For unsafe methods (everything except `GET`, `HEAD`, `OPTIONS` and `TRACE`) the CSRF check runs. A failure is
  `403` with `{"detail":"CSRF Failed: <reason>"}`, for example `CSRF Failed: CSRF cookie not set.` or
  `CSRF Failed: CSRF token missing.`

Why the check lives here: API views are marked exempt from `CsrfViewMiddleware`, because clients that authenticate
with a token or Basic credentials are not exposed to cross-site request forgery and should not need a CSRF token. A
session cookie is sent by the browser automatically, so requests authenticated that way are checked by the REST
framework itself. A script on your own pages sends the token in the `X-CSRFToken` header as usual.

## Basic authentication

`Authentication::Basic` reads `Authorization: Basic <base64(username:password)>` and checks the credentials with
`rango::contrib::auth::authenticate`, so it honours the configured authentication backends.

| Situation | Response (`401`, or `403` if Basic is not first) |
| --- | --- |
| Header absent or another scheme | not attempted |
| `Basic` with nothing after it | `Invalid basic header. No credentials provided.` |
| More than one token after `Basic` | `Invalid basic header. Credentials string should not contain spaces.` |
| Not valid base64, not UTF-8, or no `:` | `Invalid basic header. Credentials not correctly base64 encoded.` |
| Unknown user or wrong password | `Invalid username/password.` |
| Inactive user | `User inactive or deleted.` |

Use Basic authentication over HTTPS only; the password travels with every request.

## Token authentication

`Authentication::Token` reads `Authorization: Token <key>` and looks the key up in the `authtoken_token` table. On
success `ApiRequest::auth` holds the key.

### Setup

Add the app and migrate. The app's migration depends on the auth app's first migration.

```rust
installed_apps: [
    "rango.contrib.contenttypes",
    "rango.contrib.auth",
    "rango.rest",
    "rango.rest.authtoken",
    // your apps
]
.map(String::from)
.to_vec(),
```

```console
$ cargo run -- migrate
```

### The Token model

`rango::rest::authtoken::Token` is an ordinary model (app label `authtoken`, table `authtoken_token`):

| Field | Type | Notes |
| --- | --- | --- |
| `key` | `String` | Primary key, 40 hexadecimal characters. |
| `user_id` | `i64` | The owner's primary key; unique, so each user has at most one token. |
| `created` | `DateTime<Utc>` | Set when the row is created. |

`user_id` is a plain integer column rather than a foreign key, so there is no join helper and no cascade: delete a
user's token yourself when you delete the user. User models with a non-integer primary key are not supported by
token authentication.

### Issuing tokens

`get_or_create_token(db, user_pk)` returns the user's token, creating it with a random key on first use.

```rust
use rango::contrib::auth;
use rango::db::Db;
use rango::rest::authtoken::get_or_create_token;

let db = Db::default_db();
let user = auth::get_user_model()?.create_user(&db, "ann", "ann@example.com", Some("pw")).await?;
let token = get_or_create_token(&db, user.pk()).await?;
println!("{}", token.key);
```

To let clients trade a username and password for a token, route the `obtain_auth_token` view:

```rust
use rango::rest::authtoken::obtain_auth_token;
use rango::urls::path;

path("api/token/", obtain_auth_token()).name("api-token"),
```

```console
$ curl -X POST -H 'Content-Type: application/json' \
       -d '{"username": "ann", "password": "pw"}' http://127.0.0.1:8000/api/token/
{"token":"9944b09199c62bcf9418ad846dd0e4bbdfc6ee4b"}

$ curl -H 'Authorization: Token 9944b09199c62bcf9418ad846dd0e4bbdfc6ee4b' http://127.0.0.1:8000/api/private/
```

The view accepts `POST` only, with JSON or form data, performs no authentication of its own and allows anyone. It
uses the project's default throttles, so configure a rate for anonymous clients if the endpoint is public. Its error
responses are `400`:

```json
{"username":["This field is required."],"password":["This field is required."]}
```

```json
{"non_field_errors":["Unable to log in with provided credentials."]}
```

### Revoking a token

Delete the row. The next request with that key fails with `Invalid token.`

```rust
use rango::db::models::Model;
use rango::rest::authtoken::Token;

Token::objects().filter_kw("user_id", 7).delete(&db).await?;
```

### Failures

| Situation | Message |
| --- | --- |
| Header absent or another scheme | not attempted |
| `Token` with nothing after it | `Invalid token header. No credentials provided.` |
| More than one value after `Token` | `Invalid token header. Token string should not contain spaces.` |
| Unknown key | `Invalid token.` |
| The owner is inactive or no longer exists | `User inactive or deleted.` |

Tokens do not expire.

## Custom authentication

Implement `rango::rest::auth::Authenticator` and wrap it in `Authentication::Custom`. The trait uses `async_trait`,
so add the `async-trait` crate to your `Cargo.toml`.

```rust
use std::sync::Arc;

use async_trait::async_trait;
use rango::contrib::auth::{self, CurrentUser};
use rango::db::models::Model;
use rango::db::{Db, Value};
use rango::http::HttpRequest;
use rango::rest::auth::Authenticator;
use rango::rest::{ApiException, Authentication};

/// `X-Api-Key: <key>`, looked up in an `ApiKey` model with `key` and `user_id` fields.
pub struct ApiKeyAuth;

#[async_trait]
impl Authenticator for ApiKeyAuth {
    async fn authenticate(&self, request: &HttpRequest) -> rango::Result<Option<(CurrentUser, Option<String>)>> {
        let Some(key) = request.meta().get("HTTP_X_API_KEY").cloned() else {
            return Ok(None); // not attempted
        };
        let db = Db::default_db();
        let row = match ApiKey::objects().filter_kw("key", key.as_str()).get(&db).await {
            Ok(row) => row,
            Err(rango::Error::ObjectDoesNotExist { .. }) => {
                return Err(ApiException::authentication_failed("Invalid API key.").into());
            }
            Err(e) => return Err(e),
        };
        match auth::get_user_model()?.get_by_pk(&db, &Value::Int(row.user_id)).await? {
            Some(user) if user.is_active() => {
                let user = CurrentUser(Some(Arc::new(tokio::sync::Mutex::new(user))));
                Ok(Some((user, Some(key))))
            }
            _ => Err(ApiException::authentication_failed("User inactive or deleted.").into()),
        }
    }

    fn authenticate_header(&self) -> Option<String> {
        Some("ApiKey".into())
    }
}

let scheme = Authentication::Custom(Arc::new(ApiKeyAuth));
```

| Method | Purpose |
| --- | --- |
| `authenticate(&request)` | `Ok(None)` when the request has nothing for this scheme, `Ok(Some((user, auth)))` on success, `Err` to refuse the request. `auth` ends up in `ApiRequest::auth`. |
| `authenticate_header()` | The `WWW-Authenticate` value used when this scheme is first in the list. The default `None` turns `401` into `403`. |
| `openapi()` | `Option<(String, serde_json::Value)>`: a name and a security scheme object for the OpenAPI document. The default `None` leaves the scheme out of the document. |

Request headers are read from `request.meta()` under `HTTP_<NAME>`, upper-cased, with dashes replaced by
underscores. `CurrentUser` wraps the user in an `Arc<tokio::sync::Mutex<..>>`, which is why the example builds one.

## Permissions

Permissions run after authentication, before the body is parsed. Every permission in the list must pass. When one
fails, an authenticated request gets `403` with
`{"detail":"You do not have permission to perform this action."}` and an anonymous one gets
`{"detail":"Authentication credentials were not provided."}` with the status described under
[401 or 403](#401-or-403).

| Variant | Passes when |
| --- | --- |
| `Permission::AllowAny` | always |
| `Permission::IsAuthenticated` | the request has an authenticated user |
| `Permission::IsAdminUser` | the user is active and has `is_staff` set |
| `Permission::IsAuthenticatedOrReadOnly` | the method is `GET`, `HEAD` or `OPTIONS`, or the user is authenticated |
| `Permission::Custom(f)` | your function returns `true` |

`OPTIONS` requests are subject to permissions like any other method.

### Custom permissions

`Permission::Custom` wraps an `Arc` of a synchronous function from `&ApiRequest` to `bool`.

```rust
use std::sync::Arc;

use rango::rest::{ApiRequest, Permission};

/// Writes only with a token; reads for everyone.
fn token_for_writes() -> Permission {
    Permission::Custom(Arc::new(|req: &ApiRequest| {
        matches!(req.http.method(), "GET" | "HEAD" | "OPTIONS") || req.auth.is_some()
    }))
}

/// Only requests from the office network.
fn office_only() -> Permission {
    Permission::Custom(Arc::new(|req: &ApiRequest| {
        req.http.meta().get("REMOTE_ADDR").is_some_and(|ip| ip.starts_with("10.0."))
    }))
}
```

The function sees the request before the body is parsed, so `req.data` is `null` at that point. It cannot `await`:
checks that need the database (group membership, model permissions, ownership of an object) belong in the handler,
where you can return `ApiException::permission_denied(None)`.

```rust
use rango::db::Db;
use rango::rest::{ApiException, ApiRequest, Response};

async fn publish(req: ApiRequest, _args: rango::urls::ViewArgs) -> rango::Result<Response> {
    let db = Db::default_db();
    if !req.user.has_perm(&db, "blog.change_article").await? {
        return Err(ApiException::permission_denied(None).into());
    }
    // ...
    Ok(Response::no_content())
}
```

## Throttling

A throttle counts requests per client over a sliding window and refuses the ones over the limit with `429`:

```console
HTTP/1.1 429 Too Many Requests
Retry-After: 42

{"detail":"Request was throttled. Expected available in 42 seconds."}
```

| Variant | Scope | Applies to | Client identity |
| --- | --- | --- | --- |
| `Throttle::Anon(rate)` | `anon` | anonymous requests only; authenticated requests pass | IP address |
| `Throttle::User(rate)` | `user` | every request | the user's primary key, or the IP address when anonymous |

`rate` is `Some("<number>/<period>")` or `None` to take the rate of the scope from `DEFAULT_THROTTLE_RATES`.
Only the first letter of the period matters: `s` (second), `m` (minute), `h` (hour) or `d` (day), so `"10/s"`,
`"60/min"`, `"1000/hour"` and `"5000/day"` are all valid. A throttle with no rate, or a rate that cannot be parsed,
lets every request through.

```rust
ModelViewSet::new(ModelSerializer::<Article>::all()?)
    .throttles(vec![Throttle::Anon(Some("20/min".into())), Throttle::User(None)])
```

How it works:

- The IP address is the first entry of `X-Forwarded-For` when that header is present, otherwise `REMOTE_ADDR`.
  Behind a proxy, make sure the proxy overwrites `X-Forwarded-For`; a client that can set the header can choose its
  own identity.
- The request history is kept in the default cache (the `caches` setting) under the key
  `throttle_<scope>_<identity>`. The default in-process cache counts per server process; use a shared cache backend
  when you run several processes. See [Cache](../cache.md).
- Counts are per scope, not per view: all views using `Throttle::User` share one budget for a given user.
- Throttles run after permissions, so refused requests do not count.

## Global defaults

The `rest_framework` setting is a JSON object of project-wide defaults. Every key is optional.

```rust
use rango::conf::Settings;

Settings {
    rest_framework: rango::json!({
        "DEFAULT_AUTHENTICATION_CLASSES": ["TokenAuthentication", "SessionAuthentication"],
        "DEFAULT_PERMISSION_CLASSES": ["IsAuthenticatedOrReadOnly"],
        "DEFAULT_THROTTLE_CLASSES": ["AnonRateThrottle", "UserRateThrottle"],
        "DEFAULT_THROTTLE_RATES": { "anon": "100/day", "user": "1000/day" },
        "DEFAULT_PAGINATION_CLASS": "PageNumberPagination",
        "PAGE_SIZE": 20,
    }),
    ..Settings::default()
}
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `DEFAULT_AUTHENTICATION_CLASSES` | list of names | `["SessionAuthentication", "BasicAuthentication"]` | Schemes for views that do not set their own. Names: `SessionAuthentication`, `BasicAuthentication`, `TokenAuthentication`. |
| `DEFAULT_PERMISSION_CLASSES` | list of names | `["AllowAny"]` | Names: `AllowAny`, `IsAuthenticated`, `IsAdminUser`, `IsAuthenticatedOrReadOnly`. |
| `DEFAULT_THROTTLE_CLASSES` | list of names | `[]` | Names: `AnonRateThrottle`, `UserRateThrottle`. |
| `DEFAULT_THROTTLE_RATES` | object | none | Rates by scope: `{"anon": "100/day", "user": "1000/day"}`. Used by throttles created with `None`. |
| `DEFAULT_PAGINATION_CLASS` | name | none | `PageNumberPagination` or `LimitOffsetPagination`. Takes effect only together with `PAGE_SIZE`. |
| `PAGE_SIZE` | positive integer | none | Page size (or default limit) for the default pagination. |
| `SEARCH_PARAM` | string | `"search"` | Query parameter read by search. |
| `ORDERING_PARAM` | string | `"ordering"` | Query parameter read by ordering. |
| `NON_FIELD_ERRORS_KEY` | string | `"non_field_errors"` | Key of the error reported when a request body is not a JSON object. |

Rules for the name lists:

- Only the part after the last dot is looked at, so `"TokenAuthentication"` and
  `"myproject.api.TokenAuthentication"` mean the same thing. The plain names are enough.
- A name that is not in the lists above is dropped without an error. Check spelling carefully: a misspelt
  permission leaves the default list shorter than you intended, and an empty permission list allows every request.
- Custom schemes and custom permissions cannot be named in the setting. Apply them per view, or through a helper
  function that builds your `ViewConfig`.

The setting is read on each request, so `override_settings` in tests affects views that rely on the defaults. Other
keys in the object are ignored.

## Not supported yet

- Object-level permissions and model-permission classes; check in the handler with `CurrentUser::has_perm`.
- Asynchronous custom permissions.
- Scoped throttles and custom throttle classes; only the `anon` and `user` scopes exist.
- Token expiry, multiple tokens per user, and token authentication for user models with non-integer primary keys.
- JWT and OAuth schemes; write an `Authenticator` for them.

## See also

- [REST quickstart](quickstart.md)
- [Function-based API views](api-views.md)
- [Viewsets and routers](viewsets-routers.md)
- [Pagination and filtering](pagination-filtering.md)
- [OpenAPI schema](openapi.md) (security schemes)
- [Authentication overview](../auth/overview.md)
- [Sessions](../sessions.md)
- [Security](../security.md)
- [Cache](../cache.md)
- [CORS](../cors.md)
