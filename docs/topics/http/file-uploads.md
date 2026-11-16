# File uploads

This page explains how a request's uploaded files reach a view: how a form must be written, what `request.files()`
returns, how big uploads are handled, which settings limit them, and how to validate and store the files. Storage
itself is covered in [Managing files](../files.md).

## The HTML form

Browsers only send files when the form uses the `multipart/form-data` encoding and the `POST` method:

```html
<form method="post" enctype="multipart/form-data" action="{% url 'docs:upload' %}">
    {% csrf_token %}
    <input type="text" name="title">
    <input type="file" name="attachment">
    <button>Upload</button>
</form>
```

If you render a Rango form, `form.is_multipart` tells the template whether the form has a file field:

```html
<form method="post"{% if form.is_multipart %} enctype="multipart/form-data"{% endif %}>
```

Without the `enctype` the browser sends only the file name, no file is uploaded, and a required file field fails with
"No file was submitted. Check the encoding type on the form."

## Reading files in a view

`HttpRequest::files()` is an `async` method returning `&Files`, a multi-value dictionary from the field name to
`UploadedFile` values. The text fields of the same form are in `request.post()`; both come from the same parse.

```rust
use rango::http::{HttpRequest, HttpResponse};
use rango::Result;

pub async fn upload(request: HttpRequest) -> Result<HttpResponse> {
    if request.method() != "POST" {
        return Ok(HttpResponse::not_allowed(&["POST"], ""));
    }
    let title = request.post().await?.get("title").unwrap_or("untitled").to_owned();
    let files = request.files().await?;
    let Some(file) = files.get("attachment") else {
        return Ok(HttpResponse::bad_request("no file"));
    };
    Ok(HttpResponse::new(format!("{title}: {} ({} bytes)", file.name, file.size)))
}
```

Facts about parsing:

- Files are parsed only for `POST` requests whose content type is `multipart/form-data`. For other methods (`PUT`,
  `PATCH`) and other content types, `files()` is empty. For `application/x-www-form-urlencoded` bodies `post()` has the
  fields and `files()` is empty.
- A multipart part with a file name is a file; a part without one is a text field and goes to `post()`.
- A part whose file name is empty, which is what a browser sends for an untouched file input, is skipped. Treat a
  missing key in `files()` as "no file chosen".
- A missing or malformed multipart boundary is a 400 response.
- The body is parsed once and cached, so calling `post()` and `files()` repeatedly is cheap.

### Several files

`Files` is a `MultiValueDict<UploadedFile>`. A field with the `multiple` attribute gives several values under one
name:

```rust
for file in request.files().await?.getlist("photos") {
    println!("{} {}", file.name, file.size);
}
```

| Method | Returns |
|---|---|
| `get(name)` | `Option<&UploadedFile>`, the last file of that name. |
| `getlist(name)` | `&[UploadedFile]`, all files of that name (empty slice if none). |
| `contains_key(name)`, `keys()`, `len()`, `is_empty()` | Inspection. |
| `items()` | Iterator of `(name, &UploadedFile)`. |
| `lists()` | Iterator of `(name, &[UploadedFile])`. |

## `UploadedFile`

```rust
use rango::http::UploadedFile;
```

| Item | Type | Meaning |
|---|---|---|
| `name` | `String` | The file name sent by the client, reduced to its last path component and cut to 255 characters (the extension is kept). Still untrusted: it may contain spaces, dots or odd characters. |
| `content_type` | `Option<String>` | The media type the client claimed (`image/png`). Not verified. |
| `size` | `u64` | Size in bytes. |
| `charset` | `Option<String>` | The `charset` parameter of the part's content type, if any. |
| `content_type_extra` | `IndexMap<String, String>` | Reserved; currently empty. |
| `read()` | `std::io::Result<Vec<u8>>` | The whole content. |
| `temporary_file_path()` | `Option<&Path>` | Path of the temp file for large uploads; `None` for uploads kept in memory. |
| `UploadedFile::from_bytes(name, content_type, bytes)` | `UploadedFile` | Build one by hand, for tests or generated content. |

`UploadedFile` is cheap to clone: the content is shared.

Never trust `name` or `content_type`. Pick the stored name yourself, or pass the name through
`get_valid_filename` ([Managing files](../files.md)), and check the content if the type matters.

## Small and large uploads

Uploads up to `file_upload_max_memory_size` bytes (default 2621440, 2.5 MiB) are kept in memory. A larger file is
streamed into a temporary file named `rango_upload_<random>.upload` in `file_upload_temp_dir` (the operating system's
temporary directory when it is `None`). The temporary file is removed when the last clone of the `UploadedFile` is
dropped, which normally means at the end of the request. `read()` works for both kinds, but for a large file prefer
the path:

```rust
if let Some(path) = file.temporary_file_path() {
    std::fs::copy(path, "/srv/data/incoming.bin")?;
} else {
    std::fs::write("/srv/data/incoming.bin", file.read()?)?;
}
```

Important: the web server reads the whole request body into memory before the view runs, and the multipart parser
works from that buffer. The temp-file threshold keeps *parsed* files out of memory, but a very large upload still has
to fit in RAM first. Limit upload sizes at a reverse proxy (for example `client_max_body_size` in nginx).

## Limits

| Setting | Default | Meaning |
|---|---|---|
| `data_upload_max_number_files` | `100` | Maximum file parts in one request. `0` disables the check. |
| `data_upload_max_number_fields` | `1000` | Maximum non-file parts (and, for urlencoded bodies, parameters). `0` disables it. |
| `data_upload_max_memory_size` | `2621440` | Cap on the total size of the non-file fields of a multipart request, and on the body of non-multipart requests. `0` disables it. File parts do not count. |
| `file_upload_max_memory_size` | `2621440` | The in-memory threshold above. |
| `file_upload_temp_dir` | `None` | Where large uploads are spooled. |

Exceeding one of the three `data_upload_*` limits fails the request as a suspicious operation, which is answered with
400 (the failure reason is shown only when `debug` is on). Because file contents are exempt from
`data_upload_max_memory_size`, no setting limits the total size of file data; check `file.size` yourself or limit the
body at the proxy.

The settings `file_upload_handlers`, `file_upload_permissions` and `file_upload_directory_permissions` exist but are
not used: there are no custom upload handlers and permissions of saved files are the default of the process.

## Using forms

A form field `Field::file()` (or `Field::image()`) cleans to `Value::File(UploadedFile)`. Bind the request's files
along with its text data:

```rust
use rango::forms::{Data, Field, Form, Value};
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::render;
use rango::{Result, context};

fn upload_form() -> Form {
    Form::new("Upload")
        .field("title", Field::char().max_length(100))
        .field("attachment", Field::file().max_length(80))
}

pub async fn upload(request: HttpRequest) -> Result<HttpResponse> {
    let mut form = upload_form();
    if request.method() == "POST" {
        let data = Data::from_lists(request.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec())));
        form = form.bound(data, request.files().await?.clone());
        if form.is_valid() {
            if let Some(Value::File(file)) = form.cleaned_data().get("attachment") {
                // store file.read()? ...
                let _ = file.size;
            }
            return rango::shortcuts::redirect("docs:index");
        }
    }
    render(&request, "docs/upload.html", context! { "form" => form.to_template() })
}
```

What the field checks:

- `Field::file()` fails with "No file was submitted." when required and nothing was chosen, with "The submitted file is
  empty." for a zero-byte file (allow it with `.allow_empty_file(true)`), and with the filename-length message when
  `max_length(n)` is set and the **file name** is longer than `n`.
- `Field::image()` adds the same checks plus: the content must start with the signature of a common image format
  (PNG, JPEG, GIF, BMP, WebP, TIFF or ICO), and the name must have an image extension. Otherwise: "Upload a valid
  image. The file you uploaded was either not an image or a corrupted image." This is a quick signature check, not a
  full decode.
- Neither checks the size of the content. Add a validator for that, or for allowed extensions:

```rust
use rango::forms::{validators, Field};

let mut doc = Field::file();
doc.validators.push(validators::file_extension(&["pdf", "txt"]));
let doc = doc.validator(|v| match v {
    rango::forms::Value::File(f) if f.size > 5 * 1024 * 1024 => {
        Err(rango::core::exceptions::ValidationError::new("The file is larger than 5 MiB."))
    }
    _ => Ok(()),
});
```

## Saving files

With a model that has a file column and a model form, `ModelForm::save` writes the upload into storage under the
column's `upload_to` directory and stores the resulting name in the column; see
[Managing files](../files.md#saving-through-a-model-form). The generic `CreateView` and `UpdateView` do this for you
([Generic views](generic-views.md)).

To save a file yourself, use the storage API from the core crate, which the `rango` facade does not re-export (see
[Managing files](../files.md) for the dependency line):

```rust
use rango_core::files::{default_storage, get_valid_filename, Storage};

let base = file.name.rsplit(['/', '\\']).next().unwrap_or(&file.name);
let dest = format!("invoices/{}", get_valid_filename(base)?);
let stored = default_storage().save(&dest, &file.read().map_err(|e| rango::Error::Other(Box::new(e)))?).await?;
```

`save` returns the name actually used; it adds a suffix when the file already exists.

## CSRF and uploads

Multipart `POST` requests are subject to the same [CSRF check](middleware.md#csrf-protection) as other forms. Include
`{% csrf_token %}` in the form. For scripted uploads, send the token in the `X-CSRFToken` header, because the form
field is read from the parsed body only after the origin checks.

## Testing uploads

`rango::test::Client::post_multipart` builds a multipart body and sends it through the whole middleware chain:

```rust
use rango::test::{Client, FilePart};

let client = Client::new()?;
let response = client
    .post_multipart(
        "/upload/",
        &[("title", "Report")],
        &[FilePart { field: "attachment", filename: "report.txt", content_type: "text/plain", data: b"hello" }],
    )
    .await?;
assert_eq!(response.status_code, 200);
```

The client does not enforce CSRF unless you call `.enforce_csrf_checks(true)`. `Client::post(path, data)` also sends
multipart bodies, without files.

## Not supported yet

- Streaming a request body to disk as it arrives. The whole request body is buffered in memory first.
- Custom upload handlers and the `file_upload_handlers` setting; upload progress.
- `file_upload_permissions` and `file_upload_directory_permissions` on saved files.
- Reading `files()` for `PUT` and `PATCH` requests.
- Full image decoding or dimension checks in `Field::image()`.

## See also

- [Requests and responses](requests-responses.md)
- [Managing files](../files.md)
- [Forms](../forms/forms.md)
- [Model forms](../forms/modelforms.md)
- [Middleware](middleware.md)
- [Settings](../settings.md)
- [Testing](../testing.md)
