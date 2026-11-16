# Part 6: static files

This part adds a stylesheet to the polls pages, serves it during development, and shows how to collect static files
for deployment. It continues from [Part 5](05-testing.md).

## Goal

The polls pages load `polls/style.css`, both in `runserver` and, later, from a web server.

## Where static files live

A static file is identified by its path relative to a static directory, for example `polls/style.css`. Rango looks
for it in two places, in this order:

1. every directory in the `staticfiles_dirs` setting;
2. the `static/` directory of every installed app that has a path (you gave the polls app one in
   [Part 1](01-project.md)).

Put the stylesheet in a subdirectory named after the app, so files from different apps do not shadow each other:

### `src/polls/static/polls/style.css`

```css
body {
    font-family: sans-serif;
    margin: 2rem auto;
    max-width: 40rem;
}

li a {
    color: green;
}
```

## Settings

Change `static_url` in `src/settings.rs`. The generated value, `"static/"`, is relative to the current page and
resolves to the wrong place on nested URLs such as `/polls/1/`. Start it with a slash:

```rust
static_url: Some("/static/".into()),
```

For deployment you will also need a `static_root` and the staticfiles app (for the `collectstatic` command):

```rust
installed_apps: ["rango.contrib.staticfiles", "polls"].map(String::from).to_vec(),
static_root: Some("staticfiles".into()),
```

`static_root` is where `collectstatic` copies everything; it is not used while developing. Relative paths are
resolved against the directory you start the process in.

## Use the file in a template

Load the `static` library and use the `{% static %}` tag. Add this to the base template from Part 3 (or to the top
of any page):

```html
{% load static %}
<link rel="stylesheet" href="{% static 'polls/style.css' %}">
```

The tag prefixes the path with `static_url`, so the page links to `/static/polls/style.css`. Because the prefix
comes from one setting, moving static files to a CDN later means changing `static_url`, not the templates.

## Serve static files during development

`runserver` does not serve static files by itself. Add a route that serves them through the same lookup the finders
use. It refuses to run unless `debug` is on.

### `src/urls.rs`

```rust
use rango::conf::settings;
use rango::contrib::staticfiles;
use rango::http::{HttpRequest, HttpResponse};
use rango::rest::schema;
use rango::urls::{include, path};
use rango::{Result, register_urlconf, urlpatterns};

async fn static_file(request: HttpRequest, file: String) -> Result<HttpResponse> {
    if !settings().debug {
        return Err(rango::Error::Http404("Not found".into()));
    }
    staticfiles::serve(&request, &file).await
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("static/<path:file>", static_file),
    path("polls/", include("polls.urls")),
    path("api/schema/", schema::schema_view()).name("schema"),
    path("api/docs/", schema::swagger_view("/api/schema/")).name("swagger-ui"),
    path("api/redoc/", schema::redoc_view("/api/schema/")).name("redoc"),
]);
```

The route prefix must match `static_url`. `serve` returns 404 when no static directory has the file and guesses the
`Content-Type` from the extension. It reads the whole file on every request and sets no cache headers, so it is a
development convenience only.

Reload `/polls/`: the links are green.

## Find out where a file comes from

```console
$ cargo run -- findstatic polls/style.css
```

lists every directory that has a file by that name, in search order. The first one wins.

## Collect for deployment

In production a web server or CDN serves the files; the framework does not. `collectstatic` copies every static file
into `static_root`:

```console
$ cargo run -- collectstatic
```

Useful options: `--dry-run` to report without copying, `--clear` to empty `static_root` first (never point it at a
directory holding anything else), and `--ignore PATTERN` to skip names. Then configure the server to answer
`/static/` from `static_root`. Because the app's `static/` directory is found through a path fixed at compile time,
run `collectstatic` on the machine where the source tree exists at the build path (for example in the build step).

## Recap

- Static files live in `static/<app>/...` inside an app or in a directory listed in `staticfiles_dirs`.
- Use `{% load static %}` and `{% static 'path' %}`; set `static_url` with a leading slash.
- Serve with a debug-only route in development; `collectstatic` plus a real web server in production.

Next: [Part 7: the admin](07-admin.md).

## See also

- [Static files](../topics/static-files.md)
- [Deployment](../topics/deployment.md)
- [Settings](../topics/settings.md)
