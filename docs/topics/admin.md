# The admin site

`rango::contrib::admin` generates a back-office for your models: a staff login, an index of registered models, a change
list with search, filters, sorting and paging, and add, change and delete pages built from model forms. This page covers
setup, every `ModelAdmin` option, permissions, customising the look, and what is not implemented yet.

## Enabling the admin

The admin uses authentication, sessions and (for flash messages) the messages framework. Install the apps and
middleware, then mount the site in your URL configuration.

```rust
use rango::conf::Settings;

pub fn settings() -> Settings {
    Settings {
        installed_apps: [
            "rango.contrib.contenttypes",
            "rango.contrib.auth",
            "rango.contrib.sessions",
            "rango.contrib.messages",
            "rango.contrib.admin",
            "todos",
        ]
        .map(String::from)
        .to_vec(),
        middleware: [
            "rango.middleware.security.SecurityMiddleware",
            "rango.contrib.sessions.middleware.SessionMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
            "rango.contrib.auth.middleware.AuthenticationMiddleware",
            "rango.contrib.messages.middleware.MessageMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        ..Settings::default()
    }
}
```

Without `MessageMiddleware` the admin still works but shows no "saved" notices. The forms contain `{% csrf_token %}`, so
keep the CSRF middleware in production.

Run `migrate` so the content type, permission and session tables exist. When an app is migrated, the default add,
change, delete and view permissions of its models are created automatically.

## Registering models

Create one `AdminSite` with the URL prefix it is mounted at, register models on it, and include `site.urls()` under
that same prefix. The model must derive `Model` and implement `Default`.

```rust
use rango::contrib::admin::{AdminSite, ModelAdmin};
use rango::register_urlconf;
use rango::urls::{include, path};
use rango::urlpatterns;

use crate::todos::models::Todo;

register_urlconf!("todo_api.urls", || {
    let site = AdminSite::new("/admin/");
    site.register::<Todo>(
        ModelAdmin::default()
            .list_display(["title", "done", "created"])
            .list_filter(["done"])
            .search_fields(["title", "notes"]),
    )
    .expect("Todo is registered");
    urlpatterns![path("admin/", include(site.urls()))]
});
```

`AdminSite::new(prefix)` returns `Arc<AdminSite>`. `register::<M>(admin)` returns `Result<()>`; it fails when the model
is unknown to the model registry. Registering the same model twice replaces the earlier registration. The prefix passed
to `new` is used to build links, so it must match where you mount the URLs and end in a slash.

### Site titles

`AdminSite` has public `site_header` ("Rango administration"), `site_title` ("Rango site admin") and `index_title`
("Site administration"). Because `new` returns an `Arc`, change them before sharing the handle:

```rust
use rango::contrib::admin::AdminSite;

let mut site = AdminSite::new("/admin/");
{
    let s = std::sync::Arc::get_mut(&mut site).expect("not shared yet");
    s.site_header = "Todo control room".into();
    s.site_title = "Todo admin".into();
    s.index_title = "Everything".into();
}
```

## URLs

`site.urls()` returns these routes under the prefix. Names are for use with `reverse`.

| Path | Name | Page |
| --- | --- | --- |
| `` | `index` | Model index. |
| `login/` | `login` | Staff login. |
| `logout/` | `logout` | Logout (POST only). |
| `<app>/<model>/` | `changelist` | List of objects. |
| `<app>/<model>/add/` | `add` | Add form. |
| `<app>/<model>/<pk>/change/` | `change` | Edit form. |
| `<app>/<model>/<pk>/delete/` | `delete` | Delete confirmation. |

`<app>` is the model's app label and `<model>` its lower-cased struct name, for example `/admin/blog/article/`. An
unregistered pair, or an object that does not exist, gives a 404.

## `ModelAdmin` options

`ModelAdmin::default()` plus builder methods. Each builder takes anything that iterates over strings, except
`list_per_page` which takes a `usize`.

| Option | Default | Meaning |
| --- | --- | --- |
| `list_display` | `["__str__"]` | Columns of the change list. Field names, or `"__str__"` for the object's string form. |
| `list_filter` | none | Fields offered as sidebar filters. |
| `search_fields` | none | Fields matched by the search box. Without any, no search box is shown. |
| `ordering` | none | Default sort for the change list, for example `["-created"]`. |
| `list_per_page` | `100` | Rows per page. |
| `fields` | all editable | Fields shown in the add and change forms, in that order. |
| `exclude` | none | Fields left out of the forms. |

How each one behaves:

- **`list_display`**: names must be concrete model fields. A name that is not a field (for example a method name) is
  shown as the object's string form. The first column links to the change page. Values are rendered as text: booleans
  as `Yes`/`No`, null as `-`, date-times as `YYYY-MM-DD HH:MM:SS`, relations as the raw key. Headers are the field name
  with underscores replaced by spaces; click a header to sort by it, and click again to reverse. The `__str__` column
  uses the model's `str` attribute (`#[model(str = "title")]`) and falls back to `Model object (pk)`.
- **`list_filter`**: for each listed field the sidebar shows "All" plus every distinct stored value, and a click filters
  with an exact match (`?done=1`). Boolean filters use `1`/`0`. Names that are not model fields are ignored. Filters
  combine with each other and with search.
- **`search_fields`**: the query in `?q=` is lower-cased and matched with a substring test against each listed field;
  a row matches if any field matches. Names that are not model fields are ignored. Only the model's own columns
  are searched, not related models.
- **`ordering`**: used when the URL has no `?o=`. If both are empty the model's own `ordering` is used, then newest
  primary key first. Names starting with `-` sort descending; `pk` means the primary key. Names that are not concrete
  fields are skipped.
- **`list_per_page`**: pages are selected with `?p=`; a value of 0 is treated as 1. The page shows "Page N of M" and
  previous/next links that keep the active search, filters and sort.
- **`fields`** and **`exclude`**: passed straight to the model form, so the rules of [model forms](forms/modelforms.md)
  apply, including required fields and validation messages.

```rust
use rango::contrib::admin::ModelAdmin;

let admin = ModelAdmin::default()
    .list_display(["__str__", "published", "created"])
    .list_filter(["published"])
    .search_fields(["title", "body"])
    .ordering(["-created"])
    .list_per_page(25)
    .fields(["title", "body", "published"])
    .exclude(["internal_note"]);
```

## The pages

- **Index**: models grouped by app label, each with a link to its list and, when allowed, an "Add" link. A model shows
  only if the user may view or add it. A user who may do neither sees "You don't have permission to view or edit
  anything."
- **Change list**: the object count ("3 articles"), the filter sidebar, the search box, the table and the paginator.
  The "Add" button appears when the user may add.
- **Add and change**: the model form rendered field by field with errors and help text. Valid data is saved and the
  user is redirected to the change list with the success message `The article “Delta” was added successfully.` (or
  `changed`). Invalid data re-renders the form with errors. Forms with file fields use a multipart encoding.
- **Delete**: a confirmation page; confirming deletes the object, applying the model's delete rules for related rows,
  and redirects with "was deleted successfully". The page does not list the related objects that will go with it.

## Login and access

Everything except the login page requires a logged-in user who is both `is_staff` and `is_active`. An anonymous or
non-staff visitor is redirected to `<prefix>login/?next=<current path>`. The login page authenticates through your
configured authentication backends and rejects accounts that are not active staff with "Please enter the correct
username and password for a staff account." After login it redirects to `next` when that starts with `/`, otherwise to
the index. Logout is a POST (the header has a form button); a GET returns 405.

## Permissions

Access is checked per model with the permission codenames `<app_label>.<action>_<modelname>`, for example
`blog.change_article`.

| Page | Permission needed |
| --- | --- |
| Change list | `view`, or `change` (change implies view) |
| Add | `add` |
| Change | `change` |
| Delete | `delete` |

A staff user lacking the permission gets a 403 response. Superusers pass every check. Grant permissions to users or
groups with the auth models (see [authentication](auth/overview.md)).

The default four permissions are created for each model when its app is migrated. To create them yourself, for example
in tests or after registering models later, call `create_permissions`:

```rust
use rango::contrib::admin::create_permissions;
use rango::db::Db;

async fn setup() -> rango::Result<()> {
    let db = Db::default_db();
    let created = create_permissions(&db, "blog").await?; // returns how many were new
    println!("{created} permissions created");
    Ok(())
}
```

Extra permissions declared on the model through its `permissions` attribute are created as well.

## Customising the templates

The admin renders six templates: `admin/base.html`, `admin/login.html`, `admin/index.html`,
`admin/change_list.html`, `admin/change_form.html` and `admin/delete_confirmation.html`. They are compiled into the
crate. To replace one, create a template with the same name in a directory your template engine searches (for example
`templates/admin/change_list.html` with `"DIRS": ["templates"]`). The admin checks your template engine first and only
falls back to the bundled copy when it finds nothing.

Two consequences:

- A template you supply is rendered by your engine, so any `{% extends "admin/base.html" %}` in it needs your own
  `admin/base.html` as well. Overriding only `base.html` has no effect on the bundled pages, which extend the bundled
  base. To restyle everything, override all six.
- Copy the bundled templates from `crates/rango-contrib/templates/admin/` as a starting point.

Context variables, by template:

| Template | Variables |
| --- | --- |
| all | `title`, `site_title`, `site_header`, `index_title`, `admin_url`, `theme_css`, `messages`, `user_name` (not on login) |
| `login.html` | `error`, `next` |
| `index.html` | `app_list` (each with `name` and `models`; each model has `name`, `url`, `add_url`) |
| `change_list.html` | `app_name`, `verbose_name`, `verbose_name_plural`, `add_url`, `search_fields` (bool), `query`, `count`, `headers` (`label`, `url`), `rows` (`cells`, `change_url`), `filters` (`title`, `choices` with `label`, `url`, `selected`), `page`, `num_pages`, `prev_url`, `next_url` |
| `change_form.html` | `app_name`, `verbose_name_plural`, `list_url`, `form`, `delete_url` |
| `delete_confirmation.html` | `app_name`, `verbose_name`, `verbose_name_plural`, `list_url`, `object` |

The styling uses CSS variables defined in `theme_css`, an inline stylesheet bundled with the framework. Your template
can include `{{ theme_css|safe }}` and then override variables such as `--primary` or `--header-bg` in its own `<style>`
block, or ignore it entirely.

## Testing the admin

The test client can drive the admin like a browser. This is adapted from the repository's tests:

```rust
use rango::contrib::auth;
use rango::db::Db;
use rango::test::{Client, assert_contains, assert_redirects};

async fn smoke(db: &Db) {
    let um = auth::get_user_model().unwrap();
    um.create_superuser(db, "root", "r@x.org", Some("pw")).await.unwrap();

    let c = Client::new().unwrap();
    assert_redirects(&c.get("/admin/").await.unwrap(), "/admin/login/?next=/admin/", 302);
    assert_redirects(
        &c.post("/admin/login/", &[("username", "root"), ("password", "pw"), ("next", "/admin/")]).await.unwrap(),
        "/admin/",
        302,
    );
    assert_contains(&c.get("/admin/").await.unwrap(), "Site administration", 200);
}
```

See [testing](testing.md) for the client.

## Not supported yet

- Inline editing of related objects.
- Bulk actions; the only way to delete is the single-object page.
- Change history (no log of who changed what).
- Date hierarchy, `readonly_fields`, `list_editable`, custom per-column display functions.
- Autocomplete and raw-id widgets for relations; relations appear as plain inputs from the model form.
- Searching or filtering across relations, and range or date filters (filters are exact matches on distinct values).
- A user management page and an admin password-change page; manage users with the `createsuperuser` and
  `changepassword` commands (see [commands](commands.md)).
- Admin documentation pages.
- Plurals in counts: the list always says "1 articles".

## See also

- [Tutorial: the admin](../tutorial/07-admin.md)
- [Authentication overview](auth/overview.md)
- [Messages](messages.md)
- [Sessions](sessions.md)
- [Model forms](forms/modelforms.md)
- [Templates language](templates/language.md)
