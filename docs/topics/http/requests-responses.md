# Requests and responses

Every view receives an `HttpRequest` and returns an `HttpResponse`. This page describes both types, the
`QueryDict` used for query strings and form posts, the ready-made response constructors, and how to set headers
and cookies.

```rust
use rango::Result;
use rango::http::{CookieOptions, FileResponse, HttpRequest, HttpResponse, JsonResponse, QueryDict};
```

## `HttpRequest`

`HttpRequest` is a shared handle to an immutable request: cloning it is cheap, and every clone refers to the same
request. All accessors take `&self`.

### Method, path and URL

| Method | Returns | Meaning |
|---|---|---|
| `method()` | `&str` | upper-case HTTP method (`"GET"`, `"POST"`, ...) |
| `path()` | `&str` | full path: script name plus path info, without the query string |
| `path_info()` | `&str` | the part of the path the URL resolver matches |
| `script_name()` | `&str` | mount prefix (`force_script_name` setting, else what the server reported) |
| `get_full_path()` | `String` | `path()` plus `?query` if there is one |
| `get_full_path_info()` | `String` | `path_info()` plus the query string |
| `get_full_path_with_slash()` | `String` | `get_full_path()` with a trailing slash added to the path |
| `scheme()` | `String` | `"http"` or `"https"` |
| `is_secure()` | `bool` | `scheme() == "https"` |
| `get_host()` | `Result<String>` | validated host as sent in the `Host` header (with its port, if any) |
| `get_port()` | `String` | server port |
| `build_absolute_uri(location)` | `Result<String>` | absolute URL of this request or of `location` |

```rust
async fn where_am_i(request: HttpRequest) -> Result<HttpResponse> {
    let here = request.build_absolute_uri(None)?;               // http://example.com/polls/?page=2
    let other = request.build_absolute_uri(Some("/about/"))?;   // http://example.com/about/
    Ok(HttpResponse::new(format!("{here} -> {other}")))
}
```

`get_host()` checks the host against the `allowed_hosts` setting and fails with a
`Error::Suspicious { kind: DisallowedHost, .. }` (a 400 response) if it is not listed. An entry is an exact host,
a leading-dot pattern such as `".example.com"` (the domain and all subdomains), or `"*"`. When `debug` is `true`
and `allowed_hosts` is empty, `.localhost`, `127.0.0.1` and `[::1]` are allowed. Because `build_absolute_uri`
calls `get_host`, it can fail the same way.

Behind a reverse proxy, three settings change what these methods report:

| Setting | Effect |
|---|---|
| `use_x_forwarded_host` | `get_host()` prefers the `X-Forwarded-Host` header |
| `use_x_forwarded_port` | `get_port()` prefers the `X-Forwarded-Port` header |
| `secure_proxy_ssl_header` | `Some((meta_key, value))`, for example `Some(("HTTP_X_FORWARDED_PROTO".into(), "https".into()))`: the request is secure only when that header has that value |

Only enable them when the proxy sets or strips these headers itself; otherwise clients can spoof them.

### Headers and `META`

```rust
let agent = request.headers().get("User-Agent");          // Option<&str>
let same = request.headers().get("user_agent");           // case-insensitive, `_` equals `-`
let has_auth = request.headers().contains("Authorization");
let raw = request.meta().get("HTTP_USER_AGENT");          // Option<&String>
let ip = request.meta().get("REMOTE_ADDR");
```

`headers()` is a case-insensitive, order-preserving `Headers` map (`get`, `contains`, `iter`, `len`,
`is_empty`). A header sent more than once is joined with commas.

`meta()` is an `IndexMap<String, String>` with the server variables and the raw headers:

| Key | Content |
|---|---|
| `REQUEST_METHOD`, `PATH_INFO`, `SCRIPT_NAME`, `QUERY_STRING` | the request line |
| `REMOTE_ADDR` | client address |
| `SERVER_NAME`, `SERVER_PORT` | address the server listens on |
| `CONTENT_TYPE`, `CONTENT_LENGTH` | body headers |
| `HTTP_<NAME>` | every other header, upper-cased with `-` replaced by `_` (`HTTP_ACCEPT_LANGUAGE`) |

`content_type()` returns the media type without parameters (`"application/json"`), `content_params()` the
parameters (`charset`, `boundary`), and `encoding()` the body charset (the `charset` parameter, else the
`default_charset` setting; `set_encoding()` overrides it).

`accepts("text/html")` tells whether the `Accept` header allows a media type; `accepted_types()` returns the
parsed entries as `MediaType` values (`main_type`, `sub_type`, `params`, `matches()`, `quality()`). A missing
`Accept` header accepts everything.

### Query string: `query()`

```rust
async fn search(request: HttpRequest) -> Result<HttpResponse> {
    let q = request.query().get("q").unwrap_or("");              // ?q=rust
    let page: usize = request.query().get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let tags = request.query().getlist("tag");                   // ?tag=a&tag=b -> ["a", "b"]
    Ok(HttpResponse::new(format!("{q} page {page} tags {tags:?}")))
}
```

`query()` returns a `&QueryDict`. It is available for every method, not only `GET`.

### Form data: `post().await`

```rust
async fn vote(request: HttpRequest, id: i64) -> Result<HttpResponse> {
    let post = request.post().await?;
    let Some(choice) = post.get("choice") else {
        return Ok(HttpResponse::bad_request("You didn't select a choice."));
    };
    Ok(HttpResponse::new(format!("question {id}: {choice}")))
}
```

`post()` is async and returns `Result<&QueryDict>`. The body is parsed on first use and the result is cached, so
calling it again is free.

- It is populated only for `POST` requests whose content type is `application/x-www-form-urlencoded` or
  `multipart/form-data`. For any other method or content type it is empty.
- A malformed multipart body, or one without a boundary, is an `Error::BadRequest` (400).
- For form bodies sent with `PUT` or `PATCH`, parse the body yourself:
  `QueryDict::parse(&String::from_utf8_lossy(request.body()?), false)?`.

Uploaded files of the same request are in `files().await`; see [File uploads](file-uploads.md).

### Raw body

`body()` returns `Result<&Bytes>`. The whole body is already in memory when the view runs.

```rust
#[derive(serde::Deserialize)]
struct NewTodo {
    title: String,
}

async fn create(request: HttpRequest) -> Result<HttpResponse> {
    let todo: NewTodo = serde_json::from_slice(request.body()?)
        .map_err(|e| rango::Error::BadRequest(e.to_string()))?;
    JsonResponse::new(&rango::json!({ "title": todo.title }), true)
}
```

`body()` fails with `Error::Suspicious { kind: RequestDataTooBig, .. }` (400) when the body is larger than
`data_upload_max_memory_size` (default 2621440 bytes; `0` disables the check). Multipart bodies are exempt here
because their parts are limited individually.

For JSON APIs, the REST layer does this parsing for you; see [REST quickstart](../rest/quickstart.md).

### Cookies

```rust
let theme = request.cookies().get("theme").map(String::as_str).unwrap_or("light");
```

`cookies()` is an `IndexMap<String, String>` parsed leniently from the `Cookie` header.

`get_signed_cookie(key, salt, max_age)` reads a cookie written with `set_signed_cookie`. It returns `Ok(None)`
when the cookie is absent, was tampered with, or is older than `max_age`:

```rust
use std::time::Duration;

let user = request.get_signed_cookie("remember", "remember-me", Some(Duration::from_secs(86400)))?;
```

### Per-request state: `set_ext` and `ext`

A request has no settable attributes. Instead, it carries a type-indexed map: one value per Rust type.

```rust
#[derive(Clone)]
struct RequestId(String);

request.set_ext(RequestId("a1b2".into()));                    // usually in a middleware
let id: Option<std::sync::Arc<RequestId>> = request.ext::<RequestId>();
```

`set_ext` takes `&self`, so middleware and views can attach state to a request they only hold a shared handle to.
Setting a value of the same type again replaces it. The framework uses this mechanism itself: the matched
`rango::urls::ResolverMatch`, the session, the current user and the message storage are all extensions, with
typed accessor traits where it matters (`request.session()`, `request.user()`).

### Building a request by hand

`HttpRequest::builder(method, uri)` returns a `RequestBuilder` with `header`, `body`, `scheme`, `server`,
`remote_addr`, `script_name` and `build()`. It is what the development server and the test tools use; in tests,
prefer `rango::test::RequestFactory` and `rango::test::Client` ([Testing](../testing.md)).

```rust
let request = HttpRequest::builder("GET", "/polls/?page=2")
    .header("Accept", "text/html")
    .build()?;
```

The defaults are host `testserver`, port 80, scheme `http` and remote address `127.0.0.1`.

## `QueryDict`

A `QueryDict` maps a key to a **list** of values, because query strings and forms can repeat a key.

| Method | Returns | Meaning |
|---|---|---|
| `get(key)` | `Option<&str>` | the **last** value for the key |
| `getlist(key)` | `&[String]` | every value, empty if the key is absent |
| `contains_key(key)` | `bool` | |
| `keys()` | iterator of `&str` | keys in first-seen order |
| `items()` | iterator of `(&str, &str)` | each key with its last value |
| `lists()` | iterator of `(&str, &[String])` | each key with all its values |
| `len()`, `is_empty()` | | number of distinct keys |
| `urlencode(safe)` | `String` | back to `a=1&a=2&b=x`; `safe` lists characters to leave unescaped |
| `copy()` | `QueryDict` | a mutable copy |
| `is_mutable()` | `bool` | |

The dictionaries attached to a request are immutable. The mutating methods `set`, `setlist`, `appendlist` and
`remove` return an error on an immutable instance; call `copy()` first.

```rust
let mut params = request.query().copy();
params.set("page", "2")?;                 // replaces all values of "page"
params.appendlist("tag", "rust")?;        // adds one more value
let next_url = format!("{}?{}", request.path(), params.urlencode(None));
```

To build one from scratch, use `QueryDict::parse("a=1&a=2", true)?` or
`QueryDict::from_pairs(pairs, true)`; the second argument makes it mutable. `QueryDict::new()` is empty and
immutable.

Parsing decodes `+` as a space and percent-escapes, and gives a key without `=` an empty value. A query string or
form with more parameters than `data_upload_max_number_fields` (default 1000) is rejected with
`Error::Suspicious { kind: TooManyFieldsSent, .. }` (400).

The underlying generic type is `rango::utils::datastructures::MultiValueDict<V>`; `request.files()` is a
`MultiValueDict<UploadedFile>` with the same `get`/`getlist` semantics.

## `HttpResponse`

```rust
let response = HttpResponse::new("<h1>Hello</h1>");                       // 200, text/html; charset=utf-8
let text = HttpResponse::new("plain").content_type("text/plain; charset=utf-8");
let created = HttpResponse::with_status("made", 201)?;
let accepted = HttpResponse::new("queued").status(202)?;
```

`HttpResponse::new` accepts anything convertible into `bytes::Bytes`: a `String`, a `Vec<u8>`, a `&'static str`
or `Bytes`. A borrowed, non-static `&str` has to be turned into a `String` first.

| Member | Meaning |
|---|---|
| `status_code` (field) | numeric status |
| `set_status_code(code)`, `.status(code)` | set it, rejecting values outside 100–599 |
| `reason_phrase()`, `set_reason_phrase(text)` | standard phrase for the status unless overridden |
| `.content_type(value)` | builder: set `Content-Type` |
| `headers` (field) | a `Headers` map: `get`, `contains`, `set`, `set_default`, `remove`, `iter` |
| `set_header(name, value)`, `header(name)`, `has_header(name)` | header access; names are case-insensitive |
| `content()` | `Result<&Bytes>`; an error for streaming and not-yet-rendered template responses |
| `text()` | the content as a `String` |
| `set_content(bytes)`, `write(&[u8])` | replace or append to the body |
| `charset()`, `set_charset(name)` | explicit charset, else the one in `Content-Type`, else `default_charset` |
| `cookies` (field), `set_cookie`, `set_signed_cookie`, `delete_cookie` | see below |
| `is_streaming()` | whether the body is a stream |
| `url()` | the `Location` header of a redirect |
| `on_close(f)`, `close()` | run clean-up code once the response has been handed to the server |
| `flags` (field) | free-form string markers read by middleware (for example `xframe_options_exempt`) |

### Headers

```rust
let mut response = HttpResponse::new("ok");
response.set_header("Cache-Control", "no-store")?;
response.set_header("X-Request-Id", "a1b2")?;
response.headers.set_default("Vary", "Accept");      // only if not already present
response.headers.remove("X-Request-Id");
```

`set_header` fails if the name or value contains a newline, which prevents header injection. The first spelling
used for a header name is kept for output.

`rango::http::cache_utils` has helpers for caching headers: `patch_cache_control`, `patch_vary_headers`,
`patch_response_headers`, `add_never_cache_headers`, `get_max_age`, `has_vary_header` and
`get_conditional_response`. The [view decorators](views.md#view-decorators) are built on them.

### Cookies

```rust
let mut response = HttpResponse::new("saved");
response.set_cookie("theme", "dark", &CookieOptions {
    max_age: Some(60 * 60 * 24 * 365),
    httponly: true,
    samesite: Some("Lax".into()),
    ..CookieOptions::default()
})?;
response.delete_cookie("legacy", "/", None, None)?;
```

`CookieOptions` fields and defaults:

| Field | Default | Meaning |
|---|---|---|
| `max_age: Option<i64>` | `None` | lifetime in seconds; when `expires` is not given it is derived from this |
| `expires: Option<i64>` | `None` | absolute expiry, seconds since the Unix epoch |
| `path: String` | `"/"` | |
| `domain: Option<String>` | `None` | |
| `secure: bool` | `false` | forced to `true` for names starting with `__Secure-` or `__Host-` |
| `httponly: bool` | `false` | |
| `samesite: Option<String>` | `None` | `"Lax"`, `"Strict"` or `"None"` (any case); anything else is an error |

With neither `max_age` nor `expires`, the cookie lasts until the browser closes. `delete_cookie(key, path,
domain, samesite)` sends an already-expired cookie; pass the same path and domain the cookie was set with.

### Signed cookies

A signed cookie cannot be modified by the client without detection. The value is still readable; do not store
secrets in it.

```rust
response.set_signed_cookie("remember", "42", "remember-me", &CookieOptions::default())?;
// later
let value = request.get_signed_cookie("remember", "remember-me", None)?;   // Option<String>
```

The signature is keyed by the `secret_key` setting, the cookie name and the salt. Use the same salt to read the
cookie as was used to write it. See [Signing](../signing.md).

## The response family

There is one response type. The variants are constructors on `HttpResponse` or small helper types that return an
`HttpResponse`.

| Constructor | Status | Notes |
|---|---|---|
| `HttpResponse::new(content)` | 200 | |
| `HttpResponse::with_status(content, code)` | any | `Result`; the code must be 100–599 |
| `HttpResponse::redirect(url)` | 302 | `Result`; sets `Location` |
| `HttpResponse::permanent_redirect(url)` | 301 | `Result`; sets `Location` |
| `HttpResponse::not_modified()` | 304 | no `Content-Type` |
| `HttpResponse::bad_request(content)` | 400 | |
| `HttpResponse::forbidden(content)` | 403 | |
| `HttpResponse::not_found(content)` | 404 | |
| `HttpResponse::not_allowed(&["GET", "POST"], content)` | 405 | sets `Allow` |
| `HttpResponse::gone(content)` | 410 | |
| `HttpResponse::server_error(content)` | 500 | |
| `JsonResponse::new(&data, safe)` | 200 | `Result`; `Content-Type: application/json` |
| `HttpResponse::streaming(stream)` | 200 | body produced lazily |
| `FileResponse::open(path, as_attachment, filename).await` | 200 | streams a file from disk |
| `rango::template::TemplateResponse::new(&request, names, context)` | 200 | rendered after the view returns |

### Redirects

```rust
return HttpResponse::redirect("/polls/");
return HttpResponse::permanent_redirect("https://example.com/new-home/");
```

Only relative URLs and the `http`, `https` and `ftp` schemes are accepted. Anything else (`javascript:`,
`data:`, ...) fails with `Error::Suspicious { kind: DisallowedRedirect, .. }`. To redirect to a named URL pattern,
use `rango::shortcuts::redirect` ([Shortcuts](shortcuts.md)).

There are no dedicated constructors for 303, 307 or 308. Build them directly:

```rust
let mut response = HttpResponse::with_status("", 307)?;
response.set_header("Location", "/retry/")?;
```

### JSON

```rust
#[derive(serde::Serialize)]
struct Todo {
    id: i64,
    title: String,
}

async fn todo(_request: HttpRequest, id: i64) -> Result<HttpResponse> {
    JsonResponse::new(&Todo { id, title: "Write docs".into() }, true)
}

async fn todos(_request: HttpRequest) -> Result<HttpResponse> {
    JsonResponse::new(&vec!["a", "b"], false)       // a top-level array needs safe = false
}
```

`JsonResponse::new` serializes any `serde::Serialize` value, including `rango::json!({...})`. With
`safe = true`, the top-level value must be a JSON object, and anything else is an error; pass `false` to allow
arrays and scalars.

### Streaming

`HttpResponse::streaming` takes a `Stream` of `io::Result<Bytes>` chunks. Use it for large or slowly produced
bodies.

```rust
use bytes::Bytes;
use futures_util::stream;

async fn export(_request: HttpRequest) -> Result<HttpResponse> {
    let rows = (1..=3).map(|i| Ok::<_, std::io::Error>(Bytes::from(format!("row {i}\n"))));
    Ok(HttpResponse::streaming(stream::iter(rows)).content_type("text/csv"))
}
```

The `bytes` and `futures-util` crates used here are ordinary dependencies of your project; `rango` does not
re-export them. The same goes for `serde` and `serde_json` in the JSON examples (only the `json!` macro is
re-exported, as `rango::json!`).

A streaming response has no `Content-Length`, and `content()`/`text()` return an error. Middleware that needs the
whole body treats it specially: no ETag is computed, it is not stored by the cache middleware, and gzip compresses
it chunk by chunk.

### Files

```rust
async fn download(_request: HttpRequest) -> Result<HttpResponse> {
    FileResponse::open("reports/2024.pdf", true, Some("annual-report.pdf"))
        .await
        .map_err(|e| rango::Error::Http404(e.to_string()))
}
```

`FileResponse::open(path, as_attachment, filename)` streams the file in 64 KiB chunks and sets:

- `Content-Length` from the file size;
- `Content-Type` guessed from the file name (`application/octet-stream` when unknown);
- `Content-Encoding` for names ending in `.gz`, `.br`, `.bz2` or `.xz` (the type is then guessed from the name
  without that suffix);
- `Content-Disposition` when `as_attachment` is `true` or a `filename` is given.

It returns `io::Result`, so map the error to the status you want. Never build the path from user input without
validating it; `rango::views::static_serve::serve` does that for a document root
([Views](views.md#serving-files-during-development)).

### Template responses

`TemplateResponse::new` returns an `HttpResponse` whose body is rendered only after the view returned and the
`process_template_response` middleware hooks ran. Until then `content()` is an error. The generic views return
this kind of response. For the common case of rendering immediately, use `rango::shortcuts::render`.

## What the handler changes on the way out

After the middleware chain, the request handler removes the body of responses to `HEAD` requests (keeping
`Content-Length`) and of responses with a 1xx, 204 or 304 status. Views do not need to handle `HEAD` separately.

## Not supported yet

- Streaming request bodies. A request body is fully buffered before the view runs.
- Automatic parsing of form bodies for methods other than `POST`.
- Constructors for 303, 307 and 308 redirects.

## See also

- [Views](views.md)
- [URL dispatcher](urls.md)
- [Shortcuts](shortcuts.md)
- [File uploads](file-uploads.md)
- [Middleware](middleware.md)
- [Sessions](../sessions.md)
- [Signing](../signing.md)
- [Testing](../testing.md)
