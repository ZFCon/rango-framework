//! The REST framework vs a REST framework: the same API, the same requests
//! (tools/gen_rest_golden.py), byte-identical bodies.

use rango::conf::{self, DatabaseConfig, Settings};
use rango::contrib::auth;
use rango::db::migrations::{MigrationLoader, Target, executor};
use rango::db::models::Model;
use rango::db::{Db, Executor};
use rango::rest::{Authentication, ModelSerializer, ModelViewSet, Pagination, Permission, Router};
use rango::test::Client;
use rango::urls::{include, path};
use rango::{register_urlconf, urlpatterns};

#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango::db", app_label = "blog", ordering = ["id"])]
pub struct Article {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 20)]
    pub title: String,
    #[field(slug, max_length = 50, unique)]
    pub slug: String,
    #[field(text, blank)]
    pub body: String,
    #[field(default = false)]
    pub published: bool,
    #[field(default = 0)]
    pub views: i32,
    #[field(max_digits = 4, decimal_places = 2, null, blank)]
    pub rating: Option<rango::db::rust_decimal::Decimal>,
}

register_urlconf!("rest.urls", || {
    let ser = || ModelSerializer::<Article>::all().unwrap();
    let mut router = Router::new();
    router.register("articles", ModelViewSet::new(ser()).authentication(vec![]).search_fields(&["title", "body"]).ordering_fields(&["views", "title"]));
    router.register_as("paged", ModelViewSet::new(ser()).read_only().authentication(vec![]).pagination(Some(Pagination::page_number(2))), "paged");
    router.register_as("private", ModelViewSet::new(ser()).read_only().authentication(vec![Authentication::Basic]).permissions(vec![Permission::IsAuthenticated]), "private");
    urlpatterns![path("api/", include(router.urls())), path("api/schema/", rango::rest::schema::schema_view())]
});

#[tokio::test]
async fn rest_matches_golden() {
    conf::configure(Settings {
        secret_key: "x".into(),
        allowed_hosts: vec!["testserver".into()],
        root_urlconf: Some("rest.urls".into()),
        use_tz: true,
        installed_apps: ["rango.contrib.contenttypes", "rango.contrib.auth", "rango.rest"].map(String::from).to_vec(),
        password_hashers: vec!["rango.contrib.auth.hashers.MD5PasswordHasher".into()],
        databases: [("default".to_owned(), DatabaseConfig { engine: "rango.db.backends.sqlite3".into(), name: ":memory:".into(), ..DatabaseConfig::default() })].into(),
        ..Settings::default()
    })
    .unwrap();
    rango::setup().await.unwrap();
    let db = Db::default_db();
    let labels: Vec<String> = ["contenttypes", "auth"].map(String::from).to_vec();
    executor::migrate(&db, &MigrationLoader::load(Some(&labels)).unwrap(), &Target::All).await.unwrap();
    db.execute_batch(r#"CREATE TABLE "blog_article" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "title" varchar(20) NOT NULL, "slug" varchar(50) NOT NULL UNIQUE, "body" text NOT NULL, "published" bool NOT NULL, "views" integer NOT NULL, "rating" decimal NULL);"#).await.unwrap();
    auth::get_user_model().unwrap().create_user(&db, "ann", "", Some("pw")).await.unwrap();

    let steps: Vec<serde_json::Value> = serde_json::from_str(include_str!("../../rango-rest/tests/golden/rest.json")).unwrap();
    let mut bad = Vec::new();
    for s in &steps {
        let mut c = Client::new().unwrap();
        for (k, v) in s["headers"].as_object().unwrap() {
            c = c.default_header(k, v.as_str().unwrap());
        }
        let body = if s["body"].is_null() { String::new() } else { s["body"].to_string() };
        let r = c.generic(s["method"].as_str().unwrap(), s["url"].as_str().unwrap(), body, "application/json").await.unwrap();
        let mut diffs = Vec::new();
        if u64::from(r.status_code) != s["status"].as_u64().unwrap() {
            diffs.push(format!("status {} != {}", r.status_code, s["status"]));
        }
        if r.text() != s["content"].as_str().unwrap() {
            diffs.push(format!("body\n   expected {}\n   got      {}", s["content"].as_str().unwrap(), r.text()));
        }
        for (h, key) in [("Content-Type", "content_type"), ("WWW-Authenticate", "www_authenticate"), ("Allow", "allow")] {
            if r.header(h) != s[key].as_str() {
                diffs.push(format!("{h}: expected {:?} got {:?}", s[key].as_str(), r.header(h)));
            }
        }
        if !diffs.is_empty() {
            bad.push(format!("{} {} {}: {}", s["method"], s["url"], s["body"], diffs.join("; ")));
        }
    }
    assert!(bad.is_empty(), "{} of {} REST steps differ from the REST framework:\n{}", bad.len(), steps.len(), bad.join("\n"));

    // OpenAPI document: structure for the same API.
    let r = Client::new().unwrap().get("/api/schema/").await.unwrap();
    let doc: serde_json::Value = serde_json::from_str(&r.text()).unwrap();
    assert_eq!(doc["openapi"], "3.0.3");
    let list = &doc["paths"]["/api/articles/"]["get"];
    assert_eq!(list["operationId"], "articles_list");
    assert_eq!(list["tags"][0], "articles");
    let names: Vec<&str> = list["parameters"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["ordering", "search"]);
    assert_eq!(doc["paths"]["/api/articles/{id}/"]["patch"]["requestBody"]["content"]["application/json"]["schema"]["$ref"], "#/components/schemas/PatchedArticle");
    assert_eq!(doc["paths"]["/api/paged/"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"], "#/components/schemas/PaginatedArticleList");
    assert_eq!(doc["paths"]["/api/private/"]["get"]["security"], serde_json::json!([{"basicAuth": []}]));
    let article = &doc["components"]["schemas"]["Article"];
    assert_eq!(article["required"], serde_json::json!(["id", "title", "slug"]));
    assert_eq!(article["properties"]["title"], serde_json::json!({"type": "string", "maxLength": 20}));
    assert_eq!(article["properties"]["id"], serde_json::json!({"type": "integer", "readOnly": true}));
    assert_eq!(doc["components"]["securitySchemes"]["basicAuth"], serde_json::json!({"type": "http", "scheme": "basic"}));
}
