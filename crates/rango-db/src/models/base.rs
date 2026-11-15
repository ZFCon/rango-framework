//! `Model`: the trait derived by `#[derive(Model)]`, plus row data and instance state.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::connection::Executor;
use crate::models::meta::{ModelMeta, registry};
use crate::models::queryset::QuerySet;
use crate::models::signals::{ModelEvent, POST_DELETE, POST_SAVE, PRE_DELETE, PRE_SAVE};
use crate::value::Value;

/// Database values of one instance, plus related instances loaded with `select_related`/`prefetch_related`.
#[derive(Clone, Debug, Default)]
pub struct RowData {
    /// Concrete field values keyed by attname (`question_id` for a foreign key).
    pub fields: IndexMap<String, Value>,
    /// `select_related` instances keyed by relation name.
    pub related: IndexMap<String, RowData>,
    /// `prefetch_related` results keyed by lookup (`choice_set`, `authors`).
    pub prefetched: IndexMap<String, Vec<RowData>>,
    /// Annotation values.
    pub annotations: IndexMap<String, Value>,
}

/// Cache of related objects attached to an instance (the `_state.fields_cache` /
/// `_prefetched_objects_cache`). Derived models keep one in a `#[model(state)]` field.
#[derive(Clone, Debug, Default)]
pub struct ModelState {
    pub adding: bool,
    pub db: Option<String>,
    pub data: RowData,
}

/// `ProtectedError`.
#[derive(Debug, thiserror::Error)]
#[error("Cannot delete some instances of model '{model}' because they are referenced through protected foreign keys: {refs}")]
pub struct ProtectedError {
    pub model: String,
    pub refs: String,
}

/// `RestrictedError`.
#[derive(Debug, thiserror::Error)]
#[error("Cannot delete some instances of model '{model}' because they are referenced through restricted foreign keys: {refs}")]
pub struct RestrictedError {
    pub model: String,
    pub refs: String,
}

/// A database-backed model.
#[async_trait]
pub trait Model: Sized + Send + Sync + 'static {
    /// The generated typed field handles (`QuestionFields`), used by `filter!`.
    type Fields: crate::models::typed::FieldsRoot;

    /// `app_label.ModelName`.
    fn label() -> String;

    /// Build an instance from database values.
    fn from_data(data: RowData) -> Result<Self>;

    /// Concrete field values keyed by attname.
    fn to_data(&self) -> IndexMap<String, Value>;

    fn pk_value(&self) -> Value;
    fn set_pk(&mut self, v: Value);

    /// Hook run before INSERT/UPDATE (assigns `auto_now`/`auto_now_add`, defaults).
    fn pre_save(&mut self, _created: bool) {}

    /// `Model.objects`.
    fn objects() -> QuerySet<Self> {
        QuerySet::new()
    }

    fn meta() -> Result<Arc<ModelMeta>> {
        registry().get_or_err(&Self::label()).map(Arc::clone)
    }

    /// `save()`: INSERT when the primary key is unset (or no row is updated), else UPDATE.
    async fn save(&mut self, db: &dyn Executor) -> Result<()> {
        self.save_with(db, false, None).await
    }

    /// `save(force_insert=..., update_fields=...)`.
    async fn save_with(&mut self, db: &dyn Executor, force_insert: bool, update_fields: Option<&[&str]>) -> Result<()> {
        let meta = Self::meta()?;
        let vendor = db.vendor();
        let pk_unset = self.pk_value().is_null();
        let created_hint = pk_unset || force_insert;
        self.pre_save(created_hint);
        let data = self.to_data();
        let label = Self::label();
        let snapshot = |created: bool, pk: Value| ModelEvent {
            label: label.clone(),
            pk,
            created,
            update_fields: update_fields.map(|u| u.iter().map(|s| (*s).to_owned()).collect()),
            values: data.clone(),
            using: db.alias().to_owned(),
        };
        PRE_SAVE.send(Some(&label), snapshot(created_hint, self.pk_value())).await;
        let pk_field = meta.pk().clone();
        let mut created = false;
        let mut updated = false;
        if !pk_unset && !force_insert {
            let fields: Vec<_> = match update_fields {
                Some(names) => {
                    let mut v = Vec::new();
                    for n in names {
                        let f = meta.get_field(n).filter(|f| f.concrete() && !f.primary_key).ok_or_else(|| Error::Other(format!("The following fields do not exist in this model, are m2m fields, or are non-concrete fields: {n}").into()))?;
                        v.push(f.clone());
                    }
                    v
                }
                None => meta.concrete_fields().filter(|f| !f.primary_key).cloned().collect(),
            };
            if fields.is_empty() {
                updated = true;
            } else {
                let sets: Vec<String> = fields.iter().map(|f| format!("{} = %s", vendor.quote(&f.column()))).collect();
                let mut params: Vec<Value> = fields.iter().map(|f| crate::models::fields_db::prep_param(vendor, f, data.get(&f.attname()).cloned().unwrap_or(Value::Null))).collect();
                params.push(crate::models::fields_db::prep_param(vendor, &pk_field, self.pk_value()));
                let sql = format!("UPDATE {} SET {} WHERE {} = %s", vendor.quote(&meta.db_table), sets.join(", "), vendor.quote(&pk_field.column()));
                updated = db.execute(&sql, &params).await? > 0;
            }
            if !updated && update_fields.is_some() {
                return Err(Error::Other("Save with update_fields did not affect any rows.".into()));
            }
        }
        if !updated {
            created = true;
            let fields: Vec<_> = meta.concrete_fields().filter(|f| !(f.primary_key && f.ty.is_auto() && pk_unset)).cloned().collect();
            let refs: Vec<&_> = fields.iter().collect();
            let params: Vec<Value> = fields.iter().map(|f| crate::models::fields_db::prep_param(vendor, f, data.get(&f.attname()).cloned().unwrap_or(Value::Null))).collect();
            let returning = pk_unset && pk_field.ty.is_auto();
            let sql = crate::models::sql::insert_sql(vendor, &meta, &refs, 1, returning);
            let id = db.insert(&sql, &params, returning).await?;
            if let (true, Some(id)) = (returning, id) {
                self.set_pk(Value::Int(id));
            }
        }
        POST_SAVE.send(Some(&label), snapshot(created, self.pk_value())).await;
        Ok(())
    }

    /// `delete()`: cascades according to the relations' `on_delete`. Returns `(total, per model)`.
    async fn delete(&self, db: &dyn Executor) -> Result<(usize, IndexMap<String, usize>)> {
        let meta = Self::meta()?;
        let pk = self.pk_value();
        if pk.is_null() {
            return Err(Error::Other(format!("{} object can't be deleted because its {} attribute is set to None.", meta.object_name, meta.pk().name).into()));
        }
        crate::models::deletion::delete_pks(db, &meta.label(), vec![pk]).await
    }

    /// `refresh_from_db()`: reload all concrete fields.
    async fn refresh_from_db(&mut self, db: &dyn Executor) -> Result<()> {
        let pk = self.pk_value();
        let fresh = QuerySet::<Self>::new().filter_path("pk", pk)?.get(db).await?;
        *self = fresh;
        Ok(())
    }
}

/// A model with no static type: instances are plain [`RowData`] (fixtures, migrations, tests).
#[derive(Clone, Debug)]
pub struct Dynamic {
    pub label: String,
    pub data: RowData,
}

#[derive(Default)]
pub struct NoFields;

#[async_trait]
impl Model for Dynamic {
    type Fields = NoFields;
    fn label() -> String {
        String::new()
    }
    fn from_data(data: RowData) -> Result<Self> {
        Ok(Dynamic { label: String::new(), data })
    }
    fn to_data(&self) -> IndexMap<String, Value> {
        self.data.fields.clone()
    }
    fn pk_value(&self) -> Value {
        self.data.fields.get("id").cloned().unwrap_or(Value::Null)
    }
    fn set_pk(&mut self, v: Value) {
        self.data.fields.insert("id".into(), v);
    }
}

/// Send `post_delete`/`pre_delete` for instances being removed.
pub(crate) async fn send_delete_signals(label: &str, pks: &[Value], pre: bool, using: &str) {
    let sig = if pre { &PRE_DELETE } else { &POST_DELETE };
    if !sig.has_listeners(Some(label)) {
        return;
    }
    for pk in pks {
        sig.send(
            Some(label),
            ModelEvent { label: label.to_owned(), pk: pk.clone(), created: false, update_fields: None, values: IndexMap::new(), using: using.to_owned() },
        )
        .await;
    }
}
