//! M5 e2e: templates through the real stack: `render`, `TemplateResponse` + `process_template_response`,
//! `RequestContext` processors, `{% csrf_token %}`, inheritance, project `404.html`.

use std::sync::Arc;

use async_trait::async_trait;
use rango::conf::{self, Settings};
use rango::core::handlers::Middleware;
use rango::http::{HttpRequest, HttpResponse};
use rango::shortcuts::render;
use rango::template::TemplateResponse;
use rango::test::{Client, assert_contains};
use rango::urls::path;
use rango::{Result, context, register_middleware, register_urlconf, urlpatterns};

async fn index(req: HttpRequest) -> Result<HttpResponse> {
    render(&req, "polls/index.html", context! { "questions" => vec!["What's new?", "How <b>now</b>?"] })
}

async fn detail(req: HttpRequest, id: i64) -> Result<HttpResponse> {
    Ok(TemplateResponse::new(&req, ["polls/missing.html", "polls/detail.html"], context! { "id" => id }))
}

struct AddContext;

#[async_trait]
impl Middleware for AddContext {
    async fn process_template_response(&self, _req: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if let Some(t) = response.deferred_mut::<TemplateResponse>() {
            t.context_data.as_mut().unwrap().set("added_by", "middleware");
        }
        Ok(response)
    }
}
register_middleware!("tests.AddContext", || Ok(Arc::new(AddContext)));

register_urlconf!("site.urls", || urlpatterns![path("", index), path("<int:id>/", detail)]);

#[tokio::test]
async fn templates_end_to_end() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("templates_e2e");
    std::fs::create_dir_all(dir.join("polls")).unwrap();
    let w = |name: &str, body: &str| std::fs::write(dir.join(name), body).unwrap();
    w("base.html", "<title>{% block title %}Polls{% endblock %}</title>{% block content %}{% endblock %}");
    w("polls/index.html", "{% extends 'base.html' %}{% block title %}{{ block.super }} index{% endblock %}{% block content %}{% for q in questions %}<li>{{ q }} ({{ forloop.counter }}){% endfor %} {{ request.path }} {{ STATIC_URL }}{% endblock %}");
    w("polls/detail.html", "<form method=\"post\">{% csrf_token %}{{ id }} {{ added_by }}</form>");
    w("404.html", "custom 404 for {{ request_path }}");

    conf::configure(Settings {
        secret_key: "k".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("site.urls".into()),
        static_url: Some("/static/".into()),
        middleware: ["rango.middleware.csrf.CsrfViewMiddleware", "tests.AddContext"].map(String::from).to_vec(),
        templates: vec![serde_json::json!({
            "BACKEND": "rango.template.backends.rango.RangoTemplates",
            "DIRS": [dir.to_str().unwrap()],
            "OPTIONS": { "context_processors": [
                "rango.template.context_processors.request",
                "rango.template.context_processors.static",
            ]},
        })],
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();

    let client = Client::new().unwrap();
    let r = client.get("/").await.unwrap();
    assert_contains(&r, "<title>Polls index</title><li>What&#x27;s new? (1)<li>How &lt;b&gt;now&lt;/b&gt;? (2) / /static/", 200);

    let r = client.get("/7/").await.unwrap();
    assert_eq!(r.status_code, 200);
    let html = r.text();
    assert!(html.contains("name=\"csrfmiddlewaretoken\" value=\"") && html.contains("7 middleware</form>"), "{html}");
    assert!(r.cookies.contains_key("csrftoken"));
    assert!(client.get("/").await.unwrap().cookies.get("csrftoken").is_none(), "token is lazy: unused => no cookie");

    assert_contains(&client.get("/nope/").await.unwrap(), "custom 404 for /nope/", 404);
}
