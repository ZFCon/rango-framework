//! LocaleMiddleware + gettext catalogs in a view.

use rango::conf::{self, Settings};
use rango::http::{HttpRequest, HttpResponse};
use rango::test::Client;
use rango::urls::path;
use rango::utils::translation::gettext;
use rango::{Result, register_urlconf, urlpatterns};

async fn hello(_: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new(gettext("Hello")))
}

register_urlconf!("i18n.urls", || urlpatterns![path("h/", hello)]);

#[tokio::test]
async fn locale_middleware() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("i18n_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cs/LC_MESSAGES")).unwrap();
    std::fs::write(dir.join("cs/LC_MESSAGES/rango.po"), "msgid \"\"\nmsgstr \"\"\n\nmsgid \"Hello\"\nmsgstr \"Ahoj\"\n").unwrap();
    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("i18n.urls".into()),
        locale_paths: vec![dir.to_str().unwrap().into()],
        languages: vec![("en".into(), "English".into()), ("cs".into(), "Czech".into())],
        middleware: vec!["rango.middleware.locale.LocaleMiddleware".into()],
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();
    let r = Client::new().unwrap().get("/h/").await.unwrap();
    assert_eq!((r.text().as_str(), r.header("Content-Language")), ("Hello", Some("en-us")));
    let r = Client::new().unwrap().default_header("Accept-Language", "de;q=0.9, cs-CZ;q=0.8, en;q=0.5").get("/h/").await.unwrap();
    assert_eq!((r.text().as_str(), r.header("Content-Language"), r.header("Vary")), ("Ahoj", Some("cs"), Some("Accept-Language")));
    let r = Client::new().unwrap().default_header("Cookie", "rango_language=en").default_header("Accept-Language", "cs").get("/h/").await.unwrap();
    assert_eq!(r.text(), "Hello", "the cookie wins over Accept-Language");
}
