# Form fields reference

A `Field` describes one input of a form: how submitted text becomes a typed value, what is checked, and which widget
draws it. This page lists every `Field` constructor with its default widget, cleaned value and error codes, then the
options shared by all fields, and finally the `#[derive(Form)]` attributes.

```rust
use rango::forms::{Choice, Field, Form, Value, Widget};
```

Fields are built with a constructor and refined with chained calls:

```rust
let age = Field::integer().min_value(18).max_value(120).label("Age").help_text("In years");
```

Submitted data is text; a cleaned value is a `rango::forms::Value`. The variants used below are `Value::None`,
`Bool`, `Int(i64)`, `Float(f64)`, `Decimal`, `Str`, `Date`, `Time`, `DateTime` (UTC), `NaiveDateTime`, `Duration`,
`Uuid`, `Json`, `List`, `File` and `Cleared`.

## Constructors

Every field accepts the [common options](#common-options). The "Options" column lists what is specific to the
constructor. A field is required unless you call `.required(false)`; the error code `required` (message "This field
is required.") is therefore omitted from the "Error codes" column.

### Text

| Constructor | Default widget | Cleaned value | Options | Error codes |
|---|---|---|---|---|
| `Field::char()` | `TextInput` | `Str`; empty input cleans to `empty_value` (default `Str("")`) | `max_length`, `min_length`, `strip` (default `true`), `empty_value` | `max_length`, `min_length`, `null_characters_not_allowed` |
| `Field::regex(pattern)` | `TextInput` | `Str`, not stripped | `max_length`, `min_length`, `strip` (default `false`) | `invalid` ("Enter a valid value."), `null_characters_not_allowed` |
| `Field::email()` | `EmailInput` | `Str`, stripped | `max_length`, `min_length` | `invalid` ("Enter a valid email address."), `max_length` (limit 320), `null_characters_not_allowed` |
| `Field::url()` | `URLInput` | `Str`, stripped; a missing scheme is added | `assume_scheme` (default `https` when the `forms_urlfield_assume_https` setting is on, otherwise `http`) | `invalid` ("Enter a valid URL."), `null_characters_not_allowed` |
| `Field::slug()` | `TextInput` | `Str`, stripped | `max_length` | `invalid` ("Enter a valid "slug" ..."), `null_characters_not_allowed` |
| `Field::generic_ip_address(protocol, unpack_ipv4)` | `TextInput` | `Str`; IPv6 text is normalised | protocol `"both"`, `"ipv4"` or `"ipv6"` | `invalid` ("Enter a valid IPv4 or IPv6 address."), `max_length` (limit 39), `null_characters_not_allowed` |
| `Field::uuid()` | `TextInput` | `Uuid`; empty is `Value::None` | | `invalid` ("Enter a valid UUID.") |
| `Field::json()` | `Textarea` | `Json`; empty is `Value::None` | | `invalid` ("Enter a valid JSON.") |

`Field::regex` panics when the pattern does not compile (`expect("valid regex")`), as does `generic_ip_address`
when `unpack_ipv4` is true with a protocol other than `"both"`. Check patterns in tests or at start-up.

The URL field fixes a missing scheme before it validates: `example.com` becomes `http://example.com` (or `https://`),
`//example.com` becomes `http://example.com`, and text that already has a scheme is left alone. `max_length` on a
char-like field also writes `maxlength` on the input; `min_length` writes `minlength`.

### Numbers

| Constructor | Default widget | Cleaned value | Options | Error codes |
|---|---|---|---|---|
| `Field::integer()` | `NumberInput` | `Int`; empty is `Value::None` | `min_value`, `max_value`, `step_size` | `invalid` ("Enter a whole number."), `min_value`, `max_value`, `step_size` |
| `Field::float()` | `NumberInput` | `Float` | `min_value`, `max_value`, `step_size` | `invalid` ("Enter a number."; also for `NaN` and infinity), `min_value`, `max_value`, `step_size` |
| `Field::decimal()` | `NumberInput` | `Decimal` | `min_value`, `max_value`, `max_digits`, `decimal_places` | `invalid` ("Enter a number."), `min_value`, `max_value`, `max_digits`, `max_decimal_places`, `max_whole_digits` |

The integer field accepts a sign, digits with single underscores and a trailing `.0`; surrounding spaces are
ignored. The number widget gets derived attributes: `min`, `max`, `step` for integers and floats (`step="any"` for
floats without a `step_size`), and for decimals `min`, `max` and a `step` derived from `decimal_places`
(`step="0.01"` for two places, `"1"` for none, `"any"` when unset). Messages and parameters of the numeric codes are
in [validators](../topics/validators.md).

### Dates and times

| Constructor | Default widget | Cleaned value | Options | Error codes |
|---|---|---|---|---|
| `Field::date()` | `DateInput` | `Date` | `input_formats` | `invalid` ("Enter a valid date.") |
| `Field::time()` | `TimeInput` | `Time` | `input_formats` | `invalid` ("Enter a valid time.") |
| `Field::datetime()` | `DateTimeInput` | `DateTime` (UTC) when the `use_tz` setting is on, else `NaiveDateTime` | `input_formats` | `invalid` ("Enter a valid date/time."), `ambiguous_timezone` |
| `Field::duration()` | `TextInput` | `Duration` | | `invalid` ("Enter a valid duration.") |
| `Field::split_datetime()` | `SplitDateTimeWidget` | like `datetime()` | | `invalid`, `incomplete`, `invalid_date`, `invalid_time`, `required` |

Without `input_formats`, the field tries the entries of the `date_input_formats`, `time_input_formats` and
`datetime_input_formats` settings in order (a date-time field also tries the date formats and treats a date as
midnight). Patterns are `strftime` patterns; `%f` means microseconds. A naive date-time is read in the current time
zone when `use_tz` is on; an input with an explicit offset is converted to UTC. `ambiguous_timezone` is raised for a
local time that does not exist or occurs twice.

### Booleans

| Constructor | Default widget | Cleaned value | Error codes |
|---|---|---|---|
| `Field::boolean()` | `CheckboxInput` | `Bool` (`"false"` and `"0"` are false, other non-empty text is true) | `required` only, raised when the field is required and the value is false |
| `Field::null_boolean()` | `NullBooleanSelect` | `Bool` or `Value::None` | none |

A required boolean field means "must be ticked". Use `.required(false)` for an ordinary yes/no box.

### Choices

| Constructor | Default widget | Cleaned value | Error codes |
|---|---|---|---|
| `Field::choice(choices)` | `Select` | `Str` of the chosen value | `invalid_choice` ("Select a valid choice. %(value)s is not one of the available choices.") |
| `Field::typed_choice(choices, coerce)` | `Select` | the value returned by `coerce(&str) -> Result<Value, ()>`; empty gives `Str("")` | `invalid_choice` (also when `coerce` fails) |
| `Field::multiple_choice(choices)` | `SelectMultiple` | `List` of `Str` | `invalid_choice`, `invalid_list` ("Enter a list of values.") |
| `Field::typed_multiple_choice(choices, coerce)` | `SelectMultiple` | `List` of coerced values; empty gives an empty `List` | `invalid_choice`, `invalid_list` |
| `Field::model_choice(multiple)` | `Select` (or `SelectMultiple`) | the key of the chosen row's `Value`; `Value::None` when empty and optional; a `List` when `multiple` | `invalid_choice` ("Select a valid choice. That choice is not one of the available choices."), `invalid_list` |
| `Field::file_path(path, match, recursive, allow_files, allow_folders, required)` | `Select` | `Str`, the full path | `invalid_choice` |

`choices` is a `Vec<Choice>`:

```rust
use rango::forms::{choices, Choice, Field, Value};

let size = Field::choice(choices([("s", "Small"), ("l", "Large")]));
let grouped = Field::choice(vec![
    Choice::Group("Drinks".into(), vec![("tea".into(), "Tea".into()), ("cof".into(), "Coffee".into())]),
    Choice::option("water", "Water"),
]);
let qty = Field::typed_choice(choices([("1", "One"), ("2", "Two")]), |s| {
    s.parse::<i64>().map(Value::Int).map_err(|_| ())
});
```

`set_choices(&mut self, Vec<Choice>)` replaces the choices of an existing field (it also updates the widget).
`valid_value(&str)` tells whether a text is one of the choices.

`Field::model_choice` has no choices until you call `set_choices`; the model form builds them from the database
(see [model forms](../topics/forms/modelforms.md)). `file_path` reads the directory when the field is built: with
`required = false` it starts with a `---------` entry; `match` is a regex applied to file names; `recursive` descends
into sub-directories, then labels are paths relative to `path`.

### Files

| Constructor | Default widget | Cleaned value | Options | Error codes |
|---|---|---|---|---|
| `Field::file()` | `ClearableFileInput` | `File(UploadedFile)`; `Cleared` when the clear box is ticked on an optional field; the initial value when nothing new is uploaded | `max_length` (file-name length), `allow_empty_file` | `invalid` ("No file was submitted. Check the encoding type on the form."), `missing`, `empty` ("The submitted file is empty."), `contradiction`, `max_length` |
| `Field::image()` | `ClearableFileInput` with `accept="image/*"` | like `file()` | as `file()` | as `file()`, plus `invalid_image` and `invalid_extension` |

The image field checks the extension against a built-in list and then the first bytes of the upload against the
signatures of PNG, JPEG, GIF, BMP, WebP, TIFF and ICO; anything else gives `invalid_image`. A form that holds a file
field is multipart; see [file uploads](../topics/http/file-uploads.md).

### Compound fields

| Constructor | Default widget | Behaviour |
|---|---|---|
| `Field::combo(fields)` | `TextInput` | Runs the value through each sub-field in turn; every sub-field is made optional first. The output of one is the input of the next. |
| `Field::multi_value(fields, widget, compress)` | the widget you pass | Cleans one sub-value per sub-field, then `compress(&[Value]) -> Result<Value, ValidationError>` builds the result. `require_all_fields(bool)` (default `true`) controls whether every part must be present. Codes `invalid`, `incomplete` ("Enter a complete value."), `required`. |

```rust
use rango::forms::{Field, Value, Widget};

let phone = Field::multi_value(
    vec![Field::integer(), Field::integer()],
    Widget::multi(
        vec![Widget::number_input(), Widget::number_input()],
        |v| match v {
            Value::List(l) => l.clone(),
            _ => vec![Value::None, Value::None],
        },
    ),
    |parts| Ok(Value::Str(format!("{}-{}", parts[0].to_text(), parts[1].to_text()))),
);
```

For a field with `require_all_fields(false)`, a sub-field that is required but empty adds the `incomplete` error.

### Aliases

For readers who like named types, these structs have a `new()` returning the same `Field`:
`CharField`, `IntegerField`, `FloatField`, `DecimalField`, `DateField`, `TimeField`, `DateTimeField`,
`DurationField`, `EmailField`, `FileField`, `ImageField`, `URLField`, `BooleanField`, `NullBooleanField`,
`SplitDateTimeField`, `SlugField`, `UUIDField`, `JSONField` (no arguments), `ChoiceField::new(choices)`,
`MultipleChoiceField::new(choices)` and `RegexField::new(pattern)`. `CharField::new()` is `Field::char()`.

## Common options

These builder methods exist on every `Field` and return the field.

| Method | Default | Meaning |
|---|---|---|
| `required(bool)` | `true` | An empty value is an error with code `required`. Also sets the widget's `required` attribute. |
| `label(text)` | the field name, humanised | Label text. |
| `initial(value)` | none | Initial value for an unbound form. `value` is anything convertible into `Value` (`&str`, `String`, `i64`, `i32`, `bool`, `f64`, dates, `Decimal`, `Option<_>`, `Vec<_>`). |
| `help_text(text)` | empty | Text shown next to the input. |
| `disabled(bool)` | `false` | Render with `disabled`; the submitted value is ignored and the initial value is used. |
| `label_suffix(text)` | the form's suffix (`:`) | Replaces the label suffix for this field. |
| `show_hidden_initial(bool)` | `false` | Also render a hidden input with the initial value (used to detect changes; set automatically for model fields with a callable default). |
| `localize(bool)` | `false` | Ask the widget to localise the value. |
| `error_message(code, text)` | built-in messages | Replace the message for one error code. Applies to errors raised by the field and by its validators. |
| `validator(fn(&Value) -> Result<(), ValidationError>)` | none | Add a validator; it runs after the built-in ones and only for non-empty values. |
| `widget(Widget)` | per constructor | Replace the widget. The field keeps the attributes it derives (`maxlength`, `min`, ...). See [widgets](widgets.md). |
| `widget_attrs(Attrs)` | none | Merge HTML attributes into the current widget. |
| `template_name(text)` | none | Template used to render the whole field row. |

Constructor-specific options:

| Method | Applies to | Meaning |
|---|---|---|
| `max_length(usize)` | text fields; file fields (name length) | Adds a `max_length` validator and the `maxlength` attribute. |
| `min_length(usize)` | text fields | Adds a `min_length` validator and `minlength`. |
| `strip(bool)` | char, regex | Trim surrounding spaces before validating. |
| `empty_value(impl Into<Value>)` | char | What an empty submission cleans to. Use `Value::None` for nullable columns. |
| `min_value(v)`, `max_value(v)` | integer, float, decimal | Bounds; also set `min` and `max` on the number widget. |
| `step_size(f64)` | integer, float | Allowed step; the offset is `min_value` when it is set. |
| `max_digits(u32)`, `decimal_places(u32)` | decimal | Digit limits. |
| `input_formats(&[&str])` | date, time, datetime | Accepted `strftime` patterns. |
| `assume_scheme(&str)` | url | Scheme added to scheme-less input. |
| `allow_empty_file(bool)` | file, image | Accept zero-byte uploads. |
| `require_all_fields(bool)` | multi-value | See above. |

Options on a field kind they do not belong to are accepted and ignored (for example `strip` on an integer field),
except that `max_length`, `min_length`, `min_value` and `max_value` always add their validator.

Methods you may call on a built field: `clean(&Value) -> Result<Value, ValidationError>` (the whole pipeline),
`to_clean(&Value)`, `has_changed(&initial, &data)` and `hidden_widget()`.

## `#[derive(Form)]`

`#[derive(Form)]` turns a struct into a form definition. It generates two associated functions:

- `Struct::form() -> Form`, the unbound form;
- `Struct::from_form(&Form) -> Option<Struct>`, the typed result, `None` while the form is unbound or invalid.

```rust
use rango::forms::Form;

#[derive(Debug, Form)]
#[form(crate = "rango::forms")]
struct Signup {
    #[form(max_length = 30)]
    username: String,
    #[form(email, label = "E-mail")]
    email: String,
    #[form(widget = "password")]
    password: String,
    #[form(min_value = 18)]
    age: Option<u32>,
    #[form(choices = [("free", "Free"), ("pro", "Pro")])]
    plan: String,
    #[form(choices = [("a", "News"), ("b", "Offers")])]
    topics: Vec<String>,
    accept_terms: bool,
}
```

Put `#[form(crate = "rango::forms")]` on the struct when you depend on `rango` rather than on the
`rango-forms` crate directly; without it the macro refers to `::rango_forms`.

### Type mapping

The field comes from the last path segment of the field's type; `Option<T>` is looked through.

| Rust type | Form field |
|---|---|
| `String`, `str` | `Field::char()` |
| `bool` | `Field::boolean()` |
| `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `isize`, `usize` | `Field::integer()` |
| `f32`, `f64` | `Field::float()` |
| `Decimal` | `Field::decimal()` |
| `NaiveDate` | `Field::date()` |
| `NaiveTime` | `Field::time()` |
| `NaiveDateTime`, `DateTime` | `Field::datetime()` |
| `Uuid` | `Field::uuid()` |
| `Value` | `Field::json()` |
| `Vec<T>` | only with `choices` (then `Field::multiple_choice`) |
| anything else | compile error: "no form field for type ..." |

The value is read back with the `FromFormValue` trait (`rango::forms::FromFormValue`), implemented for `String`,
`bool`, the integer types, `f32`, `f64`, `Decimal`, `NaiveDate`, `NaiveTime`, `NaiveDateTime`, `DateTime<Utc>`,
`Uuid`, `serde_json::Value`, `Option<T>` and `Vec<T>`. An integer outside the target type's range makes
`from_form` return `None`. A field typed `DateTime` must be `chrono::DateTime<Utc>` to be readable. Note that the
`Value` mapping is chosen by name, so it is meant for `serde_json::Value`.

### Attributes

| Attribute | Argument | Effect |
|---|---|---|
| `email`, `url`, `slug`, `uuid`, `json` | flag | Use `Field::email()`, `url()`, `slug()`, `uuid()` or `json()` instead of the type's field. |
| `ip` | flag | `Field::generic_ip_address("both", false)`. |
| `regex = "pattern"` | string | `Field::regex(pattern)`. |
| `choices = [("v", "Label"), ...]` | array of string pairs | `Field::choice`, or `Field::multiple_choice` for a `Vec`. |
| `max_length = n`, `min_length = n` | number | `.max_length(n)`, `.min_length(n)`. |
| `min_value = n`, `max_value = n` | number | `.min_value(n)`, `.max_value(n)`. |
| `max_digits = n`, `decimal_places = n` | number | The same builder methods. |
| `label = "..."`, `help_text = "..."` | string | `.label(...)`, `.help_text(...)`. |
| `initial = expr` | any expression convertible into `Value` | `.initial(expr)`. |
| `required = true/false` | boolean literal | `.required(...)`. |
| `widget = "name"` | string | One of `textarea`, `password`, `hidden`, `radio`, `checkbox_multiple`, `select`, `number`, `email`, `date`. Any other name is silently ignored. |

An unknown key is a compile error ("unknown form option"). Without `required`, a field is required unless its
type is `Option<_>` or `bool`.

## See also

- [Working with forms](../topics/forms/forms.md)
- [Form validation](../topics/forms/validation.md)
- [Widgets reference](widgets.md)
- [Validators](../topics/validators.md)
- [Model forms](../topics/forms/modelforms.md)
