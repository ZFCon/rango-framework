//! Password validators, auth forms and reset tokens.

use rango_contrib::auth::forms::user_creation_form;
use rango_contrib::auth::models::{AbstractBaseUser, User};
use rango_contrib::auth::password_validation::validate_password;
use rango_contrib::auth::tokens::{check_token, make_token};
use rango_core::conf::{self, Settings};
use rango_forms::{Data, FileData};

#[test]
fn auth_forms_and_tokens() {
    let v = |n: &str, o: serde_json::Value| serde_json::json!({"NAME": format!("rango.contrib.auth.password_validation.{n}"), "OPTIONS": o});
    conf::configure(Settings {
        secret_key: "k".into(),
        auth_password_validators: vec![v("UserAttributeSimilarityValidator", serde_json::json!({})), v("MinimumLengthValidator", serde_json::json!({"min_length": 8})), v("CommonPasswordValidator", serde_json::json!({})), v("NumericPasswordValidator", serde_json::json!({}))],
        password_hashers: vec!["rango.contrib.auth.hashers.MD5PasswordHasher".into()],
        ..Settings::default()
    })
    .unwrap();

    let msgs = |p: &str, user: &[(&str, &str, String)]| validate_password(p, user).err().map(|e| e.messages()).unwrap_or_default();
    assert_eq!(msgs("12345", &[]), vec!["This password is too short. It must contain at least 8 characters.", "This password is too common.", "This password is entirely numeric."]);
    assert_eq!(msgs("johnsmith1", &[("username", "username", "johnsmith".into())]), vec!["The password is too similar to the username."]);
    assert!(msgs("correct horse battery", &[]).is_empty());

    let post = |p: &[(&str, &str)]| Data::from_lists(p.iter().map(|(k, v)| ((*k).to_owned(), vec![(*v).to_owned()])));
    let bad = user_creation_form().bound(post(&[("username", "ann"), ("password1", "correct horse battery"), ("password2", "different")]), FileData::new());
    assert_eq!(bad.errors().get("password2").unwrap().messages(), vec!["The two password fields didn’t match."]);
    let weak = user_creation_form().bound(post(&[("username", "ann"), ("password1", "12345"), ("password2", "12345")]), FileData::new());
    assert!(weak.errors().get("password2").unwrap().messages().contains(&"This password is too common.".to_owned()));
    assert!(user_creation_form().bound(post(&[("username", "ann"), ("password1", "correct horse battery"), ("password2", "correct horse battery")]), FileData::new()).is_valid());

    let mut u = User { id: Some(7), username: "ann".into(), email: "a@b.co".into(), ..User::default() };
    u.set_password("pw").unwrap();
    let t = make_token(&u);
    assert!(check_token(&u, &t));
    u.set_password("changed").unwrap();
    assert!(!check_token(&u, &t), "token dies when the password changes");
    assert!(!check_token(&u, "bogus"));
}
