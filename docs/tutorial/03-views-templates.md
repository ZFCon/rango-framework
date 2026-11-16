# Part 3: views and templates

This part adds the question index, the detail page and the results page, and renders them with templates. It
continues from [Part 2](02-models.md).

## Goal

Three pages, each backed by the database:

| URL | Name | Shows |
| --- | --- | --- |
| `/polls/` | `polls:index` | the five most recent questions |
| `/polls/<id>/` | `polls:detail` | one question and its choices |
| `/polls/<id>/results/` | `polls:results` | the choices with their vote counts |

## Add some data

The templates below show "No polls are available" until there are rows. [Part 7](07-admin.md) adds the admin, the
natural place to enter data; the test in [Part 5](05-testing.md) also creates rows. For now you can create them with
`cargo run -- dbshell`, or skip ahead and come back.

## Configure templates

The generated settings already contain a `templates` entry:

```rust
templates: vec![rango::json!({
    "BACKEND": "rango.template.backends.rango.RangoTemplates",
    "DIRS": ["templates"],
    "APP_DIRS": true,
    "OPTIONS": {
        "context_processors": [
            "rango.template.context_processors.debug",
            "rango.template.context_processors.request",
        ],
    },
})],
```

Templates are searched in `DIRS` (here the project's `templates/` directory) and, because `APP_DIRS` is true, in the
`templates/` directory of every installed app that has a path. You set the path of the polls app in Part 1. Put app
templates in a subdirectory named after the app, `src/polls/templates/polls/`, so names from different apps cannot
collide.

Context processors add variables to every template rendered with a request. `request` makes `{{ request }}`
available; `debug` adds `debug` and `sql_queries` when `debug` is on and the client address is in `internal_ips`.

## The views

### `src/polls/views.rs`

```rust
use rango::context;
use rango::db::Db;
use rango::db::models::Model;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::{get_object_or_404, render};
use rango::views::generic::model_value;
use rango::Result;

use super::models::{Choice, Question};

pub async fn index(req: HttpRequest) -> Result<HttpResponse> {
    let db = Db::default_db();
    let latest = Question::objects().order_by(&["-pub_date"]).slice(0, Some(5)).fetch(&db).await?;
    render(
        &req,
        "polls/index.html",
        context! { "latest_question_list" => latest.iter().map(model_value).collect::<Vec<_>>() },
    )
}

pub async fn detail(req: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    let db = Db::default_db();
    let question = get_object_or_404(Question::objects().filter_kw("pk", question_id), &db).await?;
    let choices = Choice::objects().filter_kw("question", question_id).order_by(&["id"]).fetch(&db).await?;
    render(
        &req,
        "polls/detail.html",
        context! {
            "question" => model_value(&question),
            "choices" => choices.iter().map(model_value).collect::<Vec<_>>(),
        },
    )
}

pub async fn results(req: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    let db = Db::default_db();
    let question = get_object_or_404(Question::objects().filter_kw("pk", question_id), &db).await?;
    let choices = Choice::objects().filter_kw("question", question_id).order_by(&["-votes"]).fetch(&db).await?;
    render(
        &req,
        "polls/results.html",
        context! {
            "question" => model_value(&question),
            "choices" => choices.iter().map(model_value).collect::<Vec<_>>(),
        },
    )
}
```

Points worth knowing:

- `render(&req, template_name, context)` renders a template with the request's context processors and returns a
  200 response.
- `context!` builds the variables a template sees. Values must be template values, so a model instance goes through
  `model_value`, which exposes its fields (`question.question_text`), its primary key as `question.pk`, and the
  `str` field when the object itself is printed.
- Related objects are not reachable from the template: there is no `question.choice_set`. The view fetches the
  choices and passes them in, in the order it wants.
- `get_object_or_404` runs the query and turns "no such row" into a 404 response, so the view can use `?`.

## The URLs

URL parameters are bound to the view's arguments by position. `<int:question_id>` captures an integer, which is
passed as the second argument of the view.

### `src/polls/urls.rs`

```rust
use rango::urls::path;
use rango::{register_urlconf, urlpatterns};

use super::views;

register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", views::index).name("index"),
    path("<int:question_id>/", views::detail).name("detail"),
    path("<int:question_id>/results/", views::results).name("results"),
]);
```

Because the file registers `app_name = "polls"`, the names are used as `polls:detail` and so on.

## The templates

### `src/polls/templates/polls/index.html`

```html
{% if latest_question_list %}
<ul>
  {% for question in latest_question_list %}
  <li><a href="{% url 'polls:detail' question.pk %}">{{ question }}</a></li>
  {% endfor %}
</ul>
{% else %}
<p>No polls are available.</p>
{% endif %}
```

`{{ question }}` prints the model's `str` field (`question_text`). `{% url 'polls:detail' question.pk %}` builds
`/polls/3/` from the pattern name, so changing a route in `urls.rs` never breaks a link. Output is HTML-escaped
unless you mark it safe.

### `src/polls/templates/polls/detail.html`

```html
<h1>{{ question }}</h1>
{% if error_message %}<p><strong>{{ error_message }}</strong></p>{% endif %}
<ul>
  {% for choice in choices %}
  <li>{{ choice.choice_text }}</li>
  {% endfor %}
</ul>
```

This is a read-only version; [Part 4](04-forms-generic-views.md) turns it into a voting form.

### `src/polls/templates/polls/results.html`

```html
<h1>{{ question }}</h1>
<ul>
  {% for choice in choices %}
  <li>{{ choice.choice_text }} -- {{ choice.votes }} vote{{ choice.votes|pluralize }}</li>
  {% endfor %}
</ul>
<a href="{% url 'polls:detail' question.pk %}">Vote again?</a>
```

## Remove duplication with template inheritance

Every page will want the same frame. Put it in one place and let pages fill the blanks.

### `src/polls/templates/polls/base.html`

```html
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>{% block title %}Polls{% endblock %}</title>
</head>
<body>
  {% block content %}{% endblock %}
</body>
</html>
```

A page then starts with `{% extends 'polls/base.html' %}` and puts its markup in `{% block content %}...{% endblock %}`:

```html
{% extends 'polls/base.html' %}
{% block title %}Results{% endblock %}
{% block content %}
<h1>{{ question }}</h1>
...
{% endblock %}
```

Use it for all three pages if you like; the remaining parts show only the body markup to stay short.

## A missing page

`/polls/99/` has no question, so `get_object_or_404` returns an error and the framework renders a 404 page. With
`debug: true` that is a technical page listing the URL patterns it tried; with `debug: false` it is the plain
"Not Found" page. To use your own, add `templates/404.html` (or `500.html`); the 404 template receives
`request_path`.

## Recap

- A view fetches data with explicit queries and calls `render` with a context built by `context!`.
- Models go into the context through `model_value`; related rows are fetched in the view.
- Templates look up URLs by name with `{% url %}`, and share structure with `{% extends %}` and `{% block %}`.

Next: [Part 4: forms and generic views](04-forms-generic-views.md).

## See also

- [Template language](../topics/templates/language.md)
- [The template engine](../topics/templates/engine.md)
- [Shortcuts](../topics/http/shortcuts.md)
- [URL dispatcher](../topics/http/urls.md)
