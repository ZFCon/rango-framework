//! `contrib.contenttypes`: a table of every installed model.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use rango_core::dispatch::ConnectOptions;
use rango_core::exceptions::{Error, Result};
use rango_db::migrations::executor::POST_MIGRATE;
use rango_db::models::meta::registry;
use rango_db::models::Model;
use rango_db::{Db, Executor};

/// `ContentType` (table `rango_content_type`).
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "contenttypes", db_table = "rango_content_type", unique_together = [["app_label", "model"]], verbose_name = "content type", verbose_name_plural = "content types")]
pub struct ContentType {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100, verbose_name = "app label")]
    pub app_label: String,
    #[field(max_length = 100, verbose_name = "python model class name")]
    pub model: String,
}

impl ContentType {
    /// `str(content_type)`: the model's verbose name (or `app_label | model`).
    pub fn name(&self) -> String {
        registry().get(&format!("{}.{}", self.app_label, self.model)).map_or_else(|| self.model.clone(), |m| m.verbose_name.clone())
    }

    /// `natural_key()`.
    pub fn natural_key(&self) -> (String, String) {
        (self.app_label.clone(), self.model.clone())
    }

    /// `model_class()`: the model's metadata, if installed.
    pub fn model_meta(&self) -> Option<std::sync::Arc<rango_db::models::meta::ModelMeta>> {
        registry().get(&format!("{}.{}", self.app_label, self.model)).cloned()
    }
}

static CACHE: LazyLock<Mutex<HashMap<(String, String), ContentType>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// `ContentType.objects.clear_cache()`.
pub fn clear_cache() {
    CACHE.lock().expect("ct cache").clear();
}

/// `ContentType.objects.get_for_model(model)` where `label` is `app_label.ModelName` (case-insensitive).
pub async fn get_for_model(db: &dyn Executor, label: &str) -> Result<ContentType> {
    let (app, model) = label.split_once('.').ok_or_else(|| Error::LookupError(format!("Invalid model label '{label}'.")))?;
    let key = (app.to_lowercase(), model.to_lowercase());
    if let Some(ct) = CACHE.lock().expect("ct cache").get(&key) {
        return Ok(ct.clone());
    }
    let ct = match ContentType::objects().filter_kw("app_label", key.0.clone()).filter_kw("model", key.1.clone()).get(db).await {
        Ok(ct) => ct,
        Err(Error::ObjectDoesNotExist { .. }) => ContentType::objects().create(db, ContentType { id: None, app_label: key.0.clone(), model: key.1.clone() }).await?,
        Err(e) => return Err(e),
    };
    CACHE.lock().expect("ct cache").insert(key, ct.clone());
    Ok(ct)
}

/// `get_for_id(id)`.
pub async fn get_for_id(db: &dyn Executor, id: i64) -> Result<ContentType> {
    ContentType::objects().filter_kw("pk", id).get(db).await
}

/// `get_by_natural_key(app_label, model)`.
pub async fn get_by_natural_key(db: &dyn Executor, app_label: &str, model: &str) -> Result<ContentType> {
    ContentType::objects().filter_kw("app_label", app_label).filter_kw("model", model).get(db).await
}

/// `create_contenttypes()`: one row per model of the app, and remove nothing (use `remove_stale_contenttypes`).
pub async fn create_contenttypes(db: &dyn Executor, app_label: &str) -> Result<usize> {
    let reg = registry();
    let existing: Vec<String> = ContentType::objects().filter_kw("app_label", app_label).fetch(db).await?.into_iter().map(|c| c.model).collect();
    let mut n = 0;
    for m in reg.models().filter(|m| m.app_label == app_label && !m.abstract_ && m.proxy_for.is_none()) {
        let model = m.object_name.to_lowercase();
        if !existing.contains(&model) {
            ContentType::objects().create(db, ContentType { id: None, app_label: app_label.to_owned(), model }).await?;
            n += 1;
        }
    }
    Ok(n)
}

fn ready() -> futures_util::future::BoxFuture<'static, Result<()>> {
    Box::pin(async {
        POST_MIGRATE.connect(
            |d| async move {
                if let Err(e) = create_contenttypes(&Db::using(&d.args.using), &d.args.app_label).await {
                    tracing::warn!(target: "rango.contrib.contenttypes", "create_contenttypes failed: {e}");
                }
            },
            ConnectOptions::default().uid("rango.contrib.contenttypes.management.create_contenttypes"),
        );
        Ok(())
    })
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.contenttypes").ready(ready));
rango_db::register_initial_migration!("contenttypes", []);
