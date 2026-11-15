use std::sync::Arc;

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_db::models::meta::ModelMeta;
use rango_db::models::Model;
use rango_db::Value as Db;
use rango_forms::Data;
use rango_http::HttpRequest;
use rango_template::{Object, TemplateObject, Value as T};
use rango_urls::{Arg, ViewArgs};

/// A database value as a template value (datetimes are shown in the current time zone).
pub fn db_to_template(v: &Db) -> T {
    match v {
        Db::Null => T::None,
        Db::Bool(b) => T::Bool(*b),
        Db::Int(i) => T::Int(*i),
        Db::Float(f) => T::Float(*f),
        Db::Decimal(d) => T::Decimal(*d),
        Db::Str(s) => T::Str(s.clone()),
        Db::Bytes(b) => T::Str(String::from_utf8_lossy(b).into_owned()),
        Db::Date(d) => T::Date(*d),
        Db::Time(t) => T::Time(*t),
        Db::DateTime(d) => match rango_core::utils::timezone::get_current_timezone() {
            Ok(tz) => {
                let local = d.with_timezone(&tz);
                T::DateTime(local.naive_local(), Some(chrono::Offset::fix(local.offset()).local_minus_utc()))
            }
            Err(_) => T::DateTime(d.naive_utc(), Some(0)),
        },
        Db::NaiveDateTime(d) => T::DateTime(*d, None),
        Db::Duration(d) => T::Str(rango_core::utils::duration::duration_string(*d)),
        Db::Uuid(u) => T::Str(u.to_string()),
        Db::Json(j) => T::from(j.clone()),
    }
}

struct ModelObject {
    meta: Arc<ModelMeta>,
    data: IndexMap<String, Db>,
    pk: Db,
}

impl TemplateObject for ModelObject {
    fn get(&self, key: &str) -> Option<T> {
        if key == "pk" {
            return Some(db_to_template(&self.pk));
        }
        let f = self.meta.fields.iter().find(|f| f.name == key || f.attname() == key)?;
        self.data.get(&f.attname()).map(db_to_template)
    }
    fn display(&self) -> T {
        let from_field = self.meta.str_field.as_ref().and_then(|n| self.meta.get_field(n)).and_then(|f| self.data.get(&f.attname())).map(|v| db_to_template(v).to_str());
        T::Str(from_field.unwrap_or_else(|| format!("{} object ({})", self.meta.object_name, db_to_template(&self.pk).to_str())))
    }
}

/// An instance as a template value: `{{ obj.field }}`, `{{ obj.pk }}`, `{{ obj }}` (its `str` field).
pub fn model_value<M: Model>(m: &M) -> T {
    match M::meta() {
        Ok(meta) => Object::new(ModelObject { meta, data: m.to_data(), pk: m.pk_value() }),
        Err(_) => T::None,
    }
}

pub fn arg_to_db(a: &Arg) -> Db {
    match a {
        Arg::Int(i) => Db::Int(*i),
        Arg::Str(s) => Db::Str(s.clone()),
        Arg::Uuid(u) => Db::Uuid(*u),
    }
}

pub fn arg_to_template(a: &Arg) -> T {
    match a {
        Arg::Int(i) => T::Int(*i),
        Arg::Str(s) => T::Str(s.clone()),
        Arg::Uuid(u) => T::Str(u.to_string()),
    }
}

/// URL kwargs as template variables (the reference implementation passes them to `get_context_data`).
pub fn url_kwargs_context(args: &ViewArgs) -> impl Iterator<Item = (String, T)> + '_ {
    args.kwargs.iter().map(|(k, v)| (k.clone(), arg_to_template(v)))
}

/// `request.POST` as form data.
pub async fn post_data(req: &HttpRequest) -> Result<Data> {
    Ok(Data::from_lists(req.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec()))))
}

pub async fn file_data(req: &HttpRequest) -> Result<rango_forms::FileData> {
    Ok(req.files().await?.clone())
}

pub fn model_name_lower(meta: &ModelMeta) -> String {
    meta.object_name.to_lowercase()
}

pub fn not_found(msg: impl Into<String>) -> Error {
    Error::Http404(msg.into())
}
