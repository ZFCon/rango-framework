# Static files

Static files are the CSS, JavaScript, images and fonts that ship with your code. Rango finds them in project
directories and in each app's `static/` directory, builds their URLs in templates, and collects them into one
directory for deployment. Files uploaded by users are a separate concern; see [Managing files](files.md).

Everything on this page is in `rango::contrib::staticfiles` and the template library `static`.

## Settings

| Setting | Type | Default | Meaning |
|---|---|---|---|
| `static_url` | `Option<String>` | `None` | URL prefix of static files, for example `"/static/"`. |
| `static_root` | `Option<String>` | `None` | Directory `collectstatic` copies files into. Only needed for deployment. |
| `staticfiles_dirs` | `Vec<String>` | `[]` | Extra directories to search, besides the apps' `static/` directories. |
| `storages["staticfiles"]` | JSON | `StaticFilesStorage` | Set its `BACKEND` to `...ManifestStaticFilesStorage` for hashed file names. |

```rust
use rango::conf::Settings;

Settings {
    static_url: Some("/static/".into()),
    staticfiles_dirs: vec!["static".into()],
    static_root: Some("/srv/mysite/static".into()),
    ..Settings::default()
}
```

Two details about `static_url`:

- End it with a slash. The template tag concatenates the prefix and the file path without adding one.
- Start it with a slash (or make it a full URL such as `"https://cdn.example.com/static/"`). The value is used as
  written. A relative prefix such as `"static/"` produces URLs relative to the current page, which only resolve
  correctly for pages at the site root.

Relative directories in `staticfiles_dirs` and `static_root` are resolved against the process's working directory,
which is the project root when you use `cargo run`.

The `staticfiles_finders` setting exists but is not read: the two finders described below are always active, in a
fixed order. You do not need `rango.contrib.staticfiles` in `installed_apps` for the finders, the template tag or
the commands to work.

## Where files are found

A file is identified by its path relative to a static directory, for example `polls/style.css`. Rango looks for it
in this order and uses the first match:

1. each directory in `staticfiles_dirs`, in the order listed;
2. the `static/` directory of each installed app that declares a filesystem path.

### Project directories

```rust
staticfiles_dirs: vec!["static".into(), "/opt/shared-assets".into()],
```

With this setting, `static/css/site.css` in the project is the static file `css/site.css`.

A directory can be mounted under a prefix. An entry of the form `"<prefix>\0<directory>"`, with a NUL character
between the two parts, makes every file in the directory available under `<prefix>/`:

```rust
// vendor/bootstrap/css/bootstrap.css  ->  "bootstrap/css/bootstrap.css"
staticfiles_dirs: vec!["static".into(), "bootstrap\u{0}vendor/bootstrap".into()],
```

### App directories

An app's static files live in a `static/` directory under the app's path. The path is not inferred; declare it on
the app config. The project template's `register_app!` call does not set one, so add `.path(...)`:

```rust
// src/polls/mod.rs
rango::register_app!(
    rango::apps::AppConfig::new("polls").path(concat!(env!("CARGO_MANIFEST_DIR"), "/src/polls"))
);
```

```
src/polls/
    mod.rs
    static/
        polls/
            style.css        <- the static file "polls/style.css"
            images/logo.png  <- "polls/images/logo.png"
```

Put each app's files in a subdirectory named after the app, as above. All static directories are merged into one
namespace, and the first match wins, so two apps that both ship `style.css` at the top level would shadow each
other.

`CARGO_MANIFEST_DIR` is an absolute path fixed at compile time. App static directories are therefore found on the
machine where the binary was built, or wherever the source tree exists at the same path. This matters for when you
run `collectstatic`; see [Deployment](deployment.md).

## Referring to static files in templates

Load the `static` library, then use the `{% static %}` tag with the file's relative path:

```html
{% load static %}
<link rel="stylesheet" href="{% static 'polls/style.css' %}">
<img src="{% static 'polls/images/logo.png' %}" alt="Logo">
```

With `static_url` set to `"/static/"` this renders `/static/polls/style.css`. The tag joins `static_url` and the
percent-encoded path; it does not check that the file exists.

| Form | Result |
|---|---|
| `{% static 'css/site.css' %}` | Writes the URL. |
| `{% static path_variable %}` | The path can be a template variable. |
| `{% static 'css/site.css' as site_css %}` | Stores the URL in a variable instead of writing it. |
| `{% get_static_prefix %}` | Writes `static_url` itself. |

The `rango.template.context_processors.static` context processor adds a `STATIC_URL` variable to every template
rendered with a request. `rango.template.context_processors.media` does the same for `MEDIA_URL`.

```rust
"OPTIONS": {
    "context_processors": [
        "rango.template.context_processors.request",
        "rango.template.context_processors.static",
    ],
},
```

## Serving static files during development

`runserver` does not serve static files by itself. Add a route that calls
`rango::contrib::staticfiles::serve`, which looks the path up through the finders, so files are served straight
from `staticfiles_dirs` and the apps without collecting them first:

```rust
use rango::conf::settings;
use rango::contrib::staticfiles;
use rango::http::{HttpRequest, HttpResponse};
use rango::urls::path;
use rango::{register_urlconf, urlpatterns, Result};

async fn static_file(request: HttpRequest, file: String) -> Result<HttpResponse> {
    if !settings().debug {
        return Err(rango::Error::Http404("Not found".into()));
    }
    staticfiles::serve(&request, &file).await
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("static/<path:file>", static_file),
    // ... your other routes ...
]);
```

The route prefix must match `static_url`. `serve` returns `404` when no finder has the file, guesses the
`Content-Type` from the file extension, and reads the whole file on every request without cache headers. It is a
development convenience, not a production file server.

## Finding files from the command line: `findstatic`

`findstatic` prints where a static file comes from. It lists every match, in search order; the first one is the
file that is used.

```console
$ cargo run -- findstatic polls/style.css css/missing.css
Found 'polls/style.css' here:
  /home/me/mysite/src/polls/static/polls/style.css
No matching file found for 'css/missing.css'.
```

It takes one or more paths and has no options of its own beyond the common command options.

## Collecting files for deployment: `collectstatic`

`collectstatic` copies every static file from every source directory into `static_root`, so a web server can serve
them from one place.

```console
$ cargo run -- collectstatic

12 static files copied to '/srv/mysite/static', 3 unmodified.
```

| Option | Meaning |
|---|---|
| `-n`, `--dry-run` | Report what would be copied without writing anything. |
| `-c`, `--clear` | Delete `static_root` before copying. Ignored together with `--dry-run`. |
| `-i PATTERN`, `--ignore PATTERN` | Skip files and directories whose name matches the pattern. Repeatable. |
| `--noinput`, `--no-input` | Accepted for scripts; the command never prompts. |

Behaviour:

- The command stops with an error when `static_root` is not set.
- When the same relative path exists in several source directories, the first one in search order is copied.
- A file is copied only when the destination is missing or its content differs; otherwise it counts as unmodified.
- Names matching `CVS`, `.*` or `*~` are always skipped. `--ignore` adds more patterns. A pattern is matched against
  a single file or directory name, not the whole path, and supports `*` and `?`.
- `--clear` removes the entire `static_root` directory tree. Never point `static_root` at a directory that holds
  anything else, in particular not at a source directory or at `media_root`.
- Files are copied; symbolic links are not created.

```console
$ cargo run -- collectstatic --dry-run --ignore "*.scss" --ignore node_modules
```

## Hashed file names

Long cache lifetimes are safe only if a file's URL changes whenever its content changes. Select the manifest
storage to make `collectstatic` write content-hashed names:

```rust
storages: rango::json!({
    "default": { "BACKEND": "rango.core.files.storage.FileSystemStorage" },
    "staticfiles": { "BACKEND": "rango.contrib.staticfiles.storage.ManifestStaticFilesStorage" },
}),
```

With this storage, `collectstatic`:

- writes each file as `<stem>.<first 12 hex digits of its MD5>.<ext>`, for example
  `css/site.css` becomes `css/site.5f2b8c1d9a04.css`. The file is not also written under its original name;
- writes `staticfiles.json` in `static_root`, mapping original paths to hashed ones.

Resolve the hashed URL in Rust with `StaticStorage`:

```rust
use rango::contrib::staticfiles::StaticStorage;

let storage = StaticStorage::from_settings()?;      // needs static_root
let css_url = storage.url("css/site.css")?;         // "/static/css/site.5f2b8c1d9a04.css"
```

`StaticStorage::url` reads the manifest on each call. It returns the plain URL when the manifest file does not
exist yet (so development works before the first `collectstatic`), and fails with
`Missing staticfiles manifest entry for '<name>'` when the manifest exists but has no entry for the file.

Limits of the manifest storage today:

- The `{% static %}` tag does not consult the manifest. It always produces the unhashed URL, which does not exist in
  `static_root` under this storage. Compute URLs with `StaticStorage::url` in the view and pass them to the template
  context, or keep the default storage.
- References inside CSS files (`url(...)`, `@import`) are not rewritten to hashed names.

## Using the finders from code

```rust
use rango::contrib::staticfiles::Finders;

let finders = Finders::from_settings();

// First match, or every match in search order.
let first: Option<std::path::PathBuf> = finders.find("polls/style.css");
let all = finders.find_all("polls/style.css");

// Every static file: relative path -> absolute source path. Patterns are skipped names.
let files = finders.list(&[".*".to_owned()]);
for (relative, source) in &files {
    println!("{relative} <- {}", source.display());
}
```

`Finders::with_roots(vec![(None, dir), (Some("prefix".into()), other_dir)])` builds a finder over explicit
directories, which is useful in tests. Lookups refuse paths that escape a root directory.

## Development versus production

| | Development | Production |
|---|---|---|
| Source of files | `staticfiles_dirs` and app `static/` directories, through the finders | `static_root`, filled by `collectstatic` |
| Who serves them | A route calling `staticfiles::serve`, guarded by `debug` | The web server, a CDN or object storage, at `static_url` |
| Caching | None | Set by the web server; long lifetimes with hashed names |

A typical deployment runs `collectstatic` as a build step and configures the reverse proxy to map `static_url` to
`static_root`. [Deployment](deployment.md) has a complete example.

## Not supported yet

- Automatic static serving in `runserver`.
- `{% static %}` reading the hashed-name manifest.
- Rewriting `url()` references inside CSS for the manifest storage.
- `collectstatic --link` and a `--first` option for `findstatic`.
- Selecting or replacing finders through `staticfiles_finders`.
- Storage backends other than a local `static_root` directory (for example uploading to object storage).

## See also

- [Tutorial part 6: static files](../tutorial/06-static-files.md)
- [Managing files](files.md) for user uploads and `media_root`
- [The template language](templates/language.md) and the [template tag reference](../ref/template-tags.md)
- [Applications](apps.md) for `AppConfig`
- [Commands](commands.md) and the [command reference](../ref/commands.md)
- [Deployment](deployment.md)
