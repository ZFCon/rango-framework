//! sites, redirects, flatpages and staticfiles.

use rango::conf::{self, DatabaseConfig, Settings};
use rango::contrib::{flatpages::FlatPage, redirects::Redirect, sites};
use rango::db::migrations::{MigrationLoader, Target, executor};
use rango::db::models::{ForeignKey, Model};
use rango::db::Db;
use rango::test::{Client, assert_contains, assert_redirects};
use rango::urls::path;
use rango::{register_urlconf, urlpatterns};

register_urlconf!("contrib.urls", || urlpatterns![path("real/", |_: rango::http::HttpRequest| async { Ok(rango::http::HttpResponse::new("real")) })]);

#[tokio::test]
async fn contrib_apps() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("contrib_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("flatpages")).unwrap();
    std::fs::write(dir.join("flatpages/default.html"), "<h1>{{ flatpage.title }}</h1>{{ flatpage.content }}").unwrap();
    std::fs::create_dir_all(dir.join("static/css")).unwrap();
    std::fs::write(dir.join("static/css/a.css"), "body{}").unwrap();
    std::fs::write(dir.join("static/.hidden"), "x").unwrap();

    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("contrib.urls".into()),
        site_id: Some(1),
        installed_apps: ["contenttypes", "sites", "redirects", "flatpages"].map(|a| format!("rango.contrib.{a}")).to_vec(),
        middleware: ["redirects.middleware.RedirectFallbackMiddleware", "flatpages.middleware.FlatpageFallbackMiddleware"].map(|m| format!("rango.contrib.{m}")).to_vec(),
        databases: [("default".to_owned(), DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })].into(),
        templates: vec![rango::json!({"BACKEND": "rango.template.backends.rango.RangoTemplates", "DIRS": [dir.to_str().unwrap()], "OPTIONS": {}})],
        use_tz: true,
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();
    let db = Db::default_db();
    let labels: Vec<String> = ["contenttypes", "sites", "redirects", "flatpages"].map(String::from).to_vec();
    executor::migrate(&db, &MigrationLoader::load(Some(&labels)).unwrap(), &Target::All).await.unwrap();

    // post_migrate created example.com as site 1.
    let site = sites::get_current(&db, None).await.unwrap();
    assert_eq!((site.id, site.domain.as_str()), (Some(1), "example.com"));

    Redirect::objects().create(&db, Redirect { id: None, site: ForeignKey::new(1), old_path: "/old/".into(), new_path: "/real/".into() }).await.unwrap();
    Redirect::objects().create(&db, Redirect { id: None, site: ForeignKey::new(1), old_path: "/dead/".into(), new_path: String::new() }).await.unwrap();
    let fp = FlatPage::objects().create(&db, FlatPage { id: None, url: "/about/".into(), title: "About us".into(), content: "We exist.".into(), ..FlatPage::default() }).await.unwrap();
    let ids = [rango::db::Value::Int(1)];
    rango::db::models::related::add(&db, "flatpages.FlatPage", "sites", &fp.pk_value(), &ids).await.unwrap();

    let c = Client::new().unwrap();
    assert_contains(&c.get("/real/").await.unwrap(), "real", 200);
    assert_redirects(&c.get("/old/").await.unwrap(), "/real/", 301);
    assert_eq!(c.get("/dead/").await.unwrap().status_code, 410);
    assert_contains(&c.get("/about/").await.unwrap(), "<h1>About us</h1>We exist.", 200);
    assert_eq!(c.get("/nothing/").await.unwrap().status_code, 404);

    // staticfiles: finders honour prefixes and ignore patterns.
    let f = rango::contrib::staticfiles::Finders::with_roots(vec![(None, dir.join("static")), (Some("pre".into()), dir.join("static"))]);
    assert!(f.find("css/a.css").is_some() && f.find("pre/css/a.css").is_some() && f.find("nope.css").is_none());
    let listed: Vec<String> = f.list(&[".*".into()]).into_keys().collect();
    assert_eq!(listed, vec!["css/a.css", "pre/css/a.css"]);
}
