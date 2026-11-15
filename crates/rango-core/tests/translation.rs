use rango_core::conf::{self, Settings};
use rango_core::utils::translation::*;

const PO: &str = r#"
msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"
"Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n>=2 && n<=4 ? 1 : 2);\n"

#: x.rs:1
msgid "Hello"
msgstr "Ahoj"

msgctxt "month"
msgid "May"
msgstr "Květen"

msgid "May"
msgstr "Smět"

msgid "%(n)d file"
msgid_plural "%(n)d files"
msgstr[0] "%(n)d soubor"
msgstr[1] "%(n)d soubory"
msgstr[2] "%(n)d souborů"

#, fuzzy
msgid "Fuzzy"
msgstr "Neověřeno"

msgid "Multi "
"line"
msgstr "Více "
"řádků"
"#;

#[test]
fn catalogs() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("locale_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cs/LC_MESSAGES")).unwrap();
    std::fs::write(dir.join("cs/LC_MESSAGES/rango.po"), PO).unwrap();
    conf::configure(Settings { secret_key: "k".into(), locale_paths: vec![dir.to_str().unwrap().into()], ..Settings::default() }).unwrap();
    reset_catalogs();

    assert_eq!(gettext("Hello"), "Hello", "default language has no catalog");
    activate("cs");
    assert_eq!(gettext("Hello"), "Ahoj");
    assert_eq!(gettext("Missing"), "Missing");
    assert_eq!(pgettext("month", "May"), "Květen");
    assert_eq!(gettext("May"), "Smět");
    assert_eq!(gettext("Fuzzy"), "Fuzzy", "fuzzy entries are ignored");
    assert_eq!(gettext("Multi line"), "Více řádků");
    let f = |n| ngettext("%(n)d file", "%(n)d files", n);
    assert_eq!((f(1), f(3), f(5), f(0)), ("%(n)d soubor".into(), "%(n)d soubory".into(), "%(n)d souborů".into(), "%(n)d souborů".into()));
    deactivate();
    assert_eq!(gettext("Hello"), "Hello");

    // .mo round trip
    let entries = vec![(("".to_owned(), "Hello".to_owned(), None), vec!["Ahoj".to_owned()]), (("".to_owned(), "%(n)d file".to_owned(), Some("%(n)d files".to_owned())), vec!["a".to_owned(), "b".to_owned(), "c".to_owned()])];
    let mo = rango_core::utils::translation::Catalog::to_mo(&entries, "Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n>=2 && n<=4 ? 1 : 2);\n");
    std::fs::write(dir.join("cs/LC_MESSAGES/rango.mo"), mo).unwrap();
    std::fs::remove_file(dir.join("cs/LC_MESSAGES/rango.po")).unwrap();
    reset_catalogs();
    activate("cs");
    assert_eq!(gettext("Hello"), "Ahoj");
    assert_eq!(ngettext("%(n)d file", "%(n)d files", 3), "b");
    deactivate();
}

#[test]
fn extract_messages() {
    use rango_core::management::commands::messages::{Found, extract};
    let rs = r#"let a = gettext("Save"); let b = ngettext("%(n)d row", "%(n)d rows", n); let c = pgettext("menu", "Open"); fn gettext(x: &str) {}"#;
    let f = extract(rs, false);
    assert!(f.contains(&Found { context: String::new(), id: "Save".into(), plural: None }));
    assert!(f.contains(&Found { context: String::new(), id: "%(n)d row".into(), plural: Some("%(n)d rows".into()) }));
    assert!(f.contains(&Found { context: "menu".into(), id: "Open".into(), plural: None }));
    let t = extract("{% trans \"Hello\" %} {% trans 'May' context 'month' %}", true);
    assert_eq!(t.len(), 2);
    assert_eq!(t[1].context, "month");
}
