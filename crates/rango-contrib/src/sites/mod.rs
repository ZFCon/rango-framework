//! `contrib.sites`.

use rango_core::conf::settings;
use rango_core::dispatch::ConnectOptions;
use rango_core::exceptions::{Error, Result};
use rango_db::migrations::executor::POST_MIGRATE;
use rango_db::models::Model;
use rango_db::{Db, Executor};
use rango_http::HttpRequest;

/// `Site` (table `rango_site`).
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "sites", db_table = "rango_site", ordering = ["domain"], verbose_name = "site", verbose_name_plural = "sites", str = "domain")]
pub struct Site {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100, unique, verbose_name = "domain name")]
    pub domain: String,
    #[field(max_length = 50, verbose_name = "display name")]
    pub name: String,
}

/// `Site.objects.get_current(request)`: by `SITE_ID`, else by the request's host.
pub async fn get_current(db: &dyn Executor, request: Option<&HttpRequest>) -> Result<Site> {
    if let Some(id) = settings().site_id {
        return Site::objects().filter_kw("pk", id).get(db).await;
    }
    let req = request.ok_or_else(|| Error::improperly_configured("You're using the Rango \"sites framework\" without having set the SITE_ID setting. Create a site in your database and set the SITE_ID setting or pass a request to Site.objects.get_current() to fix this error."))?;
    let host = req.get_host()?;
    let bare = host.split(':').next().unwrap_or(&host).to_owned();
    match Site::objects().filter_kw("domain__iexact", host.clone()).get(db).await {
        Ok(s) => Ok(s),
        Err(Error::ObjectDoesNotExist { .. }) => Site::objects().filter_kw("domain__iexact", bare).get(db).await,
        Err(e) => Err(e),
    }
}

/// `create_default_site`: `example.com` when the table is empty.
pub async fn create_default_site(db: &dyn Executor) -> Result<()> {
    if Site::objects().exists(db).await? {
        return Ok(());
    }
    let id = settings().site_id.unwrap_or(1);
    Site::objects().create(db, Site { id: Some(id), domain: "example.com".into(), name: "example.com".into() }).await?;
    Ok(())
}

fn ready() -> futures_util::future::BoxFuture<'static, Result<()>> {
    Box::pin(async {
        POST_MIGRATE.connect(
            |d| async move {
                if let Err(e) = create_default_site(&Db::using(&d.args.using)).await {
                    tracing::warn!(target: "rango.contrib.sites", "create_default_site failed: {e}");
                }
            },
            ConnectOptions::sender("sites").uid("rango.contrib.sites.management.create_default_site"),
        );
        Ok(())
    })
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.sites").ready(ready));
rango_db::register_initial_migration!("sites", []);
