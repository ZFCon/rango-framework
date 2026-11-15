//! E1 (M2 edition): tutorial polls app, in memory, driven through the real
//! middleware chain (Client) and over a real TCP socket.

use std::sync::Mutex;

use rango::conf::{self, Settings, override_settings};
use rango::core::handlers::Handler;
use rango::core::servers::serve;
use rango::http::{HttpRequest, HttpResponse};
use rango::middleware::csrf::get_token;
use rango::shortcuts::redirect_with;
use rango::test::{Client, assert_contains, assert_redirects};
use rango::urls::{Reverse, include, path};
use rango::{Result, register_urlconf, reverse, urlpatterns};

static VOTES: Mutex<[u32; 2]> = Mutex::new([0, 0]);
const CHOICES: [&str; 2] = ["Not much", "The sky"];

async fn index(_: HttpRequest) -> Result<HttpResponse> {
    let url = reverse!("polls:detail", 1)?;
    Ok(HttpResponse::new(format!("<ul><li><a href=\"{url}\">What's new?</a></li></ul>")))
}

async fn detail(req: HttpRequest, id: i64) -> Result<HttpResponse> {
    if id != 1 {
        return Err(rango::Error::Http404("No Question matches the given query.".into()));
    }
    let token = get_token(&req);
    let action = reverse!("polls:vote", id)?;
    let mut radios = String::new();
    for (i, c) in CHOICES.iter().enumerate() {
        radios.push_str(&format!("<input type=\"radio\" name=\"choice\" value=\"{i}\"> {c}<br>"));
    }
    Ok(HttpResponse::new(format!(
        "<h1>What's new?</h1><form action=\"{action}\" method=\"post\"><input type=\"hidden\" name=\"csrfmiddlewaretoken\" value=\"{token}\">{radios}<input type=\"submit\" value=\"Vote\"></form>"
    )))
}

async fn vote(req: HttpRequest, id: i64) -> Result<HttpResponse> {
    let choice: usize = req.post().await?.get("choice").and_then(|c| c.parse().ok()).unwrap_or(usize::MAX);
    if choice >= CHOICES.len() {
        return Ok(HttpResponse::new("<p>You didn't select a choice.</p>"));
    }
    VOTES.lock().unwrap()[choice] += 1;
    redirect_with("polls:results", Reverse::new().args([id]), false)
}

async fn results(_: HttpRequest, _id: i64) -> Result<HttpResponse> {
    let v = *VOTES.lock().unwrap();
    Ok(HttpResponse::new(format!("<h1>What's new?</h1><ul><li>{} -- {} votes</li><li>{} -- {} votes</li></ul>", CHOICES[0], v[0], CHOICES[1], v[1])))
}

register_urlconf!("polls.urls", app_name = "polls", || urlpatterns![
    path("", index).name("index"),
    path("<int:id>/", detail).name("detail"),
    path("<int:id>/vote/", vote).name("vote"),
    path("<int:id>/results/", results).name("results"),
]);
register_urlconf!("mysite.urls", || urlpatterns![path("polls/", include("polls.urls"))]);
register_urlconf!("empty.urls", || urlpatterns![]);

fn csrf_token_from(html: &str) -> String {
    let marker = "name=\"csrfmiddlewaretoken\" value=\"";
    let start = html.find(marker).expect("token field") + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_owned()
}

#[tokio::test]
async fn polls_tutorial() {
    conf::configure(Settings {
        debug: false,
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into(), "127.0.0.1".into()],
        root_urlconf: Some("mysite.urls".into()),
        middleware: [
            "rango.middleware.security.SecurityMiddleware",
            "rango.middleware.common.CommonMiddleware",
            "rango.middleware.csrf.CsrfViewMiddleware",
            "rango.middleware.clickjacking.XFrameOptionsMiddleware",
            "rango.middleware.http.ConditionalGetMiddleware",
        ]
        .map(String::from)
        .to_vec(),
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();

    // Pages, security headers, Content-Length, ETag/304.
    let client = Client::new().unwrap().enforce_csrf_checks(true);
    let r = client.get("/polls/").await.unwrap();
    assert_contains(&r, "<a href=\"/polls/1/\">What's new?</a>", 200);
    assert_eq!(r.header("X-Frame-Options"), Some("DENY"));
    assert_eq!(r.header("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(r.header("Referrer-Policy"), Some("same-origin"));
    assert_eq!(r.header("Content-Length").map(str::parse::<usize>), Some(Ok(r.content.len())));
    let etag = r.header("ETag").expect("ConditionalGetMiddleware adds ETag").to_owned();
    let not_modified = client.get("/polls/").await.unwrap();
    assert_eq!(not_modified.status_code, 200);
    let cond = Client::new().unwrap().default_header("If-None-Match", &etag).get("/polls/").await.unwrap();
    assert_eq!(cond.status_code, 304);
    assert!(cond.content.is_empty());
    assert_eq!(client.head("/polls/").await.unwrap().content.len(), 0);

    // 404 (production page), APPEND_SLASH redirect, 405 from the method decorator is not used here.
    assert_contains(&client.get("/polls/99/").await.unwrap(), "<h1>Not Found</h1>", 404);
    assert_redirects(&client.get("/polls").await.unwrap(), "/polls/", 301);
    assert_eq!(client.get("/nope/").await.unwrap().status_code, 404);
    let bad_host = Client::new().unwrap().default_header("Host", "evil.example").get("/polls/").await.unwrap();
    assert_eq!(bad_host.status_code, 400);

    // CSRF: POST without token is rejected; with token it works and redirects to the results.
    let r = client.post("/polls/1/vote/", &[("choice", "1")]).await.unwrap();
    assert_contains(&r, "CSRF verification failed", 403);
    let page = client.get("/polls/1/").await.unwrap();
    assert!(page.cookies.contains_key("csrftoken") && client.cookie("csrftoken").is_some());
    let token = csrf_token_from(&page.text());
    let r = client.post("/polls/1/vote/", &[("choice", "1"), ("csrfmiddlewaretoken", &token)]).await.unwrap();
    assert_redirects(&r, "/polls/1/results/", 302);
    assert_contains(&client.get("/polls/1/results/").await.unwrap(), "The sky -- 1 votes", 200);
    // Follow redirects automatically.
    let r = Client::new().unwrap().follow_redirects(true).post("/polls/1/vote/", &[("choice", "0")]).await.unwrap();
    assert_eq!(r.redirect_chain, vec![("/polls/1/results/".to_owned(), 302)]);
    assert_contains(&r, "Not much -- 1 votes", 200);

    // Debug pages (override_settings): technical 404 lists the tried patterns, welcome page for empty URLconf.
    override_settings(|s| s.debug = true, async {
        let r = Client::new().unwrap().get("/nope/").await.unwrap();
        assert_contains(&r, "Page not found", 404);
        assert_contains(&r, "polls/", 404);
        let r = Client::new().unwrap().get("/polls/99/").await.unwrap();
        assert_contains(&r, "No Question matches the given query.", 404);
    })
    .await;
    override_settings(
        |s| {
            s.debug = true;
            s.root_urlconf = Some("empty.urls".into());
        },
        async {
            assert_contains(&Client::new().unwrap().get("/").await.unwrap(), "The install worked successfully! Congratulations!", 200);
        },
    )
    .await;

    // Real sockets: bind an ephemeral port, speak HTTP/1.1.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handler = std::sync::Arc::new(Handler::new().unwrap());
    let server = tokio::spawn(serve(handler, listener, false));
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET /polls/ HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    assert!(buf.starts_with("HTTP/1.1 200 OK"), "{buf}");
    assert!(buf.contains("<a href=\"/polls/1/\">What's new?</a>"), "{buf}");
    assert!(buf.to_lowercase().contains("x-frame-options: deny"));
    server.abort();
}
