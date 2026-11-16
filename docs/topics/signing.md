# Cryptographic signing

Signing lets you hand a value to an untrusted party (a browser, an email recipient) and later check that it comes
back unchanged. Rango signs with HMAC and your `secret_key`; the value itself is not encrypted, only protected
against modification. This page covers `Signer`, `TimestampSigner`, the `dumps`/`loads` helpers, salts, expiry,
key rotation and signed cookies. The code is in `rango::core::signing`.

Signed data is readable by anyone who holds it. Never put a secret in a signed value; put an identifier in it.

## Quick start

```rust
use std::time::Duration;
use rango::conf::{self, Settings};
use rango::core::signing::{self, BadSignature};

conf::configure(Settings { secret_key: "a-long-random-key".into(), ..Settings::default() })?;

// Any serde type goes in, a URL-safe string comes out.
let token = signing::dumps(&("activate", 42), None, "myapp.activation", true)?;

// `loads` has two layers of Result: the outer one is a configuration error
// (for example an empty secret_key), the inner one is the verdict on the value.
match signing::loads::<(String, i64)>(&token, None, "myapp.activation", Some(Duration::from_secs(3600)), None)? {
    Ok((what, id)) => println!("{what} {id}"),
    Err(BadSignature::Expired(why)) => println!("link expired: {why}"),
    Err(BadSignature::Mismatch(why)) => println!("tampered or malformed: {why}"),
}
```

## `dumps` and `loads`

```rust
pub const DEFAULT_DUMPS_SALT: &str = "rango.core.signing";

pub fn dumps<T: Serialize + ?Sized>(
    obj: &T, key: Option<&str>, salt: &str, compress: bool,
) -> Result<String>;

pub fn loads<T: DeserializeOwned>(
    s: &str, key: Option<&str>, salt: &str,
    max_age: Option<Duration>, fallback_keys: Option<Vec<String>>,
) -> Result<std::result::Result<T, BadSignature>>;
```

- `obj` is serialised to compact JSON, optionally zlib-compressed, base64url-encoded (no padding) and then signed
  with a timestamp. The result has the shape `payload:timestamp:signature`. A compressed payload starts with a
  dot.
- `compress: true` only compresses when the compressed form is smaller than the original; otherwise the plain form
  is used. `loads` handles both forms.
- `key: None` means `settings.secret_key`. Pass `Some(..)` to use a separate key for a specific purpose.
- `salt` separates uses of the same key from each other (see [Salts](#salts)). `DEFAULT_DUMPS_SALT` is the
  conventional default; pass your own constant for every use.
- `max_age: None` accepts a value of any age. `Some(d)` rejects values signed more than `d` ago.
- `fallback_keys: None` means `settings.secret_key_fallbacks` (see [Rotating the secret key](#rotating-the-secret-key)).

Example values (key `"k"`, salt `"salt"`, signed at Unix time 1700000000, which is `1r31eq` in base 62):

| Input | Output |
|---|---|
| `"plain string"` | `InBsYWluIHN0cmluZyI:1r31eq:-heWBzoMsecoP_o9-9kZ704TmD5cykBzA7ZTKDUuf3E` |
| `{"a": 1, "b": [1, 2, "é"]}` | `eyJhIjoxLCJiIjpbMSwyLCJcdTAwZTkiXX0:1r31eq:e9sbLdGcRYdUG4oTPq13dJ-AEl44eO7_-JTQiF-_LMk` |

Non-ASCII characters are escaped as `\uXXXX` in the JSON before encoding, so the payload is pure ASCII.

`dumps` and `loads` always use a `TimestampSigner`. The outer `Result` of `loads` is an `rango::Error`; the inner
error is the `BadSignature` enum below.

## `BadSignature`

```rust
pub enum BadSignature {
    Mismatch(String), // wrong signature, missing separator, bad base64/zlib/JSON, bad timestamp
    Expired(String),  // the signature is valid but older than max_age
}
impl BadSignature { pub fn is_expired(&self) -> bool; }
```

It implements `std::error::Error` and `Display` (the message is the inner string). `Expired` is only returned
after the signature itself was verified, so an expired value is known to be genuine; a `Mismatch` is never
evidence of anything except tampering or garbage.

Decoding failures inside the payload (the signature matched, but the JSON does not deserialise into your `T`) also
surface as `Mismatch`.

## `Signer`

`Signer` signs a string as `value<sep>signature`, with no timestamp.

```rust
use rango::core::signing::{Signer, SignerOptions};

let signer = Signer::new(SignerOptions::default())?; // uses secret_key
let signed = signer.sign("hello");                   // "hello:<signature>"
assert_eq!(signer.unsign(&signed)?, "hello");        // Err(BadSignature) on any change
```

| Item | Signature |
|---|---|
| `Signer::new` | `fn new(options: SignerOptions) -> Result<Signer>` |
| `sign` | `fn sign(&self, value: &str) -> String` |
| `unsign` | `fn unsign(&self, signed_value: &str) -> Result<String, BadSignature>` |
| `sign_object` | `fn sign_object<T: Serialize + ?Sized>(&self, obj: &T, compress: bool) -> rango::Result<String>` |
| `unsign_object` | `fn unsign_object<T: DeserializeOwned>(&self, signed_obj: &str) -> Result<T, BadSignature>` |

`unsign` splits at the last separator, so the value may itself contain the separator. Comparison of signatures is
constant-time.

### `SignerOptions`

```rust
pub struct SignerOptions {
    pub key: Option<String>,             // default: settings.secret_key
    pub sep: String,                     // default: ":"
    pub salt: Option<String>,            // default: "rango.core.signing.Signer"
    pub algorithm: Option<HashAlgorithm>,// default: sha256
    pub fallback_keys: Option<Vec<String>>, // default: settings.secret_key_fallbacks
}
```

Build it with struct update syntax: `SignerOptions { salt: Some("myapp.invite".into()), ..SignerOptions::default() }`.

- An empty `key` or `salt` is treated as unset and replaced by the default.
- `algorithm` is a `rango::utils::crypto::HashAlgorithm` (`Md5`, `Sha1`, `Sha224`, `Sha256`, `Sha384`, `Sha512`);
  `"sha1".parse::<HashAlgorithm>()` also works. Keep the default unless you must interoperate with an existing
  format.
- `Signer::new` returns an error when `key` is unset and `settings.secret_key` is empty
  ("The SECRET_KEY setting must not be empty."), and when settings are not configured.
- `sep` must contain at least one character outside the signature alphabet. A separator that is empty, or made
  only of ASCII letters, digits, `-`, `=` or the punctuation between `Z` and `a` (`[ \ ] ^ _` and a backtick) is
  rejected with an error, because it could appear inside the base64 signature and make splitting ambiguous.

## `TimestampSigner`

`TimestampSigner` embeds the signing time (seconds since the epoch, in base 62) so a value can expire. The signed
form is `value:timestamp:signature`.

```rust
use std::time::Duration;
use rango::core::signing::{TimestampSigner, SignerOptions};

let signer = TimestampSigner::new(SignerOptions {
    salt: Some("myapp.password-reset".into()),
    ..SignerOptions::default()
})?;

let token = signer.sign("user-17");
let value = signer.unsign(&token, Some(Duration::from_secs(15 * 60)))?; // Err(Expired) after 15 minutes
```

| Method | Signature |
|---|---|
| `new` | `fn new(options: SignerOptions) -> Result<Self>` (default salt `rango.core.signing.TimestampSigner`) |
| `sign` | `fn sign(&self, value: &str) -> String` |
| `sign_at` | `fn sign_at(&self, value: &str, timestamp: i64) -> String` |
| `unsign` | `fn unsign(&self, value: &str, max_age: Option<Duration>) -> Result<String, BadSignature>` |
| `unsign_at` | `fn unsign_at(&self, value: &str, max_age: Option<Duration>, now: f64) -> Result<String, BadSignature>` |
| `sign_object` / `sign_object_at` | as `Signer::sign_object`, with an optional explicit timestamp |
| `unsign_object` / `unsign_object_at` | as `Signer::unsign_object`, with `max_age` (and an explicit `now`) |

The `_at` variants take the time explicitly. Use them in tests so you do not depend on the wall clock:

```rust
let signer = TimestampSigner::new(SignerOptions {
    key: Some("k".into()),
    salt: Some("s".into()),
    ..SignerOptions::default()
})?;
let signed = signer.sign_at("hello", 1_700_000_000);
assert_eq!(signed, "hello:1r31eq:T4DL73TwV8wqqAA0Osb3i-A6Ba7Eu93rfujfl64jOS8");

assert_eq!(signer.unsign_at(&signed, Some(Duration::from_secs(100)), 1_700_000_005.0)?, "hello");
assert!(signer.unsign_at(&signed, Some(Duration::from_secs(3)), 1_700_050_000.0).unwrap_err().is_expired());
```

Expiry rule: a value is expired when `now - timestamp > max_age`. A value whose timestamp lies in the future is
never expired. With `max_age: None` there is no age check at all.

The `b62_encode(i64) -> String` and `b62_decode(&str) -> Option<i64>` helpers used for the timestamp are public.
Alphabet: `0-9A-Za-z`; a leading `-` marks a negative number.

## Salts

The salt is mixed into the key used for the HMAC (`digest(salt + "signer" + secret)` is the HMAC key). Two signers
with different salts produce different signatures for the same value and key, so a token minted for one purpose
cannot be replayed in another. Use a distinct, namespaced salt for each purpose:

```rust
const INVITE_SALT: &str = "myapp.invite";
const UNSUBSCRIBE_SALT: &str = "myapp.unsubscribe";
```

The salt is not secret. If two features share a salt and key, a token from one is accepted by the other.

## Rotating the secret key

To change `secret_key` without invalidating everything already issued, move the old key into
`secret_key_fallbacks` and put the new one in `secret_key`:

```rust
Settings {
    secret_key: new_key,
    secret_key_fallbacks: vec![old_key],
    ..Settings::default()
}
```

Signing always uses `secret_key`. Verification tries `secret_key` first and then each fallback in order. This
applies to `Signer`, `TimestampSigner`, `loads`, signed cookies, and everything built on them. Remove the
fallback once the old values have expired. `rango-admin check --deploy` warns when a fallback key is weak
(`security.W025`); see [Security](security.md).

## Signed cookies

A signed cookie carries a `TimestampSigner` signature so the browser cannot change its value. It is still
readable by the user.

```rust
use std::time::Duration;
use rango::http::{CookieOptions, HttpRequest, HttpResponse};

async fn remember(request: HttpRequest) -> rango::Result<HttpResponse> {
    // Read: Ok(None) when the cookie is absent, expired, tampered with or signed for another salt.
    let seen = request.get_signed_cookie("last_visit", "myapp.visit", Some(Duration::from_secs(86_400)))?;

    let mut response = HttpResponse::new(format!("previous: {seen:?}"));
    response.set_signed_cookie(
        "last_visit",
        "today",
        "myapp.visit",
        &CookieOptions { max_age: Some(86_400), httponly: true, secure: true, samesite: Some("Lax".into()), ..CookieOptions::default() },
    )?;
    Ok(response)
}
```

| Method | Signature |
|---|---|
| `HttpResponse::set_signed_cookie` | `fn set_signed_cookie(&mut self, key: &str, value: &str, salt: &str, options: &CookieOptions) -> Result<()>` |
| `HttpRequest::get_signed_cookie` | `fn get_signed_cookie(&self, key: &str, salt: &str, max_age: Option<Duration>) -> Result<Option<String>>` |

- `get_signed_cookie` never reports a `BadSignature`: any verification failure, including expiry, returns
  `Ok(None)`. The `Err` case is a configuration problem such as an empty `secret_key`.
- The cookie name is part of the signing salt, so a signed value copied to a different cookie name does not verify.
  The `salt` argument adds a second separator on top of the name.
- `max_age` on `get_signed_cookie` checks the age of the signature, independent of the cookie's own `max_age`
  attribute. A client can keep sending an old cookie after the browser would have dropped it, so enforce
  expiry on the server when it matters.
- The cookie's key is derived from `secret_key` with a fixed prefix, so signed cookies and `dumps` values with the
  same underlying key are still unrelated.

The lower-level functions are `signing::sign_cookie(value, cookie_name, salt)`,
`signing::unsign_cookie(signed_value, cookie_name, salt, max_age)` and `signing::get_cookie_signer(salt)`. Use
the request and response methods unless you need a different transport.

### `signed_cookie_legacy_salt_fallback`

Signed cookies use a salt of the form `rango.http.cookies.v2:<salt length>:<salt><cookie name>`, which cannot be
confused between different `(salt, name)` pairs. Older values used `<cookie name><salt>`. With
`signed_cookie_legacy_salt_fallback: true` (the default), `get_signed_cookie` retries with the old form when the
new one fails for a reason other than expiry, so cookies issued before the change keep working. Set it to `false`
once all such cookies have expired.

## Not supported

- There is no setting that selects the signing backend; cookie signing always uses `TimestampSigner`.
- Signing does not encrypt. If the content must be hidden from the user, store it server side (for example in the
  [session](sessions.md)) and sign only an identifier.

## See also

- [Security](security.md)
- [Settings](settings.md)
- [Sessions](sessions.md)
- [Requests and responses](http/requests-responses.md)
- [Cache](cache.md)
