# Serializers

A serializer does two jobs: it validates incoming data (`run_validation`) and it shapes outgoing data
(`to_representation`). This page covers the plain `Serializer`, every `Field` constructor and option, validation
hooks, and `ModelSerializer<M>`, which derives its fields from a model and can create and update rows.

Data is `serde_json::Value` on both sides. A serializer never touches typed structs directly; `ModelSerializer`
converts between model instances and JSON for you.

```rust
use rango::json;
use rango::rest::{Field, ModelSerializer, Serializer};
```

`rango::json!` builds values. To name the types (`serde_json::Value`, `serde_json::Map`) in your own signatures, add
`serde_json` to your `Cargo.toml`; Rango re-exports only the macro.

## Declaring a serializer

`Serializer::new(name)` starts an empty serializer; `.field(name, field)` adds fields in order. The name is used as
the component name in the OpenAPI document.

```rust
let signup = Serializer::new("Signup")
    .field("email", Field::email())
    .field("display_name", Field::char().max_length(50))
    .field("age", Field::integer().min_value(13).required(false))
    .field("newsletter", Field::boolean().default(json!(false)))
    .field("password", Field::char().min_length(8).write_only(true));
```

## Validating input

`run_validation(&data, partial)` returns the validated data as a JSON map, or the errors as a JSON value.

```rust
let data = json!({"email": "ann@example.com", "display_name": " Ann ", "password": "s3cret-pw"});
match signup.run_validation(&data, false) {
    Ok(valid) => {
        // {"email": "ann@example.com", "display_name": "Ann", "newsletter": false, "password": "s3cret-pw"}
        assert_eq!(valid["display_name"], "Ann");
    }
    Err(errors) => {
        // {"field": ["message", ...], ...}
    }
}
```

For each field, in declaration order:

1. A read-only field is skipped; anything sent for it is ignored.
2. If the key is absent: with `partial = true` the field is skipped. Otherwise the `default` is used if there is one;
   failing that, a required field reports `This field is required.` and an optional one is left out of the result.
3. If the value is `null`: accepted when `allow_null(true)`, otherwise `This field may not be null.`
4. The value is converted by the field type (see the table below). Conversion errors stop here.
5. The field's `validator` functions run on the converted value. All of them run; every message is collected.

If any field failed, the result is `Err` with all field errors and the object-level `validate` hook does not run.

In the validated map, a field with a `source` is stored under its source name; the others under the field name.

`partial = true` is what `PATCH` uses: only the keys that are present are validated, and defaults are not applied.

### In a view

Turn the errors into a 400 response with `ApiException::validation`:

```rust
use rango::rest::{ApiException, ApiRequest, Response};

async fn signup_view(req: ApiRequest, _args: rango::urls::ViewArgs) -> rango::Result<Response> {
    let signup = Serializer::new("Signup").field("email", Field::email());
    let valid = signup.run_validation(&req.data, false).map_err(ApiException::validation)?;
    Ok(Response::created(json!({"email": valid["email"]})))
}
```

## Rendering output

`to_representation(&instance)` takes a JSON map keyed by source name and returns the output object: one key per
field, in declaration order, without the write-only fields.

```rust
let instance = json!({"email": "ann@example.com", "display_name": "Ann", "password": "hash"});
let out = signup.to_representation(instance.as_object().unwrap());
// {"email": "ann@example.com", "display_name": "Ann", "age": null, "newsletter": null}
```

Values are copied as they are; no formatting or type conversion happens on output. A key missing from the instance
is rendered as `null`.

## Field types

Every constructor returns a `Field` that is required, not nullable, readable and writable.

| Constructor | Accepts | Validated value | Error messages |
| --- | --- | --- | --- |
| `Field::boolean()` | `true`/`false`, `1`/`0`, and the strings `t`, `y`, `yes`, `true`, `on`, `1` / `f`, `n`, `no`, `false`, `off`, `0` (lower, capitalised or upper case) | boolean | `Must be a valid boolean.` |
| `Field::char()` | strings and numbers (numbers become text); surrounding whitespace is trimmed | string | `Not a valid string.`, `This field may not be blank.`, `Ensure this field has no more than N characters.`, `Ensure this field has at least N characters.` |
| `Field::email()` | as `char`, must be an email address | string | `Enter a valid email address.` |
| `Field::slug()` | as `char`, only ASCII letters, digits, `-` and `_` | string | `Enter a valid "slug" consisting of letters, numbers, underscores or hyphens.` |
| `Field::url()` | as `char`, must be a URL | string | `Enter a valid URL.` |
| `Field::uuid()` | as `char`, must parse as a UUID | hyphenated lower-case string | `Must be a valid UUID.` |
| `Field::integer()` | integers and numeric strings; `"7"`, `7.0` and `"7.0"` are accepted, `1.5` is not | integer | `A valid integer is required.`, `String value too large.` (over 1000 characters), `Ensure this value is less than or equal to N.`, `Ensure this value is greater than or equal to N.` |
| `Field::float()` | numbers and numeric strings | number | `A valid number is required.`, plus the two range messages |
| `Field::decimal(max_digits, decimal_places)` | numbers and numeric strings, including scientific notation | string, rescaled to `decimal_places` (`"4.5"` becomes `"4.50"`) | `A valid number is required.`, `String value too large.`, `Ensure that there are no more than N digits in total.`, `Ensure that there are no more than N decimal places.`, `Ensure that there are no more than N digits before the decimal point.` |
| `Field::datetime()` | ISO 8601 strings, with or without an offset | UTC string `YYYY-MM-DDTHH:MM:SS[.ffffff]Z` | `Datetime has wrong format. Use one of these formats instead: YYYY-MM-DDThh:mm[:ss[.uuuuuu]][+HH:MM\|-HH:MM\|Z].` |
| `Field::date()` | `YYYY-MM-DD` | the same string | `Date has wrong format. Use one of these formats instead: YYYY-MM-DD.` |
| `Field::time()` | `hh:mm[:ss[.uuuuuu]]` | `HH:MM:SS[.ffffff]` | `Time has wrong format. Use one of these formats instead: hh:mm[:ss[.uuuuuu]].` |
| `Field::duration()` | `[DD] [HH:[MM:]]ss[.uuuuuu]` as a string or a number of seconds | normalised duration string | `Duration has wrong format. Use one of these formats instead: [DD] [HH:[MM:]]ss[.uuuuuu].` |
| `Field::choice(choices)` | one of the listed values, compared by value or by text form (`"2"` matches `2`) | the listed value | `"x" is not a valid choice.` |
| `Field::json()` | any JSON value | unchanged | none |
| `Field::list(child)` | an array; each item goes through `child` | array | `Expected a list of items but got type "str".`, or the child's messages |
| `Field::related(model_label, many)` | a primary key, or an array of them when `many` | unchanged | `Incorrect type. Expected pk value, received list.`, `Expected a list of items but got type "int".` |
| `Field::method(f)` | nothing (always read-only) | n/a | none |
| `Field::nested(serializer, many)` | an object, or an array of objects when `many` | object / array of validated objects | the nested errors (see below) |

Notes on individual types:

- A datetime without an offset is taken as UTC. One with an offset is converted to UTC.
- `Field::decimal` takes `Option<u32>` for both arguments; `None` skips that check.
- `Field::choice` takes any iterator of `(value, label)` pairs:
  `Field::choice([(json!("draft"), "Draft".to_owned()), (json!("live"), "Live".to_owned())])`.
- Where a message names a type, the names are `NoneType`, `bool`, `int`, `float`, `str`, `list` and `dict`.
- `Field::new(kind)` builds a field from a `rango::rest::serializers::Kind` value directly. This is the only way to
  turn off whitespace trimming: `Kind::Char { trim: false, .. }`.

## Field options

All options are builder methods that take and return the field.

| Method | Default | Effect |
| --- | --- | --- |
| `.read_only(bool)` | `false` | Shown in output, ignored in input. |
| `.write_only(bool)` | `false` | Accepted in input, left out of output. |
| `.required(bool)` | unset | Whether the key must be present. When unset, a field is required unless it has a default or is read-only. |
| `.default(Json)` | none | Value used when the key is absent (not in partial validation). The default is not validated. |
| `.allow_null(bool)` | `false` | Accept `null`. |
| `.allow_blank(bool)` | `false` | Accept the empty string. Text fields only. |
| `.max_length(usize)` / `.min_length(usize)` | none | Length limits in characters. Text fields only. |
| `.min_value(i64)` / `.max_value(i64)` | none | Range limits. Integer and float fields only. |
| `.source(&str)` | the field name | Key in the instance data and in the validated data. |
| `.label(&str)` | none | Human label, stored on the field. |
| `.help_text(&str)` | none | Becomes the property `description` in the OpenAPI document. |
| `.validator(f)` | none | Adds a check; may be called several times. |

Options that do not apply to a field's type are ignored without an error: `Field::integer().max_length(5)` is an
integer field with no length check.

`field.is_required()` reports the effective requirement.

## Field-level validation

A validator is a function from the converted value to `Result<(), String>`. It runs after the type conversion, so an
integer validator always sees a JSON integer.

```rust
let even = Field::integer().validator(|v| {
    if v.as_i64().is_some_and(|n| n % 2 == 0) { Ok(()) } else { Err("Enter an even number.".to_owned()) }
});
let s = Serializer::new("Batch").field("size", even);

assert_eq!(s.run_validation(&json!({"size": 3}), false).unwrap_err(), json!({"size": ["Enter an even number."]}));
```

Validators do not run for `null` values, for defaults, or for the items of a `Field::list`.

## Object-level validation

`.validate(f)` registers one function that sees all validated fields together. It receives a mutable map, so it can
also normalise or add values. Return the errors as a JSON object: a field name mapped to a list of messages, or the
key `non_field_errors` for errors that belong to no single field.

```rust
let booking = Serializer::new("Booking")
    .field("start", Field::date())
    .field("end", Field::date())
    .validate(|attrs| {
        // ISO dates compare correctly as strings.
        if attrs["end"].as_str() < attrs["start"].as_str() {
            return Err(json!({"non_field_errors": ["end must not be before start."]}));
        }
        Ok(())
    });
```

Calling `.validate` again replaces the previous function. The hook runs only when every field passed. With partial
validation, remember that keys may be absent from the map.

## Nested and list fields

`Field::list(child)` validates an array of scalars. `Field::nested(serializer, many)` embeds another serializer.

```rust
let line = Serializer::new("Line")
    .field("sku", Field::char().max_length(20))
    .field("quantity", Field::integer().min_value(1));

let order = Serializer::new("Order")
    .field("reference", Field::char())
    .field("tags", Field::list(Field::slug()).required(false))
    .field("lines", Field::nested(line, true));
```

On output, a nested field renders the object (or each object of the array) found under its source through the nested
serializer; anything else becomes `null` (or an empty array for `many`).

Limits to know about:

- `Field::list` reports the messages of the first failing item only, without its index.
- Errors inside a nested serializer are reported as one string holding the nested error object as JSON text
  (`{"lines": ["{\"quantity\":[\"A valid integer is required.\"]}"]}`), not as a nested object.
- Nested validation is never partial.
- `ModelSerializer` does not save nested data. See [Not supported yet](#not-supported-yet).

## Related fields

`Field::related("app_label.Model", many)` represents a relation by primary key. On its own it only checks the shape
of the input (a scalar, or an array of scalars for `many`). Inside a `ModelSerializer` each key is also looked up in
the related table, and a missing row reports `Invalid pk "9" - object does not exist.`

## Method fields

`Field::method(f)` is a read-only field computed from the instance data. The function receives the whole instance map.

```rust
let s = Serializer::new("Person")
    .field("first", Field::char())
    .field("last", Field::char())
    .field("full_name", Field::method(|obj| {
        json!(format!("{} {}", obj["first"].as_str().unwrap_or(""), obj["last"].as_str().unwrap_or("")))
    }));
```

With a `ModelSerializer`, the map holds the model's concrete fields as JSON (keyed by field name), plus the id lists
of the many-to-many fields that the serializer includes. The function is synchronous and has no database access.

## ModelSerializer

`ModelSerializer<M>` builds the fields from the model `M` and adds database-backed validation and saving.

```rust
use rango::rest::ModelSerializer;

let all = ModelSerializer::<Article>::all()?;                             // every field
let some = ModelSerializer::<Article>::fields(&["id", "title", "slug"])?; // these, in this order
let most = ModelSerializer::<Article>::all()?.exclude(&["body"]);         // all but these
```

| Method | Meaning |
| --- | --- |
| `ModelSerializer::<M>::all()` | One field per concrete model field and per many-to-many field, in model order. Returns `Result`. |
| `ModelSerializer::<M>::fields(&[...])` | Only the named model fields, in the given order. An unknown name is an error: ``Field name `x` is not valid for model `Article`.`` |
| `.exclude(&[...])` | Drop fields. Unknown names are ignored. |
| `.read_only_fields(&[...])` | Mark fields read-only. |
| `.field(name, field)` | Replace a derived field (it keeps its position) or append a new one. |
| `.validate(f)` | Object-level validation, as on `Serializer`. The map is keyed by field name. |
| `.name(&str)` | Component name in the OpenAPI document (defaults to the model name). |

The underlying `Serializer` is the public `serializer` member, for example to inspect `all.serializer.fields`.

### How model fields map to serializer fields

| Model field | Serializer field |
| --- | --- |
| auto primary keys (`auto`, big, small) | `Field::integer()`, read-only |
| boolean | `Field::boolean()` |
| char, text, file, image, generic IP address | `Field::char()` |
| email / slug / URL / UUID | `Field::email()` / `Field::slug()` / `Field::url()` / `Field::uuid()` |
| integer types | `Field::integer()` with the column's range (for example 0 to 32767 for a positive small integer) |
| float | `Field::float()` |
| decimal | `Field::decimal(max_digits, decimal_places)` |
| datetime / date / time / duration | `Field::datetime()` / `Field::date()` / `Field::time()` / `Field::duration()` |
| JSON | `Field::json()` |
| binary | `Field::char()`, read-only |
| any field with `choices` | `Field::choice(...)` with the model's choices |
| foreign key, one-to-one | `Field::related(target, false)`; the value is the related primary key |
| many-to-many | `Field::related(target, true)`; the value is a list of primary keys |

Options are taken from the model field:

- `max_length` and `blank` set the text field's `max_length` and `allow_blank`.
- `null` sets `allow_null`.
- A field with a default, `blank` or `null` is not required.
- Auto primary keys, non-editable fields and `auto_now` / `auto_now_add` fields are read-only.
- `help_text` is copied. The label is the capitalised verbose name (`ID` for a primary key called `id`).
- A `unique` field that is not the primary key gets a uniqueness check.

A relation field is named after the model field (`author`, not `author_id`).

### Overriding a field

Replace the derived field when you need different rules. The replacement is used as given, so restate the options you
still want:

```rust
let ser = ModelSerializer::<Article>::all()?
    .field("title", Field::char().max_length(20).min_length(3))
    .field("internal_notes", Field::char().allow_blank(true).required(false).write_only(true))
    .read_only_fields(&["views"]);
```

A replacement loses the derived uniqueness check. `Field` has a public `unique: Option<String>` member holding the
message; set it on your field to keep the check.

### Unique validators

For each unique field, validation queries the model's table for another row with the same value. On update the
instance being updated is excluded. `null` values are not checked. The message is
`<model verbose name> with this <field verbose name> already exists.`:

```json
{"slug": ["article with this slug already exists."]}
```

Multi-column constraints (`unique_together`, unique constraints) are not checked by the serializer; a violation
surfaces as a database error.

### Using a ModelSerializer directly

`ModelViewSet` calls these for you. In a hand-written view:

```rust
use rango::db::Db;
use rango::rest::{ApiException, ApiRequest, ModelSerializer, Response};

async fn create_article(req: ApiRequest, _args: rango::urls::ViewArgs) -> rango::Result<Response> {
    let db = Db::default_db();
    let ser = ModelSerializer::<Article>::all()?;
    let valid = ser.validate_data(&db, &req.data, None, false).await?.map_err(ApiException::validation)?;
    let article = ser.create(&db, &valid).await?;
    Ok(Response::created(ser.represent(&db, &article).await?))
}
```

| Method | Meaning |
| --- | --- |
| `validate_data(db, &data, instance, partial).await` | Field validation, uniqueness checks, related-row checks, then the object-level hook. Returns `Result<Result<Map, Json>>`: the outer error is a database failure, the inner one holds the validation errors. Pass the instance when updating so that it is excluded from uniqueness checks. |
| `create(db, &validated).await` | Build a blank instance, apply the data, save it, set many-to-many rows. Needs `M: Default`. |
| `update(db, &instance, &validated).await` | Apply the data to a copy of the instance and save it. Returns the saved instance. |
| `represent(db, &instance).await` | The output JSON for one instance. |
| `instance_data(db, &instance).await` | The source-name to JSON map that `represent` renders. |

The connection is explicit everywhere, as in the rest of Rango. `represent` runs one query per included
many-to-many field to read the related ids, and none otherwise.

Validated data that does not correspond to a model field (an extra declared field, a nested serializer) is ignored
by `create` and `update`. Read it from the validated map yourself if you need it.

## Error response format

Validation errors are a JSON object mapping each failing field to a list of messages. Fields appear in serializer
order. Requests replayed against a model with `title` (max 20), `slug` (unique), `body`, `published`, `views` and
`rating` (4 digits, 2 decimal places):

```json
{"title":["This field is required."],"slug":["This field is required."]}
```

```json
{"title":["This field may not be blank."],
 "slug":["Enter a valid \"slug\" consisting of letters, numbers, underscores or hyphens."],
 "published":["Must be a valid boolean."],
 "views":["A valid integer is required."],
 "rating":["Ensure that there are no more than 4 digits in total."]}
```

A body that is not a JSON object:

```json
{"non_field_errors":["Invalid data. Expected a dictionary, but got list."]}
```

The `non_field_errors` key in that last message can be renamed with the `NON_FIELD_ERRORS_KEY` entry of the
`rest_framework` setting. Errors returned by your own `validate` hook use whatever keys you return.

In a view, `ApiException::validation(errors)` produces the 400 response with these errors as the body.

## Not supported yet

- Writing nested objects through `ModelSerializer` (nested serializers are validated but not saved).
- Hyperlinked relations and a `depth` option; relations are always primary keys.
- Asynchronous or database-aware method fields and validators.
- Validation of multi-column uniqueness.
- Per-item error positions for list fields, and structured (non-string) errors for nested serializers.

## See also

- [REST quickstart](quickstart.md)
- [Viewsets and routers](viewsets-routers.md)
- [Function-based API views](api-views.md)
- [OpenAPI schema](openapi.md)
- [Model fields reference](../../ref/model-fields.md)
- [Validators](../validators.md)
- [Serialization of model data](../serialization.md)
