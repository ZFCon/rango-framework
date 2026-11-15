# Tutorial: a polls app

Seven parts, from an empty directory to a tested app with an admin. Every snippet uses APIs exercised by the tests in
`crates/rango/tests/` (`polls_e2e`, `templates_e2e`, `generic_e2e`, `auth_e2e`, `admin_e2e`).

## Part 1 — project, app, first view

```console
$ cargo install rangoframework-cli     # installs the rango-admin tool
$ rango-admin startproject mysite
$ cd mysite
$ rango-admin startapp polls          # or: cargo run -- startapp polls
$ cargo run -- runserver
```

The generated `Cargo.toml` depends on `rangoframework`; in code the library is called `rango` (`use rango::...`).

`src/main.rs` is your `manage.py`: `cargo run -- <command>`. Register the app by name in
`src/settings.rs` (`installed_apps: vec!["polls".into()]`) and include its URLs in `src/urls.rs`:

```rust
register_urlconf!("mysite.urls", || urlpatterns![
    path("polls/", include("polls.urls")),
]);
```

`polls/views.rs`:

```rust
pub async fn index(_req: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new("Hello, world. You're at the polls index."))
}
```

## Part 2 — models, migrations, the database

`polls/models.rs` (SQLite is the default; `ENGINE: "rango.db.backends.postgresql"` works too):

```rust
#[derive(Clone, Debug, Default, Model)]
#[model(str = "question_text")]
pub struct Question {
    #[field(primary_key, auto)] pub id: Option<i64>,
    #[field(max_length = 200)]  pub question_text: String,
    #[field(auto_now_add, verbose_name = "date published")] pub pub_date: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Model)]
#[model(str = "choice_text")]
pub struct Choice {
    #[field(primary_key, auto)] pub id: Option<i64>,
    pub question: ForeignKey<Question>,
    #[field(max_length = 200)] pub choice_text: String,
    #[field(default = 0)]      pub votes: i32,
}
```

```console
$ cargo run -- makemigrations        # writes src/polls/migrations/m0001_initial.rs (a Rust file)
$ cargo run -- sqlmigrate polls 0001
$ cargo run -- migrate
```

Query with explicit connections:

```rust
let db = Db::default_db();
Question::objects().filter(filter!(question_text__startswith = "What")).fetch(&db).await?;
let q = Question::objects().create(&db, Question { question_text: "What's new?".into(), ..Question::default() }).await?;
```

## Part 3 — views and templates

Configure `templates` in settings (the generated project already does) and write `templates/polls/index.html`:

```html
{% if latest_question_list %}
<ul>{% for question in latest_question_list %}
  <li><a href="{% url 'polls:detail' question.pk %}">{{ question }}</a></li>
{% endfor %}</ul>
{% else %}<p>No polls are available.</p>{% endif %}
```

```rust
pub async fn index(req: HttpRequest) -> Result<HttpResponse> {
    let latest = Question::objects().order_by(&["-pub_date"]).slice(0, Some(5)).fetch(&Db::default_db()).await?;
    render(&req, "polls/index.html", context! { "latest_question_list" => latest.iter().map(model_value).collect::<Vec<_>>() })
}

pub async fn detail(req: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    let db = Db::default_db();
    let question = get_object_or_404(Question::objects().filter_kw("pk", question_id), &db).await?;
    render(&req, "polls/detail.html", context! { "question" => model_value(&question) })
}
```

URL parameters bind by position: `path("<int:question_id>/", views::detail).name("detail")`.
`{{ question }}` renders the `str` field; `{{ question.choice_set.all }}` is not available (no implicit queries) —
fetch the choices in the view and pass them in.

## Part 4 — a form, and generic views

```rust
pub async fn vote(req: HttpRequest, question_id: i64) -> Result<HttpResponse> {
    let db = Db::default_db();
    let choice_id: i64 = req.post().await?.get("choice").and_then(|c| c.parse().ok()).unwrap_or(0);
    match Choice::objects().filter_kw("pk", choice_id).filter_kw("question", question_id).get(&db).await {
        Ok(mut c) => {
            c.votes += 1;
            c.save(&db).await?;
            redirect_with("polls:results", Reverse::new().args([question_id]), false)
        }
        Err(Error::ObjectDoesNotExist { .. }) => render(&req, "polls/detail.html", context! { "error_message" => "You didn't select a choice." }),
        Err(e) => Err(e),
    }
}
```

Templates must carry `{% csrf_token %}` inside `<form method="post">`. The index and detail pages are one line each
with the generic views:

```rust
path("", ListView::<Question>::new().ordering(["-pub_date"]).as_view()).name("index"),
path("<int:pk>/", DetailView::<Question>::new().as_view()).name("detail"),
```

(templates `polls/question_list.html` / `polls/question_detail.html`; context names `question_list`, `question`).

## Part 5 — tests

```rust
let c = Client::new()?;
assert_contains(&c.get("/polls/").await?, "No polls are available.", 200);
```

`rango::test::Client` drives the full middleware stack, cookies and redirects (`follow_redirects(true)`).

## Part 6 — static files

Put files in `polls/static/polls/style.css`, use `{% load static %}<link href="{% static 'polls/style.css' %}">`,
and for deployment set `static_root` and run `cargo run -- collectstatic`.

## Part 7 — the admin

Add `rango.contrib.{contenttypes,auth,sessions,messages}` to `installed_apps` and the session, authentication and
message middleware to `middleware`, then in `src/urls.rs`:

```rust
register_urlconf!("mysite.urls", || {
    let site = AdminSite::new("/admin/");
    site.register::<Question>(ModelAdmin::default().list_display(["__str__", "pub_date"]).search_fields(["question_text"])).unwrap();
    urlpatterns![path("polls/", include("polls.urls")), path("admin/", include(site.urls()))]
});
```

```console
$ cargo run -- migrate
$ RANGO_SUPERUSER_PASSWORD=secret cargo run -- createsuperuser --noinput --username admin --email admin@example.com
```

Then visit `/admin/`. Permissions (`add_question`, …) are created by `migrate`; the admin checks them per model.
See `docs/features.md` for what the admin does not cover yet (inlines, bulk actions, history).
