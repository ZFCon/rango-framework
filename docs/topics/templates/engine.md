# The template engine

This page is for Rust developers. It covers how templates are configured, found, compiled and rendered: the
`templates` setting, the loader functions, `Engine`, `Context`, the `Value` type, context processors,
`TemplateResponse`, safe strings and errors. The template syntax itself is described in
[the Rango template language](language.md).

Everything lives in `rango::template`; the macros are exported at the crate root.

```rust
use rango::template::{Context, Engine, Object, TemplateObject, TemplateResponse, Value};
use rango::template::{get_template, render_to_string, select_template, safe, to_value};
use rango::{context, register_context_processor, register_library};
use rango::shortcuts::render;
```

## Rendering from a view

Most views need one call. `render` loads a template, renders it with the request bound (so context processors and
`{% csrf_token %}` work) and wraps the result in a 200 response:

```rust
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::render;
use rango::{Result, context};

async fn index(req: HttpRequest) -> Result<HttpResponse> {
    render(&req, "polls/index.html", context! {
        "questions" => vec!["What's new?", "How <b>now</b>?"],
    })
}
```

| Function | Signature | Purpose |
|---|---|---|
| `rango::shortcuts::render` | `render(&HttpRequest, names, Context) -> Result<HttpResponse>` | Render to a 200 `text/html` response. |
| `rango::shortcuts::render_with` | `render_with(&HttpRequest, names, Context, content_type: Option<&str>, status: Option<u16>) -> Result<HttpResponse>` | Same, with a content type and/or status code. |

`names` is one template name or a list of candidates; see [template names](#template-names).

## The `templates` setting

`Settings::templates` is a list of JSON objects. The engine is built from the **first** entry; further entries are
ignored.

```rust
use rango::conf::Settings;

Settings {
    templates: vec![rango::json!({
        "BACKEND": "rango.template.backends.rango.RangoTemplates",
        "DIRS": ["templates"],
        "APP_DIRS": true,
        "OPTIONS": {
            "context_processors": [
                "rango.template.context_processors.debug",
                "rango.template.context_processors.request",
                "rango.contrib.auth.context_processors.auth",
                "rango.contrib.messages.context_processors.messages",
            ],
        },
    })],
    ..Settings::default()
}
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `BACKEND` | string | - | Names the engine. There is one engine, `rango.template.backends.rango.RangoTemplates`; write it for clarity. The value is not inspected. |
| `DIRS` | list of strings | `[]` | Directories searched for templates, in order. Relative paths are relative to the process working directory. |
| `APP_DIRS` | bool | `false` | Also search the `templates/` directory of every installed app that has a filesystem path. |
| `OPTIONS` | object | `{}` | See below. |

If `templates` is empty, every loader function fails with a configuration error
(`No template engine is configured: TEMPLATES is empty.`).

### Options

| Option | Type | Default | Meaning |
|---|---|---|---|
| `context_processors` | list of strings | `[]` | Registered names of [context processors](#context-processors) to run when a template is rendered with a request. |
| `autoescape` | bool | `true` | HTML-escape variable output. Turn it off only for non-HTML output. |
| `string_if_invalid` | string | `""` | Output for undefined variables. A `%s` in the string is replaced by the variable name. |
| `builtins` | list of strings | `[]` | Names of registered tag libraries whose tags are available in every template without `{% load %}`. Unknown names are ignored. |
| `libraries` | object | `{}` | `{ "alias": "registered library name" }`: makes a library loadable as `{% load alias %}`. Unknown targets are ignored. |
| `debug` | bool | `Settings::debug` | Stored on the engine as `Engine::debug`. It does not change rendering today. |

Not honoured: `loaders` (the lookup order below is fixed) and `file_charset` (templates are read as UTF-8). A
`NAME` key is ignored.

### Caching

Compiled templates are cached by name when `Settings::debug` is `false`. A cached template is not re-read when its
file changes; restart the process after editing templates in production. With `debug: true` every `get_template`
call reads and compiles the file again, so edits show up on the next request.

The engine itself is built lazily and rebuilt whenever `Settings::debug` or `Settings::templates` changes, which
is what makes settings overrides in tests work.

## Template lookup

A template name is a relative path with forward slashes, such as `"polls/index.html"`. The engine tries, in order:

1. in-memory templates added with `Engine::locmem` (only on engines you build yourself);
2. each directory in `DIRS`, in the order listed;
3. with `APP_DIRS: true`, `<app path>/templates/` for each installed app, in app order.

The first file that exists wins. Names that would escape the directory (`../secret.html`) are rejected.

App directories work only for apps registered with a path:

```rust
rango::register_app!(rango::apps::AppConfig::new("polls").path("src/polls"));
```

With that registration `polls/index.html` resolves to `src/polls/templates/polls/index.html`. Put app templates in
a sub-directory named after the app, so two apps cannot shadow each other's `index.html`. Because `DIRS` is
searched first, a project can override any app template by placing a file with the same name in a project
directory.

## Loader functions

These use the engine built from settings.

| Function | Signature | Purpose |
|---|---|---|
| `rango::template::get_template` | `get_template(&str) -> Result<Arc<Template>, TemplateError>` | Load and compile one template. |
| `rango::template::select_template` | `select_template(&[&str]) -> Result<Arc<Template>, TemplateError>` | The first name that exists. A syntax error in a candidate is returned, not skipped. |
| `rango::template::render_to_string` | `render_to_string(names, Context, Option<&HttpRequest>) -> Result<String, TemplateError>` | Load and render. With `Some(request)`, context processors run first. |
| `rango::template::loader::engine` | `engine() -> Result<Arc<Engine>, TemplateError>` | The engine for the current settings. |

```rust
use rango::template::{render_to_string, select_template};
use rango::context;

// An email body: no request, so no context processors and no CSRF token.
let body = render_to_string("mail/welcome.txt", context! { "name" => "Ann" }, None)?;

// Prefer a specific template, fall back to a generic one.
let html = render_to_string(["polls/detail_7.html", "polls/detail.html"], context! { "id" => 7 }, None)?;

let template = select_template(&["a.html", "b.html"])?;
```

### Template names

Functions that take `names` accept any `rango::template::loader::TemplateNames`:

| Type | Meaning |
|---|---|
| `&str`, `String` | One name. |
| `[&str; N]`, `&[&str]`, `Vec<String>` | Candidates tried in order; the first that exists is used. |

If none exists the error is `TemplateError::DoesNotExist` listing all the names tried.

## Using `Engine` directly

`Engine` does not need settings. Build one for tests, for rendering strings, or for a tool that is not a web
application.

```rust
use rango::template::Engine;
use rango::context;

let mut engine = Engine::new();
engine.locmem(&[("hello.html", "Hello, {{ name|upper }}!{% if n %} ({{ n }}){% endif %}")]);

let mut ctx = context! { "name" => "world", "n" => 3 };
assert_eq!(engine.render_to_string("hello.html", &mut ctx)?, "Hello, WORLD! (3)");

// Compile a string without registering it under a name.
let t = engine.from_string("{{ a }} + {{ b }}")?;
let out = t.render(&engine, &mut context! { "a" => 1, "b" => 2 })?;
```

| Item | Meaning |
|---|---|
| `Engine::new()` | Defaults: no directories, `autoescape: true`, empty `string_if_invalid`, cache off. |
| `Engine::from_settings()` | Configured from `Settings::templates[0]` as described above. |
| `engine.locmem(&[(name, source)])` | Add an in-memory template source. Can be called more than once; earlier sources are searched first. |
| `engine.get_template(name)` | `Result<Arc<Template>, TemplateError>`. |
| `engine.select_template(&[names])` | First existing name. |
| `engine.from_string(source)` | Compile a string into a `Template` named `<unknown source>`. |
| `engine.render_to_string(name, &mut Context)` | Load and render. No request binding. |
| `engine.library(name)` | The tag library registered under `name`, if any. |
| `template.render(&engine, &mut Context)` | Render a compiled template. |

Public fields: `dirs`, `app_dirs`, `autoescape`, `string_if_invalid`, `debug`, `context_processors`,
`extra_libraries` (pairs of load name and `Arc<Library>`), `cache_enabled`.

`Template::render` sets the context's `autoescape` flag from the engine before rendering.

## Contexts

A `Context` maps names to template values. It is a stack of scopes; tags such as `{% for %}` and `{% with %}` push
a scope and pop it when they finish.

```rust
use rango::template::Context;
use rango::context;

let mut ctx = context! {
    "title" => "Polls",
    "count" => 3,
    "tags" => vec!["a", "b"],
    "owner" => None::<String>,
};
ctx.set("published", true);
```

| Item | Meaning |
|---|---|
| `context! { "k" => v, ... }` | Build a context. Each value is converted with `Value::from`. |
| `Context::new()` | An empty context. `True`, `False` and `None` are always defined. |
| `Context::from_serialize(&T)` | The fields of a `serde::Serialize` struct (or the entries of a map) become top-level variables. |
| `ctx.set(key, value)` | Set a variable in the innermost scope. Accepts anything that converts into `Value`. |
| `ctx.get(key)` | `Option<&Value>`, searching from the innermost scope outwards. |
| `ctx.push()` / `ctx.pop()` | Open and close a scope. |
| `ctx.insert_below(vars)` | Add variables underneath everything already set; used for context processors, so that view variables win. |
| `ctx.autoescape` | Whether output is escaped. |

## Template values

Templates do not see Rust types directly. Every context entry is a `rango::template::Value`:

| Variant | Holds | Notes |
|---|---|---|
| `Value::None` | nothing | Displays as `None`; false. |
| `Value::Bool(bool)` | | Displays as `True` / `False`. |
| `Value::Int(i64)` | | |
| `Value::Float(f64)` | | Whole values display with one decimal (`1.0`). |
| `Value::Decimal(Decimal)` | `rust_decimal::Decimal` | |
| `Value::Str(String)` | untrusted text | HTML-escaped on output. |
| `Value::Safe(String)` | trusted HTML | Not escaped. |
| `Value::List(Vec<Value>)` | | Indexable, iterable. |
| `Value::Map(IndexMap<String, Value>)` | | Keeps insertion order; iterating yields keys. |
| `Value::Date(NaiveDate)` | | |
| `Value::Time(NaiveTime)` | | |
| `Value::DateTime(NaiveDateTime, Option<i32>)` | local date-time and UTC offset in seconds | `None` offset means a naive value. |
| `Value::Callable(Arc<dyn Fn() -> Value + Send + Sync>)` | | Called with no arguments when looked up. |
| `Value::Object(Object)` | any `TemplateObject` | |

Useful methods: `to_str()`, `truthy()`, `is_safe()`, `iterate()`, `len()`, `as_i64()`, `as_f64()`, `repr()`.

### Conversions

`Value::from` (and therefore `context!` and `ctx.set`) accepts:

| Rust type | Becomes |
|---|---|
| `&str`, `String` | `Str` |
| `bool` | `Bool` |
| `i8`..`i64`, `u8`..`u64`, `isize`, `usize` | `Int` |
| `f64` | `Float` |
| `rust_decimal::Decimal` | `Decimal` |
| `chrono::NaiveDate`, `chrono::NaiveTime` | `Date`, `Time` |
| `chrono::DateTime<Utc>` | `DateTime` in UTC with offset `0` (no conversion to the current time zone) |
| `Vec<T>` where `T: Into<Value>` | `List` |
| `Option<T>` where `T: Into<Value>` | the value, or `None` |
| `serde_json::Value` | the matching variant, recursively |
| `Value` | itself |

There is no conversion from `f32`, `chrono::NaiveDateTime` or map types; build `Value::Float`, `Value::DateTime`
or `Value::Map` explicitly.

### Structs through serde

Any `serde::Serialize` type can be converted with `to_value`, which goes through JSON:

```rust
use rango::template::{Context, to_value};
use serde::Serialize;

#[derive(Serialize)]
struct Stats { visits: u64, top: Vec<String> }

let stats = Stats { visits: 12, top: vec!["home".into()] };

let mut ctx = Context::new();
ctx.set("stats", to_value(&stats));          // {{ stats.visits }}

let ctx = Context::from_serialize(&stats);   // {{ visits }}
```

Because the conversion goes through JSON, dates, times and decimals arrive as strings. The `date` and `time`
filters return an empty string for them. Pass such fields as `Value::Date` / `Value::DateTime` /
`Value::Decimal` when templates need to format them. A value that fails to serialize becomes `Value::None`.

### Objects: `TemplateObject`

Implement `TemplateObject` to expose a Rust type with attributes, a display form and optionally iteration:

```rust
use rango::template::{Object, TemplateObject, Value};
use rango::context;

struct Money { cents: i64, currency: String }

impl TemplateObject for Money {
    fn get(&self, key: &str) -> Option<Value> {
        match key {
            "cents" => Some(Value::Int(self.cents)),
            "currency" => Some(Value::Str(self.currency.clone())),
            _ => None,
        }
    }

    fn display(&self) -> Value {
        Value::Str(format!("{}.{:02} {}", self.cents / 100, self.cents % 100, self.currency))
    }
}

let ctx = context! { "price" => Object::new(Money { cents: 1250, currency: "EUR".into() }) };
// {{ price }} -> 12.50 EUR      {{ price.currency }} -> EUR
```

| Method | Default | Meaning |
|---|---|---|
| `get(&self, key) -> Option<Value>` | required | `{{ obj.key }}`. Return `None` for unknown names. |
| `display(&self) -> Value` | `"<object>"` | `{{ obj }}`. Return `Value::Safe` for ready-made HTML. |
| `iterate(&self) -> Option<Vec<Value>>` | `None` | `{% for x in obj %}`, `obj.0`, `obj\|length`, `x in obj`. |
| `truthy(&self) -> bool` | `true` | `{% if obj %}`. |

`Object::new(value)` wraps the type and returns a `Value`. The type must be `Send + Sync`. Forms, bound fields,
error lists, the current user, messages and pages are exposed to templates this way.

### Model instances: `model_value`

Model instances are passed with `rango::views::generic::model_value`:

```rust
use rango::context;
use rango::template::Value;
use rango::views::generic::model_value;

// `question: Question` and `choices: Vec<Choice>` were fetched earlier in the view.
let ctx = context! {
    "question" => model_value(&question),
    "choices" => Value::List(choices.iter().map(model_value).collect()),
};
```

In the template:

- `{{ question.question_text }}` reads a field by its name; `{{ question.pk }}` is the primary key.
- A foreign key is available under the field name and under its column attribute (`question`, `question_id`);
  both give the stored key, not the related object.
- `{{ question }}` shows the field named by the model's `str` option (`#[model(str = "title")]`), or
  `Question object (1)` when none is set.
- Date-time columns are converted to the current time zone.

The value is a snapshot of the instance's columns. It has no methods and cannot follow relations, because
rendering is synchronous and holds no database connection. Query related rows in the view and add them to the
context under their own names. Generic views do this conversion for you.

## Context processors

A context processor is a function that receives the request and returns extra variables. Processors run when a
template is rendered **with a request** (`render`, `TemplateResponse`, `render_to_string(.., Some(&req))`).
Variables set by the view take precedence over processor output.

Built-in processors, enabled by listing their names in `OPTIONS["context_processors"]`:

| Name | Adds |
|---|---|
| `rango.template.context_processors.request` | `request`: a map with `path`, `path_info`, `method`, `scheme`, `GET` (last value per key), `COOKIES`, `META`, `is_secure`, `get_full_path`, `get_host` (when the host is valid) and `resolver_match` (`url_name`, `view_name`, `app_name`, `namespace`, `route`) when a route matched. |
| `rango.template.context_processors.debug` | `debug` (`True`) and `sql_queries` (an empty list), only when `Settings::debug` is on and the client address is in `internal_ips`. |
| `rango.template.context_processors.static` | `STATIC_URL` |
| `rango.template.context_processors.media` | `MEDIA_URL` |
| `rango.template.context_processors.tz` | `TIME_ZONE` |
| `rango.contrib.auth.context_processors.auth` | `user`: the logged-in user or an anonymous user object (`is_authenticated`, `username`, ...). There is no `perms` variable. |
| `rango.contrib.messages.context_processors.messages` | `messages` (when the messages middleware is active) and `DEFAULT_MESSAGE_LEVELS`. |

`rango.template.context_processors.csrf` always runs and does not need to be listed. It adds `csrf_token` as a
lazy value: the token (and its cookie) is only created if the template uses it.

A name in `context_processors` that is not registered is a configuration error at render time.

### Writing a context processor

```rust
use indexmap::IndexMap;
use rango::http::HttpRequest;
use rango::template::Value;

fn site(_request: &HttpRequest) -> IndexMap<String, Value> {
    [("site_name".to_owned(), Value::from("Polls"))].into_iter().collect()
}

rango::register_context_processor!("polls.context_processors.site", site);
```

The signature is `fn(&HttpRequest) -> IndexMap<String, Value>` (`rango::template::loader::ContextProcessor`).
`IndexMap` comes from the `indexmap` crate (version 2), which your project must list as a dependency. Register the
function under any dotted name and add that name to `OPTIONS["context_processors"]`. Processors are
synchronous and cannot query the database; read what you need from the request, or have middleware attach it to
the request first.

## `TemplateResponse`

`render` produces the body immediately. `TemplateResponse` defers rendering until the view has returned, which
lets middleware change the template or the context first.

```rust
use rango::http::{HttpRequest, HttpResponse};
use rango::template::TemplateResponse;
use rango::{Result, context};

async fn detail(req: HttpRequest, id: i64) -> Result<HttpResponse> {
    Ok(TemplateResponse::new(&req, ["polls/missing.html", "polls/detail.html"], context! { "id" => id }))
}
```

`TemplateResponse::new(&request, names, context)` returns an `HttpResponse` whose body is not produced yet. After
the view returns, the handler calls each middleware's `process_template_response` and then renders the response
with the request bound. Reading the body before that point fails.

Middleware reaches the pending response with `HttpResponse::deferred_mut`:

```rust
use std::sync::Arc;
use async_trait::async_trait;
use rango::core::handlers::Middleware;
use rango::http::{HttpRequest, HttpResponse};
use rango::template::TemplateResponse;
use rango::{Result, register_middleware};

struct AddContext;

#[async_trait]
impl Middleware for AddContext {
    async fn process_template_response(&self, _req: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if let Some(t) = response.deferred_mut::<TemplateResponse>() {
            if let Some(ctx) = t.context_data.as_mut() {
                ctx.set("added_by", "middleware");
            }
        }
        Ok(response)
    }
}

register_middleware!("myproject.middleware.AddContext", || Ok(Arc::new(AddContext)));
```

| Field | Type | Meaning |
|---|---|---|
| `template_names` | `Vec<String>` | Candidates, tried in order. |
| `context_data` | `Option<Context>` | The context; taken when the response renders. |
| `request` | `Option<HttpRequest>` | The request used for context processors. |

`process_template_response` is called only for responses that still have a deferred body. The example needs the
`async-trait` crate. See [middleware](../http/middleware.md).

## Safe strings

Output is escaped unless the value is marked safe. On the Rust side there are two related types:

| Item | Path | Purpose |
|---|---|---|
| `Value::Safe(String)` | `rango::template::Value` | A template value that is not escaped. |
| `safe(s)` | `rango::template::safe` | Shorthand that builds `Value::Safe` from anything convertible to `String`. |
| `SafeString` | `rango::utils::safestring::SafeString` | A string type for HTML built in Rust code. |
| `mark_safe(s)` | `rango::utils::safestring::mark_safe` | Declare a string safe. The caller vouches for it. |
| `format_html!` | `rango::format_html!` | `format!` that escapes every argument and returns a `SafeString`. |
| `escape`, `conditional_escape` | `rango::utils::html` | Escape always / escape unless already a `SafeString`. |
| `format_html_join` | `rango::utils::html::format_html_join` | Join `SafeString` fragments with an escaped separator. |

Build HTML with `format_html!`, never with `format!` plus `mark_safe`, so that interpolated data is escaped:

```rust
use rango::format_html;
use rango::template::safe;
use rango::context;

let label = "Tom & Jerry";
let link = format_html!("<a href=\"{}\">{}</a>", "/shows/1/", label);
assert_eq!(link.as_str(), "<a href=\"/shows/1/\">Tom &amp; Jerry</a>");

let ctx = context! { "link" => safe(link) };   // {{ link }} is output as HTML
```

`format_html!` escapes positional and `name = value` arguments. Identifiers captured inline in the format string
(`"{label}"`) are **not** escaped, so always pass values as arguments.

A `SafeString` is not converted to a safe template value automatically: `Value::from(String)` is always `Str`.
Wrap it with `safe(...)` as above.

## Errors

All template functions return `rango::template::TemplateError`:

| Variant | When |
|---|---|
| `Syntax(String)` | The template source is invalid: unknown tag, unclosed block, bad `{% if %}` expression, unknown library in `{% load %}`. The message carries the line number. Raised when the template is compiled. |
| `DoesNotExist(String)` | No template with that name. The message is the name, or the comma-separated list of candidates. Also raised by `{% extends %}` and `{% include %}` while rendering. |
| `Render(String)` | A failure during rendering: unknown filter (`Invalid filter: 'x'`), `{% url %}` with no match, wrong number of values in a `{% for a, b in ... %}`, a failing custom tag. No line number is attached. |
| `Config(String)` | No usable `templates` setting, or an unregistered context processor. |

`TemplateError` converts into `rango::Error`, so `?` works in views. `Config` becomes an improperly-configured
error; the other variants are wrapped as a generic error and produce a 500 response like any other view failure.

```rust
use rango::template::{TemplateError, get_template};

match get_template("polls/optional.html") {
    Ok(t) => { /* render it */ }
    Err(TemplateError::DoesNotExist(_)) => { /* fall back */ }
    Err(e) => return Err(e.into()),
}
```

Two things are reported later than you might expect:

- Unknown filters are found when the expression is evaluated, not when the template is compiled. A typo inside a
  branch that is never taken goes unnoticed.
- Undefined variables are not errors at all. Set `string_if_invalid` to something visible, such as
  `"[missing: %s]"`, while developing.

## Not supported yet

- Multiple engines, or any backend other than the built-in one.
- Configurable loaders (`OPTIONS["loaders"]`), and invalidation of cached templates without a restart.
- A `perms` variable from the auth context processor, and an i18n context processor.
- Asynchronous context processors or database access during rendering.

## See also

- [The Rango template language](language.md)
- [Custom tags and filters](custom-tags.md)
- [Built-in tag reference](../../ref/template-tags.md) and [filter reference](../../ref/template-filters.md)
- [Shortcuts](../http/shortcuts.md)
- [Middleware](../http/middleware.md)
- [Generic views](../http/generic-views.md)
- [Settings](../settings.md) and the [settings reference](../../ref/settings.md)
- [Form rendering](../forms/rendering.md)
- [Messages](../messages.md)
- [Security](../security.md)
