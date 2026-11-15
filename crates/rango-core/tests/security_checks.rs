use rango_core::checks::{CheckContext, run_checks};
use rango_core::conf::{self, Settings};

#[tokio::test]
async fn deploy_checks() {
    conf::configure(Settings { secret_key: "short".into(), debug: true, ..Settings::default() }).unwrap();
    let ids = |deploy| async move { run_checks(&CheckContext { app_labels: None, databases: None }, None, deploy).await.unwrap().into_iter().filter_map(|m| m.id).collect::<Vec<_>>() };
    assert!(ids(false).await.iter().all(|i| !i.starts_with("security.")), "security checks are deploy-only");
    let got = ids(true).await;
    for want in ["security.W001", "security.W002", "security.W003", "security.W009", "security.W018"] {
        assert!(got.iter().any(|i| i == want), "{want} missing from {got:?}");
    }
}
