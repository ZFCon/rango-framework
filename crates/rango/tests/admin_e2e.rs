//! The admin: login, index, change list (search/filter/order/paging), add, change, delete, permissions.

use rango::conf::{self, DatabaseConfig, Settings};
use rango::contrib::admin::{AdminSite, ModelAdmin, create_permissions};
use rango::contrib::auth;
use rango::db::migrations::{MigrationLoader, Target, executor};
use rango::db::models::Model;
use rango::db::{Db, Executor};
use rango::test::{Client, assert_contains, assert_redirects};
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango::db", app_label = "blog", str = "title", ordering = ["id"])]
pub struct Article {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100)]
    pub title: String,
    #[field(blank)]
    pub body: String,
    #[field(default = false)]
    pub published: bool,
}

register_urlconf!("adminsite.urls", || {
    let site = AdminSite::new("/admin/");
    site.register::<Article>(ModelAdmin::default().list_display(["__str__", "published"]).list_filter(["published"]).search_fields(["title", "body"]).list_per_page(2)).unwrap();
    urlpatterns![path("admin/", include(site.urls()))]
});

#[tokio::test]
async fn admin_site() {
    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("adminsite.urls".into()),
        use_tz: true,
        installed_apps: ["contenttypes", "auth", "sessions", "messages"].map(|a| format!("rango.contrib.{a}")).to_vec(),
        middleware: ["sessions.middleware.SessionMiddleware", "auth.middleware.AuthenticationMiddleware", "messages.middleware.MessageMiddleware"].map(|m| format!("rango.contrib.{m}")).to_vec(),
        password_hashers: vec!["rango.contrib.auth.hashers.MD5PasswordHasher".into()],
        databases: [("default".to_owned(), DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })].into(),
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();
    let db = Db::default_db();
    let labels: Vec<String> = ["contenttypes", "auth", "sessions"].map(String::from).to_vec();
    executor::migrate(&db, &MigrationLoader::load(Some(&labels)).unwrap(), &Target::All).await.unwrap();
    db.execute_batch(r#"CREATE TABLE "blog_article" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "title" varchar(100) NOT NULL, "body" text NOT NULL, "published" bool NOT NULL);"#).await.unwrap();
    create_permissions(&db, "blog").await.unwrap();
    let um = auth::get_user_model().unwrap();
    um.create_superuser(&db, "root", "r@x.org", Some("pw")).await.unwrap();
    um.create_user(&db, "plain", "p@x.org", Some("pw")).await.unwrap();
    for (t, p) in [("Alpha", true), ("Beta", false), ("Gamma", true)] {
        Article::objects().create(&db, Article { id: None, title: t.into(), body: format!("about {t}"), published: p }).await.unwrap();
    }

    let c = Client::new().unwrap();
    assert_redirects(&c.get("/admin/").await.unwrap(), "/admin/login/?next=/admin/", 302);
    assert_contains(&c.post("/admin/login/", &[("username", "plain"), ("password", "pw")]).await.unwrap(), "staff account", 200);
    assert_redirects(&c.post("/admin/login/", &[("username", "root"), ("password", "pw"), ("next", "/admin/")]).await.unwrap(), "/admin/", 302);

    let index = c.get("/admin/").await.unwrap();
    assert_contains(&index, "Site administration", 200);
    assert_contains(&index, "href=\"/admin/blog/article/\">Articles</a>", 200);

    // change list: paging (2 per page), search, filter, ordering
    let list = c.get("/admin/blog/article/").await.unwrap();
    assert_contains(&list, "3 articles", 200);
    assert!(list.text().contains(">Alpha</a>") && list.text().contains(">Beta</a>") && !list.text().contains(">Gamma</a>"), "{}", list.text());
    assert_contains(&c.get("/admin/blog/article/?p=2").await.unwrap(), ">Gamma</a>", 200);
    assert_contains(&c.get("/admin/blog/article/?q=beta").await.unwrap(), "1 articles", 200);
    assert_contains(&c.get("/admin/blog/article/?published=1").await.unwrap(), "2 articles", 200);
    let desc = c.get("/admin/blog/article/?o=-id").await.unwrap().text();
    assert!(desc.find("Gamma").unwrap() < desc.find("Beta").unwrap(), "descending order");

    // add (validation error, then success + flash message)
    assert_contains(&c.get("/admin/blog/article/add/").await.unwrap(), "<label for=\"id_title\">Title:</label>", 200);
    assert_contains(&c.post("/admin/blog/article/add/", &[("title", ""), ("body", "x")]).await.unwrap(), "This field is required.", 200);
    assert_redirects(&c.post("/admin/blog/article/add/", &[("title", "Delta"), ("body", "d"), ("published", "on")]).await.unwrap(), "/admin/blog/article/", 302);
    assert_contains(&c.get("/admin/blog/article/").await.unwrap(), "The article “Delta” was added successfully.", 200);
    assert_eq!(Article::objects().count(&db).await.unwrap(), 4);

    // change + delete
    assert_contains(&c.get("/admin/blog/article/4/change/").await.unwrap(), "value=\"Delta\"", 200);
    assert_redirects(&c.post("/admin/blog/article/4/change/", &[("title", "Delta 2"), ("body", "d")]).await.unwrap(), "/admin/blog/article/", 302);
    assert_eq!(Article::objects().filter_kw("pk", 4).get(&db).await.unwrap().title, "Delta 2");
    assert_contains(&c.get("/admin/blog/article/4/delete/").await.unwrap(), "delete the article “Delta 2”", 200);
    assert_redirects(&c.post("/admin/blog/article/4/delete/", &[]).await.unwrap(), "/admin/blog/article/", 302);
    assert_eq!(Article::objects().count(&db).await.unwrap(), 3);
    assert_eq!(c.get("/admin/blog/article/99/change/").await.unwrap().status_code, 404);
    assert_eq!(c.get("/admin/blog/nope/").await.unwrap().status_code, 404);

    // a staff user without permissions is refused
    let mut staff = um.get_by_natural_key(&db, "plain").await.unwrap().unwrap();
    let _ = &mut staff;
    db.execute("UPDATE auth_user SET is_staff = 1 WHERE username = %s", &[rango::db::Value::Str("plain".into())]).await.unwrap();
    let c2 = Client::new().unwrap();
    assert_redirects(&c2.post("/admin/login/", &[("username", "plain"), ("password", "pw")]).await.unwrap(), "/admin/", 302);
    assert_eq!(c2.get("/admin/blog/article/").await.unwrap().status_code, 403);
    assert!(!c2.get("/admin/").await.unwrap().text().contains("Articles"));
}
