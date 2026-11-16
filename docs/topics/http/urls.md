# URL dispatcher

A URLconf maps request paths to views. This page covers declaring URLconfs, route syntax and converters,
including other URLconfs, naming and reversing URLs, how captured values reach a view, and what happens when
nothing matches.

## A first URLconf

A URLconf is a named list of patterns registered with `register_urlconf!`. The `root_urlconf` setting names the
one the request handler starts from.

```rust
// src/polls/urls.rs
use rango::urls::path;
use rango::{register_urlconf, urlpatterns};

use super::views;

register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", views::index).name("index"),
    path("<int:id>/", views::detail).name("detail"),
    path("<int:id>/vote/", views::vote).name("vote"),
]);
```

```rust
// src/urls.rs
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};

register_urlconf!("mysite.urls", || urlpatterns![
    path("polls/", include("polls.urls")),
]);
```

```rust
// src/settings.rs
Settings {
    root_urlconf: Some("mysite.urls".into()),
    ..Settings::default()
}
```

How it fits together:

- `register_urlconf!(name, build)` and `register_urlconf!(name, app_name = "...", build)` register at link time.
  The file only has to be part of the binary (`mod urls;` in `main.rs`); nothing is called at start-up.
- `build` is a function or non-capturing closure returning `Vec<UrlEntry>`. It runs once, the first time the URLconf
  is needed, and the result is cached.
- `urlpatterns![a, b, c]` turns `path(...)`/`re_path(...)` results into that `Vec<UrlEntry>`.
- The name is an arbitrary string. `"<app>.urls"` is the convention used by `rango-admin startapp`.
- If `root_urlconf` is `None`, every request fails with `Error::ImproperlyConfigured` (a 500 response).
- A name that was never registered is not reported as a configuration error while resolving: the resolver finds
  no patterns and every request is a 404. If all URLs of a project or of one `include()` return 404, check that
  the name is spelled exactly as in `register_urlconf!` and that the module is compiled into the binary.

Patterns are tried in order and the first match wins. The path is matched without its leading slash, so
`/polls/5/` is matched against `polls/5/`. The query string is not part of the match.

## Routes and path converters

`path(route, target)` takes a route with `<converter:name>` placeholders. Without a converter (`<name>`) the
`str` converter is used.

| Converter | Matches | Value passed on |
|---|---|---|
| `str` | one or more characters except `/` (`[^/]+`) | `Arg::Str` |
| `int` | one or more digits (`[0-9]+`) | `Arg::Int` (`i64`); a number too large for `i64` does not match |
| `slug` | letters, digits, `-` and `_` (`[-a-zA-Z0-9_]+`) | `Arg::Str` |
| `uuid` | a lowercase, dashed UUID such as `075194d3-6885-417e-a8a8-6c931e272f00` | `Arg::Uuid` |
| `path` | one or more characters including `/` (`.+`) | `Arg::Str` |

```rust
register_urlconf!("blog.urls", || urlpatterns![
    path("articles/<int:year>/", year_archive).name("year"),
    path("articles/<int:year>/<slug:slug>/", article).name("article"),
    path("files/<path:p>", file).name("file"),
    path("u/<uuid:id>/", user).name("user"),
]);
```

Rules for routes:

- A route pointing at a view must match the whole remaining path. A route pointing at `include(...)` only has to
  match a prefix.
- Parameter names must be valid identifiers, and there may be no whitespace inside `<...>`.
- An unknown converter, an invalid parameter name or whitespace in the brackets makes `path()` panic with a
  message naming the route. Because the builder runs lazily, the panic happens when the URLconf is first used,
  not at start-up.

### Custom converters

Implement `PathConverter` and register it under a name before the URLconf is first used (for example at the top of
`main`, before `rango::setup()`).

```rust
use rango::urls::{Arg, PathConverter, register_converter};

struct FourDigitYear;

impl PathConverter for FourDigitYear {
    fn regex(&self) -> &str {
        "[0-9]{4}"
    }
    fn to_clean(&self, value: &str) -> Option<Arg> {
        value.parse().ok().map(Arg::Int)
    }
    fn to_url(&self, value: &str) -> Option<String> {
        Some(format!("{value:0>4}"))
    }
}

register_converter("yyyy", FourDigitYear);
// path("articles/<yyyy:year>/", year_archive)
```

- `regex()` is the fragment the placeholder expands to.
- `to_clean()` converts the matched text. Returning `None` means "this pattern does not match", and the resolver
  moves on to the next pattern.
- `to_url()` is used by `reverse`; it has a default that returns the value unchanged. Returning `None` makes that
  candidate pattern fail to reverse.

A converter produces one of the three `Arg` variants: `Arg::Str(String)`, `Arg::Int(i64)` or `Arg::Uuid(Uuid)`.
`get_converter(name)` looks up a registered converter.

## Regular expression routes

`re_path(regex, target)` matches with a regular expression instead of a route.

```rust
use rango::urls::re_path;

register_urlconf!("legacy.urls", || urlpatterns![
    re_path(r"^re/(?P<a>\d+)/(?P<b>[a-z]+)/$", named).name("re-named"),
    re_path(r"^pos/(\d+)/(\w+)/$", positional).name("re-pos"),
    re_path(r"^opt/(?:(?P<x>\d+)/)?end/$", optional).name("re-opt"),
]);
```

- Named groups (`(?P<name>...)`) are captured as named parameters. Unnamed groups are captured only when the
  pattern has no named groups at all.
- Captured values are always `Arg::Str`. A view parameter of an integer type still works because the string is
  parsed (see [binding](#how-captured-values-reach-the-view)).
- Nothing is anchored for you: write `^` and `$` yourself. Without `$`, a view pattern also matches longer paths;
  without `^`, it matches anywhere in the path.
- `\Z` is accepted and treated as `\z`.
- An invalid regular expression panics when the URLconf is built, like an invalid route.

## Including other URLconfs

`include()` accepts either the name of a registered URLconf or an inline pattern list.

```rust
use rango::urls::{include, path};

fn polls() -> Vec<rango::urls::UrlEntry> {
    urlpatterns![path("", index).name("index"), path("<int:pk>/", detail).name("detail")]
}

register_urlconf!("site.urls", || urlpatterns![
    // by name: app_name (and the default namespace) come from the registration
    path("polls/", include("polls.urls")),
    // inline lists, with an application namespace and two instance namespaces
    path("a/", include(polls()).app_name("polls").namespace("polls")),
    path("b/", include(polls()).app_name("polls").namespace("polls2")),
    // a prefix that captures a parameter
    path("api/<str:version>/", include(urlpatterns![
        path("items/", items).name("items"),
        path("items/<int:id>/", item).name("item"),
    ])),
]);
```

- `include("name")` reads the `app_name` given to `register_urlconf!`. When there is one, the instance namespace
  defaults to the same value.
- `.app_name("x")` sets the application namespace; if no namespace was set yet, it also becomes the instance
  namespace.
- `.namespace("y")` sets the instance namespace. A namespace is only registered for reversing when the include
  also has an app name.
- Parameters captured by the prefix are passed to every included view, ahead of the view's own parameters.
  With the last example, `/api/v2/items/7/` calls `item(req, "v2", 7)`.
- Includes nest to any depth; namespaces are joined with `:` (`"outer:inner:leaf"`).
- Naming an unregistered URLconf makes everything under that prefix a 404 (see the note above).

## Naming patterns and extra arguments

`.name("detail")` names a pattern so it can be reversed. Names only have to be unique within their namespace.

`.kwargs([...])` attaches fixed extra arguments to a pattern. They are appended after the captured parameters:

```rust
use rango::urls::Arg;

path("archive/", listing).kwargs([("page_size", Arg::Int(50))]).name("archive")
// async fn listing(req: HttpRequest, page_size: i64) -> Result<HttpResponse>
```

## How captured values reach the view

Captured values bind to the view's parameters **by position**, in the order they appear in the URL (prefix
parameters first, then the pattern's own, then extra `kwargs`). Rust functions do not expose parameter names, so
the name in the route and the name of the function parameter do not have to agree.

```rust
// path("articles/<int:year>/<slug:slug>/", article)
async fn article(req: HttpRequest, year: i32, slug: String) -> Result<HttpResponse> { /* ... */ }
```

A parameter type must implement `FromArg`. Implementations exist for:

| Type | Accepts |
|---|---|
| `String` | any captured value (integers and UUIDs are formatted) |
| `i64`, `i32`, `i16`, `u64`, `u32`, `u16`, `usize`, `isize` | `Arg::Int` that fits the type, or `Arg::Str` that parses |
| `uuid::Uuid` | `Arg::Uuid`, or `Arg::Str` that parses |

A view takes the request plus zero to six URL parameters. If the URL supplies fewer values than the view declares,
or a value does not convert (for example `70000` into a `u16`), the view fails with
`Error::ImproperlyConfigured("view ...: URL parameter N is missing or has the wrong type")`, which is a 500
response. Choose a parameter type wide enough for the converter.

### Reading parameters by name: `ViewArgs`

The raw captured values are a `ViewArgs`:

```rust
pub struct ViewArgs {
    pub args: Vec<Arg>,                   // unnamed regex groups
    pub kwargs: IndexMap<String, Arg>,    // named parameters, in URL order
}
```

`ViewArgs::positional()` returns the named values if there are any, otherwise the unnamed ones; that list is what
function views bind to. Generic views read `kwargs` by name (`pk`, `slug`, `year`, ...; see
[Generic views](generic-views.md)).

A function view can reach the same data through the `ResolverMatch` the handler attaches to the request:

```rust
use rango::urls::ResolverMatch;

async fn item(req: HttpRequest, _version: String, _id: i64) -> Result<HttpResponse> {
    let m = req.ext::<ResolverMatch>().expect("set by the request handler");
    let version = m.kwargs.get("version").map(ToString::to_string).unwrap_or_default();
    Ok(HttpResponse::new(format!("{} via {}", version, m.view_name())))
}
```

`ResolverMatch` fields: `func`, `args`, `kwargs`, `url_name`, `app_names`, `namespaces`, `route`, `tried`,
`captured_kwargs`, `extra_kwargs`; methods `app_name()`, `namespace()`, `view_name()` and `view_args()`.

To write a view that receives `ViewArgs` directly, build a `rango::urls::View` by hand:

```rust
use rango::urls::View;

let by_name = View::new("by_name", |req, args| Box::pin(async move {
    let pk = args.kwargs.get("pk").map(ToString::to_string).unwrap_or_default();
    Ok(HttpResponse::new(pk))
}));
// path("things/<int:pk>/", by_name)
```

## Reversing URLs

`reverse!` builds the URL for a named pattern. It returns `Result<String>`.

```rust
use rango::reverse;

let home = reverse!("home")?;                         // no arguments
let url = reverse!("polls:detail", 1)?;               // positional
let url = reverse!("polls:detail", id = 1)?;          // by parameter name
let url = reverse!("article", year = 2024, slug = "launch")?;
```

The function form takes a `Reverse` builder and exposes two more options:

```rust
use rango::urls::{Reverse, reverse};

let url = reverse("polls:detail", Reverse::new().args([1]))?;
let url = reverse("polls:detail", Reverse::new().kwarg("id", 1))?;
let url = reverse("polls:index", Reverse::new().current_app("polls2"))?;   // pick an instance namespace
let url = reverse("item", Reverse::new().kwarg("id", 3).urlconf("other.urls"))?;
```

| Builder method | Meaning |
|---|---|
| `Reverse::new()` | no arguments |
| `.args(iter)` | positional values (anything `ToString`) |
| `.kwarg(name, value)` | one named value; call it repeatedly |
| `.urlconf(name)` | reverse against this URLconf instead of `root_urlconf` |
| `.current_app(ns)` | prefer this instance namespace when the name uses an application namespace |

Rules:

- Positional and named values cannot be mixed in one call; that is an error.
- With positional values, the count must equal the number of parameters in the pattern. With named values, the
  set of names must equal the pattern's parameters (extra `kwargs` of the pattern may be given too, but must then
  equal the pattern's value).
- The values must match the converters' regular expressions; `reverse!("detail", "abc")` against `<int:id>` fails.
- The result is percent-encoded and starts with the script prefix (`/` unless the application is mounted under a
  sub-path; see `get_script_prefix()`, `set_script_prefix()` and the `force_script_name` setting).
- A namespaced name is `"namespace:name"`, nested as `"outer:inner:name"`. If the first part is an application
  namespace, the instance is chosen by `current_app`, else the instance named like the application, else the last
  instance declared in the URLconf.

Failures are `Error::NoReverseMatch` with one of these messages:

- `Reverse for 'x' not found. 'x' is not a valid view function or pattern name.`
- `Reverse for 'x' with arguments '[...]' not found. N pattern(s) tried: [...]`
- `'ns' is not a registered namespace` (or `... inside 'outer'`)

In a view, a `NoReverseMatch` that is not handled becomes a 500 response.

`rango::shortcuts::redirect` and `resolve_url` accept pattern names as well; see [Shortcuts](shortcuts.md).

### In templates: `{% url %}`

```html
<a href="{% url 'polls:index' %}">All polls</a>
<a href="{% url 'polls:detail' question.id %}">{{ question }}</a>
<a href="{% url 'article' year=2024 slug=article.slug %}">Read</a>

{% url 'polls:results' question.id as results_url %}
{% if results_url %}<a href="{{ results_url }}">Results</a>{% endif %}
```

The first argument is the pattern name (a quoted string or a variable). Positional and `name=value` arguments
follow the same rules as `reverse`. Without `as`, a pattern that cannot be reversed is a template error. With
`as var`, the URL is stored in `var` instead of being written, and a failed lookup stores an empty string.

## Resolving a path yourself

```rust
use rango::urls::{is_valid_path, resolve};

match resolve("/polls/5/", None)? {
    Ok(m) => println!("{} {:?}", m.view_name(), m.kwargs),
    Err(not_found) => println!("no match for {}; tried {:?}", not_found.path, not_found.tried),
}
assert!(is_valid_path("/polls/", None));
```

`resolve(path, urlconf)` returns `Result<Result<ResolverMatch, Resolver404>>`: the outer error is a configuration
problem, the inner one is "no pattern matched". `Resolver404` carries the unmatched `path` and the patterns
`tried`, and converts into `Error::Http404` with `?`/`.into()`. `get_resolver(None)` returns the cached root
`UrlResolver`; `clear_url_caches()` drops the cache (useful in tests that change `root_urlconf`).

## Trailing slashes

Routes are matched literally: `path("polls/", ...)` matches `/polls/` and not `/polls`. With
`rango.middleware.common.CommonMiddleware` installed and `append_slash: true` (the default), a request that ends
in a 404, has no trailing slash, and would match with one, is answered with a permanent redirect (301) to the
slash-terminated URL.

With `debug: true`, such a redirect for a `POST`, `PUT` or `PATCH` request is replaced by an error explaining that
the request body would be lost; point the form at the URL with the slash. See [Middleware](middleware.md).

## When nothing matches, and error views

If no pattern matches, the handler produces `Error::Http404`. Views produce the same error themselves with
`Err(Error::Http404("...".into()))`. What the client sees:

- `debug: true`: a diagnostic page listing every pattern that was tried, in order. For a project whose root
  URLconf has no patterns, a request for `/` shows the welcome page instead.
- `debug: false`: the 404 error view, which renders the project's `404.html` template if there is one.

The four error views can be replaced process-wide with `set_error_handlers`:

```rust
use std::sync::Arc;

use rango::core::handlers::set_error_handlers;
use rango::http::HttpResponse;

set_error_handlers(|h| {
    h.handler404 = Arc::new(|request, _error| {
        HttpResponse::not_found(format!("Nothing at {}", request.path()))
    });
});
```

| Field | Called for | Default |
|---|---|---|
| `handler400` | `Error::BadRequest`, `Error::Suspicious { .. }` | `rango::views::defaults::bad_request` |
| `handler403` | `Error::PermissionDenied` | `rango::views::defaults::permission_denied` |
| `handler404` | `Error::Http404` | `rango::views::defaults::page_not_found` |
| `handler500` | every other error | `rango::views::defaults::server_error` |

Each handler is an `Arc<dyn Fn(&HttpRequest, &Error) -> HttpResponse + Send + Sync>`. Handlers are synchronous and
must set the status code of the response they return. Call `set_error_handlers` once during start-up, before the
server starts handling requests. With `debug: true` the 400, 404 and 500 cases show the debug pages and the custom
handlers are not called; `handler403` is used in both modes. See [Views](views.md#returning-errors).

## Choosing a URLconf per request

A middleware can make one request resolve against a different URLconf by attaching `RequestUrlconf`:

```rust
use rango::urls::RequestUrlconf;

request.set_ext(RequestUrlconf("partners.urls".into()));
```

This affects resolving (and the `append_slash` check) for that request. It does not change what `reverse` uses;
pass `.urlconf("partners.urls")` to `Reverse`, or wrap the code in `rango::urls::with_urlconf(name, future)`.

## Not supported yet

- Language-prefixed URL patterns (`i18n_patterns`).
- Reversing by view function. Only pattern names can be reversed.
- Declaring error handlers inside a URLconf; use `set_error_handlers`.
- Converters that produce values other than strings, `i64` and UUIDs.

## See also

- [Views](views.md)
- [Generic views](generic-views.md)
- [Shortcuts](shortcuts.md)
- [Middleware](middleware.md)
- [Requests and responses](requests-responses.md)
- [Template language](../templates/language.md)
- [Settings reference](../../ref/settings.md)
