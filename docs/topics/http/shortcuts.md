# Shortcuts

`rango::shortcuts` collects the small functions that nearly every view uses: render a template into a response,
redirect to a URL or a named pattern, and fetch an object or a list or answer 404. This page documents each one with
its exact signature, the errors it returns and the cases where a plain response or a `TemplateResponse` is the better
choice.

```rust
use rango::shortcuts::{
    get_list_or_404, get_object_or_404, redirect, redirect_with, render, render_with, resolve_url,
};
```

## `render`

```rust
pub fn render(request: &HttpRequest, names: impl TemplateNames, context: Context) -> Result<HttpResponse>
```

Renders a template with the request's context processors and returns a 200 HTML response.

```rust
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::render;
use rango::{Result, context};

pub async fn index(request: HttpRequest) -> Result<HttpResponse> {
    render(&request, "polls/index.html", context! { "latest" => vec!["What's new?", "How are you?"] })
}
```

`names` is anything the loader accepts: a `&str`, a `String`, an array or slice of `&str`, or a `Vec<String>`. When
several names are given, the first template that exists is used:

```rust
render(&request, ["polls/special.html", "polls/detail.html"], context! { "id" => 7 })
```

Because a `request` is passed, the built-in `csrf_token` variable and the processors listed in the `templates`
setting (for example `rango.template.context_processors.request`) add their variables below yours: a variable you set
wins over a processor's variable of the same name. See [Template engine](../templates/engine.md).

Errors: a template-not-found error when no candidate exists, template syntax or render errors, and an error if the template
engine is not configured. In a view these become a 500 response (with the technical page when `debug` is on).

`render` renders **immediately** and returns a finished response. That means `process_template_response` hooks of
middleware do not see it. When a middleware should be able to change the context first, return a
`rango::template::TemplateResponse` instead:

```rust
use rango::template::TemplateResponse;

Ok(TemplateResponse::new(&request, "polls/index.html", context! { "latest" => latest }))
```

`TemplateResponse::new(&request, names, context)` is itself an `HttpResponse`, rendered after the view returns. See
[Middleware](middleware.md#template-responses).

## `render_with`

```rust
pub fn render_with(
    request: &HttpRequest,
    names: impl TemplateNames,
    context: Context,
    content_type: Option<&str>,
    status: Option<u16>,
) -> Result<HttpResponse>
```

The same as `render` with a custom content type and status code. Pass `None` for either to keep the default (HTML,
200).

```rust
use rango::shortcuts::render_with;

// A plain-text robots file.
render_with(&request, "robots.txt", context! {}, Some("text/plain"), None)

// An error page rendered by hand with status 410.
render_with(&request, "gone.html", context! { "what" => "article" }, None, Some(410))
```

An invalid status code (outside 100 to 599) is an error.

## `redirect`

```rust
pub fn redirect(to: &str) -> Result<HttpResponse>
```

Answers 302 Found. `to` is a URL or the name of a URL pattern. Use it after a successful `POST`, so that a refresh
does not submit the form twice.

```rust
use rango::shortcuts::redirect;

redirect("/polls/")?;             // a path: used as is
redirect("polls:index")?;         // a pattern name: reversed
redirect("https://example.com/")  // an absolute URL: used as is
```

## `redirect_with`

```rust
pub fn redirect_with(to: &str, args: Reverse, permanent: bool) -> Result<HttpResponse>
```

Redirect with reversal arguments and the choice between 302 (`permanent = false`) and 301 (`permanent = true`).
`Reverse` is the builder of [`reverse`](urls.md#reversing-urls).

```rust
use rango::shortcuts::redirect_with;
use rango::urls::Reverse;

pub async fn vote(request: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    // ... record the vote ...
    redirect_with("polls:results", Reverse::new().args([question_id]), false)
}

redirect_with("article", Reverse::new().kwarg("slug", "launch"), true)
```

## `resolve_url`

```rust
pub fn resolve_url(to: &str, args: Reverse) -> Result<String>
```

The function that `redirect` and `redirect_with` use to find the target. Call it yourself when you need the URL as a
string. The rules, in order:

1. A value starting with `./` or `../` is returned unchanged.
2. Otherwise `to` is reversed as a pattern name with `args`; success returns the URL.
3. If reversing fails with `NoReverseMatch` and `to` contains a `/` or a `.`, `to` is assumed to be a URL and returned
   unchanged.
4. If it fails and `to` contains neither, the `NoReverseMatch` error is returned. This is how a mistyped pattern name
   (`"polls:detial"`) is reported instead of being turned into a redirect to a nonsense URL.

Consequently a pattern name that itself contains a dot is treated as a URL when it does not match. Keep pattern names
free of dots and slashes.

Other errors from reversing (for example a wrong number of arguments for a pattern that exists) are returned as they
are, and they are not rescued by step 3.

## `get_object_or_404`

```rust
pub async fn get_object_or_404<M: Model>(qs: QuerySet<M>, db: &dyn Executor) -> Result<M>
```

Runs `qs.get(db)` and converts "no such row" into `Error::Http404("No <Model> matches the given query.")`. In a view
returning that error gives the 404 page; any other error (a database failure, or several matching rows) is returned
unchanged.

```rust
use rango::db::Db;
use rango::shortcuts::{get_object_or_404, render};

pub async fn detail(request: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    let db = Db::default_db();
    let question = get_object_or_404(Question::objects().filter_kw("pk", question_id), &db).await?;
    render(&request, "polls/detail.html", context! { "question" => rango::views::generic::model_value(&question) })
}
```

The queryset is the filter, so any filter applies:

```rust
let published = get_object_or_404(
    Article::objects().filter_kw("slug", slug).filter_kw("status", "published"),
    &db,
)
.await?;
```

A query that matches more than one row returns `Error::MultipleObjectsReturned`, which is a 500 and not a 404.
Make the filter specific enough to match at most one row.

## `get_list_or_404`

```rust
pub async fn get_list_or_404<M: Model>(qs: QuerySet<M>, db: &dyn Executor) -> Result<Vec<M>>
```

Fetches the queryset and answers 404 (`No <Model> matches the given query.`) when it is empty.

```rust
let choices = get_list_or_404(Choice::objects().filter_kw("question", question_id), &db).await?;
```

## Where the 404 shows up

The shortcuts return an `Err(Error::Http404(..))`; the handler turns it into a response. With `debug` off you get the
`404.html` template if your project has one, otherwise a plain built-in page; with `debug` on you get the technical 404
page that shows the message. To raise a 404 yourself, return `Err(rango::Error::Http404("message".into()))`. See
[Writing views](views.md) and [URL dispatcher](urls.md#when-nothing-matches-and-error-views).

## Not supported yet

- Passing a model instance to `redirect` to use its `get_absolute_url`. Compute the URL yourself and pass the string.
- Template-engine selection in `render` (the `using` option). There is one configured engine.
- `get_object_or_404` with a model type or a manager instead of a queryset: build the queryset first with
  `Model::objects()`.

## See also

- [Writing views](views.md)
- [URL dispatcher](urls.md)
- [Generic views](generic-views.md)
- [Middleware](middleware.md)
- [Template engine](../templates/engine.md)
- [Making queries](../db/queries.md)
