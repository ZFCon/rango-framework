# Formsets

A formset manages several forms of the same kind on one page: a list of order lines, a set of contact addresses, the
children of a parent row. This page covers building a `FormSet`, its management form, validating it, deleted and
ordered forms, and `ModelFormSet` for editing model rows.

```rust
use rango::forms::{Data, FileData, Field, Form, FormSet, all_valid};
```

## Building a formset

A `FormSet` wraps the form that is repeated. Like `Form`, it is a value: configure it with builder calls, then bind
it to submitted data per request.

```rust
fn address_form() -> Form {
    Form::new("AddressForm")
        .field("street", Field::char().max_length(100))
        .field("city", Field::char().max_length(60))
        .field("note", Field::char().required(false))
}

fn addresses() -> FormSet {
    FormSet::new(address_form()).extra(2).can_delete(true)
}
```

`FormSet::new(form)` takes the prototype form. Every form in the set is a clone of it with its own prefix
(`form-0`, `form-1`, ...), so field names in the HTML are `form-0-street`, `form-1-street` and so on.

### Options

| Method | Default | Meaning |
|---|---|---|
| `extra(usize)` | `1` | Blank forms shown after the initial ones. |
| `min_num(i64)` | `0` | Minimum number of forms shown. |
| `max_num(i64)` | `1000` | Maximum number of forms shown. Also moves `absolute_max` to `max_num + 1000` unless you set it afterwards. |
| `absolute_max(i64)` | `2000` | Hard ceiling for the number of forms accepted from the client. |
| `validate_min(bool)` | `false` | Report an error when fewer than `min_num` forms are filled in. |
| `validate_max(bool)` | `false` | Report an error when more than `max_num` forms are submitted. |
| `can_delete(bool)` | `false` | Add a `DELETE` checkbox to each form. |
| `can_delete_extra(bool)` | `true` | With `can_delete`, also give the *extra* (not initial) forms a `DELETE` checkbox. |
| `can_order(bool)` | `false` | Add an `ORDER` number field to each form. |
| `prefix(&str)` | `form` | Prefix of the management form and of every form (`<prefix>-<index>-<field>`). |
| `auto_id(Option<&str>)` | `Some("id_%s")` | The id pattern passed to every form; `None` turns ids off. |
| `form_hook(fn(Form, Option<usize>) -> Form)` | none | Adjust every form after it is built. The index is `None` for `empty_form`. |
| `clean(fn(&FormSet) -> Result<(), ValidationError>)` | none | A formset-wide check, see [below](#formset-wide-validation). |
| `with_initial(Vec<IndexMap<String, Value>>)` | none | Initial values: one map per initial form. |
| `bound(Data, FileData)` | unbound | Attach submitted data. |

The public fields of `FormSet` (`extra`, `min_num`, `prefix`, `error_messages`, ...) mirror these options. The
formset forms are built with `use_required_attribute` off, so the browser does not block the blank extra forms.

### How many forms are shown

For an unbound formset the total is `max(initial, min_num) + extra`, capped by `max_num` (but never below the number
of initial forms). With three initial forms, `extra(2)` and `max_num(4)` the formset shows four forms. For a bound
formset the total is the `TOTAL_FORMS` value that was submitted, limited to `absolute_max`.

The first `initial_form_count()` forms are the *initial* forms (they correspond to existing data); the rest are
*extra* forms. `initial_forms()` and `extra_forms()` return the two slices of `forms()`.

### Initial data

```rust
use rango::forms::Value;
use rango::db::indexmap::IndexMap;

let rows: Vec<IndexMap<String, Value>> = vec![
    [("street".to_owned(), Value::from("1 Nile St")), ("city".to_owned(), Value::from("Cairo"))]
        .into_iter()
        .collect(),
];
let formset = addresses().with_initial(rows);
```

`rango::db::indexmap` re-exports the `indexmap` crate, so you do not need a separate dependency.

## The management form

The browser tells the server how many forms to expect through four hidden fields, together called the management
form. Render it exactly once, before the forms:

```html
<form method="post">
  {% csrf_token %}
  {{ formset.management_form }}
  {% for form in formset %}
    {{ form.as_p }}
  {% endfor %}
  <button>Save</button>
</form>
```

For `FormSet::new(...).extra(2)` the management form renders:

```html
<input type="hidden" name="form-TOTAL_FORMS" value="2" id="id_form-TOTAL_FORMS"><input type="hidden" name="form-INITIAL_FORMS" value="0" id="id_form-INITIAL_FORMS"><input type="hidden" name="form-MIN_NUM_FORMS" value="0" id="id_form-MIN_NUM_FORMS"><input type="hidden" name="form-MAX_NUM_FORMS" value="1000" id="id_form-MAX_NUM_FORMS">
```

| Field | Meaning |
|---|---|
| `TOTAL_FORMS` | How many forms are in the page (required). |
| `INITIAL_FORMS` | How many of them are initial forms (required). |
| `MIN_NUM_FORMS`, `MAX_NUM_FORMS` | Informational copies of the options. |

`formset.render()`, `as_div()`, `as_p()`, `as_table()` and `as_ul()` render the management form followed by every
form in that style; `{{ formset }}` in a template is `render()`. A formset is iterable in templates, as the loop
above shows. These values are also available in templates: `management_form`, `forms`, `empty_form`,
`non_form_errors`, `errors`, `is_bound`, `is_valid`, `is_multipart`, `media`, `total_form_count`,
`initial_form_count`, `can_delete` and `can_order`.

If a bound request lacks `TOTAL_FORMS` or `INITIAL_FORMS`, or they are not numbers, the formset is invalid with a
non-form error of code `missing_management_form`:

```
ManagementForm data is missing or has been tampered with. Missing fields: form-TOTAL_FORMS, form-INITIAL_FORMS. You may need to file a bug report if the issue persists.
```

### Adding forms with JavaScript

`formset.empty_form()` returns a form whose prefix is `form-__prefix__`. Render it inside a `<template>` element,
replace `__prefix__` with the next index in JavaScript, and increase `TOTAL_FORMS` by one:

```html
<template id="empty">{{ formset.empty_form.as_p }}</template>
```

The empty form is never validated or saved. Keep `max_num` and `absolute_max` in mind: a client that submits more
than `absolute_max` forms is rejected with `too_many_forms`.

## Handling a POST

```rust
use rango::forms::{Data, FileData};
use rango::http::HttpRequest;

async fn save_addresses(req: &HttpRequest) -> rango::Result<bool> {
    let data = Data::from_lists(req.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec())));
    let files = req.files().await?.clone();
    let formset = addresses().bound(data, files);

    if !formset.is_valid() {
        return Ok(false); // render the formset again: errors are attached to each form
    }
    for form in formset.forms() {
        if form.has_changed() {
            let cleaned = form.cleaned_data();
            // cleaned["street"], cleaned["city"], ...
        }
    }
    Ok(true)
}
```

`is_valid()` is `false` for an unbound formset. A bound formset is valid when its management form is valid, every
form that is not marked for deletion is valid, and there are no non-form errors.

Extra forms the user left untouched are valid and have empty `cleaned_data`: they are built with
`empty_permitted`, so a form identical to its initial values is skipped. Check `form.has_changed()` before using
one.

### Reading the result

| Method | Returns |
|---|---|
| `forms()` | All forms (`&[Form]`). |
| `cleaned_data()` | `Option<Vec<&IndexMap<String, Value>>>`: one map per form, `None` while the formset is invalid. |
| `errors()` | One `ErrorDict` per form that is not deleted. |
| `non_form_errors()` | Errors about the formset as a whole (an `ErrorList`). |
| `total_error_count()` | Non-form errors plus every field error. |
| `has_changed()` | Whether any form differs from its initial values. |
| `is_multipart()` | Whether a form holds a file field. |
| `media()` | Merged CSS and JavaScript of the forms' widgets. |

To validate several formsets together, use `all_valid(&[&a, &b])`.

## Deleting forms

With `can_delete(true)` every form has a boolean field `DELETE` (label "Delete"), named `form-<i>-DELETE`.

- `deleted_forms()` returns the forms whose box was ticked, as `Vec<&Form>`. It is empty while the formset is
  invalid. An *extra* form that was not changed is never counted as deleted.
- Forms marked for deletion are left out of validation: errors in them are ignored, and they do not appear in
  `errors()`.
- `validate_max` and `validate_min` count only forms that are not deleted.

```rust
let fs = FormSet::new(address_form()).can_delete(true).bound(data, FileData::new());
if fs.is_valid() {
    let doomed = fs.deleted_forms();
}
```

With `can_delete_extra(false)` only initial forms get the checkbox.

## Ordering forms

With `can_order(true)` every form has an integer field `ORDER` (label "Order", optional). Initial forms are
prefilled with `1, 2, 3, ...` according to their position. `ordered_forms()` returns
`Option<Vec<&Form>>`: `None` unless the formset is valid and `can_order` is set; otherwise the forms sorted by their
`ORDER` value, ascending. Forms with an empty `ORDER` come last, in their original order; unchanged extra forms and
deleted forms are left out.

```rust
let fs = FormSet::new(address_form()).can_order(true).bound(data, FileData::new());
if let Some(forms) = fs.ordered_forms() {
    for (position, form) in forms.iter().enumerate() {
        let street = form.cleaned_data().get("street");
    }
}
```

## Limits and counting errors

With `validate_max(true)`, more submitted forms than `max_num` (after subtracting deleted ones) gives a non-form
error with code `too_many_forms`: `Please submit at most 1 form.` A `TOTAL_FORMS` above `absolute_max` gives the same
error regardless of `validate_max`.

With `validate_min(true)`, fewer filled-in forms than `min_num` gives `too_few_forms`: `Please submit at least 2
forms.` Blank extra forms do not count towards the minimum.

The messages are singular or plural by the limit. Replace them through the public `error_messages` map for
`missing_management_form`; the two limit messages are fixed.

## Formset-wide validation

The `clean` hook runs after the individual forms and after the limit checks pass. It receives the formset and
returns `Err(ValidationError)` to add a non-form error. Inside the hook look at `forms()`, not `cleaned_data()`
(which itself asks for validity), and skip forms that have errors:

```rust
use rango::core::exceptions::ValidationError;

let fs = FormSet::new(address_form())
    .clean(|fs| {
        let mut seen = std::collections::HashSet::new();
        for form in fs.forms().iter().filter(|f| f.is_valid()) {
            if let Some(city) = form.cleaned_data().get("city") {
                if !seen.insert(city.to_text()) {
                    return Err(ValidationError::new("Each city can appear once.").with_code("duplicate_city"));
                }
            }
        }
        Ok(())
    })
    .bound(data, FileData::new());
```

Show the error with `{{ formset.non_form_errors }}`.

## Model formsets

`ModelFormSet<M>` is a `FormSet` of model forms that knows how to save. Each form carries a hidden primary-key field
(named like the model's primary key, e.g. `form-0-id`) that ties it to its row.

```rust
use rango::forms::{ModelFormOptions, ModelFormSet};

let db = rango::db::Db::default_db();
let existing = Author::objects().fetch(&db).await?;
let opts = || ModelFormOptions::fields(["name", "age"]);

let fs = ModelFormSet::<Author>::new(&db, existing.clone(), opts(), 1).await?;
let html = fs.formset.as_p();
```

`ModelFormSet::new(&db, instances, options, extra)` builds one form per instance plus `extra` blank ones. The model
needs `Model + Default + Clone`. Options are the same `ModelFormOptions` as for [model forms](modelforms.md). The
inner formset is the public field `formset`; use it to render and to read errors.

Bind and save:

```rust
let fs = ModelFormSet::<Author>::new(&db, existing.clone(), opts(), 1)
    .await?
    .bound(data, FileData::new());
if fs.formset.is_valid() {
    let saved: Vec<Author> = fs.save(&db).await?;
}
```

Build the formset with the same `instances` for the GET and the POST so that each submitted primary key matches a
row.

`save` walks the forms in order:

- a form marked for deletion deletes its row;
- an unchanged extra form is skipped;
- an unchanged initial form is skipped;
- a changed initial form updates its row; a changed extra form inserts a new row;
- many-to-many fields of each saved form are set.

It returns only the rows that were inserted or updated. `save` fails when the formset is not valid.

To turn deletion or ordering on, configure the inner formset before binding:

```rust
let mut fs = ModelFormSet::<Author>::new(&db, existing, opts(), 1).await?;
fs.formset = fs.formset.can_delete(true);
let fs = fs.bound(data, FileData::new());
```

A form is a deletion only when it matches an existing instance; ticking `DELETE` on an unsaved extra form is
ignored.

### Inline formsets

An inline formset edits the children of one parent row. `ModelFormSet::inline` takes the parent's primary key (a
`rango::db::Value`), the name of the foreign-key field on the child, the existing children and the options. The
foreign-key field is excluded from the forms and is set to the parent on every saved row.

```rust
use rango::db::Value as DbValue;

let fs = ModelFormSet::<Chapter>::inline(
    &db,
    DbValue::Int(book_id),
    "book",
    children,
    ModelFormOptions::fields(["title", "number"]),
    3,
)
.await?;
```

## Not supported yet

- Model formsets do not check uniqueness against the database or run a model-level hook; the single-form checks of
  [`ModelForm::is_valid`](modelforms.md#validation-and-uniqueness) are not applied per form.
- `ModelFormSet` takes the number of `extra` forms only; the other formset options are set through the public
  `formset` field as shown above.
- `ModelFormSet::save` has no `commit = false` mode.
- `form_hook` receives and returns a `Form`; there is no per-form keyword mechanism beyond that.

## See also

- [Working with forms](forms.md)
- [Form validation](validation.md)
- [Rendering forms](rendering.md)
- [Model forms](modelforms.md)
- [Form fields reference](../../ref/form-fields.md)
