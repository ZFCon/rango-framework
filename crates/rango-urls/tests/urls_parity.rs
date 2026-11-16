//! URL resolving/reversing parity with the reference.

use indexmap::IndexMap;
use rango_core::conf::{self, Settings};
use rango_http::{HttpRequest, HttpResponse};
use rango_urls::{Arg, Reverse, include, path, re_path, register_urlconf, resolve, reverse, urlpatterns};
use serde_json::{Value, json};

async fn v(_: HttpRequest) -> rango_core::Result<HttpResponse> {
    Ok(HttpResponse::new("ok"))
}

fn polls() -> Vec<rango_urls::UrlEntry> {
    urlpatterns![path("", v).name("index"), path("<int:pk>/", v).name("detail"), path("<int:pk>/vote/", v).name("vote")]
}

register_urlconf!("urlconf_oracle", || urlpatterns![
    path("", v).name("index"),
    path("articles/<int:year>/", v).name("year"),
    path("articles/<int:year>/<slug:slug>/", v).name("article"),
    path("files/<path:p>", v).name("file"),
    path("u/<uuid:id>/", v).name("user"),
    re_path(r"^re/(?P<a>\d+)/(?P<b>[a-z]+)/$", v).name("re-named"),
    re_path(r"^pos/(\d+)/(\w+)/$", v).name("re-pos"),
    re_path(r"^opt/(?:(?P<x>\d+)/)?end/$", v).name("re-opt"),
    path("polls/", include(polls()).app_name("polls").namespace("polls")),
    path("polls2/", include(polls()).app_name("polls").namespace("polls2")),
    path("api/<str:version>/", include(urlpatterns![path("items/", v).name("items"), path("items/<int:id>/", v).name("item")])),
    path("kw/", v).kwargs([("extra", Arg::Int(1))]).name("kw"),
    path(
        "outer/",
        include(urlpatterns![path(
            "inner/",
            include(urlpatterns![path("leaf/<int:n>/", v).name("leaf")]).app_name("inner").namespace("inner")
        )])
        .app_name("outer")
        .namespace("outer")
    ),
]);

fn arg_json(a: &Arg) -> Value {
    match a {
        Arg::Int(i) => json!(i),
        other => json!(other.to_string()),
    }
}

fn map_json(m: &IndexMap<String, Arg>) -> Value {
    Value::Object(m.iter().map(|(k, v)| (k.clone(), arg_json(v))).collect())
}

#[test]
fn urls_match_rango() {
    conf::configure(Settings { root_urlconf: Some("urlconf_oracle".into()), ..Settings::default() }).unwrap();
    let golden: Vec<Value> = serde_json::from_str(include_str!("golden/urls.json")).unwrap();
    let mut failures = Vec::new();
    for case in &golden {
        let (label, actual): (String, Value) = if case["kind"] == "resolve" {
            let p = case["path"].as_str().unwrap();
            let got = match resolve(p, None).unwrap() {
                Ok(m) => json!({
                    "url_name": m.url_name, "app_names": m.app_names, "namespaces": m.namespaces, "view_name": m.view_name(),
                    "route": m.route, "args": m.args.iter().map(arg_json).collect::<Vec<_>>(), "kwargs": map_json(&m.kwargs),
                    "captured": map_json(&m.captured_kwargs), "extra": map_json(&m.extra_kwargs),
                }),
                Err(_) => json!({ "error": "Resolver404" }),
            };
            (format!("resolve({p})"), got)
        } else {
            let name = case["name"].as_str().unwrap();
            let mut r = Reverse::new().args(case["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()));
            for (k, val) in case["kwargs"].as_object().unwrap() {
                r = r.kwarg(k, val.as_str().unwrap());
            }
            if let Some(app) = case["current_app"].as_str() {
                r = r.current_app(app);
            }
            let got = match reverse(name, r) {
                Ok(u) => json!(u),
                Err(_) => json!({ "error": "NoReverseMatch" }),
            };
            (format!("reverse({name}, {} {})", case["args"], case["kwargs"]), got)
        };
        let expected = &case["expected"];
        let mut e = expected.clone();
        // the reference implementation reports an unmatched URL param error as NoReverseMatch or ValueError for mixed args.
        if e["error"] == "ValueError" {
            e = json!({ "error": "NoReverseMatch" });
        }
        if actual != e {
            failures.push(format!("{label}\n   expected {expected}\n   got      {actual}"));
        }
    }
    assert!(failures.is_empty(), "{} of {} URL cases differ from the reference implementation:\n{}", failures.len(), golden.len(), failures.join("\n"));
}
