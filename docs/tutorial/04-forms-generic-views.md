# Part 4: forms and generic views

This part turns the detail page into a voting form, handles the submission, and then replaces the index view with a
ready-made generic view. It continues from [Part 3](03-views-templates.md).

## Goal

Choose an answer on `/polls/<id>/`, submit it, and land on the results page with the vote counted.

## A form in the template

### `src/polls/templates/polls/detail.html`

```html
<h1>{{ question }}</h1>
{% if error_message %}<p><strong>{{ error_message }}</strong></p>{% endif %}

<form action="{% url 'polls:vote' question.pk %}" method="post">
  {% csrf_token %}
  {% for choice in choices %}
    <input type="radio" name="choice" id="choice{{ forloop.counter }}" value="{{ choice.pk }}">
    <label for="choice{{ forloop.counter }}">{{ choice.choice_text }}</label><br>
  {% endfor %}
  <input type="submit" value="Vote">
</form>
```

Three things to note:

- `{% url 'polls:vote' question.pk %}` builds the form's target from the pattern name you add below.
- `{% csrf_token %}` is required in every `POST` form. `CsrfViewMiddleware` is in the project's middleware list and
  rejects a `POST` that does not carry a matching token with a 403 ("CSRF verification failed"). The tag renders a
  hidden `csrfmiddlewaretoken` field and, as a side effect, sets the `csrftoken` cookie. If a page never uses the
  token, no cookie is set.
- `forloop.counter` numbers the loop from 1; it is used to pair each radio button with its label.

## The vote view

Add `vote` to `src/polls/views.rs` (the other views are unchanged):

```rust
use rango::db::models::F;
use rango::shortcuts::redirect_with;
use rango::urls::Reverse;

pub async fn vote(req: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    let db = Db::default_db();
    let question = get_object_or_404(Question::objects().filter_kw("pk", question_id), &db).await?;
    let choice_id: i64 = req.post().await?.get("choice").and_then(|c| c.parse().ok()).unwrap_or(0);

    let updated = Choice::objects()
        .filter_kw("pk", choice_id)
        .filter_kw("question", question_id)
        .update(&db, vec![("votes", (F("votes") + 1).into())])
        .await?;

    if updated == 0 {
        // Nothing matched: show the form again with a message.
        let choices = Choice::objects().filter_kw("question", question_id).order_by(&["id"]).fetch(&db).await?;
        return render(
            &req,
            "polls/detail.html",
            context! {
                "question" => model_value(&question),
                "choices" => choices.iter().map(model_value).collect::<Vec<_>>(),
                "error_message" => "You didn't select a choice.",
            },
        );
    }
    // Redirect after a successful POST so a refresh does not vote twice.
    redirect_with("polls:results", Reverse::new().args([question_id]), false)
}
```

Walking through it:

- `req.post().await?` parses the form body. `.get("choice")` returns the submitted value as a string; anything
  missing or non-numeric becomes `0`, which matches no row.
- The `filter_kw` calls select exactly one choice, and only if it belongs to this question, so a forged `choice`
  value from another question changes nothing.
- `F("votes") + 1` is an *F expression*: the increment happens inside the database (`votes = votes + 1`) rather than
  in Rust. If two visitors vote at the same moment, reading the count, adding one and saving would lose a vote; the
  single `UPDATE` does not. `update` returns the number of rows it changed, which is used to detect a bad choice.
- `redirect_with("polls:results", Reverse::new().args([question_id]), false)` builds the URL from the name and
  returns a 302. The last argument says whether the redirect is permanent. For a pattern without arguments,
  `redirect("polls:index")` is enough.

Add the route to `src/polls/urls.rs`:

```rust
path("<int:question_id>/vote/", views::vote).name("vote"),
```

Try it in the browser: choose an answer and submit; submit without choosing to see the error message.

## Generic views

The index page does the same thing as hundreds of other pages: fetch some rows of one model, render them. Generic
views capture those patterns. Replace the `index` route in `src/polls/urls.rs`:

```rust
use rango::urls::path;
use rango::views::generic::{ListView, View};
use rango::{register_urlconf, urlpatterns};

use super::models::Question;
use super::views;

register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", ListView::<Question>::new().ordering(["-pub_date"]).paginate_by(5).as_view()).name("index"),
    path("<int:question_id>/", views::detail).name("detail"),
    path("<int:question_id>/results/", views::results).name("results"),
    path("<int:question_id>/vote/", views::vote).name("vote"),
]);
```

`ListView::<Question>` reads the rows itself, so `views::index` can be deleted. By default it looks for the template
`polls/question_list.html` (`<app>/<model>_list.html`) and passes the rows as `question_list` and `object_list`.
With `paginate_by(5)` it also passes `is_paginated`, `page_obj` and `paginator`, and a `?page=` beyond the last page
is a 404. Rename the template to match:

### `src/polls/templates/polls/question_list.html`

```html
{% if question_list %}
<ul>
  {% for question in question_list %}
  <li><a href="{% url 'polls:detail' question.pk %}">{{ question }}</a></li>
  {% endfor %}
</ul>
{% else %}
<p>No polls are available.</p>
{% endif %}

{% if is_paginated %}
<p>
  {% if page_obj.has_previous %}<a href="?page={{ page_obj.previous_page_number }}">previous</a>{% endif %}
  page {{ page_obj.number }} of {{ paginator.num_pages }}
  {% if page_obj.has_next %}<a href="?page={{ page_obj.next_page_number }}">next</a>{% endif %}
</p>
{% endif %}
```

You can override the defaults on the builder: `.template_name("polls/index.html")`,
`.context_object_name("latest_question_list")`, `.queryset(|req| ...)` to restrict the rows.

### `DetailView`

`DetailView` shows one object chosen by the `pk` in the URL and answers 404 when it does not exist:

```rust
use rango::views::generic::DetailView;

path("<int:pk>/summary/", DetailView::<Question>::new().as_view()).name("summary"),
```

It renders `polls/question_detail.html` with the object as `question` and `object`. The URL parameter must be called
`pk` for this view. It cannot replace the `detail` and `results` views above, because those also need the question's
choices, and the hooks `DetailView` offers (`extra`, `context`) are synchronous and have no database connection. A
page that needs related rows stays an ordinary function view; a page that shows one model alone is one line.

Rango also has `TemplateView`, `RedirectView`, `FormView`, `CreateView`, `UpdateView`, `DeleteView` and date-based
views; see [Generic views](../topics/http/generic-views.md).

## Not supported yet

- There is no `{{ question.choice_set.all }}`-style access from templates. This is deliberate, and not a gap that
  will be filled: queries are written where they run.
- `DetailView` and `ListView` cannot run extra queries for the context; use a function view for that.

## Recap

- A `POST` form needs `{% csrf_token %}`; the vote view reads the body with `req.post()`.
- `F("votes") + 1` with `update` increments in the database, safe under concurrent votes.
- Redirect after a successful `POST` with `redirect_with`.
- `ListView` and `DetailView` replace the boilerplate for pages that show rows of one model.

Next: [Part 5: testing](05-testing.md).

## See also

- [Generic views](../topics/http/generic-views.md)
- [Forms](../topics/forms/forms.md)
- [Model forms](../topics/forms/modelforms.md)
- [Request and response objects](../topics/http/requests-responses.md)
- [Security](../topics/security.md)
