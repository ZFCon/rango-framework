//! `contrib.auth.forms` (form definitions plus the async steps that touch the database).

use rango_core::exceptions::{Error, Result, ValidationError};
use rango_core::utils::translation::gettext;
use rango_db::Db;
use rango_forms::{Field, Form, Value, Widget};
use rango_http::HttpRequest;

use super::models::AbstractBaseUser;
use super::password_validation::validate_password;
use super::{Credentials, authenticate, get_user_model};

fn password_field(label: &str) -> Field {
    Field::char().label(gettext(label)).strip(false).widget(Widget::password_input().with_attrs(rango_forms::attrs! {"autocomplete" => "current-password"}))
}

/// `AuthenticationForm`: username + password.
pub fn authentication_form() -> Form {
    Form::new("AuthenticationForm")
        .field("username", Field::char().label("Username").max_length(150).widget_attrs(rango_forms::attrs! {"autofocus" => true, "autocapitalize" => "none", "autocomplete" => "username"}))
        .field("password", password_field("Password"))
}

/// Check the credentials of a bound [`authentication_form`]; adds the "invalid login" error to the form when they fail.
pub async fn authenticate_form(db: &Db, form: &mut Form) -> Result<Option<(Box<dyn AbstractBaseUser>, String)>> {
    if !form.is_valid() {
        return Ok(None);
    }
    let c = form.cleaned_data();
    let creds = Credentials { username: c.get("username").and_then(|v| v.as_str()).map(str::to_owned), password: c.get("password").and_then(|v| v.as_str()).map(str::to_owned) };
    match authenticate(db, &creds).await? {
        Some(r) => Ok(Some(r)),
        None => {
            let field = get_user_model()?.username_field();
            form.add_error(None, ValidationError::new(gettext("Please enter a correct %(username)s and password. Note that both fields may be case-sensitive.")).with_code("invalid_login").with_params([("username", field)])).map_err(|e| Error::Other(e.into()))?;
            Ok(None)
        }
    }
}

fn new_password_fields(form: Form, first: &str, second: &str) -> Form {
    let f1 = Field::char().label(gettext("New password")).strip(false).widget(Widget::password_input().with_attrs(rango_forms::attrs! {"autocomplete" => "new-password"})).help_text(super::password_validation::password_validators_help_texts().join("\n"));
    let f2 = Field::char().label(gettext("New password confirmation")).strip(false).widget(Widget::password_input().with_attrs(rango_forms::attrs! {"autocomplete" => "new-password"}));
    let (a, b) = (first.to_owned(), second.to_owned());
    form.field(first, f1).field(second, f2).clean(move |c| {
        let (p1, p2) = (c.cleaned.get(&a).cloned(), c.cleaned.get(&b).cloned());
        if let (Some(Value::Str(p1)), Some(Value::Str(p2))) = (&p1, &p2) {
            if p1 != p2 {
                return Err(ValidationError::from_dict([(b.clone(), ValidationError::new(gettext("The two password fields didn’t match.")).with_code("password_mismatch"))]));
            }
            validate_password(p1, &[]).map_err(|e| ValidationError::from_dict([(b.clone(), e)]))?;
        }
        Ok(())
    })
}

/// `SetPasswordForm`: `new_password1` / `new_password2`.
pub fn set_password_form() -> Form {
    new_password_fields(Form::new("SetPasswordForm"), "new_password1", "new_password2")
}

/// `PasswordChangeForm`: `old_password` + the set-password fields.
pub fn password_change_form() -> Form {
    new_password_fields(Form::new("PasswordChangeForm").field("old_password", password_field("Old password").widget_attrs(rango_forms::attrs! {"autofocus" => true})), "new_password1", "new_password2")
        .field_order(&["old_password", "new_password1", "new_password2"])
}

/// Verify `old_password` of a bound [`password_change_form`] and set the new password on `user`.
pub async fn save_password_change(db: &Db, form: &mut Form, user: &mut dyn AbstractBaseUser) -> Result<bool> {
    if !form.is_valid() {
        return Ok(false);
    }
    let old = form.cleaned_data().get("old_password").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    if !user.check_password(&old) {
        form.add_error(Some("old_password"), ValidationError::new(gettext("Your old password was entered incorrectly. Please enter it again.")).with_code("password_incorrect")).map_err(|e| Error::Other(e.into()))?;
        return Ok(false);
    }
    let new = form.cleaned_data().get("new_password1").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    user.set_password(&new)?;
    user.save_fields(db, &["password"]).await?;
    Ok(true)
}

/// `UserCreationForm`: `username`, `password1`, `password2`.
pub fn user_creation_form() -> Form {
    let f = Form::new("UserCreationForm").field("username", Field::char().label("Username").max_length(150).help_text("Required. 150 characters or fewer. Letters, digits and @/./+/-/_ only.").validator(|v| {
        let s = v.to_text();
        if s.chars().all(|c| c.is_alphanumeric() || "@.+-_".contains(c)) {
            Ok(())
        } else {
            Err(ValidationError::new(gettext("Enter a valid username. This value may contain only unaccented lowercase a-z and uppercase A-Z letters, numbers, and @/./+/-/_ characters.")).with_code("invalid"))
        }
    }));
    new_password_fields(f, "password1", "password2").field_order(&["username", "password1", "password2"])
}

/// Create the user of a valid [`user_creation_form`] (reports a taken username as a form error).
pub async fn save_user_creation(db: &Db, form: &mut Form) -> Result<Option<Box<dyn AbstractBaseUser>>> {
    if !form.is_valid() {
        return Ok(None);
    }
    let username = form.cleaned_data().get("username").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let um = get_user_model()?;
    if um.get_by_natural_key(db, &username).await?.is_some() {
        form.add_error(Some("username"), ValidationError::new(gettext("A user with that username already exists.")).with_code("unique")).map_err(|e| Error::Other(e.into()))?;
        return Ok(None);
    }
    let pw = form.cleaned_data().get("password1").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    Ok(Some(um.create_user(db, &username, "", Some(&pw)).await?))
}

/// `request.POST` into form data.
pub async fn post_data(req: &HttpRequest) -> Result<rango_forms::Data> {
    Ok(rango_forms::Data::from_lists(req.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec()))))
}
