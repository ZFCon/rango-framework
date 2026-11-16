# Generic views

Generic views are ready-made views for the common jobs: showing a template, redirecting, listing and showing
objects, editing them and archiving them by date. This page covers every builder in `rango::views::generic`: how to
construct it, every method, the template names and context variables it uses, pagination, and how to turn it into a
route with `as_view()`. It also shows how to write your own view type by implementing the `View` trait.

## Using a generic view

Each generic view is a builder: create it, chain methods, call `as_view()`, and give the result to `path()`.

```rust
use rango::urls::path;
use rango::views::generic::{DetailView, ListView, TemplateView, View};
use rango::{register_urlconf, urlpatterns};

use crate::models::Article;

register_urlconf!("blog.urls", || urlpatterns![
    path("about/", TemplateView::new("about.html").extra("site", "Rango").as_view()).name("about"),
    path("articles/", ListView::<Article>::new().paginate_by(10).ordering(["-published"]).as_view()).name("list"),
    path("articles/<int:pk>/", DetailView::<Article>::new().as_view()).name("detail"),
]);
```

`View` here is the trait `rango::views::generic::View`; it must be in scope for `as_view()` to resolve. (It is not the
same type as `rango::urls::View`, which is what `as_view()` returns.)

Things all builders have in common:

- Model-based builders are generic over the model: `ListView::<Article>::new()`. The model is a type that derives
  `Model`.
- They use the default database connection. `using("alias")` selects another entry from the `databases` setting.
- `extra(key, value)` adds a fixed template variable. The value is anything that converts into
  `rango::template::Value`: `&str`, `String`, `bool`, integers, `f64`, dates and so on.
- Hooks that take a closure take plain `Fn` closures that are `Send + Sync + 'static`. Because model data is read with
  explicit connections, the closures are synchronous; they cannot run queries. For logic that needs the database,
  write the view as a function or implement `View` yourself (see the end of this page).
- Template context always starts with the URL's named captures (for `<int:pk>` the variable `pk`), then the
  view-specific variables, then `extra` values, then the context hook. Later entries replace earlier ones with the
  same name.
- Instances are rendered as template objects: `{{ article.title }}`, `{{ article.pk }}`, `{{ article }}` (the model's
  `str` field, or `Article object (3)` when there is none). A foreign-key column is available as its raw value
  (`{{ choice.question_id }}`); related objects are not loaded for you.
- Model template names are built from the model's app label and its object name in lower case, with no separators:
  model `BlogPost` in app `blog` gives `blog/blogpost_list.html`.

HTTP methods that a view does not handle get a `405 Method Not Allowed` response with an `Allow` header.
`HEAD` runs the `GET` code and drops the body; `OPTIONS` answers with `Allow` and an empty body.

## `TemplateView`

Renders a template. `GET` only.

```rust
use rango::views::generic::{TemplateView, View};

TemplateView::new("about.html")
    .extra("site", "Rango")
    .content_type("text/plain")
    .context(|request, _args, ctx| {
        ctx.set("path", request.path());
    })
    .as_view()
```

| Item | Meaning |
|---|---|
| `TemplateView::new(template_name)` | Required; the template to render. |
| `extra(key, value)` | Add a template variable. |
| `content_type(mime)` | Set the `Content-Type` of the response (default is HTML). |
| `context(\|request, args, ctx\| ...)` | Hook that edits the `Context` last. `args` is `&ViewArgs`. |

Context: the URL captures, the `extra` values, then whatever the hook sets. The response is a template response, so it
is rendered with the request's context processors.

## `RedirectView`

Redirects to a fixed URL or to a named pattern. It answers `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE` and
`OPTIONS`.

```rust
use rango::views::generic::{RedirectView, View};

// /old/5/  ->  /articles/5/  (301)
path("old/<int:pk>/", RedirectView::to_pattern("detail").permanent(true).as_view());

// /go/rango/  ->  /docs/rango/?...
path("go/<str:name>/", RedirectView::to_url("/docs/%(name)s/").query_string(true).as_view());
```

| Item | Meaning |
|---|---|
| `RedirectView::to_url(url)` | Fixed target. Each `%(name)s` is replaced with the URL capture of that name. |
| `RedirectView::to_pattern(name)` | Target reversed from a pattern name (`"detail"`, `"polls:index"`). Named captures are passed as keyword arguments; if there are none, the unnamed captures are passed positionally. |
| `permanent(bool)` | `true` answers 301, the default `false` answers 302. |
| `query_string(bool)` | Append the request's query string to the target. Default `false`. |
| `get_redirect_url(&request, &args)` | The computed target, `Ok(None)` if neither a URL nor a pattern is set. |

If the reversal fails the error is `NoReverseMatch` (a 500 response). A view with neither a URL nor a pattern
answers `410 Gone`. The struct fields (`url`, `pattern_name`, `permanent`, `query_string`) are public.

## `ListView`

Lists objects, optionally paginated.

```rust
use rango::views::generic::{ListView, View};

ListView::<Article>::new()
    .queryset(|_request| Article::objects().filter_kw("status", "published"))
    .ordering(["-published", "title"])
    .paginate_by(20)
    .paginate_orphans(3)
    .context_object_name("articles")
    .template_name("blog/index.html")
    .context(|_request, _args, objects, ctx| {
        ctx.set("shown", objects.len() as i64);
    })
    .as_view()
```

| Method | Default | Meaning |
|---|---|---|
| `ListView::<M>::new()` | | All rows of `M`. |
| `queryset(\|request\| QuerySet<M>)` | `M::objects()` | Base queryset; receives the request, so it can filter by user or query string. |
| `ordering(["f", "-g"])` | none | Replaces the queryset's ordering. |
| `template_name(name)` | see below | Template to render. |
| `context_object_name(name)` | `<model>_list` | Extra name for the list in the context. |
| `paginate_by(n)` | no pagination | Page size. |
| `paginate_orphans(n)` | `0` | A last page that would have `n` or fewer items is merged into the previous one. |
| `page_kwarg(name)` | `"page"` | Name of the URL capture or query parameter holding the page. |
| `allow_empty(bool)` | `true` | When `false`, an empty list is a 404. |
| `using(alias)` | `"default"` | Database alias. |
| `extra(key, value)` | | Fixed extra variable. |
| `context(\|request, args, objects, ctx\| ...)` | none | Hook run last; `objects` is `&[M]`, the items shown. |

Public fields mirror these (`queryset`, `using`, `template_name`, `template_name_suffix` = `"_list"`,
`context_object_name`, `paginate_by`, `paginate_orphans`, `page_kwarg`, `allow_empty`, `ordering`, `extra_context`).

**Template**: `template_name` if given, otherwise `<app_label>/<model>_list.html`.

**Context**

| Variable | Value |
|---|---|
| `object_list` | The items to show (the current page when paginating). |
| `<model>_list`, or `context_object_name` | The same list. |
| `is_paginated` | `true` when there is more than one page. Always `false` without `paginate_by`. |
| `page_obj` | The current page; `none` without pagination. |
| `paginator` | Paginator summary; `none` without pagination. |

`page_obj` and `paginator` are the same object. It supports `number`, `has_next`, `has_previous`, `has_other_pages`,
`next_page_number`, `previous_page_number`, `start_index`, `end_index`, `num_pages`, `count`, `per_page` and
`page_range` (a list `1..=num_pages`). It iterates over the positions on the page. `next_page_number` and
`previous_page_number` are only defined when such a page exists.

### Pagination

With `paginate_by(n)` the view builds a `Paginator` (see [Pagination](../pagination.md)) over the queryset. The page
number comes from the URL capture named `page_kwarg` if there is one, otherwise from the query string
(`/articles/?page=2`), otherwise it is `1`. The word `last` selects the last page.

Anything else is a 404: a value that is not `last` and not an integer, a page below 1, or a page past the end.
Because the paginator needs a stable order, always give the list an `ordering` (or order the queryset), otherwise
pages can repeat or skip rows.

```html
{% for article in articles %}<li>{{ article }}</li>{% endfor %}
{% if is_paginated %}
  {% if page_obj.has_previous %}<a href="?page={{ page_obj.previous_page_number }}">previous</a>{% endif %}
  Page {{ page_obj.number }} of {{ paginator.num_pages }}
  {% if page_obj.has_next %}<a href="?page={{ page_obj.next_page_number }}">next</a>{% endif %}
{% endif %}
```

## `DetailView`

Shows one object, found from the URL.

```rust
use rango::views::generic::{DetailView, View};

path("articles/<int:pk>/", DetailView::<Article>::new().as_view());
path("by-slug/<slug:slug>/", DetailView::<Article>::new().slug_field("url_slug").as_view());
```

| Method | Default | Meaning |
|---|---|---|
| `DetailView::<M>::new()` | | |
| `queryset(\|request\| QuerySet<M>)` | `M::objects()` | Rows the object may come from. Use it to hide unpublished rows. |
| `pk_url_kwarg(name)` | `"pk"` | URL capture holding the primary key. |
| `slug_url_kwarg(name)` | `"slug"` | URL capture holding the slug. |
| `slug_field(name)` | `"slug"` | Model field compared with the slug. |
| `query_pk_and_slug(bool)` | `false` | When both captures are present, filter by both instead of only the pk. |
| `template_name(name)` | see below | Template to render. |
| `context_object_name(name)` | `<model>` | Extra name for the object. |
| `using(alias)` | `"default"` | Database alias. |
| `extra(key, value)` | | Fixed extra variable. |
| `context(\|request, args, object, ctx\| ...)` | none | Hook run last; `object` is `&M`. |
| `get_object(&request, &args, &db).await` | | The lookup on its own, for use in your own views. |

Lookup: the primary key is used if the pk capture exists, else the slug. If neither capture is in the route the view
fails with an improperly-configured error ("Generic detail view must be called with either an object pk or a slug in
the URLconf."). No matching row is a 404 with the message `No <verbose name> found matching the query`.

**Template**: `template_name`, otherwise `<app_label>/<model>_detail.html`.

**Context**: `object` and `<model>` (or `context_object_name`) hold the instance, plus URL captures, `extra` values and
the hook's additions.

## `FormView`

Shows a form, validates it on `POST`/`PUT`, re-renders with errors when invalid, and redirects or calls your handler
when valid.

```rust
use rango::views::generic::{FormView, View};

FormView::new("contact.html", |_request| contact_form())
    .success_url("/contact/thanks/")
    .as_view()
```

| Item | Meaning |
|---|---|
| `FormView::new(template_name, \|request\| Form)` | The closure builds a fresh unbound `rango::forms::Form` per request. |
| `success_url(url)` | Redirect target (302) after a valid submission. |
| `success_url_with(\|request\| Ok(url))` | Compute the target per request. |
| `form_valid(\|request, form\| Box::pin(async move { ... }))` | Replace the default redirect; the closure gets the request and the validated form and returns a `Result<HttpResponse>` future. |
| `extra(key, value)` | Fixed extra variable. |

Context: the URL captures, the `extra` values and `form`. If the form is valid and there is no `form_valid` and no
success URL, the view fails with "No URL to redirect to. Provide a success_url." Walkthrough and examples:
[Forms](../forms/forms.md#formview).

## `CreateView` and `UpdateView`

Both edit a model through a model form. They take the form options at construction. `CreateView` needs
`M: Model + Default`.

```rust
use rango::forms::ModelFormOptions;
use rango::views::generic::{CreateView, UpdateView, View};

path(
    "articles/add/",
    CreateView::<Article>::new(ModelFormOptions::fields(["title", "body", "published"]))
        .success_url("/articles/")
        .as_view(),
);
path(
    "articles/<int:pk>/edit/",
    UpdateView::<Article>::new(ModelFormOptions::fields(["title", "body"]))
        .absolute_url(|a| format!("/articles/{}/", a.id.unwrap()))
        .as_view(),
);
```

`ModelFormOptions::fields([...])` lists the form fields; `ModelFormOptions::all()` uses every editable field;
`.exclude([...])`, `.widget`, `.label`, `.help_text` and `.error_message` refine it. See
[Model forms](../forms/modelforms.md).

| Method | Default | Meaning |
|---|---|---|
| `new(opts)` | | The form options. |
| `template_name(name)` | `<app_label>/<model>_form.html` | Template. |
| `success_url(url)` | | Fixed redirect after saving. |
| `success_url_with(\|request\| Ok(url))` | | Computed redirect; it gets the request only, not the saved object. |
| `absolute_url(\|object\| String)` | | Redirect target derived from the saved object, used when no success URL is set. |
| `queryset(\|request\| QuerySet<M>)` | `M::objects()` | Where `UpdateView` finds the object (also used by its lookup). |
| `pk_url_kwarg(name)` | `"pk"` | URL capture of the primary key (`UpdateView`). |
| `using(alias)` | `"default"` | Database alias. |
| `extra(key, value)` | | Fixed extra variable. |

Behaviour:

- `GET` renders the form. `UpdateView` first loads the object (404 if missing, same lookup as `DetailView`, so a
  `slug` capture works too) and fills the form from it; `CreateView` renders an empty form.
- `POST` (and `PUT`) bind the submitted data and files. Invalid input re-renders the template with field errors
  (status 200). Valid input is saved and answered with a 302.
- The redirect target is `success_url`/`success_url_with` if set, else `absolute_url` applied to the saved object. If
  neither is set the view fails with "No URL to redirect to. Either provide a url or define a get_absolute_url method
  on the Model." Note that the save has already happened by then; always set one of them.
- Forms that include file fields work as long as the form tag has `enctype="multipart/form-data"`; see
  [File uploads](file-uploads.md).

**Context**: `form`, the URL captures, `extra` values; for `UpdateView` also `object` and `<model>`. `CreateView` has no
`object`.

## `DeleteView`

Asks for confirmation on `GET`, deletes on `POST` or `DELETE`.

```rust
use rango::views::generic::{DeleteView, View};

path("articles/<int:pk>/delete/", DeleteView::<Article>::new().success_url("/articles/").as_view());
```

| Method | Default | Meaning |
|---|---|---|
| `DeleteView::<M>::new()` | | |
| `template_name(name)` | `<app_label>/<model>_confirm_delete.html` | Confirmation template. |
| `success_url(url)` | none | Where to redirect (302) after deleting. |
| `queryset(\|request\| QuerySet<M>)` | `M::objects()` | Rows that may be deleted. |
| `using(alias)` | `"default"` | Database alias. |

Context on `GET`: `object`, `<model>`, URL captures. The template should post back with a CSRF token:

```html
<form method="post">{% csrf_token %}
  <p>Delete "{{ object }}"?</p>
  <button>Delete</button>
</form>
```

Deletion goes through the model deletion machinery, so related rows are handled as the foreign keys specify.
`success_url` has no default; without it the view fails with an improperly-configured error after the row has been
deleted, so always set it.

## `DateView`: archive views

One builder, `DateView`, provides all the date-based views. Pick the kind with `DateViewKind` and name the date field.

```rust
use rango::views::generic::{DateView, DateViewKind, View};

DateView::<Article>::new(DateViewKind::Year, "published")
    .template_name("year.html")
    .make_object_list(true)
    .as_view()
```

| `DateViewKind` | URL captures required | Shows |
|---|---|---|
| `ArchiveIndex` | none | The 15 newest objects, newest first. |
| `Year` | `year` | Objects of that year. |
| `Month` | `year`, `month` | Objects of that month. |
| `Week` | `year`, `week` | Objects of that week. |
| `Day` | `year`, `month`, `day` | Objects of that day. |
| `Today` | none | Objects of today. |
| `Detail` | `year`, `month`, `day` and `pk` (or `slug`) | One object, if it falls on that day. |

```rust
path("archive/", DateView::<Article>::new(DateViewKind::ArchiveIndex, "published").as_view());
path("archive/<int:year>/", DateView::<Article>::new(DateViewKind::Year, "published").as_view());
path("archive/<int:year>/<str:month>/", DateView::<Article>::new(DateViewKind::Month, "published").as_view());
path("archive/<int:year>/<str:month>/<int:day>/", DateView::<Article>::new(DateViewKind::Day, "published").as_view());
```

| Method | Default | Meaning |
|---|---|---|
| `new(kind, date_field)` | | `date_field` is a `Date` or `DateTime` field of the model. |
| `template_name(name)` | see below | Template. |
| `allow_future(bool)` | `false` | Allow dates after today. When `false`, objects dated in the future are hidden and a period that starts in the future is a 404 ("Future dates not allowed"). |
| `allow_empty(bool)` | `true` for `ArchiveIndex`, `false` otherwise | When `false`, a period with no objects is a 404 ("No objects available"). |
| `make_object_list(bool)` | `false` | See the note below. |
| `month_format(fmt)` | `"%b"` | How the `month` capture is parsed, as a `strftime` pattern. `%b` expects `jan`, `feb`; use `%m` for numbers. |
| `queryset(\|request\| QuerySet<M>)` | `M::objects()` | Base queryset. |
| `using(alias)` | `"default"` | Database alias. |
| `extra(key, value)` | | Fixed extra variable. |

The `day` capture is parsed with the public field `day_format` (default `"%d"`), which has no builder method; set it
on the struct before `as_view()` if you need a different pattern. `context_object_name` is also a public field.
Invalid or missing captures give a 404 (`Invalid date string ...`) or an improperly-configured error naming the
missing capture.

`Week` uses weeks that start on Sunday, numbered from the first Sunday of the year (`%U` style) and has no option to
change this.

**Templates**: `template_name` if set, otherwise `<app_label>/<model>` plus a suffix:

| Kind | Suffix |
|---|---|
| `ArchiveIndex` | `_archive.html` |
| `Year` | `_archive_year.html` |
| `Month` | `_archive_month.html` |
| `Week` | `_archive_week.html` |
| `Day`, `Today` | `_archive_day.html` |
| `Detail` | `_detail.html` |

**Context** (all list kinds also get `object_list`, `<model>_list` or `context_object_name`, the URL captures and `extra`
values):

| Kind | Extra variables |
|---|---|
| `ArchiveIndex` | `date_list` (the years that have objects, newest first), `latest` (the objects). |
| `Year` | `date_list` (first day of each month with objects), `year`, `previous_year`, and `next_year` unless it would be in the future and `allow_future` is off. |
| `Month` | `date_list` (days with objects), `month`, `previous_month`, `next_month` (same rule). |
| `Week` | `week`, `previous_week`, `next_week`. |
| `Day`, `Today` | `day`, `previous_day`, `next_day`. |
| `Detail` | Same as `DetailView`: `object` and `<model>`. |

All the date values are template dates, so use the `date` filter: `{{ year|date:"Y" }}`.

Limits to know:

- Period boundaries for `DateTime` fields are midnight UTC; they do not follow the current time zone.
- `ArchiveIndex` loads only the 15 newest objects, and `date_list` is derived from those, not from the whole table.
- Date views are not paginated.
- `make_object_list` currently changes nothing: the `Year` view always includes `object_list`.
- For `Detail`, `extra` and `context_object_name` are not applied, and the lookup uses the `pk`/`slug` defaults
  (`pk`, `slug`, field `slug`).

## Writing your own view type

When the builders do not fit, implement the `View` trait. Every method has a default; override the ones you need.

```rust
use async_trait::async_trait;
use rango::http::{HttpRequest, HttpResponse};
use rango::urls::{path, ViewArgs};
use rango::views::generic::View;
use rango::Result;

struct Ping;

#[async_trait]
impl View for Ping {
    async fn get(&self, _req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        Ok(HttpResponse::new("pong"))
    }
    async fn post(&self, req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        let body = req.post().await?;
        Ok(HttpResponse::new(format!("got {} fields", body.len())))
    }
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "OPTIONS"]
    }
}

path("ping/", Ping.as_view());
```

| Method | Default |
|---|---|
| `get`, `post`, `put`, `patch`, `delete`, `trace` | `405 Method Not Allowed` with the `Allow` header. |
| `head` | Calls `get`. |
| `options` | Empty 200 with `Allow`. |
| `allowed_methods()` | `["GET", "HEAD", "OPTIONS"]`. Override it so `Allow` lists what you implemented. |
| `dispatch(request, args)` | Picks the method above from the request method; unknown methods give 405. |
| `as_view(self)` | Wraps `self` in an `Arc` and returns the route value. |

The view instance is shared between requests and threads, so keep per-request data in local variables and request
extensions, not in `self`.

## Not supported yet

- Mixins and inheritance; configuration is by builder methods and closures instead. A `form_invalid` hook, `get_form`
  customisation, initial data for model forms, `get_success_url` that receives the saved object, and a
  `get_object` override are not available on the builders.
- `slug_field` and `slug_url_kwarg` builders on `UpdateView` and `DeleteView` (they use the `slug` defaults).
- Pagination and ordering options on `DateView`; multi-object `ListView` on several querysets.
- Async context hooks. Hooks are synchronous.

## See also

- [Writing views](views.md)
- [URL dispatcher](urls.md)
- [Shortcuts](shortcuts.md)
- [Pagination](../pagination.md)
- [Forms](../forms/forms.md)
- [Model forms](../forms/modelforms.md)
- [File uploads](file-uploads.md)
- [Tutorial part 4: forms and generic views](../../tutorial/04-forms-generic-views.md)
