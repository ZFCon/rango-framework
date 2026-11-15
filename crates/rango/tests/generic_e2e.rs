//! Class-based views + forms + templates + ORM through the real middleware stack.

use rango::conf::{self, DatabaseConfig, Settings};
use rango::db::models::Model;
use rango::db::{Db, Executor};
use rango::forms::ModelFormOptions;
use rango::test::{Client, assert_contains, assert_redirects};
use rango::urls::path;
use rango::views::generic::{CreateView, DateView, DateViewKind, DeleteView, DetailView, ListView, RedirectView, TemplateView, UpdateView, View};
use rango::{register_urlconf, urlpatterns};

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango::db", app_label = "blog", str = "title")]
pub struct Article {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100)]
    pub title: String,
    #[field(blank)]
    pub body: String,
    pub published: rango::db::chrono::NaiveDate,
}

register_urlconf!("blog.urls", || urlpatterns![
    path("about/", TemplateView::new("about.html").extra("site", "Rango").as_view()).name("about"),
    path("old/<int:pk>/", RedirectView::to_pattern("detail").permanent(true).as_view()),
    path("articles/", ListView::<Article>::new().paginate_by(2).ordering(["id"]).as_view()).name("list"),
    path("articles/add/", CreateView::<Article>::new(ModelFormOptions::fields(["title", "body", "published"])).success_url("/articles/").as_view()),
    path("articles/<int:pk>/", DetailView::<Article>::new().as_view()).name("detail"),
    path("articles/<int:pk>/edit/", UpdateView::<Article>::new(ModelFormOptions::fields(["title", "body"])).absolute_url(|a| format!("/articles/{}/", a.id.unwrap())).as_view()),
    path("articles/<int:pk>/delete/", DeleteView::<Article>::new().success_url("/articles/").as_view()),
    path("archive/<int:year>/", DateView::<Article>::new(DateViewKind::Year, "published").template_name("year.html").make_object_list(true).allow_future(true).as_view()),
]);

#[tokio::test]
async fn generic_views() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("generic_e2e");
    std::fs::create_dir_all(dir.join("blog")).unwrap();
    let w = |n: &str, b: &str| std::fs::write(dir.join(n), b).unwrap();
    w("about.html", "About {{ site }}");
    w("blog/article_list.html", "{% for a in object_list %}<li>{{ a }}</li>{% endfor %}{% if is_paginated %}p{{ page_obj.number }}/{{ paginator.num_pages }}{% if page_obj.has_next %} next={{ page_obj.next_page_number }}{% endif %}{% endif %}");
    w("blog/article_detail.html", "{{ object.title }}|{{ article.body }}|{{ object.published|date:'Y' }}");
    w("blog/article_form.html", "<form method=\"post\">{% csrf_token %}{{ form.as_p }}</form>");
    w("blog/article_confirm_delete.html", "delete {{ object }}?");
    w("year.html", "{{ year|date:'Y' }}:{% for m in date_list %}{{ m|date:'m' }},{% endfor %}:{{ object_list|length }}");

    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("blog.urls".into()),
        middleware: ["rango.middleware.csrf.CsrfViewMiddleware"].map(String::from).to_vec(),
        databases: [("default".to_owned(), DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })].into(),
        templates: vec![rango::json!({"BACKEND": "rango.template.backends.rango.RangoTemplates", "DIRS": [dir.to_str().unwrap()], "OPTIONS": {"context_processors": []}})],
        use_tz: true,
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();
    let db = Db::default_db();
    db.execute_batch(r#"CREATE TABLE "blog_article" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "title" varchar(100) NOT NULL, "body" text NOT NULL, "published" date NOT NULL);"#).await.unwrap();
    for (t, d) in [("One", "2024-01-05"), ("Two", "2024-03-09"), ("Three", "2023-12-31")] {
        Article::objects().create(&db, Article { id: None, title: t.into(), body: format!("body of {t}"), published: d.parse().unwrap() }).await.unwrap();
    }

    let c = Client::new().unwrap();
    assert_contains(&c.get("/about/").await.unwrap(), "About Rango", 200);
    assert_redirects(&c.get("/old/2/").await.unwrap(), "/articles/2/", 301);
    assert_eq!(c.post("/about/", &[]).await.unwrap().status_code, 405);

    // ListView with pagination (page 2 = last), 404 beyond.
    assert_contains(&c.get("/articles/").await.unwrap(), "<li>One</li><li>Two</li>p1/2 next=2", 200);
    assert_contains(&c.get("/articles/?page=2").await.unwrap(), "<li>Three</li>p2/2", 200);
    assert_eq!(c.get("/articles/?page=9").await.unwrap().status_code, 404);

    // DetailView by pk; 404 when missing.
    assert_contains(&c.get("/articles/1/").await.unwrap(), "One|body of One|2024", 200);
    assert_eq!(c.get("/articles/99/").await.unwrap().status_code, 404);

    // CreateView: form rendered with csrf field; invalid re-renders with errors; valid saves + redirects.
    let r = c.get("/articles/add/").await.unwrap();
    assert!(r.text().contains("<label for=\"id_title\">Title:</label>") && r.text().contains("name=\"csrfmiddlewaretoken\""), "{}", r.text());
    let r = c.post("/articles/add/", &[("title", ""), ("published", "x")]).await.unwrap();
    assert_contains(&r, "This field is required.", 200);
    assert_contains(&r, "Enter a valid date.", 200);
    let r = c.post("/articles/add/", &[("title", "Four"), ("body", "b4"), ("published", "2022-02-02")]).await.unwrap();
    assert_redirects(&r, "/articles/", 302);
    assert_eq!(Article::objects().count(&db).await.unwrap(), 4);

    // UpdateView: initial values, redirect to the object URL.
    assert_contains(&c.get("/articles/4/edit/").await.unwrap(), "value=\"Four\"", 200);
    let r = c.post("/articles/4/edit/", &[("title", "Quattro"), ("body", "b")]).await.unwrap();
    assert_redirects(&r, "/articles/4/", 302);
    assert_contains(&c.get("/articles/4/").await.unwrap(), "Quattro|b|2022", 200);

    // DeleteView.
    assert_contains(&c.get("/articles/4/delete/").await.unwrap(), "delete Quattro?", 200);
    assert_redirects(&c.post("/articles/4/delete/", &[]).await.unwrap(), "/articles/", 302);
    assert_eq!(Article::objects().count(&db).await.unwrap(), 3);

    // Date-based year archive.
    assert_contains(&c.get("/archive/2024/").await.unwrap(), "2024:01,03,:2", 200);
}
