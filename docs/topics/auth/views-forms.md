# Auth views and forms

Rango ships views for logging in, logging out and changing a password, and the forms behind them, plus a form for
creating users. This page describes each one, the templates you have to supply, and how to build a signup page.
The views are in `rango::contrib::auth::views`, the forms in `rango::contrib::auth::forms`.

## Mounting the views

There is no bundled URLconf for the auth views. Add the ones you need to your own, choosing the paths and names:

```rust
use rango::contrib::auth::decorators::login_required;
use rango::contrib::auth::views::{LoginView, LogoutView, PasswordChangeView};
use rango::urls::path;
use rango::views::generic::{TemplateView, View};
use rango::{register_urlconf, urlpatterns};

register_urlconf!("mysite.urls", || urlpatterns![
    path("accounts/login/", LoginView::default().as_view()).name("login"),
    path("accounts/logout/", LogoutView::default().as_view()).name("logout"),
    path(
        "accounts/password_change/",
        PasswordChangeView { success_url: "/accounts/password_change/done/".into(), ..PasswordChangeView::default() }.as_view(),
    )
    .name("password_change"),
    path(
        "accounts/password_change/done/",
        login_required(TemplateView::new("registration/password_change_done.html").as_view()),
    )
    .name("password_change_done"),
]);
```

Each view is a plain struct with public fields and a `Default` implementation. Configure it with struct update
syntax, then call `as_view()`, which comes from the `rango::views::generic::View` trait (import it).

The views need `SessionMiddleware` and `AuthenticationMiddleware`, and the forms post back with `{% csrf_token %}`,
so keep `CsrfViewMiddleware` enabled. See [Enabling authentication](overview.md#enabling-authentication).

**Templates are not bundled.** Every template named on this page must exist in your project's template
directories. Minimal versions are shown below.

## `LoginView`

Shows a username and password form, and logs the user in when it is submitted correctly.

| Field | Default | Meaning |
|---|---|---|
| `template_name` | `"registration/login.html"` | template to render |
| `redirect_field_name` | `"next"` | name of the parameter that carries the URL to return to |

Behaviour:

- `GET` renders the template with an unbound form.
- `POST` validates the form and authenticates through the configured backends. On success it calls `login` and
  redirects. On failure it renders the template again with the errors. A wrong password, an unknown user and an
  inactive user all produce the same non-field error: *Please enter a correct username and password. Note that both
  fields may be case-sensitive.* (error code `invalid_login`).
- The redirect target after login is the `next` value from the posted form or, failing that, from the query
  string. It is used only if it is safe: a path on this site, or a full URL for the current host (with `https`
  when the request is secure). Anything else is discarded, so the parameter cannot send users to another site. With no usable `next`, the view redirects to the `login_redirect_url` setting
  (default `"/accounts/profile/"`), which is used as a URL as written.
- Allowed methods: `GET`, `HEAD`, `POST`, `OPTIONS`. The view takes no URL parameters.
- A user who is already logged in still sees the form.

Template context:

| Variable | Value |
|---|---|
| `form` | the authentication form |
| `next` (the value of `redirect_field_name`) | the validated return URL from the query string, or an empty string |

`login_required` sends visitors to `login_url` with `?next=<path>`. The template must pass that value back in a
hidden field, otherwise the user lands on `login_redirect_url` instead of the page they asked for:

```html
<!-- templates/registration/login.html -->
<h1>Log in</h1>
<form method="post">
  {% csrf_token %}
  {{ form.as_p }}
  <input type="hidden" name="next" value="{{ next }}">
  <button type="submit">Log in</button>
</form>
```

Set `login_url` to wherever you mounted the view (a path, or a pattern name such as `"login"`):

```rust
Settings {
    login_url: "/accounts/login/".into(),
    login_redirect_url: "/".into(),
    ..Settings::default()
}
```

## `LogoutView`

Logs the user out. It accepts **`POST` only**; a `GET` request gets a 405 response. Logging out changes state, and a
`GET` could be triggered by any page that embeds a link or image pointing at your site.

| Field | Default | Meaning |
|---|---|---|
| `template_name` | `None` | template for the "logged out" page; `None` means `"registration/logged_out.html"` |

After flushing the session, the view picks the first that applies:

1. a `next` value from the posted form or the query string, if it passes the same safety check as in `LoginView`:
   redirect there;
2. the `logout_redirect_url` setting, if set: redirect there;
3. otherwise render the template, with an empty context.

Log out with a small form rather than a link:

```html
<form method="post" action="/accounts/logout/">
  {% csrf_token %}
  <button type="submit">Log out</button>
</form>
```

```html
<!-- templates/registration/logged_out.html -->
<p>You have been logged out. <a href="/accounts/login/">Log in again</a></p>
```

With `logout_redirect_url: Some("/".into())` the template is never needed.

## `PasswordChangeView`

Lets a logged-in user change their own password by entering the old one and the new one twice.

| Field | Default | Meaning |
|---|---|---|
| `template_name` | `"registration/password_change_form.html"` | template to render |
| `success_url` | `"/password_change/done/"` | where to redirect after a successful change |

Behaviour:

- An anonymous request (either method) is redirected to `login_url` with `?next=`; no `login_required` wrapper is
  needed.
- `GET` renders the form.
- `POST` checks the old password, checks that the two new passwords match, runs the
  [password validators](passwords.md#password-validation), saves the new password, and redirects to `success_url`.
  On any failure the form is rendered again with errors.
- After saving, the view calls `update_session_auth_hash`, so the session making the request stays logged in. The
  user's other sessions are logged out, because they are tied to the old password.
- Allowed methods: `GET`, `HEAD`, `POST`, `OPTIONS`. The view takes no URL parameters.

Template context: `form`.

There is no built-in "done" view. `success_url` must point at a view of yours; the example at the top of the page
uses a `TemplateView`. Set `success_url` explicitly, since the default path only suits a project that mounts the
views at the site root.

```html
<!-- templates/registration/password_change_form.html -->
<h1>Change password</h1>
<form method="post">
  {% csrf_token %}
  {{ form.as_p }}
  <button type="submit">Change my password</button>
</form>
```

## The forms

Rango forms are values built by functions, so each built-in form is a function returning a `rango::forms::Form`.
Steps that need the database (checking credentials, saving a user) are separate `async` functions that take the
bound form.

| Form | Fields | Companion function |
|---|---|---|
| `authentication_form()` | `username`, `password` | `authenticate_form(&db, &mut form)` |
| `user_creation_form()` | `username`, `password1`, `password2` | `save_user_creation(&db, &mut form)` |
| `password_change_form()` | `old_password`, `new_password1`, `new_password2` | `save_password_change(&db, &mut form, &mut user)` |
| `set_password_form()` | `new_password1`, `new_password2` | none: set the password yourself |

All password fields render as `<input type="password">` and are not stripped of surrounding whitespace.

`post_data(&req).await?` turns the request's `POST` data into the `rango::forms::Data` that `Form::bound` expects.
Bind with `form.bound(post_data(&req).await?, Default::default())`; the second argument is the (empty) file data.

### `authentication_form`

`username` (at most 150 characters) and `password`.

`authenticate_form(&db, &mut form) -> Result<Option<(Box<dyn AbstractBaseUser>, String)>>` runs field validation,
then `authenticate`. It returns the user and the backend name on success. When the credentials are rejected it adds
the `invalid_login` error to the form and returns `Ok(None)`; when the form itself is invalid it returns `Ok(None)`
without contacting the backends.

A login view written by hand, equivalent to what `LoginView` does:

```rust
use rango::contrib::auth::forms::{authenticate_form, authentication_form, post_data};
use rango::contrib::auth::login;
use rango::db::Db;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::{redirect, render};
use rango::{Result, context};

async fn my_login(req: HttpRequest) -> Result<HttpResponse> {
    let db = Db::default_db();
    let mut form = authentication_form();
    if req.method() == "POST" {
        form = form.bound(post_data(&req).await?, Default::default());
        if let Some((user, backend)) = authenticate_form(&db, &mut form).await? {
            login(&req, &db, user, Some(&backend)).await?;
            return redirect("/");
        }
    }
    render(&req, "registration/login.html", context! { "form" => form.to_template() })
}
```

### `user_creation_form`

`username`, `password1` and `password2`.

- `username`: at most 150 characters; letters, digits and `@`, `.`, `+`, `-`, `_` only.
- `password1` and `password2` must match (error code `password_mismatch`, on `password2`).
- The password must pass the configured validators; their errors are attached to `password2`, and their help texts
  appear under `password1`.

`save_user_creation(&db, &mut form) -> Result<Option<Box<dyn AbstractBaseUser>>>` creates the user through the user
model's `create_user` and returns it. It returns `Ok(None)` when the form is invalid, and also when the username is
already taken, in which case it adds *A user with that username already exists.* to the `username` field (error
code `unique`). The user is created with an empty email address.

### `password_change_form`

`old_password`, `new_password1` and `new_password2`, in that order. The two new passwords follow the same rules as
in the creation form.

`save_password_change(&db, &mut form, &mut user) -> Result<bool>` verifies `old_password` against `user`, sets the
new password and saves the `password` column. It returns `false` if the form is invalid or the old password is
wrong; in the second case the error *Your old password was entered incorrectly. Please enter it again.* is added to
`old_password` (error code `password_incorrect`).

It does not touch the session. A view that uses it should call `update_session_auth_hash(&req)` afterwards, as
[`PasswordChangeView`](#passwordchangeview) does.

### `set_password_form`

`new_password1` and `new_password2`, without asking for the old password. It is meant for flows where the user has
proved their identity another way, such as a [password reset link](passwords.md#building-a-reset-flow). There is no
save function: read `new_password1` from `cleaned_data()` and call `user.set_password(..)` and
`user.save_fields(&db, &["password"])`.

## A signup view

The creation form and its save function are enough for a registration page that logs the new user in:

```rust
use rango::contrib::auth::forms::{post_data, save_user_creation, user_creation_form};
use rango::contrib::auth::login;
use rango::db::Db;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::{redirect, render};
use rango::{Result, context};

pub async fn signup(req: HttpRequest) -> Result<HttpResponse> {
    let db = Db::default_db();
    let mut form = user_creation_form();
    if req.method() == "POST" {
        form = form.bound(post_data(&req).await?, Default::default());
        if let Some(user) = save_user_creation(&db, &mut form).await? {
            login(&req, &db, user, None).await?;
            return redirect("/");
        }
    }
    render(&req, "registration/signup.html", context! { "form" => form.to_template() })
}
```

```rust
path("accounts/signup/", signup).name("signup"),
```

```html
<!-- templates/registration/signup.html -->
<h1>Sign up</h1>
<form method="post">
  {% csrf_token %}
  {{ form.non_field_errors }}
  {% for field in form.visible_fields %}
    <p>
      {{ field.errors }}
      {{ field.label_tag }} {{ field }}
      {% if field.help_text %}<small>{{ field.help_text }}</small>{% endif %}
    </p>
  {% endfor %}
  <button type="submit">Create account</button>
</form>
```

`login(.., None)` records the first backend in `authentication_backends`, which is what you want for a user who has
just been created rather than authenticated.

### Collecting more than a username

The form is an ordinary `Form`, so more fields can be added. `save_user_creation` only knows about the username and
password, so create the user yourself when you need the extra values:

```rust
use rango::contrib::auth::{get_user_model, login};
use rango::forms::Field;

let mut form = user_creation_form().field("email", Field::email());
if req.method() == "POST" {
    form = form.bound(post_data(&req).await?, Default::default());
    if form.is_valid() {
        let get = |name: &str| form.cleaned_data().get(name).and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let (username, email, password) = (get("username"), get("email"), get("password1"));
        let users = get_user_model()?;
        if users.get_by_natural_key(&db, &username).await?.is_none() {
            let user = users.create_user(&db, &username, &email, Some(&password)).await?;
            login(&req, &db, user, None).await?;
            return redirect("/");
        }
        // otherwise report that the username is taken
    }
}
```

### Checking similarity to the username

The built-in forms run the password validators without any user attributes, so
`UserAttributeSimilarityValidator` cannot compare the password with the username. Run it yourself between
validation and saving:

```rust
use rango::Error;
use rango::contrib::auth::password_validation::validate_password;

if form.is_valid() {
    let get = |name: &str| form.cleaned_data().get(name).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let (username, password) = (get("username"), get("password1"));
    if let Err(e) = validate_password(&password, &[("username", "username", username)]) {
        form.add_error(Some("password2"), e).map_err(|e| Error::Other(e.into()))?;
    }
}
if let Some(user) = save_user_creation(&db, &mut form).await? {
    // ...
}
```

## Not supported yet

- Password reset views, forms and emails. The tokens exist; [Passwords](passwords.md#password-reset-tokens) shows
  how to build the flow.
- A "password change done" view (use a `TemplateView`).
- A bundled URLconf that mounts all auth views at once.
- Bundled `registration/*.html` templates.
- A user change form and an administrator's set-password form.
- Redirecting already-authenticated users away from the login page.

## See also

- [Authentication overview](overview.md)
- [Passwords](passwords.md)
- [Custom user models](custom-user.md)
- [Forms](../forms/forms.md)
- [Form rendering](../forms/rendering.md)
- [Generic views](../http/generic-views.md)
- [URL dispatcher](../http/urls.md)
