# Model forms

A `ModelForm<M>` builds a form from a model's fields, binds it to a new or an existing instance, checks uniqueness
against the database and saves the instance together with its many-to-many relations and uploaded files. This page
covers creating and editing, choosing and customising the fields, validation, saving, and what is not supported.

Everything that touches the database is `async` and takes the connection explicitly: choices are loaded when the
form is built, uniqueness is checked in `is_valid(&db)`, and `save(&db)` writes. A model form is therefore never
valid or saved by a synchronous call.

```rust
use rango::db::Db;
use rango::db::models::{ManyToMany, Model};
use rango::forms::{Data, FileData, ModelForm, ModelFormOptions};
```

## A model to work with

The examples use these models.

```rust
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(app_label = "lib", str = "name")]
pub struct Author {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100, unique)]
    pub name: String,
    #[field(blank)]
    pub age: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(app_label = "lib", unique_together = [["title", "published"]])]
pub struct Book {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 200)]
    pub title: String,
    pub authors: ManyToMany<Author>,
    pub published: chrono::NaiveDate,
    #[field(max_digits = 6, decimal_places = 2, default = "9.99")]
    pub price: rust_decimal::Decimal,
}
```

A model must implement `Model`; creating a form for a new row also needs `Default`.

## An add form

`ModelForm::<M>::new(&db, options)` returns a form for a new instance. The inner `Form` is `form.form()`; render it
like any other form.

```rust
let db = Db::default_db();
let form = ModelForm::<Author>::new(&db, ModelFormOptions::all()).await?;
let html = form.form().as_p();
```

Which fields the form has, and how they look, is decided by the [options](#choosing-and-customising-fields). The
instance behind a new form is `M::default()` with the model's `default = ...` values applied; those defaults also
become the fields' initial values (the `price` field above starts at `9.99`).

## Handling a POST

Bind the submitted data with `.bound(data, files)`, then `save` when valid:

```rust
use rango::http::HttpRequest;

async fn create_author(req: &HttpRequest) -> rango::Result<Option<Author>> {
    let db = Db::default_db();
    let data = Data::from_lists(req.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec())));
    let files = req.files().await?.clone();

    let mut form = ModelForm::<Author>::new(&db, ModelFormOptions::all())
        .await?
        .bound(data, files);
    if form.is_valid(&db).await? {
        Ok(Some(form.save(&db).await?.clone()))
    } else {
        // render form.form() again; it carries the errors
        Ok(None)
    }
}
```

`is_valid` returns `Result<bool>`: the `Err` side is a database error, not a validation failure. `save` returns
`Result<&M>`, a reference to the stored instance with its primary key filled in; `form.instance` is the same value.
If the form is not valid, `save` returns an error ("The Author could not be created because the data didn't
validate." or "... changed ...") and writes nothing. Call `is_valid` first and branch on it.

You rarely write this by hand for plain create/edit pages: `CreateView` and `UpdateView` do exactly the above and
take the same `ModelFormOptions`:

```rust
use rango::forms::ModelFormOptions;
use rango::views::generic::{CreateView, UpdateView};

CreateView::<Article>::new(ModelFormOptions::fields(["title", "body", "published"])).success_url("/articles/");
UpdateView::<Article>::new(ModelFormOptions::fields(["title", "body"]));
```

See [generic views](../http/generic-views.md).

## An edit form

`ModelForm::<M>::edit(&db, instance, options)` starts from a loaded instance. Initial values come from the instance;
a many-to-many field starts with the ids currently stored in the join table.

```rust
let author = Author::objects().fetch(&db).await?.into_iter().next().expect("an author");
let mut form = ModelForm::<Author>::edit(&db, author, ModelFormOptions::all())
    .await?
    .bound(data, files);
if form.is_valid(&db).await? {
    form.save(&db).await?;
}
```

`edit` does not need `Default`. Saving an edit form issues an `UPDATE`; saving an add form an `INSERT`. After a
successful `save`, `form.is_new` is `false`, so saving the same form again updates the row instead of inserting
another one.

A blank optional field clears the column: submitting `age=""` for the `Author` above stores `NULL`.

## Choosing and customising fields

`ModelFormOptions` says which model fields appear and overrides the generated fields.

| Constructor or method | Effect |
|---|---|
| `ModelFormOptions::all()` | Every editable field. |
| `ModelFormOptions::fields(["a", "b"])` | Only these fields, **in this order**. Names that are not model fields are ignored. |
| `.exclude(["a"])` | Drop fields (use with `all()`). |
| `.widget("field", Widget)` | Replace the widget of one field. |
| `.label("field", "Text")` | Replace the label. |
| `.help_text("field", "Text")` | Replace the help text. |
| `.error_message("field", "code", "Text")` | Replace one error message of one field. |
| `.field_override("field", Field)` | Replace the generated field completely with your own `Field`. |
| `.choices("field", Vec<Choice>)` | For a relation field, use these choices instead of loading every row of the related table. |

```rust
use rango::forms::Widget;

let opts = ModelFormOptions::fields(["title", "authors", "published"])
    .widget("title", Widget::text_input().with_attrs(rango::forms::attrs! { "class" => "wide" }))
    .label("published", "Publication date")
    .help_text("authors", "Hold Ctrl to select several")
    .error_message("title", "required", "Every book needs a title.");
```

The fields of `ModelFormOptions` are public, so you can also build the struct directly.

To fine-tune the generated form further, `map_form` hands you the inner `Form`:

```rust
let form = ModelForm::<Book>::new(&db, opts)
    .await?
    .map_form(|f| {
        f.prefix("book").clean_field("title", |_form, value| Ok(value.clone()))
    });
```

Use it to add per-field hooks, a form-wide `clean`, or a `prefix` (see [validation](validation.md)).

### How model fields become form fields

| Model field | Form field | Notes |
|---|---|---|
| auto primary key, non-editable fields | none | |
| `Char` | `Field::char()` | `max_length` carried over; a nullable column cleans empty input to `None`. |
| `Text` | `Field::char()` with a `Textarea` | Same `max_length` and `None` rules. |
| `Boolean` | `Field::boolean()` | Never required. A nullable column gives `Field::null_boolean()`. |
| integer types | `Field::integer()` | `min_value`/`max_value` set from the column's range (for example 0 to 32767 for a positive small integer). |
| `Float` | `Field::float()` | |
| `Decimal` | `Field::decimal()` | `max_digits` and `decimal_places` carried over. |
| `Date`, `DateTime`, `Time`, `Duration` | the matching field | |
| `Email`, `Url`, `Slug` | `Field::email()`, `url()`, `slug()` | `max_length` carried over. |
| `Uuid`, `Json` | `Field::uuid()`, `Field::json()` | |
| `GenericIpAddress` | `Field::generic_ip_address("both", false)` | |
| `File`, `Image` | `Field::file()`, `Field::image()` | See [file uploads](#file-uploads). |
| `Binary` | none | Binary columns are never offered. |
| any field with `choices` | a typed choice field | Integer columns coerce the submitted text back to an integer; a blank entry `---------` is added first when the field is `blank` or has no default. |
| `ForeignKey`, `OneToOne` | `Field::model_choice(false)` | One entry per row of the related table; a `---------` entry is added first when the field is optional or has no default. |
| `ManyToMany` | `Field::model_choice(true)` | A multiple select. |

For every field: `required` is the opposite of `blank`; the label is the model field's verbose name (or its name with
underscores turned into spaces) with the first letter capitalised; `help_text` is carried over; a `default` becomes
the initial value, and a default that is a function also sets `show_hidden_initial`.

Reverse relations are never form fields. Many-to-many fields are included by name like any other field.

### Relation choices

Choices are read when the form is built, with one query `SELECT <pk>, <label> FROM <table> ORDER BY ...`. The label
is the column named by the related model's `#[model(str = "...")]`; without it the label is `"<Model> object (<pk>)"`.
Ordering follows the related model's `ordering`, otherwise the primary key. A submitted value that is not among the
loaded choices is rejected with code `invalid_choice`:

```
Select a valid choice. 9 is not one of the available choices.
```

For large tables supply a restricted list yourself instead of loading everything:

```rust
use rango::forms::Choice;

let few = vec![Choice::option(1_i64, "Ann"), Choice::option(2_i64, "Bob")];
let opts = ModelFormOptions::all().choices("authors", few);
```

## Validation and uniqueness

`ModelForm::is_valid(&db)` does three things in order:

1. It runs the ordinary form validation. If that fails, it returns `false`.
2. The first time it runs after binding, it copies the cleaned values into a candidate instance, calls the
   `model_clean` hook if there is one, and checks uniqueness.
3. It returns whether the form has no errors.

Step 2 happens once per binding; calling `is_valid` again does not repeat the queries.

### Model-level checks

Register a check that sees the whole candidate instance, not single fields:

```rust
use rango::core::exceptions::ValidationError;

let mut form = ModelForm::<Book>::new(&db, ModelFormOptions::all())
    .await?
    .model_clean(|book: &mut Book| {
        if book.title.trim().is_empty() {
            Err(ValidationError::new("A book needs a title.").with_code("no_title"))
        } else {
            Ok(())
        }
    })
    .bound(data, files);
```

The closure receives `&mut M`, so it may also normalise values. An error it returns becomes a form-wide (non-field)
error. The closure runs only after field validation succeeded.

### Uniqueness

The check queries the table with a `SELECT 1 ... LIMIT 1`:

- Every column declared `unique` (except the primary key) is checked on its own. The error is attached to that field
  with code `unique` and the message `Author with this Name already exists.`
- Every group in `unique_together` is checked as a unit. The error is a form-wide error with code `unique_together`
  and the message `Book with this Title and Published already exists.`
- When editing, the instance's own row is excluded.
- A check is skipped when any of its fields is missing from the form (excluded or left out of `fields`) or when a
  value is `NULL`.

The model name in the message is the model's verbose name with the first letter capitalised; field names in the
message are the fields' labels.

```rust
let mut dup = ModelForm::<Author>::new(&db, ModelFormOptions::all())
    .await?
    .bound(data(&[("name", &["Ann"])]), FileData::new());
assert!(!dup.is_valid(&db).await?);
assert_eq!(
    dup.form().errors().get("name").unwrap().messages(),
    vec!["Author with this Name already exists."]
);
```

The check is a read followed by a later write, so two concurrent requests can both pass it; keep a database
`UNIQUE` constraint on the column as well.

## Many-to-many fields

A many-to-many field is a multiple select named after the model field. Its cleaned value is a list of keys.
`save` first inserts or updates the row, then replaces the whole relation with the submitted keys:

```rust
let post = data(&[
    ("title", &["Rust"]),
    ("authors", &["1", "2"]),
    ("published", &["2024-05-06"]),
    ("price", &["12.50"]),
]);
let mut f = ModelForm::<Book>::new(&db, ModelFormOptions::all()).await?.bound(post, FileData::new());
let book = f.save(&db).await?.clone();
```

Leaving the field out of `ModelFormOptions::fields(...)` leaves the relation untouched.

## File uploads

A `File` or `Image` model field becomes a clearable file input. The form must be rendered with
`enctype="multipart/form-data"` (`form.form().is_multipart()` tells you) and bound with the request's files:

```rust
let files = req.files().await?.clone();
let mut form = ModelForm::<Doc>::new(&db, ModelFormOptions::all()).await?.bound(data, files);
```

`save` then:

1. takes the uploaded file name, drops any directory part and makes it safe (`my report.txt` becomes
   `my_report.txt`);
2. prefixes the field's `upload_to` directory (`docs/my_report.txt`);
3. stores the bytes through the default file storage, which renames the file when the name is taken
   (`docs/my_report_<random>.txt`);
4. writes the stored name into the column.

When no file is uploaded the stored name is left unchanged. Ticking the "Clear" box on an optional field stores an
empty string. See [managing files](../files.md) for storage configuration and `media_root`.

File names are only written in `save`, after validation; `form.instance` after `is_valid` therefore still holds the
previous name.

## Saving details

- `save` writes the row, then the many-to-many relations, as separate statements. The code does not open a
  transaction; wrap the call in one when partial writes would be a problem (see [transactions](../db/transactions.md)).
- Fields not in the form keep the instance's current values (for a new form, the model defaults).
- A field that is absent from the submitted data, has a model default and cleans to an empty value keeps the instance's current value (the default for a new form) instead of becoming empty.
- `form.construct_instance()` returns a new `M` built from the cleaned data without touching the database, which is
  the way to inspect or adjust the instance before storing it yourself. It does not store files or many-to-many
  relations.

## Not supported yet

- There is no `commit = false` save that defers only the many-to-many step. Use `construct_instance()` and write the
  relation yourself.
- Uniqueness checks cover `unique` columns and `unique_together` only: database constraints declared another way and
  date-scoped uniqueness are not checked in the form.
- The related-table choices are plain `(key, label)` pairs; there is no queryset-based filtering other than passing
  your own `choices`.
- Reverse relations and `Binary` columns cannot be edited through a model form.

## See also

- [Forms](forms.md), [validation](validation.md) and [rendering](rendering.md)
- [Formsets](formsets.md), including `ModelFormSet`
- [Form fields reference](../../ref/form-fields.md) and [widgets reference](../../ref/widgets.md)
- [Generic views](../http/generic-views.md) and [tutorial part 4](../../tutorial/04-forms-generic-views.md)
- [Models](../db/models.md) and [relations](../db/relations.md)
