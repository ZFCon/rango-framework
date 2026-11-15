//! sessions + auth + messages through the middleware stack (SQLite).

use rango::conf::{self, DatabaseConfig, Settings};
use rango::contrib::auth::{self, Credentials, RequestUserExt, decorators::login_required};
use rango::contrib::messages;
use rango::contrib::sessions::RequestSessionExt;
use rango::db::migrations::{MigrationLoader, Target, executor};
use rango::db::Db;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::redirect;
use rango::test::{Client, assert_contains, assert_redirects};
use rango::urls::path;
use rango::{Result, register_urlconf, urlpatterns};

async fn login_view(req: HttpRequest) -> Result<HttpResponse> {
    let post = req.post().await?;
    let creds = Credentials::new(post.get("username").unwrap_or(""), post.get("password").unwrap_or(""));
    let db = Db::default_db();
    match auth::authenticate(&db, &creds).await? {
        Some((user, backend)) => {
            auth::login(&req, &db, user, Some(&backend)).await?;
            messages::success(&req, "Welcome back")?;
            redirect("/secret/")
        }
        None => Ok(HttpResponse::new("bad credentials")),
    }
}

async fn secret(req: HttpRequest) -> Result<HttpResponse> {
    let notes: Vec<String> = messages::get_messages(&req).map(|m| m.take().into_iter().map(|x| format!("{}:{}", x.level_tag(), x.message)).collect()).unwrap_or_default();
    Ok(HttpResponse::new(format!("hello {} [{}]", req.user().username().await, notes.join(","))))
}

async fn counter(req: HttpRequest) -> Result<HttpResponse> {
    let n: i64 = req.session().get_as("n").unwrap_or(0) + 1;
    req.session().set("n", n);
    Ok(HttpResponse::new(n.to_string()))
}

async fn logout_view(req: HttpRequest) -> Result<HttpResponse> {
    auth::logout(&req).await?;
    redirect("/login/")
}

register_urlconf!("authsite.urls", || urlpatterns![
    path("login/", login_view),
    path("secret/", login_required(secret)),
    path("counter/", counter),
    path("logout/", logout_view),
]);

#[tokio::test]
async fn login_flow() {
    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("authsite.urls".into()),
        login_url: "/login/".into(),
        use_tz: true,
        installed_apps: ["contenttypes", "auth", "sessions", "messages"].map(|a| format!("rango.contrib.{a}")).to_vec(),
        middleware: ["sessions.middleware.SessionMiddleware", "auth.middleware.AuthenticationMiddleware", "messages.middleware.MessageMiddleware"].map(|m| format!("rango.contrib.{m}")).to_vec(),
        password_hashers: vec!["rango.contrib.auth.hashers.MD5PasswordHasher".into()], // fast
        databases: [("default".to_owned(), DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })].into(),
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();
    let db = Db::default_db();
    let labels: Vec<String> = ["contenttypes", "auth", "sessions"].map(String::from).to_vec();
    let loader = MigrationLoader::load(Some(&labels)).unwrap();
    executor::migrate(&db, &loader, &Target::All).await.unwrap();
    auth::get_user_model().unwrap().create_user(&db, "ann", "ann@example.com", Some("s3cret")).await.unwrap();

    let c = Client::new().unwrap();
    // anonymous -> redirected to login with ?next=
    assert_redirects(&c.get("/secret/").await.unwrap(), "/login/?next=/secret/", 302);
    assert_contains(&c.post("/login/", &[("username", "ann"), ("password", "wrong")]).await.unwrap(), "bad credentials", 200);
    assert!(c.cookie("sessionid").is_none(), "no session cookie for anonymous/failed login");

    // login sets the session cookie; the message is delivered once.
    assert_redirects(&c.post("/login/", &[("username", "ann"), ("password", "s3cret")]).await.unwrap(), "/secret/", 302);
    assert!(c.cookie("sessionid").is_some());
    assert_contains(&c.get("/secret/").await.unwrap(), "hello ann [success:Welcome back]", 200);
    assert_contains(&c.get("/secret/").await.unwrap(), "hello ann []", 200);

    // plain session counter
    let c2 = Client::new().unwrap();
    assert_contains(&c2.get("/counter/").await.unwrap(), "1", 200);
    assert_contains(&c2.get("/counter/").await.unwrap(), "2", 200);

    // logout flushes the session
    assert_redirects(&c.get("/logout/").await.unwrap(), "/login/", 302);
    assert_redirects(&c.get("/secret/").await.unwrap(), "/login/?next=/secret/", 302);

    // contenttypes were created by post_migrate
    assert!(rango::contrib::contenttypes::get_by_natural_key(&db, "auth", "user").await.is_ok());
}
