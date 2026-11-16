# Managing files

This page covers user-uploaded and generated files ("media"): the `Storage` trait, the built-in
`FileSystemStorage`, how names are chosen, how file columns on models use storage, and how to serve media during
development. Files that ship with your code (CSS, JavaScript, images) are covered in
[Static files](static-files.md).

## Importing the files API

The files module lives in the core crate, which the `rango` facade does not re-export at the moment. Add the core
crate next to `rangoframework`, at the same version:

```toml
[dependencies]
rangoframework = "0.1"
rangoframework-core = "0.1"   # library name: rango_core
```

```rust
use rango_core::files::{default_storage, get_valid_filename, ContentFile, FileSystemStorage, Storage};
```

`Storage` is a trait; bring it into scope to call its methods on a storage value.

## Settings

| Setting | Default | Meaning |
|---|---|---|
| `media_root` | `""` | Directory on disk where the default storage keeps files. Must be set before anything is saved. |
| `media_url` | `""` | URL prefix that serves `media_root`, for example `"/media/"`. Needed for `url()`. |

```rust
use rango::conf::Settings;

Settings {
    media_root: "/srv/mysite/media".into(),
    media_url: "/media/".into(),
    ..Settings::default()
}
```

Keep `media_root` outside your source tree and separate from `static_root`. Uploaded files are untrusted input;
they should never land in a directory your web server treats as code or as your own static assets.

## The default storage

`default_storage()` returns an `Arc<dyn Storage>`: a `FileSystemStorage` rooted at `media_root` with `media_url` as
its base URL. It is what model forms use to save uploads.

```rust
use rango_core::files::default_storage;

async fn save_avatar(user_id: i64, bytes: &[u8]) -> rango::Result<String> {
    let storage = default_storage();
    let name = storage.save(&format!("avatars/{user_id}.png"), bytes).await?;
    // `name` is what you store in the database, e.g. "avatars/42.png"
    Ok(name)
}
```

## The `Storage` trait

Every method that touches the backend is `async` and returns `rango::Result`.

| Method | Returns | Meaning |
|---|---|---|
| `save(name, content: &[u8])` | `String` | Stores the bytes under `name`, or under a free variant of it, and returns the name actually used. |
| `open(name)` | `Vec<u8>` | The whole file content. |
| `delete(name)` | `()` | Removes the file. Deleting a file that does not exist is not an error. |
| `exists(name)` | `bool` | Whether a file with this name exists. |
| `size(name)` | `u64` | Size in bytes. |
| `listdir(path)` | `(Vec<String>, Vec<String>)` | Directory names and file names directly under `path`, each sorted. |
| `url(name)` | `String` | Public URL of the file (not `async`). |
| `path(name)` | `PathBuf` | Filesystem path, for storages that have one (not `async`). The trait default returns an error. |
| `get_available_name(name)` | `String` | A name that does not collide with an existing file. |

Names are always relative to the storage root and use `/` as the separator, on every platform.

```rust
use rango_core::files::{FileSystemStorage, Storage};

let st = FileSystemStorage::new("/srv/mysite/media", "/media/");

let n1 = st.save("docs/a b.txt", b"hello").await?;
assert_eq!(n1, "docs/a b.txt");
assert_eq!(st.open(&n1).await?, b"hello");
assert_eq!(st.url("docs/a b.txt")?, "/media/docs/a%20b.txt");
assert_eq!(st.size(&n1).await?, 5);

let (dirs, files) = st.listdir("docs").await?;
assert!(dirs.is_empty());
assert_eq!(files, vec!["a b.txt"]);

st.delete(&n1).await?;
assert!(!st.exists(&n1).await?);
```

## `FileSystemStorage`

```rust
pub struct FileSystemStorage {
    pub location: PathBuf,   // root directory
    pub base_url: String,    // URL prefix
}
```

| Constructor | Meaning |
|---|---|
| `FileSystemStorage::new(location, base_url)` | A storage rooted at any directory. |
| `FileSystemStorage::media()` | `location` and `base_url` taken from `media_root` and `media_url`. |

Behaviour to know about:

- `save` creates missing parent directories and converts backslashes in the name to `/`.
- `url` joins `base_url` and the percent-encoded name with exactly one `/` between them. It fails with
  `This file is not accessible via a URL.` when `base_url` is empty.
- `path` returns the absolute path of a name, after the same safety check as every other method.
- Every name is resolved inside `location`. A name that escapes it (`../secret`, an absolute path) is refused with
  `Error::Suspicious` of kind `SuspiciousFileOperation`. If such an error propagates out of a view, the client gets
  a `400 Bad Request`.

```rust
assert!(st.save("../escape.txt", b"x").await.is_err());
```

- File I/O is synchronous `std::fs` behind the `async` signatures, and `open` returns the whole file in memory. For
  large files, get the filesystem path with `path(name)` and stream it yourself (for example with a
  `rango::http::FileResponse`).
- The `file_upload_permissions` and `file_upload_directory_permissions` settings are not applied; files and
  directories are created with the process's default permissions (umask).

A second storage for a different purpose is one constructor call:

```rust
let exports = FileSystemStorage::new("/srv/mysite/exports", "");   // no public URL
let name = exports.save("2024/report.csv", csv.as_bytes()).await?;
let on_disk = exports.path(&name)?;
```

## How names are chosen

### Collisions: `get_available_name`

`save` never overwrites. If the name is taken, `get_available_name` inserts an underscore and seven random
alphanumeric characters before the extension, and repeats until the name is free:

```rust
let n1 = st.save("docs/a b.txt", b"hello").await?;   // "docs/a b.txt"
let n2 = st.save("docs/a b.txt", b"again").await?;   // e.g. "docs/a b_x7Kq2Lm.txt"
assert!(n2.starts_with("docs/a b_") && n2.ends_with(".txt") && n2 != n1);
```

Always keep the name `save` returns; it can differ from the one you asked for. To replace a file on purpose, delete
the old one first.

A custom `Storage` can override `get_available_name` to apply a different policy.

### Cleaning user-supplied names: `get_valid_filename`

`get_valid_filename(name)` turns an arbitrary string into a safe file name: surrounding whitespace is trimmed,
spaces become underscores, and every character that is not alphanumeric, `-`, `_` or `.` is removed.

```rust
use rango_core::files::get_valid_filename;

assert_eq!(get_valid_filename("john's portrait in 2004.jpg")?, "johns_portrait_in_2004.jpg");
assert_eq!(get_valid_filename("  a b  ")?, "a_b");
assert!(get_valid_filename("").is_err());      // nothing usable left
assert!(get_valid_filename("..").is_err());
```

It works on a single path component. Take the last component of a client-supplied path before calling it, then join
it to a directory you control:

```rust
let base = upload.name.rsplit(['/', '\\']).next().unwrap_or(&upload.name);
let dest = format!("invoices/{}", get_valid_filename(base)?);
let stored = default_storage().save(&dest, &upload.read().map_err(|e| rango::Error::Other(Box::new(e)))?).await?;
```

`upload` here is a `rango::http::UploadedFile`; see [File uploads](http/file-uploads.md).

## `ContentFile`

`ContentFile` pairs bytes with an optional name. It is a plain value type for passing generated content around
before it is saved.

```rust
use rango_core::files::ContentFile;

let report = ContentFile::named("report.txt", "generated text");
assert_eq!(report.size(), 14);
assert_eq!(report.name.as_deref(), Some("report.txt"));

let anonymous = ContentFile::new(vec![0u8; 16]);
assert!(anonymous.name.is_none());

let stored = default_storage().save("reports/report.txt", &report.content).await?;
```

Both fields (`name: Option<String>`, `content: Vec<u8>`) are public. `Storage::save` takes the name and the bytes
separately. There is no open-file handle type: `open` returns bytes.

## File and image columns on models

A model column declared with `#[field(file, ...)]` or `#[field(image, ...)]` is a `String` holding the name of a
file in the default storage. The database stores only that name; the bytes live in storage.

```rust
use rango::db::models::Model;

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(app_label = "lib")]
pub struct Doc {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100)]
    pub title: String,
    #[field(file, max_length = 100, upload_to = "docs", blank)]
    pub attachment: String,
}
```

- `upload_to = "docs"` is a fixed directory under the storage root. It is used literally; it is not a date pattern
  and cannot be a function.
- `max_length` bounds the stored name, directory included.
- An empty string means "no file".

### Saving through a model form

`ModelForm::save` is what moves an upload into storage. For each file or image field with an uploaded file it:

1. takes the last path component of the client's file name and cleans it with `get_valid_filename`;
2. prefixes the `upload_to` directory;
3. calls `default_storage().save(...)`, which may rename on collision;
4. writes the returned name into the column.

With `media_root` configured, uploading `my report.txt` twice to the model above stores
`docs/my_report.txt` and then something like `docs/my_report_Ab3dE9x.txt`. Ticking the "clear" checkbox of the
widget sets the column to an empty string; it does not delete the old file from storage.

See [Model forms](forms/modelforms.md) for the form side and [File uploads](http/file-uploads.md) for reading
`request.files()` yourself.

### Using the stored name

Ask storage for the URL or the bytes:

```rust
use rango_core::files::default_storage;

let doc = Doc::objects().get(&db).await?;       // narrowed by a filter in real code
if !doc.attachment.is_empty() {
    let url = default_storage().url(&doc.attachment)?;      // "/media/docs/my_report.txt"
    let bytes = default_storage().open(&doc.attachment).await?;
}
```

Deleting a model row does not delete its file. Call `default_storage().delete(&doc.attachment)` yourself when the
file should go too.

### Saving without a form

Nothing is saved automatically outside `ModelForm`. Save to storage, then put the returned name in the field:

```rust
let name = default_storage().save("docs/generated.txt", b"hello").await?;
let doc = Doc::objects()
    .create(&db, Doc { id: None, title: "Generated".into(), attachment: name })
    .await?;
```

### Image columns

`#[field(image, ...)]` behaves like a file column. Rango does not open the image: width and height are not read or
stored, and the content is not verified to be an image. Validate uploads yourself if that matters (for example with
a file-extension validator; see [Validators](validators.md)).

## Serving media in development

`runserver` does not serve `media_root` on its own. Add a route that uses `rango::views::static_serve::serve`, and
only when `debug` is on:

```rust
use std::path::Path;

use rango::conf::settings;
use rango::http::{HttpRequest, HttpResponse};
use rango::urls::path;
use rango::views::static_serve::serve;
use rango::{register_urlconf, urlpatterns, Result};

async fn media(request: HttpRequest, file: String) -> Result<HttpResponse> {
    if !settings().debug {
        return Err(rango::Error::Http404("Not found".into()));
    }
    serve(&request, &file, Path::new(&settings().media_root), false).await
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("media/<path:file>", media),
    // ... your other routes ...
]);
```

`serve(request, path, document_root, show_indexes)` normalises the path, refuses anything outside
`document_root`, answers `304 Not Modified` to a matching `If-Modified-Since`, sets `Last-Modified`, and returns
`404` for missing files. With `show_indexes` set to `true` it renders a plain listing for directories; otherwise a
directory is a `404`.

This view exists for convenience during development. In production let the web server or a CDN serve `media_root`
at `media_url`; see [Deployment](deployment.md).

## Writing a storage backend

Implement `Storage` for your own type to keep files somewhere else. The required methods are `save`, `open`,
`delete`, `exists`, `size`, `listdir` and `url`; `path` and `get_available_name` have defaults.

```rust
use async_trait::async_trait;
use rango_core::files::Storage;
use rango_core::Result;

pub struct BucketStorage { /* client, bucket name, public base URL */ }

#[async_trait]
impl Storage for BucketStorage {
    async fn save(&self, name: &str, content: &[u8]) -> Result<String> {
        let name = self.get_available_name(name).await?;
        // upload `content` under `name` ...
        Ok(name)
    }
    async fn open(&self, name: &str) -> Result<Vec<u8>> { todo!() }
    async fn delete(&self, name: &str) -> Result<()> { todo!() }
    async fn exists(&self, name: &str) -> Result<bool> { todo!() }
    async fn size(&self, name: &str) -> Result<u64> { todo!() }
    async fn listdir(&self, path: &str) -> Result<(Vec<String>, Vec<String>)> { todo!() }
    fn url(&self, name: &str) -> Result<String> { todo!() }
}
```

Use such a storage by constructing it and calling it directly. It cannot replace the default storage; see below.

## The `storages` setting

`Settings` has a `storages` field with two aliases:

```rust
storages: rango::json!({
    "default": { "BACKEND": "rango.core.files.storage.FileSystemStorage" },
    "staticfiles": { "BACKEND": "rango.contrib.staticfiles.storage.StaticFilesStorage" },
}),
```

Only the `staticfiles` entry has an effect today: setting its `BACKEND` to a name ending in
`ManifestStaticFilesStorage` turns on hashed file names in `collectstatic` (see [Static files](static-files.md)).
The `default` entry is not read. `default_storage()` always returns a `FileSystemStorage` built from `media_root`
and `media_url`, and model forms always save through it.

## Not supported yet

- Selecting the default storage through `storages["default"]`, or registering custom storage backends by name.
- Streaming reads and writes; content is passed as whole byte buffers, and a request body is read into memory
  before its uploads are parsed.
- Image dimensions (`width_field`/`height_field`) and image content verification.
- `upload_to` as a date pattern or a function.
- `file_upload_permissions`, `file_upload_directory_permissions` and custom upload handlers (`file_upload_handlers`).
- Deleting stored files when a row is deleted or a field is cleared.
- A limit on the length of names produced by `get_available_name`.

## See also

- [File uploads](http/file-uploads.md)
- [Model forms](forms/modelforms.md)
- [Model field reference](../ref/model-fields.md)
- [Static files](static-files.md)
- [Validators](validators.md)
- [Deployment](deployment.md)
- [Security](security.md)
