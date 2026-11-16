# Form and field validation

Validation turns submitted text into typed values and collects everything that is wrong with it. This page
describes the order in which the steps run, how to add your own checks at each level, and how errors are
represented and customised.

## What runs, and in which order

Validation starts the first time `is_valid()`, `errors()` or `cleaned_data()` is called on a bound form. The result
is cached.

For each field, in form order:

1. **Read the raw value.** The field's widget extracts it from the submitted data (a disabled field uses its
   initial value instead).
2. **Convert.** The field parses the text into its type (`"12"` → `Value::Int(12)`). Failure produces an error,
   usually with code `invalid`.
3. **Field checks.** `required` and, for choice fields, membership of the choices.
4. **Validators.** Every validator attached to the field runs and *all* failures are collected. Validators are
   skipped when the value is empty (`None`, `""`, empty list).
5. **`clean_field` hook**, if one is registered for the field. It receives the converted value and returns the
   value to store.

If any of steps 2–5 fails, the errors are recorded under the field name and the field is left out of
`cleaned_data()`. Steps 2–4 stop at the first failing step: a value that cannot be converted is not passed to
validators.

After all fields:

6. **`clean` hook**, the form-wide step. It runs even when some fields failed, so it must cope with missing keys.

A [`ModelForm`](modelforms.md) adds a database stage (model-level hook and uniqueness checks) inside its own
`is_valid(&db)`.

A form built with `.empty_permitted(true)` whose data equals its initial values skips all of this and is valid with
empty cleaned data. Formsets use that for their blank extra forms.

## Required and optional fields

Every field is required unless built with `.required(false)`.

```rust
use rango::forms::{Field, Form};

let form = Form::new("Signup")
    .field("username", Field::char().max_length(30))
    .field("nickname", Field::char().required(false))
    .field("age", Field::integer().required(false))
    .field("terms", Field::boolean());
```

- A required field with an empty value fails with code `required` ("This field is required.").
- An optional text field cleans to `""` (or whatever `.empty_value(...)` says); other optional fields clean to
  `Value::None`; an optional multiple-choice field cleans to an empty list.
- A required `Field::boolean()` must be *true*: an unchecked box is an error. Use `.required(false)` for a plain
  yes/no answer.
- `Field::null_boolean()` never fails `required`; it cleans to `Bool(true)`, `Bool(false)` or `None`.

## Built-in validation through field options

Most checks are attached by the field's own options:

```rust
let form = Form::new("Product")
    .field("code", Field::regex("^[A-Z]{3}-[0-9]{4}$"))
    .field("name", Field::char().min_length(3).max_length(50))
    .field("quantity", Field::integer().min_value(1).max_value(999))
    .field("pack_size", Field::integer().min_value(0).step_size(5.0))
    .field("price", Field::decimal().max_digits(6).decimal_places(2));
```

Each option adds a validator and, where one exists, the matching HTML attribute (`maxlength`, `min`, `max`,
`step`) so the browser can check first. The server-side check always runs. The options and the error codes they
produce are listed per field in the [form fields reference](../../ref/form-fields.md).

## Adding validators to a field

A field validator is a function from the converted `&Value` to `Result<(), ValidationError>`:

```rust
use rango::core::exceptions::ValidationError;
use rango::forms::Field;

let even = Field::integer().validator(|v| match v.as_i64() {
    Some(n) if n % 2 != 0 => Err(ValidationError::new("%(value)s is not an even number.")
        .with_code("odd")
        .with_params([("value", n)])),
    _ => Ok(()),
});
```

`.validator(...)` can be called several times; all validators run and all their errors are reported together.

The validators in [`rango::core::validators`](../validators.md) work on plain Rust types (`&str`, `&f64`,
`&Decimal`), so adapt the value in the closure:

```rust
use rango::core::validators::{validate_slug, MinLengthValidator, Validator};

let handle = Field::char()
    .validator(|v| validate_slug(&v.to_text()))
    .validator(|v| MinLengthValidator::new(3).validate(&v.to_text()));
```

`rango::forms::validators` holds the same validators already adapted to `Value`, as used by the built-in fields:
`max_length(n)`, `min_length(n)`, `prohibit_null_characters()`, `regex(pattern)`, `email()`, `slug()`,
`unicode_slug()`, `url()`, `ip(protocol, unpack_ipv4)`, `decimal(max_digits, decimal_places)`,
`file_extension(&[..])`, `max_value(Value)`, `min_value(Value)` and `step_value(step, offset)`. They return a
ready-made `FieldValidator`; push it onto the field's public `validators` list:

```rust
use rango::forms::{validators, Field};

let mut slug = Field::char();
slug.validators.push(validators::unicode_slug());
```

## Per-field hooks: `clean_field`

`Form::clean_field(name, hook)` registers a hook that runs after the field's own cleaning succeeded. It receives a
`FormClean` handle and the field's cleaned value, and returns the value to store (which may differ) or an error
for that field.

```rust
use rango::core::exceptions::ValidationError;
use rango::forms::{Field, Form, Value};

let form = Form::new("Signup")
    .field("username", Field::char().max_length(30))
    .clean_field("username", |_c, value| {
        let name = value.to_text().to_lowercase();
        if name == "admin" {
            return Err(ValidationError::new("This username is reserved.").with_code("reserved"));
        }
        Ok(Value::Str(name))          // normalised value replaces the original
    });
```

One hook per field name; registering a second one for the same name replaces the first. The hook is not called
when the field's own validation failed.

## Whole-form hook: `clean`

`Form::clean(hook)` registers the cross-field step. The hook gets a `FormClean`:

```rust
pub struct FormClean<'a> {
    pub form: &'a Form,                              // the form being validated
    pub cleaned: &'a mut IndexMap<String, Value>,    // cleaned values so far; may be modified
    pub errors: &'a mut ErrorDict,                   // errors so far
}
```

Returning `Err(error)` records a *non-field* error, unless the error was built with `ValidationError::from_dict`,
in which case each entry is attached to the named field. `c.add_error(field, error)` does the same without
leaving the hook, so several problems can be reported at once.

```rust
use rango::core::exceptions::ValidationError;
use rango::forms::{Field, Form, Value};

let form = Form::new("Booking")
    .field("start", Field::date())
    .field("end", Field::date())
    .field("email", Field::email().required(false))
    .field("phone", Field::char().required(false))
    .clean(|c| {
        // Fields that failed are missing from `cleaned`, so match on what is there.
        let wrong_order = matches!(
            (c.cleaned.get("start"), c.cleaned.get("end")),
            (Some(Value::Date(start)), Some(Value::Date(end))) if end < start
        );
        if wrong_order {
            // Attach the error to one field and keep going.
            let _ = c.add_error(Some("end"), ValidationError::new("The end date is before the start date.").with_code("order"));
        }
        let empty = |v: Option<&Value>| v.is_none_or(Value::is_empty);
        if empty(c.cleaned.get("email")) && empty(c.cleaned.get("phone")) {
            // A non-field error.
            return Err(ValidationError::new("Give an e-mail address or a phone number.").with_code("contact"));
        }
        Ok(())
    });
```

A real example from `rango::contrib::auth::forms`, where the password confirmation error is attached to the second
field:

```rust
form.clean(move |c| {
    let (p1, p2) = (c.cleaned.get("new_password1").cloned(), c.cleaned.get("new_password2").cloned());
    if let (Some(Value::Str(p1)), Some(Value::Str(p2))) = (&p1, &p2) {
        if p1 != p2 {
            return Err(ValidationError::from_dict([(
                "new_password2".to_owned(),
                ValidationError::new("The two password fields didn’t match.").with_code("password_mismatch"),
            )]));
        }
    }
    Ok(())
})
```

A form has one `clean` hook; calling `.clean(...)` again replaces it. To combine checks, call them from one closure.

The hook may also change values: `c.cleaned.insert("slug".into(), Value::from("computed"))`.

## `add_error`

```rust
FormClean::add_error(&mut self, field: Option<&str>, error: ValidationError) -> Result<(), String>
Form::add_error(&mut self, field: Option<&str>, error: ValidationError) -> Result<(), String>
```

- `field = Some(name)` attaches the error to that field; `None` makes it a non-field error.
- The field is removed from the cleaned data.
- A `ValidationError::from_dict` error must be passed with `field = None`; its keys name the fields.
- The call returns `Err(message)` (and adds nothing) when the field does not exist on the form, or when a dict
  error is combined with `Some(field)`.

### Adding errors after validation

Hooks are synchronous, so checks that need the database or another service run after `is_valid()`, using the
form-level `add_error`. This is how the login form reports bad credentials:

```rust
let mut form = login_form().bound(data, Default::default());
if form.is_valid() {
    let username = form.cleaned_data()["username"].to_text();
    if !account_exists(&db, &username).await? {
        form.add_error(Some("username"), ValidationError::new("Unknown account.").with_code("unknown"))
            .map_err(|e| rango::Error::Other(e.into()))?;
    }
}
if form.is_valid() {            // false once an error was added
    // ...
}
```

## `ValidationError`

`rango::core::exceptions::ValidationError` has three shapes:

| Constructor | Shape | Use |
|---|---|---|
| `ValidationError::new(message)` | one message | the common case |
| `ValidationError::from_list([e1, e2])` | a flat list | several messages for one target |
| `ValidationError::from_dict([(field, error), ...])` | errors keyed by field name | cross-field errors from a `clean` hook |

A single error can carry a machine-readable code and named parameters:

```rust
let e = ValidationError::new("Ensure this value is at most %(limit)s (it is %(value)s).")
    .with_code("too_big")
    .with_params([("limit", 10), ("value", 12)]);
assert_eq!(e.messages(), vec!["Ensure this value is at most 10 (it is 12)."]);
assert_eq!(e.code(), Some("too_big"));
```

Placeholders use the `%(name)s` form (`%(name)d` is accepted too, `%%` is a literal percent sign) and are filled
in when the message is read. Give every error a code: codes are what `has_error`, per-field message overrides and
JSON output key on.

| Method | Returns |
|---|---|
| `code()` | the code of a single error |
| `messages()` | every formatted message, flattened |
| `error_list()` | every single error, flattened |
| `error_dict()` | the field → errors map of a dict error, else `None` |
| `message_dict()` | field → formatted messages (empty for non-dict errors) |

`ValidationError` converts into `rango::Error` with `?`.

## Reading errors

`form.errors()` returns an `ErrorDict`, a map from field name to `ErrorList`. Non-field errors are stored under
the key `"__all__"` (`rango::core::exceptions::NON_FIELD_ERRORS`); read them with `form.non_field_errors()`.

```rust
let errors = form.errors();
if let Some(list) = errors.get("email") {
    assert_eq!(list.messages(), vec!["Enter a valid email address."]);
    assert_eq!(list.as_data()[0].code(), Some("invalid"));
}
assert!(form.has_error("email", Some("invalid")));
assert!(form.non_field_errors().is_empty());
```

| `ErrorDict` | |
|---|---|
| `get(field)` | `Option<&ErrorList>` |
| `contains_key(field)`, `is_empty()` | presence checks |
| `items` | the underlying `IndexMap<String, ErrorList>` |
| `as_data()` | field → `&[ValidationError]` (codes and params intact) |
| `get_json_data(escape_html)` / `as_json(escape_html)` | `{"field": [{"message": "...", "code": "..."}]}` |
| `render()` / `as_ul()` / `as_text()` | HTML list or plain text |

| `ErrorList` | |
|---|---|
| `messages()` | formatted messages |
| `as_data()` | the `ValidationError`s |
| `len()`, `is_empty()` | size |
| `push(error)`, `extend(errors)` | add errors |
| `get_json_data(escape_html)` / `as_json(escape_html)` | `[{"message": "...", "code": "..."}]` |
| `render()` / `as_ul()` / `as_text()` | `<ul class="errorlist">…</ul>` or plain text |

`as_json` is convenient for answering an AJAX submission:

```rust
// {"age": [{"message": "Enter a whole number.", "code": "invalid"}]}
let body = form.errors().as_json(false);
```

Pass `true` to HTML-escape the messages when the client inserts them into the page as markup.

## Customising error messages

Each field keeps a map from error code to message. Replace entries with `.error_message(code, message)`:

```rust
let age = Field::integer()
    .min_value(18)
    .error_message("required", "Please tell us your age.")
    .error_message("invalid", "Age must be a whole number.")
    .error_message("min_value", "You must be at least %(limit_value)s.");
```

The override applies to the field's own errors and to errors raised by its validators: when a validator fails
with a code that has an entry in the field's messages, the field's message is used and the validator's parameters
(`%(limit_value)s`, `%(show_value)s`, `%(value)s`, …) are still substituted.

That rule also applies to your own validators. A custom validator on `Field::integer()` that reports code
`invalid` will show the field's "Enter a whole number." message, because the field defines `invalid`. Pick a code
the field does not define when the message must stay yours.

For model forms the same overrides are set with `ModelFormOptions::error_message(field, code, message)`.

The default messages pass through `gettext` when the error is created, so they follow the active language. See
[internationalization](../i18n.md).

## Where the built-in validators live

`rango::core::validators` contains the framework's validators (`RegexValidator`, `EmailValidator`, `UrlValidator`,
`MaxValueValidator`, `MinLengthValidator`, `DecimalValidator`, `FileExtensionValidator`, `validate_slug`,
`validate_ipv46_address`, …). They are independent of forms and are documented in [validators](../validators.md).

## Not supported yet

- Asynchronous hooks (`clean_field` and `clean` are synchronous closures).
- More than one `clean` hook per form, or more than one `clean_field` hook per field.
- Per-form overrides of the error list class; `ErrorList` always renders with the `errorlist` CSS class.

## See also

- [Working with forms](forms.md)
- [Rendering forms](rendering.md)
- [Model forms](modelforms.md)
- [Validators](../validators.md)
- [Form fields reference](../../ref/form-fields.md)
