# Custom user models

The default user is `rango::contrib::auth::User` (label `auth.User`). To use your own model, implement two traits,
register the implementation, and point the `auth_user_model` setting at it. This page covers the traits, the
registration, what the rest of the framework expects from the model, and the current limits.

## How it fits together

Authentication does not hard-code the `User` struct. It works with two traits:

- `AbstractBaseUser` describes one user instance: key, username, password hash, flags, and how to save selected fields.
- `UserModel` is the "manager" side: look a user up by primary key or username, and create users and superusers.

`get_user_model()` returns the registered `Arc<dyn UserModel>` whose `label()` equals the `auth_user_model` setting.
`authenticate`, `login`, the session lookup, `createsuperuser` and `changepassword` all go through it.

## Steps

1. Define the model with the usual `Model` derive (see [models](../db/models.md)).
2. Implement `AbstractBaseUser` for the struct.
3. Implement `UserModel` for a unit struct.
4. Call `register_user_model` at startup, before the first request.
5. Set `auth_user_model` to `"<app_label>.<ModelName>"`.
6. Create and run migrations for the model.

## The model

A user model needs the credential fields and, if you use permissions, many-to-many fields to groups and permissions.
The table names matter, see "Permissions" below.

```rust
use chrono::{DateTime, Utc};
use rango::contrib::auth::{Group, Permission};
use rango::db::models::{ManyToMany, Model};

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango::db", app_label = "accounts", str = "email")]
pub struct Member {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 128)]
    pub password: String,
    #[field(null, blank)]
    pub last_login: Option<DateTime<Utc>>,
    #[field(max_length = 254, unique)]
    pub email: String,
    #[field(max_length = 100, blank)]
    pub display_name: String,
    #[field(default = true)]
    pub is_active: bool,
    #[field(default = false)]
    pub is_staff: bool,
    #[field(default = false)]
    pub is_superuser: bool,
    #[field(blank)]
    pub groups: ManyToMany<Group>,
    #[field(blank)]
    pub user_permissions: ManyToMany<Permission>,
}
```

## `AbstractBaseUser`

Defined in `rango::contrib::auth` (also `rango::contrib::auth::models`). The trait is declared with `async_trait`,
so add the `async-trait` crate to your `Cargo.toml` and put `#[async_trait]` on the impl.

Required methods:

| Method | Meaning |
| --- | --- |
| `pk() -> rango::db::Value` | Primary key value. |
| `get_username() -> String` | The login identifier. |
| `password() -> String` | Stored password hash. |
| `set_password_hash(String)` | Store a new hash. |
| `last_login()` / `set_last_login(DateTime<Utc>)` | Last login time. |
| `model_label() -> String` | `"app_label.ModelName"`; must match the registered model. |
| `as_any() -> &dyn Any` | For downcasting back to your struct. |
| `to_template() -> rango::template::Value` | What `{{ user }}` shows in templates. |
| `save_fields(db, &[&str]).await` | Persist only the named fields. Called after login (`last_login`) and password hash upgrades (`password`). |

Methods with defaults you may override:

| Method | Default |
| --- | --- |
| `is_active()` | `true` |
| `is_staff()`, `is_superuser()` | `false` |
| `email()` | empty string |
| `get_full_name()`, `get_short_name()` | `get_username()` |

Provided and normally left alone: `is_authenticated()`, `set_password(raw)`, `set_unusable_password()`,
`has_usable_password()`, `check_password(raw)` and `get_session_auth_hash()`. The session auth hash is derived from the
password hash, so changing a password invalidates the user's other sessions.

```rust
use std::any::Any;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rango::contrib::auth::AbstractBaseUser;
use rango::db::models::Model;
use rango::db::{Executor, Value as DbValue};
use rango::template::Value as T;
use rango::Result;

#[async_trait]
impl AbstractBaseUser for Member {
    fn pk(&self) -> DbValue {
        self.pk_value()
    }
    fn get_username(&self) -> String {
        self.email.clone()
    }
    fn password(&self) -> String {
        self.password.clone()
    }
    fn set_password_hash(&mut self, encoded: String) {
        self.password = encoded;
    }
    fn last_login(&self) -> Option<DateTime<Utc>> {
        self.last_login
    }
    fn set_last_login(&mut self, when: DateTime<Utc>) {
        self.last_login = Some(when);
    }
    fn is_active(&self) -> bool {
        self.is_active
    }
    fn is_staff(&self) -> bool {
        self.is_staff
    }
    fn is_superuser(&self) -> bool {
        self.is_superuser
    }
    fn email(&self) -> String {
        self.email.clone()
    }
    fn model_label(&self) -> String {
        "accounts.Member".into()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn to_template(&self) -> T {
        T::Str(self.email.clone())
    }
    async fn save_fields(&mut self, db: &dyn Executor, fields: &[&str]) -> Result<()> {
        self.save_with(db, false, Some(fields)).await
    }
}
```

`to_template` can return a plain string, but then `{{ user.email }}` has nothing to read. The built-in `User` returns
a template object that answers `username`, `email`, `is_staff`, `is_authenticated` and so on, via
`rango::template::{Object, TemplateObject}`. Do the same when your templates need attributes.

## `UserModel`

```rust
use async_trait::async_trait;
use rango::contrib::auth::{AbstractBaseUser, UserModel};
use rango::db::models::Model;
use rango::db::{Executor, Value as DbValue};
use rango::{Error, Result};

pub struct MemberModel;

fn boxed(m: Member) -> Box<dyn AbstractBaseUser> {
    Box::new(m)
}

#[async_trait]
impl UserModel for MemberModel {
    fn label(&self) -> String {
        "accounts.Member".into()
    }
    fn username_field(&self) -> String {
        "email".into()
    }
    async fn get_by_pk(&self, db: &dyn Executor, pk: &DbValue) -> Result<Option<Box<dyn AbstractBaseUser>>> {
        match Member::objects().filter_kw("pk", pk.clone()).get(db).await {
            Ok(m) => Ok(Some(boxed(m))),
            Err(Error::ObjectDoesNotExist { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn get_by_natural_key(&self, db: &dyn Executor, username: &str) -> Result<Option<Box<dyn AbstractBaseUser>>> {
        match Member::objects().filter_kw("email", username).get(db).await {
            Ok(m) => Ok(Some(boxed(m))),
            Err(Error::ObjectDoesNotExist { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn create_user(&self, db: &dyn Executor, username: &str, _email: &str, password: Option<&str>) -> Result<Box<dyn AbstractBaseUser>> {
        let mut m = Member { email: username.to_owned(), is_active: true, ..Member::default() };
        match password {
            Some(p) => m.set_password(p)?,
            None => m.set_unusable_password(),
        }
        m.save(db).await?;
        Ok(boxed(m))
    }
    async fn create_superuser(&self, db: &dyn Executor, username: &str, email: &str, password: Option<&str>) -> Result<Box<dyn AbstractBaseUser>> {
        let mut m = Member { email: username.to_owned(), is_active: true, is_staff: true, is_superuser: true, ..Member::default() };
        let _ = email;
        match password {
            Some(p) => m.set_password(p)?,
            None => m.set_unusable_password(),
        }
        m.save(db).await?;
        Ok(boxed(m))
    }
}
```

| Method | Used by |
| --- | --- |
| `label()` | Matching against `auth_user_model`. |
| `username_field()` (default `"username"`) | Name shown in the login form's "invalid login" error. |
| `get_by_pk(db, pk)` | Restoring the user from the session on each request. Return `Ok(None)` when absent. |
| `get_by_natural_key(db, username)` | `ModelBackend::authenticate`, `createsuperuser`, `changepassword`. |
| `create_user`, `create_superuser` | `createsuperuser` and your own signup code. |

`username_field` does not pick the lookup column; your `get_by_natural_key` decides what the "username" is. It is
used in the failed-login message of the built-in login form. The built-in forms always name the form field `username`,
whatever the model's field is called.

## Registering

Registration is process-global and keyed by `label()`. Do it once during startup, for example in your app's
`ready` hook or in `main` before running commands.

```rust
use std::sync::Arc;
use rango::contrib::auth::register_user_model;

register_user_model(Arc::new(MemberModel));
```

Then select it:

```rust
use rango::conf::Settings;

let settings = Settings {
    auth_user_model: "accounts.Member".into(),
    ..Settings::default()
};
```

`get_user_model()` returns `Err(ImproperlyConfigured)` ("AUTH_USER_MODEL refers to model ... that has not been
installed") when the setting names a label with no registered implementation.

```rust
use rango::contrib::auth::get_user_model;

let um = get_user_model()?;
um.create_superuser(&db, "root@example.org", "root@example.org", Some("pw")).await?;
```

## Requirements and limits

- **Permissions need specific table names.** `ModelBackend` computes permissions with SQL against the join tables
  `<db_table>_groups` and `<db_table>_user_permissions`, each with a column `<modelname>_id` (the lowercase
  struct name, for example `member_id`). Declaring `groups: ManyToMany<Group>` and
  `user_permissions: ManyToMany<Permission>` on the model produces them. A user model without these two fields can
  authenticate but permission checks (and therefore the admin for non-superusers) will fail with a database error.
  Superusers pass every check without querying.
- **`model_label` must be registered** as a model, or permission lookup returns an `ImproperlyConfigured` error.
- **Primary keys** may be integers, strings or UUIDs. The session stores the key as JSON (a number or string).
- **Choose the model before the first migration.** Changing `auth_user_model` later means rewriting foreign keys
  yourself.
- **Hand-written trait impls.** There is no derive for `AbstractBaseUser` or `UserModel`, so each custom model needs
  the code above.
- **Only the model backends exist.** `authentication_backends` accepts
  `rango.contrib.auth.backends.ModelBackend` and `...AllowAllUsersModelBackend`; any other name is an error.
- **No admin user page.** The admin has no built-in user editor or password-change page.
- **Built-in forms** (login, user creation) use a form field named `username` and a `password`. They call
  `authenticate`, `get_by_natural_key` and `create_user(db, username, "", password)`, so they work with a custom model;
  the creation form passes an empty email, so your `create_user` must cope with that.

## See also

- [Authentication overview](overview.md)
- [Passwords](passwords.md)
- [Views and forms](views-forms.md)
- [Models](../db/models.md)
- [Sessions](../sessions.md)
- [Admin](../admin.md)
