//! CORS middleware vs the CORS middleware.

use rango::conf::{self, Settings, override_settings};
use rango::http::{HttpRequest, HttpResponse};
use rango::test::Client;
use rango::urls::re_path;
use rango::{Result, register_urlconf, urlpatterns};

async fn ok(_: HttpRequest) -> Result<HttpResponse> {
    Ok(HttpResponse::new("ok"))
}

register_urlconf!("cors.urls", || urlpatterns![re_path("^.*$", ok)]);

const NAMES: [&str; 8] = [
    "vary",
    "access-control-allow-origin",
    "access-control-allow-credentials",
    "access-control-expose-headers",
    "access-control-allow-headers",
    "access-control-allow-methods",
    "access-control-max-age",
    "access-control-allow-private-network",
];

#[tokio::test]
async fn cors_matches_reference() {
    let base = Settings { secret_key: "x".into(), allowed_hosts: vec!["testserver".into()], root_urlconf: Some("cors.urls".into()), middleware: vec!["rango.middleware.cors.CorsMiddleware".into()], ..Settings::default() };
    conf::configure(base.clone()).unwrap();
    rango::setup().await.unwrap();
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("golden/cors.json")).unwrap();
    let mut bad = Vec::new();
    for c in &cases {
        // Apply the case's CORS_* settings on top of the base settings.
        let mut v = serde_json::to_value(&base).unwrap();
        for (k, val) in c["settings"].as_object().unwrap() {
            v[k] = val.clone();
        }
        let s: Settings = serde_json::from_value(v).unwrap();
        let got = override_settings(
            move |cur| *cur = s,
            async {
                let mut client = Client::new().unwrap();
                for (k, val) in c["headers"].as_object().unwrap() {
                    client = client.default_header(&k.replace('_', "-"), val.as_str().unwrap());
                }
                client.generic(c["method"].as_str().unwrap(), c["path"].as_str().unwrap(), "", "text/plain").await.unwrap()
            },
        )
        .await;
        let e = &c["expected"];
        let mut diffs = Vec::new();
        if u64::from(got.status_code) != e["status"].as_u64().unwrap() || got.text() != e["body"].as_str().unwrap() {
            diffs.push(format!("status/body {} {:?}", got.status_code, got.text()));
        }
        for n in NAMES {
            let want = e["headers"].get(n).and_then(|x| x.as_str());
            if got.header(n) != want {
                diffs.push(format!("{n}: expected {want:?} got {:?}", got.header(n)));
            }
        }
        if !diffs.is_empty() {
            bad.push(format!("{} {} {} {}: {}", c["settings"], c["method"], c["path"], c["headers"], diffs.join("; ")));
        }
    }
    assert!(bad.is_empty(), "{} of {} CORS cases differ:\n{}", bad.len(), cases.len(), bad.join("\n"));
}
