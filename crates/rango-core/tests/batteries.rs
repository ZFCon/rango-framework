//! cache, file storage and mail.

use rango_core::cache::{Timeout, caches};
use rango_core::conf::{self, Settings};
use rango_core::files::{FileSystemStorage, Storage};
use rango_core::mail::{EmailMessage, clear_outbox, outbox, send_mail};

#[tokio::test]
async fn cache_files_mail() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("batteries");
    let _ = std::fs::remove_dir_all(&dir);
    conf::configure(Settings {
        secret_key: "k".into(),
        caches: serde_json::json!({
            "default": {"BACKEND": "rango.core.cache.backends.locmem.LocMemCache", "LOCATION": "t", "KEY_PREFIX": "p", "TIMEOUT": 60},
            "disk": {"BACKEND": "rango.core.cache.backends.filebased.FileBasedCache", "LOCATION": dir.join("cache").to_str().unwrap()},
            "none": {"BACKEND": "rango.core.cache.backends.dummy.DummyCache"},
        }),
        email_backend: "rango.core.mail.backends.locmem.EmailBackend".into(),
        default_from_email: "site@example.com".into(),
        ..Settings::default()
    })
    .unwrap();

    // cache
    let c = caches("default").unwrap();
    assert!(c.get::<String>("a").await.unwrap().is_none());
    c.set("a", "x", Timeout::Default).await.unwrap();
    assert_eq!(c.get::<String>("a").await.unwrap().as_deref(), Some("x"));
    assert!(!c.add("a", "y", Timeout::Default).await.unwrap());
    c.set("n", &1, Timeout::Default).await.unwrap();
    assert_eq!(c.incr("n", 4).await.unwrap(), 5);
    assert_eq!(c.get_or_set("g", || 7, Timeout::Default).await.unwrap(), 7);
    c.set("gone", "x", Timeout::Secs(0.0)).await.unwrap();
    assert!(!c.has_key("gone").await.unwrap(), "timeout 0 means do not cache");
    assert_eq!(c.make_key("a", None), "p:1:a");
    let disk = caches("disk").unwrap();
    disk.set("k", &vec![1, 2, 3], Timeout::Never).await.unwrap();
    assert_eq!(disk.get::<Vec<i32>>("k").await.unwrap(), Some(vec![1, 2, 3]));
    assert!(caches("none").unwrap().get::<i32>("k").await.unwrap().is_none());
    disk.clear().await.unwrap();
    assert!(disk.get::<Vec<i32>>("k").await.unwrap().is_none());

    // storage
    let st = FileSystemStorage::new(dir.join("media"), "/media/");
    let n1 = st.save("docs/a b.txt", b"hello").await.unwrap();
    let n2 = st.save("docs/a b.txt", b"again").await.unwrap();
    assert_eq!(n1, "docs/a b.txt");
    assert!(n2.starts_with("docs/a b_") && n2.ends_with(".txt") && n2 != n1, "{n2}");
    assert_eq!(st.open(&n1).await.unwrap(), b"hello");
    assert_eq!(st.url("docs/a b.txt").unwrap(), "/media/docs/a%20b.txt");
    assert!(st.save("../escape.txt", b"x").await.is_err(), "path traversal is refused");
    st.delete(&n1).await.unwrap();
    assert!(!st.exists(&n1).await.unwrap());

    // mail
    clear_outbox();
    assert_eq!(send_mail("Héllo", "body", None, &["a@x.org", "b@x.org"], Some("<b>hi</b>")).await.unwrap(), 1);
    let sent = outbox();
    assert_eq!(sent.len(), 1);
    let text = sent[0].as_string().unwrap();
    assert!(text.contains("Subject: =?utf-8?b?") && text.contains("From: site@example.com") && text.contains("To: a@x.org, b@x.org") && text.contains("multipart/alternative"), "{text}");
    assert!(EmailMessage::new("bad\nsubject", "b", None, &["a@x.org"]).as_string().is_err());
}
