# Sites, flat pages and redirects

Three small apps share one table of sites. `rango.contrib.sites` stores the domains your project serves, `rango.contrib.flatpages` serves hand-written pages stored in the database, and `rango.contrib.redirects` turns missing URLs into redirects you manage as data.

## Enable them

```rust
use rango::conf::Settings;

let settings = Settings {
    site_id: Some(1),
    installed_apps: ["contenttypes", "sites", "redirects", "flatpages"].map(|a| format!("rango.contrib.{a}")).to_vec(),
    middleware: [
        "redirects.middleware.RedirectFallbackMiddleware",
        "flatpages.middleware.FlatpageFallbackMiddleware",
    ].map(|m| format!("rango.contrib.{m}")).to_vec(),
    ..Settings::default()
};
```

Run `cargo run -- migrate`. Both `redirects` and `flatpages` depend on the `sites` migration. Put the two fallback middleware last in the list: they act on a 404 coming back from the rest of the stack.

## Sites

`Site` (table `rango_site`) has `id`, `domain` (unique, up to 100 chars) and `name` (up to 50 chars).

After `migrate`, a `post_migrate` hook creates a default site, `example.com`, when the table is empty. Its id is `site_id` if set, otherwise 1. Change it through the admin or by saving a `Site`.

Find the current site:

```rust
use rango::contrib::sites;
use rango::db::Db;

let db = Db::default_db();
let site = sites::get_current(&db, Some(&request)).await?;   // Site
println!("{}", site.domain);
```

`get_current(db, request: Option<&HttpRequest>)` resolves in this order:

1. If the `site_id` setting is `Some(id)`, load that row. This needs no request.
2. Otherwise look up the request's host (case-insensitive), then the host without its port.
3. With no `site_id` and no request it returns an `ImproperlyConfigured` error.

A missing row returns `ObjectDoesNotExist`. `create_default_site(db)` is public if you need to call it from setup code.

Models that belong to one site hold a `ForeignKey<Site>` (as `Redirect` does) or a `ManyToMany<Site>` (as `FlatPage` does).

## Flat pages

`FlatPage` (table `rango_flatpage`, ordered by `url`):

| Field | Type | Meaning |
|---|---|---|
| `url` | `String` (100) | Absolute path, with leading slash, for example `/about/`. |
| `title` | `String` (200) | Page title. |
| `content` | `String`, blank | Body. Rendered without escaping, so it may hold HTML. Only trusted editors should be able to write it. |
| `enable_comments` | `bool` | Exposed to the template as `flatpage.enable_comments`. |
| `template_name` | `String` (70), blank | Template to try first. Falls back to `flatpages/default.html`. |
| `registration_required` | `bool` | Anonymous users are redirected to the login page. |
| `sites` | `ManyToMany<Site>` | The sites the page appears on. |

Create a page and attach it to a site (adapted from `crates/rango/tests/contrib_e2e.rs`):

```rust
use rango::contrib::flatpages::FlatPage;
use rango::db::models::{related, Model};
use rango::db::{Db, Value};

let db = Db::default_db();
let page = FlatPage::objects()
    .create(&db, FlatPage { id: None, url: "/about/".into(), title: "About us".into(), content: "We exist.".into(), ..FlatPage::default() })
    .await?;
related::add(&db, "flatpages.FlatPage", "sites", &page.pk_value(), &[Value::Int(1)]).await?;
```

The page is served only when its `sites` include the current site.

### Template

Provide `flatpages/default.html` in one of your template directories. The context has one variable, `flatpage`, with `url`, `title`, `content` and `enable_comments`.

```html
<!doctype html>
<title>{{ flatpage.title }}</title>
<h1>{{ flatpage.title }}</h1>
{{ flatpage.content }}
```

If `template_name` is set it is tried first, then `flatpages/default.html`.

### How a page is found

`FlatpageFallbackMiddleware` runs on the way out. When the response status is 404, it looks for a flat page at `request.path_info()` for the current site. If found, its rendered page replaces the 404. If not, the original 404 stays. Only 404 responses are touched, so real views always win over flat pages.

When `append_slash` is true and the requested URL has no trailing slash, but a page exists at the slash form, the middleware answers with a permanent redirect to the slashed path.

You can also call the view function directly from a URLconf: `flatpages::flatpage(request: &HttpRequest, url: &str) -> Result<HttpResponse>`. It returns `Error::Http404` when nothing matches.

## Redirects

`Redirect` (table `rango_redirect`, unique on `(site, old_path)`):

| Field | Type | Meaning |
|---|---|---|
| `site` | `ForeignKey<Site>` | The site the redirect applies to. |
| `old_path` | `String` (200) | Absolute path without domain, for example `/events/search/`. Include the query string to match one. |
| `new_path` | `String` (200), blank | An absolute path or full URL. Empty means the page is gone. |

```rust
use rango::contrib::redirects::Redirect;
use rango::db::models::ForeignKey;

Redirect::objects().create(&db, Redirect { id: None, site: ForeignKey::new(1), old_path: "/old/".into(), new_path: "/real/".into() }).await?;
Redirect::objects().create(&db, Redirect { id: None, site: ForeignKey::new(1), old_path: "/dead/".into(), new_path: String::new() }).await?;
```

`RedirectFallbackMiddleware` acts only on 404 responses. It matches `request.get_full_path()` (path plus query string); when `append_slash` is true and the path lacks a trailing slash, it also tries the slashed form. A match with a non-empty `new_path` returns a 301. A match with an empty `new_path` returns 410 Gone. No match leaves the 404 untouched.

The middleware looks up the current site on every 404, so it needs `site_id` set or a request host that matches a `Site`.

## Test behaviour

`crates/rango/tests/contrib_e2e.rs` exercises all three apps with `rango::test::Client`: a redirect (`/old/` returns 301 to `/real/`), a gone entry (`/dead/` returns 410), a flat page, and a plain 404. See [Testing](testing.md).

## Not supported yet

- No `{% get_current_site %}`-style template tag and no `CurrentSiteManager`.
- No sitemap of flat pages.
- Flat pages have no template tag to fetch other pages.

## See also

- [Sitemaps](sitemaps.md)
- [Content types](contenttypes.md)
- [Middleware](http/middleware.md)
- [Settings](settings.md)
