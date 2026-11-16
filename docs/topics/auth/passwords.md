# Passwords

How Rango stores passwords, how to check and change them, how to enforce password rules, and how to issue the
one-time tokens behind a "forgot my password" flow. The code is in `rango::contrib::auth::hashers`,
`rango::contrib::auth::password_validation` and `rango::contrib::auth::tokens`.

## How passwords are stored

The `password` column of a user never holds the raw password. It holds a string that names the algorithm and carries
everything needed to verify a candidate password later:

```text
<algorithm>$<iterations>$<salt>$<hash>
pbkdf2_sha256$1000000$<22 random characters>$<base64 of the derived key>
```

Because the algorithm is part of the stored value, a database can hold hashes made with different algorithms at the
same time, and the preferred algorithm can change without invalidating existing passwords.

Use the methods on the user rather than touching the column:

```rust
use rango::contrib::auth;
use rango::db::Db;

let db = Db::default_db();
let mut user = auth::get_user_model()?.get_by_natural_key(&db, "ann").await?.expect("ann exists");

user.set_password("correct horse battery")?;        // hash with the preferred algorithm
user.save_fields(&db, &["password"]).await?;        // write only that column

assert!(user.check_password("correct horse battery"));
```

| Method (on `AbstractBaseUser`) | Meaning |
|---|---|
| `set_password(raw) -> Result<()>` | hash `raw` with the first configured hasher and store the result in memory |
| `check_password(raw) -> bool` | verify `raw`; if the stored hash is outdated, re-hash in memory (see [Upgrades](#upgrades-on-login)) |
| `set_unusable_password()` | store a value that no password matches |
| `has_usable_password() -> bool` | `false` after `set_unusable_password()` |
| `password() -> String` | the encoded value |

None of these touch the database. Follow them with `save_fields(&db, &["password"])`.

An unusable password is `!` followed by 40 random characters. It is what `create_user(.., None)` stores, and what
`createsuperuser` stores when `RANGO_SUPERUSER_PASSWORD` is not set. Such an account cannot authenticate with a
password until one is set.

## Hashing functions

The same operations are available without a user:

```rust
use rango::contrib::auth::hashers::{check_password, is_password_usable, make_password};

let encoded = make_password(Some("hunter2"), None, "default")?;
let (ok, must_update) = check_password(Some("hunter2"), &encoded);
assert!(ok && !must_update);

let unusable = make_password(None, None, "default")?;
assert!(!is_password_usable(Some(&unusable)));
assert!(!check_password(Some("anything"), &unusable).0);
```

| Function | Meaning |
|---|---|
| `make_password(password: Option<&str>, salt: Option<&str>, hasher: &str) -> Result<String>` | Encode a password. `None` as the password produces an unusable value. `None` as the salt generates a random one. `hasher` is an algorithm name from the table below, or `"default"` for the first configured hasher. |
| `check_password(password: Option<&str>, encoded: &str) -> (bool, bool)` | `(matches, must_update)`. Never errors: an unusable value, an unknown algorithm or a malformed hash give `(false, false)`. |
| `is_password_usable(encoded: Option<&str>) -> bool` | `false` when the value starts with `!`. |
| `get_hasher(algorithm: &str) -> Result<Box<dyn PasswordHasher>>` | The configured hasher for an algorithm name, or `"default"`. |
| `get_hashers() -> Result<Vec<Box<dyn PasswordHasher>>>` | All configured hashers, in order. |
| `identify_hasher(encoded: &str) -> Result<Box<dyn PasswordHasher>>` | The hasher that produced an encoded value. |

`check_password` returns two values, unlike the method on the user. `must_update` is `true` when the password
matched but the hash should be regenerated; the caller decides whether to do that.

Passing your own salt is for tests and data migration. A salt containing `$` is rejected.

## Choosing algorithms: `password_hashers`

The `password_hashers` setting is an ordered list. The **first** entry hashes new passwords. **All** entries can
verify existing ones. The default is:

```rust
password_hashers: vec![
    "rango.contrib.auth.hashers.PBKDF2PasswordHasher".into(),
    "rango.contrib.auth.hashers.PBKDF2SHA1PasswordHasher".into(),
    "rango.contrib.auth.hashers.Argon2PasswordHasher".into(),
    "rango.contrib.auth.hashers.BCryptSHA256PasswordHasher".into(),
    "rango.contrib.auth.hashers.ScryptPasswordHasher".into(),
],
```

The available hashers:

| Setting entry (after `rango.contrib.auth.hashers.`) | Algorithm name | Parameters | Encoded form |
|---|---|---|---|
| `PBKDF2PasswordHasher` | `pbkdf2_sha256` | 1,000,000 iterations, HMAC-SHA256 | `pbkdf2_sha256$<iterations>$<salt>$<base64 hash>` |
| `PBKDF2SHA1PasswordHasher` | `pbkdf2_sha1` | 1,000,000 iterations, HMAC-SHA1 | `pbkdf2_sha1$<iterations>$<salt>$<base64 hash>` |
| `Argon2PasswordHasher` | `argon2` | Argon2id, 102,400 KiB memory, 2 passes, 8 lanes | `argon2$argon2id$v=19$m=102400,t=2,p=8$<base64 salt>$<base64 hash>` |
| `BCryptSHA256PasswordHasher` | `bcrypt_sha256` | cost 12; the password is SHA-256 hashed first, so long passwords are not affected by bcrypt's 72-byte input limit | `bcrypt_sha256$$2b$12$<salt and hash>` |
| `BCryptPasswordHasher` | `bcrypt` | cost 12; subject to bcrypt's 72-byte input limit | `bcrypt$$2b$12$<salt and hash>` |
| `ScryptPasswordHasher` | `scrypt` | N=16384, r=8, p=5 | `scrypt$<N>$<salt>$<r>$<p>$<base64 hash>` |
| `MD5PasswordHasher` | `md5` | one salted MD5 round; **insecure** | `md5$<salt>$<hex hash>` |

To prefer Argon2, move it to the front and keep the others so that existing hashes still verify:

```rust
password_hashers: vec![
    "rango.contrib.auth.hashers.Argon2PasswordHasher".into(),
    "rango.contrib.auth.hashers.PBKDF2PasswordHasher".into(),
    "rango.contrib.auth.hashers.PBKDF2SHA1PasswordHasher".into(),
    "rango.contrib.auth.hashers.BCryptSHA256PasswordHasher".into(),
    "rango.contrib.auth.hashers.ScryptPasswordHasher".into(),
],
```

Things to know:

- Only the last segment of each entry is examined, and it must be one of the seven names above. Anything else
  raises `ImproperlyConfigured: Unknown password hasher '...'`. Custom hashers cannot be plugged in through the
  setting, although the `PasswordHasher` trait is public.
- A hash whose algorithm is not in the list does not verify. Removing a hasher locks out every user whose password
  still uses it.
- An empty list makes `make_password(.., "default")` fail.
- The parameters (iterations, cost, memory) are fixed in the hasher and not configurable.
- `MD5PasswordHasher` exists for two purposes only: reading old databases, and making test suites fast.

  ```rust
  // tests only
  password_hashers: vec!["rango.contrib.auth.hashers.MD5PasswordHasher".into()],
  ```

## Upgrades on login

Stored hashes are upgraded gradually, at the only moment the raw password is available: a successful login.

`check_password` reports `must_update` when the password matched and either

- the hash was made by an algorithm other than the first one in `password_hashers`, or
- it is a PBKDF2 hash with a different iteration count, or a bcrypt hash with a different cost, from the current
  hasher's.

The user's `check_password(raw)` method acts on that by calling `set_password(raw)` in memory. `ModelBackend` then
saves the `password` column after every successful `authenticate`, so the upgrade reaches the database. The result:
after you reorder `password_hashers`, each user's hash moves to the new algorithm the next time they log in, with no
migration and no forced reset.

If you verify passwords yourself with the user's `check_password`, save the column afterwards for the same reason:

```rust
if user.check_password(raw) {
    user.save_fields(&db, &["password"]).await?;
}
```

Argon2 and scrypt hashes are only upgraded when the preferred algorithm changes, not when their parameters would.

## Password validation

Hashing protects stored passwords; validation rejects weak ones before they are stored. Validation is off by default
(`auth_password_validators` is empty). Each entry is a JSON object with a `NAME` and optional `OPTIONS`:

```rust
auth_password_validators: vec![
    rango::json!({"NAME": "rango.contrib.auth.password_validation.UserAttributeSimilarityValidator"}),
    rango::json!({"NAME": "rango.contrib.auth.password_validation.MinimumLengthValidator", "OPTIONS": {"min_length": 10}}),
    rango::json!({"NAME": "rango.contrib.auth.password_validation.CommonPasswordValidator"}),
    rango::json!({"NAME": "rango.contrib.auth.password_validation.NumericPasswordValidator"}),
],
```

| Validator | Options | Rejects | Error code |
|---|---|---|---|
| `MinimumLengthValidator` | `min_length` (default `8`) | passwords with fewer characters than `min_length` | `password_too_short` |
| `NumericPasswordValidator` | none | passwords made only of ASCII digits | `password_entirely_numeric` |
| `CommonPasswordValidator` | none | passwords found in a bundled list of about 20,000 common passwords; the comparison ignores case and surrounding whitespace | `password_too_common` |
| `UserAttributeSimilarityValidator` | `max_similarity` (default `0.7`, between 0 and 1) | passwords too similar to one of the user attributes passed in | `password_too_similar` |

As with hashers, only the last segment of `NAME` is examined. An entry with an unrecognised name is skipped without
an error, so check the spelling. The common-password list is compiled into the binary and cannot be replaced
through `OPTIONS`.

### Running the validators

```rust
use rango::contrib::auth::password_validation::validate_password;

match validate_password("12345", &[]) {
    Ok(()) => { /* acceptable */ }
    Err(e) => {
        for message in e.messages() {
            println!("{message}");
        }
    }
}
```

With the four validators above this prints:

```text
This password is too short. It must contain at least 10 characters.
This password is too common.
This password is entirely numeric.
```

`validate_password(password, user_attrs)` runs every configured validator and collects all failures into one
`ValidationError`, so the user sees every problem at once.

The second argument feeds `UserAttributeSimilarityValidator`. It is a slice of
`(attribute name, display name, value)` tuples describing the user the password is for. The validator compares the
lower-cased password with each value, and with each word inside it, and rejects the password when the overlap ratio
reaches `max_similarity`:

```rust
let attrs = [
    ("username", "username", "johnsmith".to_owned()),
    ("email", "email address", "john@example.com".to_owned()),
];
let err = validate_password("johnsmith1", &attrs).unwrap_err();
assert_eq!(err.messages(), vec!["The password is too similar to the username."]);
```

The display name is what appears in the message. Empty values are skipped. With an empty slice the similarity
validator never rejects anything.

`password_validators_help_texts() -> Vec<String>` returns one sentence per configured validator ("Your password
must contain at least 10 characters.", ...), for showing the rules next to a password field.

### Where validation happens

Validation is not part of `set_password`, `create_user`, `createsuperuser` or `changepassword`. Those accept any
password. It runs:

- in the built-in forms that set a password (`user_creation_form`, `set_password_form`, `password_change_form`),
  which attach the errors to the confirmation field and use the help texts as the first field's help text;
- wherever you call `validate_password` yourself.

The built-in forms call the validators with no user attributes, so `UserAttributeSimilarityValidator` has no effect
in them. To enforce it, call `validate_password` in your view with the attributes of the user in question and add
the error to the form. See [Auth views and forms](views-forms.md).

## Changing a password

For a logged-in user changing their own password, use [`PasswordChangeView`](views-forms.md), or the pieces behind
it: `password_change_form()` and `save_password_change()`. They check the old password, validate the new one, save
it, and the view keeps the current session logged in.

To set a password from code (an administrator action, a script, a data fix):

```rust
let mut user = auth::get_user_model()?.get_by_natural_key(&db, "ann").await?.expect("ann exists");
user.set_password("a new password")?;
user.save_fields(&db, &["password"]).await?;
```

From the command line:

```console
$ cargo run -- changepassword ann --password 'a new password'
Password changed successfully for user 'ann'
```

Any password change ends every session of that user, because sessions are tied to the password hash. The
[overview](overview.md#sessions-and-password-changes) explains how to keep the current session alive.

## Password reset tokens

A reset link must prove that its holder received the email, must stop working after use, and should not need a
database table. `rango::contrib::auth::tokens` provides such tokens:

```rust
use rango::contrib::auth::tokens::{check_token, make_token};

let token = make_token(&*user);          // e.g. "9k2-5f0c1e..."
assert!(check_token(&*user, &token));
```

| Function | Meaning |
|---|---|
| `make_token(user: &dyn AbstractBaseUser) -> String` | a token for this user in their current state |
| `check_token(user: &dyn AbstractBaseUser, token: &str) -> bool` | whether the token is genuine, still matches the user's state, and has not expired |

A token is `<day>-<signature>`: the day it was made (counted from 2001-01-01, in base 36) and 32 hex characters of
an HMAC-SHA256 keyed with `secret_key`. The signature covers the user's primary key, password hash, last login
time and email address. Nothing is stored. A token therefore stops validating as soon as any of these happens:

- the password changes, which makes every reset link single-use;
- the user logs in (`last_login` changes);
- the email address changes;
- it expires;
- `secret_key` changes.

Expiry is governed by `password_reset_timeout` (seconds, default `259200`). Tokens carry a day, not a time, so the
setting is applied in whole days: it is divided by 86,400 and rounded down, and a token is accepted on the day it was
made and for that many UTC days afterwards. A value below 86,400 means tokens are valid only until the end of the UTC
day they were made on.

Only integer primary keys take part in the signature. With another key type the token is still bound to the
password, last login and email.

### Building a reset flow

Rango ships the tokens but not the reset views, forms or emails. The flow has two views. The first takes an email
address, finds the user, and emails a link containing the user's id and a token (see [Sending email](../mail.md)).
Respond the same way whether or not the address is known, so the form cannot be used to discover accounts:

```rust
use rango::contrib::auth::tokens::make_token;
use rango::contrib::auth::User;
use rango::db::models::Model;

let found = User::objects().filter_kw("email", email).filter_kw("is_active", true).fetch(&db).await?;
for user in found.iter().filter(|u| rango::contrib::auth::AbstractBaseUser::has_usable_password(*u)) {
    let link = format!("https://example.com/reset/{}/{}/", user.id.unwrap_or_default(), make_token(user));
    // send `link` to user.email
}
```

The second view handles the link. It checks the token before showing the form and again before saving:

```rust
use rango::contrib::auth::forms::{post_data, set_password_form};
use rango::contrib::auth::get_user_model;
use rango::contrib::auth::tokens::check_token;
use rango::db::{Db, Value};
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::{redirect, render};
use rango::{Error, Result, context};

// path("reset/<int:uid>/<str:token>/", reset_confirm)
pub async fn reset_confirm(req: HttpRequest, uid: i64, token: String) -> Result<HttpResponse> {
    let db = Db::default_db();
    let Some(mut user) = get_user_model()?.get_by_pk(&db, &Value::Int(uid)).await? else {
        return Err(Error::Http404("This reset link is not valid.".into()));
    };
    if !check_token(&*user, &token) {
        return Err(Error::Http404("This reset link is not valid.".into()));
    }
    let mut form = set_password_form();
    if req.method() == "POST" {
        form = form.bound(post_data(&req).await?, Default::default());
        if form.is_valid() {
            let new = form.cleaned_data().get("new_password1").and_then(|v| v.as_str()).unwrap_or("").to_owned();
            user.set_password(&new)?;
            user.save_fields(&db, &["password"]).await?;
            return redirect("/accounts/login/");
        }
    }
    render(&req, "registration/password_reset_confirm.html", context! { "form" => form.to_template() })
}
```

Once the password is saved the token no longer validates, so the link cannot be replayed.

## Not supported yet

- Password reset views, forms and email templates (the tokens are provided; see above).
- Registering custom hashers or custom validators through the settings.
- Configurable hasher parameters (iterations, cost, memory).
- A custom word list for `CommonPasswordValidator`, and a `user_attributes` option for
  `UserAttributeSimilarityValidator` (the caller chooses the attributes).
- Automatic validation in `createsuperuser` and `changepassword`.

## See also

- [Authentication overview](overview.md)
- [Auth views and forms](views-forms.md)
- [Custom user models](custom-user.md)
- [Signing](../signing.md)
- [Security](../security.md)
- [Settings reference](../../ref/settings.md)
