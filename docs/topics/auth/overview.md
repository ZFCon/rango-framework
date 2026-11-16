# Authentication overview

`rango::contrib::auth` provides user accounts, groups, permissions, and the login state that ties a request to a
user. This page covers enabling it, the models, logging users in and out, protecting views, and permission checks.
Password storage, the built-in views and forms, and custom user models have their own pages (see the end).

## Enabling authentication

Authentication stores its login state in the session, and permissions refer to content types, so three apps and two
middleware are needed:

```rust
use rango::conf::Settings;

pub fn settings() -> Settings {
    Settings {
        installed_apps: [
            "rango.contrib.contenttypes",
            "rango.contrib.auth",
            "rango.contrib.sessions",
            // your apps ...
        ]
        .map(String::from)
        .to_vec(),
        middleware: [
            "rango.middleware.security.SecurityMiddleware",
            "rango.middleware.common.CommonMiddleware",
            "rango.contrib.sessions.middleware.SessionMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
            "rango.contrib.auth.middleware.AuthenticationMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        ..Settings::default()
    }
}
```

`SessionMiddleware` must come before `AuthenticationMiddleware`. If it does not, every request fails with an
`ImproperlyConfigured` error that names both middleware.

Then create the tables:

```console
$ cargo run -- migrate
```

This creates `auth_user`, `auth_group`, `auth_permission` and their join tables (`auth_user_groups`,
`auth_user_user_permissions`, `auth_group_permissions`), plus `rango_session` and `rango_content_type` from the other
two apps. After each `migrate`, the auth app also creates the default permissions of every migrated app (see
[Permissions and groups](#permissions-and-groups)).

## The models

All three models live in `rango::contrib::auth` (also under `rango::contrib::auth::models`).

### `User`

| Field | Type | Notes |
|---|---|---|
| `id` | `Option<i64>` | auto primary key |
| `username` | `String` | unique, at most 150 characters |
| `password` | `String` | the encoded hash, never the raw password (see [Passwords](passwords.md)) |
| `first_name`, `last_name` | `String` | optional, at most 150 characters |
| `email` | `String` | optional, at most 254 characters |
| `is_active` | `bool` | default `true`; inactive users cannot log in and have no permissions |
| `is_staff` | `bool` | default `false`; allows access to the [admin site](../admin.md) |
| `is_superuser` | `bool` | default `false`; an active superuser passes every permission check |
| `last_login` | `Option<DateTime<Utc>>` | set by `login()` |
| `date_joined` | `DateTime<Utc>` | set when the row is created |
| `groups` | `ManyToMany<Group>` | |
| `user_permissions` | `ManyToMany<Permission>` | |

`User` is an ordinary model, so it is queried like any other:

```rust
use rango::contrib::auth::User;
use rango::db::Db;
use rango::db::models::Model;

let db = Db::default_db();
let staff = User::objects().filter_kw("is_staff", true).fetch(&db).await?;
```

### `Group`

A named set of permissions: `id`, `name` (unique, at most 150 characters) and `permissions`
(`ManyToMany<Permission>`). A user has every permission of every group they belong to.

### `Permission`

`id`, `name` (human-readable, such as `Can add article`), `content_type` (`ForeignKey<ContentType>`, the model the
permission belongs to) and `codename` (such as `add_article`). `content_type` and `codename` are unique together.
Permission checks use the string `"<app_label>.<codename>"`, for example `"blog.add_article"`.

`rango::contrib::auth::models::permission_str(&db, &permission)` formats a permission as `app | model | name`.

### The user trait

Code that handles "the current user" does not use `User` directly. It works with `Box<dyn AbstractBaseUser>`, a trait
that the built-in `User` implements and that a [custom user model](custom-user.md) implements too. The methods you
will use most:

| Method | Meaning |
|---|---|
| `pk()` | primary key as a `rango::db::Value` |
| `get_username()`, `email()`, `get_full_name()`, `get_short_name()` | identity fields as `String` |
| `is_active()`, `is_staff()`, `is_superuser()` | flags |
| `set_password(raw)`, `check_password(raw)`, `set_unusable_password()`, `has_usable_password()` | see [Passwords](passwords.md) |
| `save_fields(&db, &["password"])` | write only the named columns |
| `as_any()` | downcast to the concrete model |

To reach fields that are not on the trait, downcast:

```rust
use rango::contrib::auth::User;

if let Some(u) = user.as_any().downcast_ref::<User>() {
    println!("{} joined {}", u.username, u.date_joined);
}
```

## Creating users

Users are created through the user model's manager object, returned by `get_user_model()`. It hashes the password for
you; never assign a raw password to `User::password`.

```rust
use rango::contrib::auth;
use rango::db::Db;

let db = Db::default_db();
let users = auth::get_user_model()?;

// username, email, password
let ann = users.create_user(&db, "ann", "ann@example.com", Some("s3cret")).await?;

// is_staff and is_superuser are both set
let root = users.create_superuser(&db, "root", "root@example.com", Some("pw")).await?;

// None stores an unusable password: the account exists but cannot log in with a password
let svc = users.create_user(&db, "reports", "", None).await?;
```

Both return `Box<dyn AbstractBaseUser>`. An empty username is rejected with an error. A duplicate username fails with
the database's unique-constraint error, so look the name up first when it comes from user input:

```rust
if users.get_by_natural_key(&db, "ann").await?.is_some() {
    // already taken
}
```

`get_by_natural_key(&db, username)` and `get_by_pk(&db, &pk)` both return `Ok(None)` when no such user exists.

### From the command line

```console
$ RANGO_SUPERUSER_PASSWORD='pw' cargo run -- createsuperuser --username root --email root@example.com
Superuser created successfully.
```

`createsuperuser` does not prompt. It reads:

| Value | Source |
|---|---|
| username | `--username`, else `RANGO_SUPERUSER_USERNAME`; required |
| email | `--email`, else `RANGO_SUPERUSER_EMAIL`; defaults to empty |
| password | `RANGO_SUPERUSER_PASSWORD` only; when unset the account gets an unusable password |
| database | `--database` (default `default`) |

`--noinput` (or `--no-input`) is accepted and changes nothing, since there is no interactive mode. The command fails
with `Error: That username is already taken.` if the user exists. The password is not run through the password
validators.

To set or replace a password later:

```console
$ cargo run -- changepassword root --password 'new-pw'
Password changed successfully for user 'root'
```

`changepassword` takes the username, a required `--password`, and `--database`. The password is given on the command
line, so it ends up in shell history; prefer it for development and provisioning scripts.

## Logging a user in

Logging in is two steps: `authenticate` checks credentials and returns the user, and `login` records that user in the
session.

```rust
use rango::contrib::auth::{self, Credentials};
use rango::db::Db;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::redirect;
use rango::Result;

async fn login_view(req: HttpRequest) -> Result<HttpResponse> {
    let post = req.post().await?;
    let creds = Credentials::new(post.get("username").unwrap_or(""), post.get("password").unwrap_or(""));
    let db = Db::default_db();
    match auth::authenticate(&db, &creds).await? {
        Some((user, backend)) => {
            auth::login(&req, &db, user, Some(&backend)).await?;
            redirect("/")
        }
        None => Ok(HttpResponse::new("bad credentials")),
    }
}
```

`Credentials` has two public fields, `username: Option<String>` and `password: Option<String>`;
`Credentials::new(username, password)` fills both.

`authenticate(&db, &credentials)` asks each configured backend in order and returns
`Ok(Some((user, backend_name)))` from the first one that accepts, or `Ok(None)`. It returns `None` for a wrong
password, an unknown username and (with the default backend) an inactive user alike, so a login form cannot be used
to find out which usernames exist.

`login(&req, &db, user, backend)` then:

- gives the session a new key (or flushes it entirely if it belonged to a different user), so a session id known
  before login is useless afterwards;
- stores the user's primary key, the backend name and the session auth hash in the session;
- rotates the CSRF token;
- sets `last_login` and saves that column;
- makes the user available as `req.user()` for the rest of the request.

Pass the backend name that `authenticate` returned. With `None`, the first backend in `authentication_backends` is
recorded.

For a ready-made login page, use [`LoginView`](views-forms.md).

## Logging a user out

```rust
async fn logout_view(req: HttpRequest) -> Result<HttpResponse> {
    auth::logout(&req).await?;
    redirect("/login/")
}
```

`logout(&req)` flushes the session (all session data is deleted, not only the login) and makes `req.user()`
anonymous. Log users out in response to a `POST`, so that a link or an image tag on another site cannot do it.

## The current user

`AuthenticationMiddleware` loads the user for each request. Read it with the `RequestUserExt` extension trait:

```rust
use rango::contrib::auth::RequestUserExt;

async fn profile(req: HttpRequest) -> Result<HttpResponse> {
    let user = req.user();
    if user.is_anonymous() {
        return redirect("/login/");
    }
    Ok(HttpResponse::new(format!("hello {}", user.username().await)))
}
```

`req.user()` returns a `CurrentUser`, which is cheap to clone. It wraps an optional shared handle to the user, which
is why most accessors are `async`: they lock the handle briefly.

| Method | Returns |
|---|---|
| `is_authenticated()`, `is_anonymous()` | `bool`, no `await` needed |
| `username().await` | `String` (empty for anonymous) |
| `pk().await` | `Option<rango::db::Value>` |
| `is_staff().await` | `true` only if the user is staff *and* active |
| `is_superuser().await` | `bool` |
| `has_perm(&db, "app.codename").await` | `Result<bool>` |
| `has_perms(&db, &["a.b", "c.d"]).await` | `Result<bool>`, all must hold |
| `has_module_perms(&db, "app").await` | `Result<bool>`, any permission in that app |
| `snapshot(\|u\| ...).await` | runs a closure on `&dyn AbstractBaseUser`; `None` for anonymous |
| `to_template()` | the value the `auth` context processor puts in templates |

`snapshot` reads anything else:

```rust
use rango::contrib::auth::User;

let email: Option<String> = req.user().snapshot(|u| u.email()).await;
let first_name: Option<String> = req
    .user()
    .snapshot(|u| u.as_any().downcast_ref::<User>().map(|u| u.first_name.clone()))
    .await
    .flatten();
```

The field `CurrentUser.0` is the handle itself (`Option<UserHandle>`), for the cases where you need to modify the
user: `handle.lock().await` gives mutable access.

Without `AuthenticationMiddleware`, `req.user()` is always anonymous. When a session carries a login, the middleware
runs one query per request to load the user.

## Protecting views

The wrappers in `rango::contrib::auth::decorators` take a view and return a view, so they are applied in the URLconf.

### `login_required`

```rust
use rango::contrib::auth::decorators::login_required;
use rango::urls::path;
use rango::{register_urlconf, urlpatterns};

register_urlconf!("mysite.urls", || urlpatterns![
    path("login/", login_view),
    path("secret/", login_required(secret)),
]);
```

Anonymous requests are redirected to the `login_url` setting with the original path in the `next` query parameter:
`/secret/` becomes `/login/?next=/secret/`. `login_url` may be a path or the name of a URL pattern.

### `permission_required`

```rust
use rango::contrib::auth::decorators::permission_required;

path("articles/add/", permission_required(add_article, &["blog.add_article"], false)),
path("articles/<int:id>/delete/", permission_required(delete_article, &["blog.delete_article"], true)),
```

The user must hold every listed permission. The third argument decides what happens otherwise: `false` redirects to
the login page (also for a logged-in user who lacks the permission), `true` returns a `PermissionDenied` error, which
becomes a 403 response.

### `user_passes_test`

For any other condition, supply the test. It receives the `CurrentUser` and a `Db`, and returns a boxed future:

```rust
use std::sync::Arc;

use rango::contrib::auth::REDIRECT_FIELD_NAME;
use rango::contrib::auth::decorators::user_passes_test;

path(
    "staff/",
    user_passes_test(staff_page, None, REDIRECT_FIELD_NAME, Arc::new(|user, _db| Box::pin(async move { Ok(user.is_staff().await) }))),
),
```

The arguments are the view, an optional login URL (`None` uses the `login_url` setting), the name of the query
parameter that carries the return path (`REDIRECT_FIELD_NAME` is `"next"`), and the test. A failed test redirects to
the login URL.

### Checking inside a view

The wrappers are conveniences. A view can make the same decisions itself, which is the only option when the check
depends on the object being accessed:

```rust
use rango::contrib::auth::{REDIRECT_FIELD_NAME, RequestUserExt, redirect_to_login};
use rango::Error;

async fn edit(req: HttpRequest, id: i64) -> Result<HttpResponse> {
    let user = req.user();
    if user.is_anonymous() {
        return redirect_to_login(&req.get_full_path(), None, REDIRECT_FIELD_NAME);
    }
    if !user.has_perm(&Db::default_db(), "blog.change_article").await? {
        return Err(Error::PermissionDenied(String::new()));
    }
    // ...
}
```

Generic views have no login or permission mixins; wrap the result of `as_view()` instead:
`login_required(ListView::<Article>::new().as_view())`.

### Requiring login everywhere

`LoginRequiredMiddleware` turns the default around: every view requires a logged-in user unless it is marked exempt.
Add it after `AuthenticationMiddleware`:

```rust
"rango.contrib.auth.middleware.AuthenticationMiddleware",
"rango.contrib.auth.middleware.LoginRequiredMiddleware",
```

Anonymous requests are redirected to `login_url` with `?next=`. A view is exempt when its `login_required_exempt` flag
is set. There is no ready-made wrapper for that flag; it is one line with `View::with_flags`:

```rust
use rango::urls::{IntoView, View};

fn login_not_required<M>(view: impl IntoView<M>) -> View {
    view.into_view().with_flags(|f| f.login_required_exempt = true)
}

register_urlconf!("mysite.urls", || urlpatterns![
    path("login/", login_not_required(login_view)),
    path("", home), // requires login
]);
```

The login view itself must be exempt, or the redirect loops. The admin site's own login page is not exempt, so with
this middleware an anonymous visitor to the admin is sent to your `login_url` rather than to the admin login form.

## Permissions and groups

### Default permissions

After `migrate`, each model of each migrated app has four permissions: `add_<model>`, `change_<model>`,
`delete_<model>` and `view_<model>`, where `<model>` is the lower-cased struct name. An `Article` model in the app
`blog` gets `blog.add_article`, `blog.change_article`, `blog.delete_article` and `blog.view_article`. Existing rows
are left alone, so running `migrate` again is harmless.

The same step can be run by hand, for instance in tests that create tables without migrations:

```rust
use rango::contrib::admin::create_permissions;

let created: usize = create_permissions(&db, "blog").await?;
```

There is no model attribute for declaring extra permissions. Create additional `Permission` rows yourself, attached
to the model's content type.

### Granting permissions

Permissions are granted by adding rows to the many-to-many relations of `User` and `Group`:

```rust
use rango::contrib::auth::{Group, Permission, User};
use rango::db::models::{Model, related};

let perm = Permission::objects().filter_kw("codename", "change_article").get(&db).await?;
let ann = User::objects().filter_kw("username", "ann").get(&db).await?;

// directly to a user
related::add(&db, "auth.User", "user_permissions", &ann.pk_value(), &[perm.pk_value()]).await?;

// through a group
let editors = Group::objects().create(&db, Group { name: "editors".into(), ..Group::default() }).await?;
related::add(&db, "auth.Group", "permissions", &editors.pk_value(), &[perm.pk_value()]).await?;
related::add(&db, "auth.User", "groups", &ann.pk_value(), &[editors.pk_value()]).await?;
```

Two models in different apps can share a codename, so filter on the content type as well when that can happen.

### Checking permissions

```rust
if req.user().has_perm(&db, "blog.change_article").await? {
    // ...
}
```

The rules:

- an anonymous user has no permissions;
- an active superuser has every permission, without a query;
- otherwise each authentication backend is asked; `ModelBackend` answers from the user's own permissions and those
  of the user's groups;
- an inactive user has no permissions, superuser or not.

Permissions are not cached on the user: each `has_perm` call on a non-superuser runs two queries. When a view checks
several permissions, fetch the set once:

```rust
use rango::contrib::auth::ModelBackend;

let handle = req.user().0.expect("logged in");
let perms = ModelBackend { allow_inactive: false }.get_all_permissions(&db, &**handle.lock().await).await?;
if perms.contains("blog.change_article") && perms.contains("blog.delete_article") { /* ... */ }
```

`get_all_permissions` returns a `BTreeSet<String>` of `app_label.codename` strings.

Permissions are per model, not per object. "May this user edit *this* article" is application logic.

## Authentication backends

The `authentication_backends` setting lists the backends that `authenticate` and the permission checks consult. Two
are built in:

| Dotted name | Behaviour |
|---|---|
| `rango.contrib.auth.backends.ModelBackend` | the default: username and password against the user model; rejects inactive users |
| `rango.contrib.auth.backends.AllowAllUsersModelBackend` | the same, but inactive users may log in |

```rust
Settings {
    authentication_backends: vec!["rango.contrib.auth.backends.AllowAllUsersModelBackend".into()],
    ..Settings::default()
}
```

`ModelBackend` looks the user up with the user model's `get_by_natural_key`, checks the password, and saves the
password column again if the hash was upgraded (see [Passwords](passwords.md)). For an unknown username it still
runs the hasher once, so that response time does not reveal whether the account exists.

### Writing a backend

A backend is a type implementing `rango::contrib::auth::AuthBackend`:

```rust
#[async_trait]
pub trait AuthBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn authenticate(&self, db: &Db, credentials: &Credentials) -> Result<Option<Box<dyn AbstractBaseUser>>>;
    async fn get_user(&self, db: &Db, pk: &Value) -> Result<Option<Box<dyn AbstractBaseUser>>>;
    async fn has_perm(&self, db: &Db, user: &dyn AbstractBaseUser, perm: &str) -> Result<bool> { Ok(false) }
    async fn has_module_perms(&self, db: &Db, user: &dyn AbstractBaseUser, app_label: &str) -> Result<bool> { Ok(false) }
}
```

The trait is public, but **custom backends cannot be registered yet**: `authentication_backends` accepts only the
two names above, and any other name raises `ImproperlyConfigured` as soon as the backends are needed.

Until a registry exists, put custom credential checks in your own login view. Verify the user however you need to
(an email address, a one-time code, an external service), load the user, and call `login` with the name of
`ModelBackend`. The session then reloads that user by primary key on later requests:

```rust
use rango::contrib::auth::{self, AbstractBaseUser, User};
use rango::db::models::Model;

async fn email_login(req: HttpRequest) -> Result<HttpResponse> {
    let post = req.post().await?;
    let db = Db::default_db();
    let found = User::objects().filter_kw("email", post.get("email").unwrap_or("")).fetch(&db).await?;
    let Some(found) = found.into_iter().next() else { return Ok(HttpResponse::new("bad credentials")) };
    let mut user: Box<dyn AbstractBaseUser> = Box::new(found);
    if !user.is_active() || !user.check_password(post.get("password").unwrap_or("")) {
        return Ok(HttpResponse::new("bad credentials"));
    }
    user.save_fields(&db, &["password"]).await?; // check_password may have upgraded the hash
    auth::login(&req, &db, user, Some("rango.contrib.auth.backends.ModelBackend")).await?;
    redirect("/")
}
```

Save the password column after `check_password`, as shown. If the hash was upgraded in memory and not saved, the
session auth hash recorded by `login` would not match the stored password and the user would be logged out on the
next request.

## The user in templates

Add the `auth` context processor to the `templates` setting:

```rust
templates: vec![rango::json!({
    "BACKEND": "rango.template.backends.rango.RangoTemplates",
    "DIRS": ["templates"],
    "APP_DIRS": true,
    "OPTIONS": {
        "context_processors": [
            "rango.template.context_processors.request",
            "rango.contrib.auth.context_processors.auth",
        ],
    },
})],
```

Every template rendered with a request then has a `user` variable:

```html
{% if user.is_authenticated %}
  <p>Signed in as {{ user.username }}.</p>
  <form method="post" action="/logout/">{% csrf_token %}<button>Log out</button></form>
{% else %}
  <a href="/login/">Log in</a>
{% endif %}
```

For the built-in `User` the available attributes are `pk`, `id`, `username`, `first_name`, `last_name`, `email`,
`is_staff`, `is_active`, `is_superuser`, `is_authenticated`, `is_anonymous`, `date_joined`, `last_login`,
`get_username`, `get_full_name` and `get_short_name`. `{{ user }}` prints the username. For anonymous visitors
`user.is_authenticated` is false, `user.is_anonymous` is true, and `{{ user }}` prints `AnonymousUser`.

The processor does not provide a `perms` variable. Check permissions in the view and pass the result into the
context.

## Sessions and password changes

At login the session stores a hash derived from the user's password hash and `secret_key`. On every request the
stored value is compared with a freshly computed one; if they differ the session is flushed and the request is
anonymous. The effect is that **changing a password logs the user out of every session**, on every device.

The session where the user changed the password would be logged out too. To keep it, change the password through
the request's user handle and then call `update_session_auth_hash`:

```rust
use rango::contrib::auth::{self, RequestUserExt};

let Some(handle) = req.user().0 else { /* not logged in */ };
{
    let mut user = handle.lock().await;
    user.set_password("a new password")?;
    user.save_fields(&db, &["password"]).await?;
} // release the lock before the next call
auth::update_session_auth_hash(&req).await?;
```

`update_session_auth_hash(&req)` gives the session a new key and stores the hash of the user currently attached to
the request. It locks the user handle itself, so the guard above must be dropped first.
[`PasswordChangeView`](views-forms.md) does all of this for you.

Changing `secret_key` changes every hash and therefore logs everyone out.

## Settings

| Setting | Default | Meaning |
|---|---|---|
| `login_url` | `"/accounts/login/"` | where `login_required` and friends redirect; a path or a URL pattern name |
| `login_redirect_url` | `"/accounts/profile/"` | where `LoginView` goes after login when there is no `next` |
| `logout_redirect_url` | `None` | where `LogoutView` goes after logout; `None` renders a template instead |
| `authentication_backends` | `["rango.contrib.auth.backends.ModelBackend"]` | backends, in order |
| `auth_user_model` | `"auth.User"` | see [Custom user models](custom-user.md) |
| `password_hashers` | PBKDF2, PBKDF2-SHA1, Argon2, bcrypt-SHA256, scrypt | see [Passwords](passwords.md) |
| `auth_password_validators` | `[]` | see [Passwords](passwords.md) |
| `password_reset_timeout` | `259200` (3 days, in seconds) | lifetime of password reset tokens |

Constants in `rango::contrib::auth`: `SESSION_KEY` (`"_auth_user_id"`), `BACKEND_SESSION_KEY`
(`"_auth_user_backend"`), `HASH_SESSION_KEY` (`"_auth_user_hash"`) are the session keys used by `login`, and
`REDIRECT_FIELD_NAME` is `"next"`.

## Testing

The test client has no login shortcut. Log in the way a browser would, by posting to a login view; the client keeps
the session cookie:

```rust
use rango::test::{Client, assert_contains, assert_redirects};

let c = Client::new()?;
assert_redirects(&c.get("/secret/").await?, "/login/?next=/secret/", 302);
assert_redirects(&c.post("/login/", &[("username", "ann"), ("password", "s3cret")]).await?, "/secret/", 302);
assert_contains(&c.get("/secret/").await?, "hello ann", 200);
```

Hashing is deliberately slow. In tests, set
`password_hashers: vec!["rango.contrib.auth.hashers.MD5PasswordHasher".into()]` to keep them fast.

## Not supported yet

- Registering custom authentication backends through `authentication_backends`.
- A `perms` object in templates.
- Remote-user (web-server provided) authentication.
- Object-level permissions.
- Login and permission mixins for generic views (wrap `as_view()` instead).
- An interactive `createsuperuser` prompt.

## See also

- [Passwords](passwords.md)
- [Auth views and forms](views-forms.md)
- [Custom user models](custom-user.md)
- [Sessions](../sessions.md)
- [The admin site](../admin.md)
- [Middleware](../http/middleware.md)
- [REST authentication and permissions](../rest/auth-permissions.md)
- [Settings reference](../../ref/settings.md)
