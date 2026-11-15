//! `#[derive(Form)]`: typed forms.

use rango_core::conf::{self, Settings};
use rango_forms::{Data, FileData, Form};

#[derive(Debug, PartialEq, Form)]
#[form(crate = "rango_forms")]
struct Contact {
    #[form(max_length = 10)]
    name: String,
    #[form(email, label = "Your e-mail")]
    email: String,
    #[form(widget = "textarea", help_text = "Be nice")]
    message: String,
    cc_myself: bool,
    #[form(min_value = 18)]
    age: Option<u32>,
    #[form(choices = [("a", "Alpha"), ("b", "Beta")])]
    pick: String,
    #[form(choices = [("x", "X"), ("y", "Y")])]
    tags: Vec<String>,
}

#[test]
fn derived_form() {
    conf::configure(Settings { secret_key: "k".into(), ..Settings::default() }).unwrap();
    let post = |pairs: &[(&str, &[&str])]| Data::from_lists(pairs.iter().map(|(k, v)| ((*k).to_owned(), v.iter().map(|s| (*s).to_owned()).collect())));
    let ok = Contact::form().bound(post(&[("name", &["Ann"]), ("email", &["a@b.co"]), ("message", &["hi"]), ("cc_myself", &["on"]), ("age", &["30"]), ("pick", &["b"]), ("tags", &["x", "y"])]), FileData::new());
    assert_eq!(
        Contact::from_form(&ok),
        Some(Contact { name: "Ann".into(), email: "a@b.co".into(), message: "hi".into(), cc_myself: true, age: Some(30), pick: "b".into(), tags: vec!["x".into(), "y".into()] })
    );
    let bad = Contact::form().bound(post(&[("name", &["much too long name"]), ("email", &["nope"]), ("age", &["5"]), ("pick", &["z"])]), FileData::new());
    assert_eq!(Contact::from_form(&bad), None);
    let e = bad.errors();
    assert_eq!(e.get("name").unwrap().messages(), vec!["Ensure this value has at most 10 characters (it has 18)."]);
    assert_eq!(e.get("email").unwrap().messages(), vec!["Enter a valid email address."]);
    assert_eq!(e.get("age").unwrap().messages(), vec!["Ensure this value is greater than or equal to 18."]);
    assert_eq!(e.get("pick").unwrap().messages(), vec!["Select a valid choice. z is not one of the available choices."]);
    assert_eq!(e.get("message").unwrap().messages(), vec!["This field is required."]);
    assert!(Contact::form().as_p().contains("<label for=\"id_email\">Your e-mail:</label>"));
}
