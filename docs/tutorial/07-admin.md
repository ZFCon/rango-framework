# Part 7: the admin

This part enables the admin site, creates a superuser, and registers the polls models so staff can create and edit
questions and choices in the browser. It continues from [Part 6](06-static-files.md).

## Goal

Log in at `/admin/`, add a question with its choices, and see it on the public pages.

## Enable the required apps and middleware

The admin needs the content-type registry, users and permissions, sessions (to stay logged in) and messages (for
"added successfully" notices). Edit `src/settings.rs`:

```rust
installed_apps: [
    "rango.contrib.contenttypes",
    "rango.contrib.auth",
    "rango.contrib.sessions",
    "rango.contrib.messages",
    "rango.contrib.admin",
    "rango.contrib.staticfiles",
    "polls",
]
.map(String::from)
.to_vec(),
middleware: [
    "rango.middleware.cors.CorsMiddleware",
    "rango.middleware.security.SecurityMiddleware",
    "rango.middleware.common.CommonMiddleware",
    "rango.contrib.sessions.middleware.SessionMiddleware",
    "rango.middleware.csrf.CsrfViewMiddleware",
    "rango.contrib.auth.middleware.AuthenticationMiddleware",
    "rango.contrib.messages.middleware.MessageMiddleware",
    "rango.middleware.clickjacking.XFrameOptionsMiddleware",
]
.map(String::from)
.to_vec(),
```

The order of the middleware matters: sessions come before authentication, which reads the session, and messages
come after sessions, which store them.

Each of these apps brings its own migrations. Apply them:

```console
$ cargo run -- migrate
```

`migrate` also creates the add, change, delete and view permissions for every model of every installed app. The
admin checks those permissions per model.

## Create a superuser

```console
$ RANGO_SUPERUSER_PASSWORD=choose-a-password cargo run -- createsuperuser --noinput --username admin --email admin@example.com
```

The command never prompts for a password; it reads it from `RANGO_SUPERUSER_PASSWORD`. Put a real value there and do
not keep it in your shell history on shared machines. `cargo run -- changepassword admin --password <new>` changes it
later.

## Register the models

The generated `src/polls/admin.rs` is empty. Describe how each model should look in the admin:

### `src/polls/admin.rs`

```rust
use rango::contrib::admin::{AdminSite, ModelAdmin};

use super::models::{Choice, Question};

pub fn register(site: &AdminSite) {
    site.register::<Question>(
        ModelAdmin::default()
            .list_display(["__str__", "pub_date"])
            .search_fields(["question_text"])
            .ordering(["-pub_date"]),
    )
    .expect("Question is registered");

    site.register::<Choice>(
        ModelAdmin::default()
            .list_display(["__str__", "votes"])
            .search_fields(["choice_text"]),
    )
    .expect("Choice is registered");
}
```

`ModelAdmin` is a builder:

| Method | Effect |
| --- | --- |
| `list_display([...])` | Columns of the change list. `"__str__"` is the model's `str` field. |
| `list_filter([...])` | Sidebar filters for the listed fields. |
| `search_fields([...])` | Fields searched by the search box. |
| `ordering([...])` | Default order; a leading `-` reverses it. |
| `list_per_page(n)` | Rows per page. |
| `fields([...])` | Which fields the add and change forms show, and in what order. |

## Mount the admin

The site is created inside the URL configuration and mounted with `include`:

### `src/urls.rs`

```rust
use rango::contrib::admin::AdminSite;
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};

use crate::polls;

register_urlconf!("mysite.urls", || {
    let site = AdminSite::new("/admin/");
    polls::admin::register(&site);
    urlpatterns![
        path("admin/", include(site.urls())),
        path("polls/", include("polls.urls")),
        // ... static, schema and docs routes from the previous parts
    ]
});
```

The argument to `AdminSite::new` is the URL prefix the site is mounted at, so it must match the `path("admin/", ...)`
line.

Run the server and open `http://127.0.0.1:8000/admin/`. Log in with the superuser. You will see "Site
administration" with a section for Questions and one for Choices. From each model's page you get:

- a change list with the columns you chose, the search box, sorting by column and paging;
- add, change and delete forms generated from the model, with validation errors shown next to the fields;
- a notice after each save, shown through the messages app.

Add a question, then add choices that point at it by selecting the question on the choice form. Open
`/polls/` to see it.

A staff user without permissions on a model gets a 403 for it, and the model is hidden from the index. Only
superusers (such as the one you just created) see everything; the admin has no user or group pages yet, so other
accounts get permissions through code.

## Not supported yet

The admin covers the common workflow. These are not implemented: inline editing of related objects (choices inside
the question page), bulk actions such as "delete selected", a history of changes, date hierarchy navigation,
autocomplete and raw-id widgets, `readonly_fields`, `list_editable`, and password change or user management pages.
The checklist in [features](../features.md) tracks them. As a consequence, choices are added on their own page.

## Recap

- Enable five contrib apps and three middleware, run `migrate`, create a superuser with
  `RANGO_SUPERUSER_PASSWORD`.
- Describe each model with `ModelAdmin` in `admin.rs`, register it on an `AdminSite`, mount the site's `urls()`.

Next: [Part 8: a REST API](08-rest-api.md).

## See also

- [The admin site](../topics/admin.md)
- [Authentication](../topics/auth/overview.md)
- [Sessions](../topics/sessions.md)
- [Messages](../topics/messages.md)
