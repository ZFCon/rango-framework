# Validators

A validator checks one value and either accepts it or returns a `ValidationError` carrying a message, a code and
parameters. `rango::core::validators` contains the built-in validators used by form fields and available to your own
code. This page lists every one of them with its code and default message.

```rust
use rango::core::exceptions::ValidationError;
use rango::core::validators::{
    validate_email, EmailValidator, MaxLengthValidator, RegexValidator, Validator,
};
```

## The `Validator` trait

```rust
pub trait Validator<T: ?Sized>: Send + Sync + Debug {
    fn validate(&self, value: &T) -> Result<(), ValidationError>;
}
```

`Validator<str>` is implemented by the text validators, `Validator<Decimal>` by `DecimalValidator`,
`Validator<f64>` by `StepValueValidator`, and `Validator<T>` for any `T: PartialOrd + Display + Clone + Send + Sync + Debug`
by `MaxValueValidator<T>` and `MinValueValidator<T>`.

The validators that take no configuration are plain functions: `validate_email`, `validate_slug` and so on. They have
the signature `fn(&str) -> Result<(), ValidationError>`.

```rust
use rango::core::validators::{validate_slug, MaxLengthValidator, Validator};

assert!(validate_slug("my-post_1").is_ok());

let err = validate_slug("no spaces").unwrap_err();
assert_eq!(err.code(), Some("invalid"));

let v = MaxLengthValidator::new(5);
let err = v.validate("abcdef").unwrap_err();
assert_eq!(err.messages(), vec!["Ensure this value has at most 5 characters (it has 6)."]);
```

Default messages are produced when the error is raised, so the language active at that moment is used. Every
configurable validator has a `with_message(...)` builder that replaces the default text.

### Wrapping a closure

`FnValidator` turns a closure into a validator:

```rust
use rango::core::exceptions::ValidationError;
use rango::core::validators::{FnValidator, Validator};

let no_admin = FnValidator(|v: &str| {
    if v.eq_ignore_ascii_case("admin") {
        Err(ValidationError::new("That name is reserved.").with_code("reserved"))
    } else {
        Ok(())
    }
});
assert!(no_admin.validate("admin").is_err());
```

## Using validators on form fields

A form field stores validators as `Fn(&Value) -> Result<(), ValidationError>`. Two routes lead there.

Your own closure goes through `Field::validator`:

```rust
use rango::core::exceptions::ValidationError;
use rango::forms::{Field, Value};

let field = Field::char().validator(|v: &Value| {
    if v.to_text().starts_with("rango-") {
        Ok(())
    } else {
        Err(ValidationError::new("Must start with rango-.").with_code("bad_prefix"))
    }
});
```

The built-in validators are lifted to that shape by the helpers in `rango::forms::fields::validators`:
`max_length(n)`, `min_length(n)`, `prohibit_null_characters()`, `regex(pattern)`, `email()`, `slug()`,
`unicode_slug()`, `url()`, `ip(protocol, unpack_ipv4)`, `decimal(max_digits, decimal_places)`,
`file_extension(&[...])`, `max_value(Value)`, `min_value(Value)` and `step_value(step, offset)`. Most fields already
attach the right ones; see the [form fields reference](../ref/form-fields.md) for which. Validators run only for a
non-empty value, and every failing validator contributes an error (see [validation](forms/validation.md)).

A field's `error_message(code, text)` replaces the message of any validator error with that code, so
`Field::char().max_length(10).error_message("max_length", "Too long.")` works without touching the validator.

## Regular expressions

### `RegexValidator`

```rust
RegexValidator::new(pattern: &str) -> Result<RegexValidator>
```

| Builder | Effect |
|---|---|
| `with_message(text)` | Message used on failure. |
| `with_default_message(fn() -> String)` | Lazily computed default message (used for translations). |
| `with_code(code)` | Error code. Default `invalid`. |
| `inverse_match(bool)` | Fail when the pattern *does* match. |

Default message: `Enter a valid value.` Code: `invalid`. Parameter: `value`.

Patterns use the `fancy_regex` syntax (look-around is available). The match is a search, not an anchor: write `^`
and `\Z` yourself. `\Z` (end of text) is accepted and rewritten to `\z`. Use `(?i)` for case-insensitive matching.
An invalid pattern returns an `ImproperlyConfigured` error from `new`.

```rust
let v = RegexValidator::new(r"^[A-Z]{3}-\d{4}\Z")?
    .with_message("Use the form ABC-1234.")
    .with_code("bad_sku");
```

## Text validators

| Validator | Accepts | Code | Default message |
|---|---|---|---|
| `validate_email` / `EmailValidator` | an address of at most 320 characters | `invalid` | Enter a valid email address. |
| `validate_slug` | letters, digits, `_`, `-` (`^[-a-zA-Z0-9_]+\Z`) | `invalid` | Enter a valid "slug" consisting of letters, numbers, underscores or hyphens. |
| `validate_unicode_slug` | Unicode letters, digits, `_`, `-` (`^[-\w]+\Z`) | `invalid` | Enter a valid "slug" consisting of Unicode letters, numbers, underscores, or hyphens. |
| `validate_integer` | `^-?\d+\Z` | `invalid` | Enter a valid integer. |
| `validate_comma_separated_integer_list` | non-negative integers separated by `,` | `invalid` | Enter only digits separated by commas. |
| `validate_domain_name` | a domain name, IDNA allowed | `invalid` | Enter a valid domain name. |
| `UrlValidator` | a URL with an allowed scheme | `invalid` | Enter a valid URL. |
| `MinLengthValidator` | at least `limit` characters | `min_length` | Ensure this value has at least %(limit_value)d character(s) (it has %(show_value)d). |
| `MaxLengthValidator` | at most `limit` characters | `max_length` | Ensure this value has at most %(limit_value)d character(s) (it has %(show_value)d). |
| `ProhibitNullCharactersValidator` | text without `\0` | `null_characters_not_allowed` | Null characters are not allowed. |

The message of the length validators is singular or plural according to the limit. Length is counted in characters,
not bytes. All of these report the offending text as the `value` parameter; the length validators also set
`limit_value` and `show_value` (the actual length).

### `EmailValidator`

```rust
let v = EmailValidator::default()
    .with_allowlist(vec!["localhost".into(), "intranet".into()])
    .with_message("That does not look like an e-mail address.")
    .with_code("bad_email");
v.validate("me@intranet")?;
```

The local part may be a dotted atom or a quoted string. The domain must be a hostname with a top-level domain, a
bracketed IPv4 or IPv6 literal such as `[127.0.0.1]`, or one of the names in `domain_allowlist` (default
`["localhost"]`). `validate_email(value)` is `EmailValidator::default().validate(value)`.

### `UrlValidator`

`UrlValidator::new()` returns a `Result` (it compiles a regex). Builders: `with_schemes(&["https"])` and
`with_message(text)`; the public field `schemes` holds the allowed list, default `http`, `https`, `ftp`, `ftps`.
A URL is rejected when it contains a tab, carriage return or newline, exceeds the maximum URL length, uses another
scheme, has an invalid bracketed IPv6 host, or has a host longer than 253 characters. Host forms accepted: domain
names with a TLD, `localhost`, IPv4 and bracketed IPv6; optional `user:password@` and `:port`.

### `DomainNameValidator`

`DomainNameValidator::new(accept_idna: bool)`. With `accept_idna = false` only ASCII names validate. Names longer
than 255 bytes fail in both modes. `validate_domain_name` is the `accept_idna = true` instance.

### Comma-separated integer lists

`int_list_validator(sep, message: Option<&str>, code, allow_negative) -> Result<RegexValidator>` builds a validator
for lists such as `1;2;-3`. The separator is escaped for you.

```rust
use rango::core::validators::{int_list_validator, Validator};

let v = int_list_validator(";", None, "invalid", true)?;
assert!(v.validate("1;2;-3").is_ok());
assert!(v.validate("1;;2").is_err());
```

## IP address validators

| Function | Accepts | Code | Message |
|---|---|---|---|
| `validate_ipv4_address` | dotted IPv4 | `invalid` | Enter a valid IPv4 address. |
| `validate_ipv6_address` | IPv6 text | `invalid` | Enter a valid IPv6 address. |
| `validate_ipv46_address` | either | `invalid` | Enter a valid IPv4 or IPv6 address. |

The message is `Enter a valid %(protocol)s address.`; `protocol` and `value` are the parameters.

`ip_address_validators(protocol: &str, unpack_ipv4: bool) -> Result<fn(&str) -> Result<(), ValidationError>>`
returns the function for `"both"`, `"ipv4"` or `"ipv6"` (case-insensitive). It returns an error for any other
protocol name, and when `unpack_ipv4` is `true` with a protocol other than `"both"`.

## Number validators

### `MaxValueValidator<T>` and `MinValueValidator<T>`

```rust
MaxValueValidator::new(limit: T)
MaxValueValidator::from_fn(|| limit)   // limit computed at validation time
.with_message(text)
```

`T` can be any type that is `PartialOrd + Display + Clone + Send + Sync + Debug`: integers, `Decimal`, dates. A
limit from `from_fn` is re-evaluated on every call, which suits "not before today".

| Validator | Fails when | Code | Default message |
|---|---|---|---|
| `MaxValueValidator` | `value > limit` | `max_value` | Ensure this value is less than or equal to %(limit_value)s. |
| `MinValueValidator` | `value < limit` | `min_value` | Ensure this value is greater than or equal to %(limit_value)s. |

Parameters: `limit_value`, `show_value`, `value`.

```rust
use rango::core::validators::{MinValueValidator, Validator};

let adult = MinValueValidator::new(18_i32);
assert_eq!(adult.validate(&16).unwrap_err().code(), Some("min_value"));
```

### `StepValueValidator`

`StepValueValidator::new(step: f64)`, optionally `.with_offset(f64)` and `.with_message(text)`. Validates `f64`
values. A value passes when it is a whole multiple of `step` counted from `offset` (default `0`); the comparison
allows a rounding error of `1e-9`. Code: `step_size`.

- Without an offset: `Ensure this value is a multiple of step size %(limit_value)s.`
- With an offset: `Ensure this value is a multiple of step size %(limit_value)s, starting from %(offset)s, e.g. %(offset)s, %(valid_value1)s, %(valid_value2)s, and so on.`

### `DecimalValidator`

`DecimalValidator::new(max_digits: Option<u32>, decimal_places: Option<u32>)` validates a `rust_decimal::Decimal`.
Checks run in this order and stop at the first failure.

| Check | Code | Message |
|---|---|---|
| total digits above `max_digits` | `max_digits` | Ensure that there are no more than %(max)s digit(s) in total. |
| decimal places above `decimal_places` | `max_decimal_places` | Ensure that there are no more than %(max)s decimal place(s). |
| digits before the point above `max_digits - decimal_places` (needs both) | `max_whole_digits` | Ensure that there are no more than %(max)s digit(s) before the decimal point. |

Parameters: `max`, `value`. Digit counting treats `0.5` as one digit and counts trailing zeros kept in the value's
scale, so `1.50` has three digits and two places.

## File validators

`FileExtensionValidator::new(Some(&["pdf", "png"]))` validates a file *name*. Extensions are compared in lower case
and without the dot. `new(None)` accepts anything. Code `invalid_extension`; message
`File extension "%(extension)s" is not allowed. Allowed extensions are: %(allowed_extensions)s.`; parameters
`extension`, `allowed_extensions`, `value`. `with_message(text)` overrides the message.

Image form fields attach this validator with a built-in list of image extensions.

## Not supported yet

- Validators are not generic over arbitrary types for text: the text validators implement `Validator<str>` only, so
  use the helpers in `rango::forms::fields::validators` to attach them to a field holding a `Value`.
- `ProhibitNullCharactersValidator` has no message or code builder; only the default is available.

## See also

- [Form and field validation](forms/validation.md)
- [Form fields reference](../ref/form-fields.md)
- [Model forms](forms/modelforms.md)
