# Working with forms

A form describes a set of input fields, turns submitted text into typed values, collects validation errors and
renders itself as HTML. This page covers the life of a form: defining it, binding it to submitted data, validating,
reading the cleaned values, and wiring it into a view.

## Forms are values

A `Form` is an ordinary value. You build it with a chain of builder calls, then either render it as it is (an
*unbound* form) or attach submitted data with `.bound(data, files)` (a *bound* form). There is no form subclass:
the same value carries the field definitions, the per-request data and the validation result. `Form` is `Clone`, so
a function that returns a fresh form per request is the usual pattern.

```rust
use rango::forms::{Field, Form, Widget};

fn contact_form() -> Form {
    Form::new("ContactForm")
        .field("name", Field::char().max_length(100))
        .field("email", Field::email().label("Your e-mail"))
        .field("message", Field::char().widget(Widget::textarea()))
        .field("cc_myself", Field::boolean().required(false))
}
```

`Form::new(name)` takes a display name (used in error messages and in the `Debug` output). `.field(name, field)`
adds a field; fields render and validate in insertion order. Every `Field` constructor and option is listed in the
[form fields reference](../../ref/form-fields.md).

## Defining a form with `#[derive(Form)]`

For forms whose result maps onto a struct, derive the definition from the struct. The macro generates two
associated functions:

- `Contact::form() -> Form` — the unbound form definition;
- `Contact::from_form(&form) -> Option<Contact>` — the typed result, `None` while the form is unbound or invalid.

```rust
use rango::forms::Form;

#[derive(Debug, Form)]
#[form(crate = "rango::forms")]
struct Contact {
    #[form(max_length = 100)]
    name: String,
    #[form(email, label = "Your e-mail")]
    email: String,
    #[form(widget = "textarea", help_text = "Be nice")]
    message: String,
    cc_myself: bool,
    #[form(min_value = 18)]
    age: Option<u32>,
    #[form(choices = [("a", "Alpha"), ("b", "Beta")])]
    pick: String,
    #[form(choices = [("x", "X"), ("y", "Y")])]
    tags: Vec<String>,
}
```

`use rango::forms::Form` imports both the `Form` type and the derive macro.

The field type picks the form field (`String` is a text field, `bool` a checkbox, integers a number field, and so
on) and `Option<T>` makes the field optional. `bool` fields are optional too, because an unchecked box is a valid
answer; write `#[form(required = true)]` for a box that must be ticked. The complete attribute table and the
type mapping are in the [form fields reference](../../ref/form-fields.md#the-formattribute).

> **The `crate` attribute.** The macro refers to the forms crate as `::rango_forms` unless told otherwise. A project
> that depends only on `rangoframework` must add `#[form(crate = "rango::forms")]` to the struct, as above.

The derived form is a plain `Form`, so everything on this page applies to it. You can also extend it before use:

```rust
let form = Contact::form()
    .prefix("contact")
    .clean(|c| {
        // cross-field validation, see validation.md
        Ok(())
    });
```

## Bound and unbound forms

| | Unbound | Bound |
|---|---|---|
| Created by | `contact_form()` | `contact_form().bound(data, files)` |
| `form.is_bound` | `false` | `true` |
| `form.is_valid()` | always `false` | runs validation |
| `form.errors()` | empty | the validation errors |
| `form.cleaned_data()` | empty | the cleaned values of the valid fields |
| Rendered widgets show | initial values | the submitted values |

`bound` takes the submitted text values and the uploaded files:

```rust
use rango::forms::{Data, FileData};

let data = Data::from_lists([
    ("name".to_owned(), vec!["Ann".to_owned()]),
    ("email".to_owned(), vec!["ann@example.com".to_owned()]),
    ("message".to_owned(), vec!["hi".to_owned()]),
]);
let form = contact_form().bound(data, FileData::new());
assert!(form.is_valid());
```

`Data` is a multi-value dictionary of strings (`MultiValueDict<String>`) and `FileData` is the same structure
holding `rango::http::UploadedFile` values. Both have `new()`, `from_lists(iter of (String, Vec<V>))` and
`from_pairs(iter of (String, V))`. `form.bound_data(data)` is a shortcut for `form.bound(data, FileData::new())`.

Binding again replaces the data and discards the previous validation result.

### Getting the data from a request

`HttpRequest::post()` returns the parsed request body and `HttpRequest::files()` the uploaded files. Convert the
first into `Data`; the second already has the right type:

```rust
use rango::forms::Data;
use rango::http::HttpRequest;

async fn bind(req: &HttpRequest, form: rango::forms::Form) -> rango::Result<rango::forms::Form> {
    let data = Data::from_lists(req.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec())));
    let files = req.files().await?.clone();
    Ok(form.bound(data, files))
}
```

The same conversion is available as `rango::contrib::auth::forms::post_data(&req)`. The generic views use an
identical private helper; it is not exported from `rango::views::generic`.

## Validating and reading the result

Validation runs once, the first time `is_valid()`, `errors()` or `cleaned_data()` is called on a bound form, and
the result is cached in the form.

```rust
let form = contact_form().bound(data, FileData::new());
if form.is_valid() {
    let cleaned = form.cleaned_data();              // &IndexMap<String, Value>
    let name = cleaned["name"].as_str().unwrap_or_default();
    let cc = cleaned["cc_myself"].as_bool().unwrap_or(false);
    // ...
} else {
    for (field, errors) in &form.errors().items {
        println!("{field}: {:?}", errors.messages());
    }
}
```

| Method | Returns |
|---|---|
| `is_valid()` | `true` when the form is bound and has no errors |
| `errors()` | `&ErrorDict`: field name → `ErrorList` (see [validation](validation.md#reading-errors)) |
| `non_field_errors()` | the `ErrorList` of errors not tied to one field |
| `has_error(field, code)` | whether `field` has an error, optionally with a specific code |
| `cleaned_data()` | `&IndexMap<String, Value>` of the fields that validated |
| `add_error(field, error)` | adds an error after validation (needs `&mut self`) |

`cleaned_data()` only contains fields that passed validation. When a field fails, its key is absent, so index
with `get` unless `is_valid()` returned `true`.

### Cleaned values

Cleaned values are `rango::forms::Value`, an enum with one variant per kind of result:

`None`, `Bool(bool)`, `Int(i64)`, `Float(f64)`, `Decimal(Decimal)`, `Str(String)`, `Date(NaiveDate)`,
`Time(NaiveTime)`, `DateTime(DateTime<Utc>)`, `NaiveDateTime(NaiveDateTime)`, `Duration(TimeDelta)`, `Uuid(Uuid)`,
`Json(serde_json::Value)`, `List(Vec<Value>)`, `File(UploadedFile)` and `Cleared` (a file field whose "clear"
box was ticked).

Helpers: `as_str()`, `as_i64()`, `as_bool()` (each `Option`), `is_empty()` (`None`, `""`, an empty list or an empty
JSON object), `truthy()` and `to_text()`. `Value` implements `From` for the common Rust types (`&str`, `String`,
`bool`, `i32`, `i64`, `u32`, `usize`, `f64`, the `chrono` date types, `Decimal`, `Uuid`, `Option<T>`, `Vec<T>`),
which is what `.initial(...)` and `.min_value(...)` accept.

With a derived form, skip the enum and read the struct:

```rust
let form = Contact::form().bound(data, FileData::new());
match Contact::from_form(&form) {
    Some(contact) => println!("{} <{}>", contact.name, contact.email),
    None => println!("{:?}", form.errors()),
}
```

`from_form` returns `None` when the form is invalid and also when a cleaned value cannot be converted to the struct
field's type (for example a negative number into a `u32` field).

## A complete view

The usual shape: render an unbound form on `GET`; on `POST` bind, validate, act and redirect, or render the bound
form again so the user sees the errors next to what they typed.

```rust
use rango::forms::{Data, Field, Form, Widget};
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::{redirect, render};
use rango::{Result, context};

fn contact_form() -> Form {
    Form::new("ContactForm")
        .field("name", Field::char().max_length(100))
        .field("email", Field::email().label("Your e-mail"))
        .field("message", Field::char().widget(Widget::textarea()))
        .field("cc_myself", Field::boolean().required(false))
}

pub async fn contact(req: HttpRequest) -> Result<HttpResponse> {
    let form = if req.method() == "POST" {
        let data = Data::from_lists(req.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec())));
        let form = contact_form().bound(data, req.files().await?.clone());
        if form.is_valid() {
            let cleaned = form.cleaned_data();
            let _message = cleaned["message"].as_str().unwrap_or_default();
            // send the e-mail, save a row, ...
            return redirect("/contact/thanks/");
        }
        form
    } else {
        contact_form()
    };
    render(&req, "contact.html", context! { "form" => form })
}
```

```html
<!-- templates/contact.html -->
<form method="post">
  {% csrf_token %}
  {{ form.as_p }}
  <button type="submit">Send</button>
</form>
```

A `Form` converts into a template value (`context! { "form" => form }` or `form.to_template()`), so templates can
use `{{ form }}`, `{{ form.as_p }}`, `{{ form.email.errors }}` and loop over the fields. See
[rendering forms](rendering.md).

Redirecting after a successful `POST` keeps a browser reload from submitting the data twice.

## Initial values

Initial values are what an unbound form shows. They can be set per field or per form; the form-level value wins.

```rust
use rango::forms::Value;

let form = Form::new("Profile")
    .field("name", Field::char().initial("Anonymous"))
    .field("newsletter", Field::boolean().required(false).initial(true))
    .initial([("name".to_owned(), Value::from("Bob"))]);
```

`Form::initial` accepts any iterator of `(String, Value)` and replaces the whole set.

Initial values are not a fallback for missing data: once a form is bound, a field that was not submitted is
validated as empty. The exceptions are disabled fields (`Field::disabled(true)`), which always use the initial
value and ignore submitted data, and file fields, which keep their initial value when no new file is uploaded.

## Prefixes

A prefix namespaces the HTML names and ids, so that two forms with a `name` field can live in one `<form>` tag:

```rust
let billing = address_form().prefix("billing");
let shipping = address_form().prefix("shipping");
```

The `name` field of the first form renders as `name="billing-name" id="id_billing-name"` and is read from the
`billing-name` key of the submitted data. `cleaned_data()` and `errors()` keep using the unprefixed field names.
`form.add_prefix("name")` returns the HTML name.

## Detecting changes

```rust
let form = profile_form().initial(current_values).bound(data, FileData::new());
if form.has_changed() {
    println!("changed: {:?}", form.changed_data());   // Vec<String> of field names
}
```

A field counts as changed when its submitted value differs from its initial value after conversion. Disabled
fields never count. For a field built with `.show_hidden_initial(true)` the comparison uses the initial value that
was rendered into the page (a hidden `initial-<name>` input) instead of the one the server has now.

## Form options

All are builder methods on `Form`; the matching public fields can also be assigned directly.

| Method | Default | Effect |
|---|---|---|
| `field(name, Field)` | — | add a field |
| `initial(iter)` | empty | form-level initial values |
| `prefix(&str)` | none | prefix for HTML names and ids |
| `auto_id(Option<&str>)` | `Some("id_%s")` | pattern for widget ids; `%s` is the HTML name. `None` or `Some("")` turns ids and `<label for>` off; a pattern without `%s` uses the bare name |
| `label_suffix(&str)` | `":"` | appended to labels that do not already end in `:`, `?`, `.` or `!` |
| `field_order(&[&str])` | insertion order | listed fields first, in that order; the others keep their relative order after them |
| `required_css_class(&str)` | none | CSS class for rows and labels of required fields |
| `error_css_class(&str)` | none | CSS class for rows of fields with errors |
| `use_required_attribute(bool)` | `true` | whether required fields get the HTML `required` attribute |
| `empty_permitted(bool)` | `false` | a bound form whose data equals its initial values validates as empty (no errors, no cleaned data); also turns `use_required_attribute` off |
| `renderer(Arc<dyn Renderer>)` | from the `form_renderer` setting | how templates are found, see [rendering](rendering.md#renderers) |
| `clean_field(name, hook)` | none | per-field validation hook, see [validation](validation.md) |
| `clean(hook)` | none | whole-form validation hook |

Other accessors: `get(name) -> Option<BoundField>`, `bound_fields()`, `visible_fields()`, `hidden_fields()`,
`is_multipart()`, `media()`, `get_initial_for_field(&field, name)`.

## File uploads

A form with a file field needs both the files and the right `enctype`:

```rust
let form = Form::new("Upload").field("attachment", Field::file());
let bound = form.bound(data, req.files().await?.clone());
```

```html
<form method="post"{% if form.is_multipart %} enctype="multipart/form-data"{% endif %}>
```

The cleaned value is `Value::File(UploadedFile)`; read it with `file.read()` or pass it to a storage. See
[file uploads](../http/file-uploads.md).

## `FormView`

`rango::views::generic::FormView` packages the view above: it renders the template with the form on `GET`,
validates on `POST` (and `PUT`), re-renders with errors when invalid and redirects when valid.

```rust
use rango::urls::path;
use rango::views::generic::{FormView, View};

path("contact/", FormView::new("contact.html", |_req| contact_form())
    .success_url("/contact/thanks/")
    .as_view())
```

The closure builds a fresh unbound form for each request and receives the request, so it can set initial values
from the query string or the session.

| Builder | Purpose |
|---|---|
| `FormView::new(template_name, \|req\| form)` | template and form factory |
| `success_url(&str)` | where to redirect after a valid submission |
| `success_url_with(\|req\| Ok(url))` | compute the redirect per request |
| `form_valid(\|req, form\| Box::pin(async move { ... }))` | replace the default "redirect to the success URL" step |
| `extra(key, value)` | extra template context |

The template receives `form`, the URL keyword arguments and the `extra` values. To do something with the data,
supply `form_valid`:

```rust
FormView::new("contact.html", |_req| contact_form()).form_valid(|_req, form| {
    Box::pin(async move {
        let name = form.cleaned_data()["name"].to_text();
        // ... act on the data ...
        rango::shortcuts::redirect(&format!("/contact/thanks/?name={name}"))
    })
})
```

If the form is valid and neither `form_valid` nor a success URL is configured, the view fails with an
"improperly configured" error.

For forms backed by a model, use `CreateView` and `UpdateView` ([model forms](modelforms.md#generic-views)).

## Not supported yet

- Form inheritance or mixing: compose forms with functions that take and return a `Form` instead.
- Asynchronous clean hooks. Hooks are synchronous closures; do database checks after `is_valid()` and report
  failures with `form.add_error(...)` (see [validation](validation.md#adding-errors-after-validation)).
- `#[derive(Form)]` without the `crate` attribute in projects that only depend on `rangoframework`.

## See also

- [Form and field validation](validation.md)
- [Rendering forms](rendering.md)
- [Model forms](modelforms.md)
- [Formsets](formsets.md)
- [Form fields reference](../../ref/form-fields.md)
- [Widgets reference](../../ref/widgets.md)
- [Generic views](../http/generic-views.md)
- [Tutorial part 4: forms and generic views](../../tutorial/04-forms-generic-views.md)
