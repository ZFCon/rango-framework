use rango_core::conf::{self, Settings};
use rango_core::utils::translation::{activate, deactivate, reset_catalogs};
use rango_template::{Engine, context};

const PO: &str = r#"
msgid ""
msgstr ""
"Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n>=2 && n<=4 ? 1 : 2);\n"

msgid "Hello"
msgstr "Ahoj"

msgid "Hello, %(name)s!"
msgstr "Ahoj, %(name)s!"

msgctxt "month"
msgid "May"
msgstr "Květen"

msgid "%(c)s file"
msgid_plural "%(c)s files"
msgstr[0] "%(c)s soubor"
msgstr[1] "%(c)s soubory"
msgstr[2] "%(c)s souborů"
"#;

fn setup() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("locale_tpl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cs/LC_MESSAGES")).unwrap();
        std::fs::write(dir.join("cs/LC_MESSAGES/rango.po"), PO).unwrap();
        conf::configure(Settings { secret_key: "k".into(), locale_paths: vec![dir.to_str().unwrap().into()], ..Settings::default() }).unwrap();
        reset_catalogs();
    });
}

#[test]
fn i18n_tags() {
    setup();
    let mut e = Engine::new();
    e.locmem(&[
        ("a.html", "{% load i18n %}{% trans 'Hello' %}|{% trans 'May' context 'month' %}|{% trans 'Nope' %}|{% trans 'Hello' as h %}[{{ h }}]"),
        ("b.html", "{% load i18n %}{% blocktrans with name=who %}Hello, {{ name }}!{% endblocktrans %}"),
        ("c.html", "{% load i18n %}{% blocktrans count c=n %}{{ c }} file{% plural %}{{ c }} files{% endblocktrans %}"),
        ("d.html", "{% load i18n %}{% language 'cs' %}{% trans 'Hello' %}{% endlanguage %}/{% trans 'Hello' %}/{% get_current_language as L %}{{ L }}"),
    ]);
    let r = |name: &str, mut ctx| e.render_to_string(name, &mut ctx).unwrap();
    assert_eq!(r("a.html", context! {}), "Hello|May|Nope|[Hello]");
    activate("cs");
    assert_eq!(r("a.html", context! {}), "Ahoj|Květen|Nope|[Ahoj]");
    assert_eq!(r("b.html", context! {"who" => "<Ann>"}), "Ahoj, &lt;Ann&gt;!");
    assert_eq!((r("c.html", context! {"n" => 1}), r("c.html", context! {"n" => 3}), r("c.html", context! {"n" => 9})), ("1 soubor".into(), "3 soubory".into(), "9 souborů".into()));
    deactivate();
    assert_eq!(r("d.html", context! {}), "Ahoj/Hello/en-us");
}

#[test]
fn cache_tag() {
    use rango_core::cache::{Timeout, caches};
    setup();
    let mut e = Engine::new();
    e.locmem(&[("t.html", "{% load cache %}{% cache 60 frag who %}[{{ n }}]{% endcache %}")]);
    let r = |who: &str, n: i64| {
        let mut c = context! {"who" => who, "n" => n};
        e.render_to_string("t.html", &mut c).unwrap()
    };
    assert_eq!(r("a", 1), "[1]");
    assert_eq!(r("a", 2), "[1]", "cached per vary value");
    assert_eq!(r("b", 3), "[3]");
    let key = rango_template::cachetag::make_template_fragment_key("frag", &["a".to_owned()]);
    assert!(caches("default").unwrap().get_blocking::<String>(&key).unwrap().is_some());
    let _ = Timeout::Never;
}
