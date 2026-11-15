//! Validation of input and representation of output.
//!
//! Data is `serde_json::Value` on both sides. Error messages are the.

use std::marker::PhantomData;
use std::sync::Arc;

use indexmap::IndexMap;
use rango_core::exceptions::Result;
use rango_db::models::base::RowData;
use rango_db::models::fields_db::prep_param;
use rango_db::models::meta::{FieldType, ModelMeta, registry};
use rango_db::models::{Model, related};
use rango_db::serializers::{from_json, to_json};
use rango_db::{Db, Executor, Value as DbValue};
use rust_decimal::Decimal;
use serde_json::{Map, Value as Json, json};

pub type Validator = Arc<dyn Fn(&Json) -> std::result::Result<(), String> + Send + Sync>;
type MField = rango_db::models::meta::Field;

#[derive(Clone)]
pub enum Kind {
    Boolean,
    /// `CharField` and its formats (`email`, `slug`, `uri`, `uuid`).
    Char { max_length: Option<usize>, min_length: Option<usize>, allow_blank: bool, trim: bool, format: Option<&'static str> },
    Integer { min: Option<i64>, max: Option<i64> },
    Float { min: Option<f64>, max: Option<f64> },
    Decimal { max_digits: Option<u32>, decimal_places: Option<u32> },
    DateTime,
    Date,
    Time,
    Duration,
    Choice(Vec<(Json, String)>),
    Json,
    List(Box<Field>),
    /// `PrimaryKeyRelatedField` to the model with this label.
    Related { model: String, many: bool },
    /// `SerializerMethodField`: computed from the instance's data, read-only.
    Method(Arc<dyn Fn(&Map<String, Json>) -> Json + Send + Sync>),
    /// A nested serializer (`many` for lists).
    Nested(Arc<Serializer>, bool),
}

/// A serializer field with the core arguments.
#[derive(Clone)]
pub struct Field {
    pub kind: Kind,
    pub read_only: bool,
    pub write_only: bool,
    /// `None` = required unless there is a default or the field is read-only.
    pub required: Option<bool>,
    pub default: Option<Json>,
    pub allow_null: bool,
    /// Key in the instance data (defaults to the field name).
    pub source: Option<String>,
    pub label: Option<String>,
    pub help_text: Option<String>,
    pub validators: Vec<Validator>,
    /// `UniqueValidator` message (checked against the database by [`ModelSerializer`]).
    pub unique: Option<String>,
}

fn pytype(j: &Json) -> &'static str {
    match j {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(n) if n.is_f64() => "float",
        Json::Number(_) => "int",
        Json::String(_) => "str",
        Json::Array(_) => "list",
        Json::Object(_) => "dict",
    }
}

fn text(j: &Json) -> Option<String> {
    match j {
        Json::String(s) => Some(s.clone()),
        Json::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

impl Field {
    pub fn new(kind: Kind) -> Self {
        Field { kind, read_only: false, write_only: false, required: None, default: None, allow_null: false, source: None, label: None, help_text: None, validators: vec![], unique: None }
    }
    pub fn boolean() -> Self {
        Self::new(Kind::Boolean)
    }
    pub fn char() -> Self {
        Self::new(Kind::Char { max_length: None, min_length: None, allow_blank: false, trim: true, format: None })
    }
    fn formatted(format: &'static str) -> Self {
        Self::new(Kind::Char { max_length: None, min_length: None, allow_blank: false, trim: true, format: Some(format) })
    }
    pub fn email() -> Self {
        Self::formatted("email")
    }
    pub fn slug() -> Self {
        Self::formatted("slug")
    }
    pub fn url() -> Self {
        Self::formatted("uri")
    }
    pub fn uuid() -> Self {
        Self::formatted("uuid")
    }
    pub fn integer() -> Self {
        Self::new(Kind::Integer { min: None, max: None })
    }
    pub fn float() -> Self {
        Self::new(Kind::Float { min: None, max: None })
    }
    pub fn decimal(max_digits: Option<u32>, decimal_places: Option<u32>) -> Self {
        Self::new(Kind::Decimal { max_digits, decimal_places })
    }
    pub fn datetime() -> Self {
        Self::new(Kind::DateTime)
    }
    pub fn date() -> Self {
        Self::new(Kind::Date)
    }
    pub fn time() -> Self {
        Self::new(Kind::Time)
    }
    pub fn duration() -> Self {
        Self::new(Kind::Duration)
    }
    pub fn choice<I: IntoIterator<Item = (Json, String)>>(choices: I) -> Self {
        Self::new(Kind::Choice(choices.into_iter().collect()))
    }
    pub fn json() -> Self {
        Self::new(Kind::Json)
    }
    pub fn list(child: Field) -> Self {
        Self::new(Kind::List(Box::new(child)))
    }
    pub fn related(model_label: &str, many: bool) -> Self {
        Self::new(Kind::Related { model: model_label.to_owned(), many })
    }
    /// `SerializerMethodField`.
    pub fn method(f: impl Fn(&Map<String, Json>) -> Json + Send + Sync + 'static) -> Self {
        let mut x = Self::new(Kind::Method(Arc::new(f)));
        x.read_only = true;
        x
    }
    pub fn nested(serializer: Serializer, many: bool) -> Self {
        Self::new(Kind::Nested(Arc::new(serializer), many))
    }

    pub fn read_only(mut self, v: bool) -> Self {
        self.read_only = v;
        self
    }
    pub fn write_only(mut self, v: bool) -> Self {
        self.write_only = v;
        self
    }
    pub fn required(mut self, v: bool) -> Self {
        self.required = Some(v);
        self
    }
    pub fn default(mut self, v: Json) -> Self {
        self.default = Some(v);
        self
    }
    pub fn allow_null(mut self, v: bool) -> Self {
        self.allow_null = v;
        self
    }
    pub fn allow_blank(mut self, v: bool) -> Self {
        if let Kind::Char { allow_blank, .. } = &mut self.kind {
            *allow_blank = v;
        }
        self
    }
    pub fn max_length(mut self, n: usize) -> Self {
        if let Kind::Char { max_length, .. } = &mut self.kind {
            *max_length = Some(n);
        }
        self
    }
    pub fn min_length(mut self, n: usize) -> Self {
        if let Kind::Char { min_length, .. } = &mut self.kind {
            *min_length = Some(n);
        }
        self
    }
    pub fn min_value(mut self, n: i64) -> Self {
        match &mut self.kind {
            Kind::Integer { min, .. } => *min = Some(n),
            Kind::Float { min, .. } => *min = Some(n as f64),
            _ => {}
        }
        self
    }
    pub fn max_value(mut self, n: i64) -> Self {
        match &mut self.kind {
            Kind::Integer { max, .. } => *max = Some(n),
            Kind::Float { max, .. } => *max = Some(n as f64),
            _ => {}
        }
        self
    }
    pub fn source(mut self, s: &str) -> Self {
        self.source = Some(s.to_owned());
        self
    }
    pub fn label(mut self, s: &str) -> Self {
        self.label = Some(s.to_owned());
        self
    }
    pub fn help_text(mut self, s: &str) -> Self {
        self.help_text = Some(s.to_owned());
        self
    }
    pub fn validator(mut self, f: impl Fn(&Json) -> std::result::Result<(), String> + Send + Sync + 'static) -> Self {
        self.validators.push(Arc::new(f));
        self
    }

    pub fn is_required(&self) -> bool {
        self.required.unwrap_or(self.default.is_none() && !self.read_only)
    }

    /// `to_internal_value(data)`: the validated value or the error messages.
    pub fn to_internal_value(&self, data: &Json) -> std::result::Result<Json, Vec<String>> {
        let one = |m: &str| Err(vec![m.to_owned()]);
        match &self.kind {
            Kind::Boolean => match data {
                Json::Bool(b) => Ok(json!(b)),
                Json::Number(n) if n.as_i64() == Some(1) => Ok(json!(true)),
                Json::Number(n) if n.as_i64() == Some(0) => Ok(json!(false)),
                Json::String(s) if ["t", "T", "y", "Y", "yes", "Yes", "YES", "true", "True", "TRUE", "on", "On", "ON", "1"].contains(&s.as_str()) => Ok(json!(true)),
                Json::String(s) if ["f", "F", "n", "N", "no", "No", "NO", "false", "False", "FALSE", "off", "Off", "OFF", "0"].contains(&s.as_str()) => Ok(json!(false)),
                _ => one("Must be a valid boolean."),
            },
            Kind::Char { max_length, min_length, allow_blank, trim, format } => {
                let Some(s) = text(data) else { return one("Not a valid string.") };
                let s = if *trim { s.trim().to_owned() } else { s };
                if s.is_empty() {
                    return if *allow_blank { Ok(json!("")) } else { one("This field may not be blank.") };
                }
                let mut errs = Vec::new();
                let n = s.chars().count();
                if let Some(m) = max_length.filter(|m| n > *m) {
                    errs.push(format!("Ensure this field has no more than {m} characters."));
                }
                if let Some(m) = min_length.filter(|m| n < *m) {
                    errs.push(format!("Ensure this field has at least {m} characters."));
                }
                match format {
                    Some("email") if rango_core::validators::validate_email(&s).is_err() => errs.push("Enter a valid email address.".into()),
                    Some("slug") if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => errs.push("Enter a valid \"slug\" consisting of letters, numbers, underscores or hyphens.".into()),
                    Some("uri") if rango_core::validators::UrlValidator::new().is_ok_and(|v| rango_core::validators::Validator::validate(&v, s.as_str()).is_err()) => errs.push("Enter a valid URL.".into()),
                    Some("uuid") => match uuid::Uuid::parse_str(&s) {
                        Ok(u) => return Ok(json!(u.to_string())),
                        Err(_) => errs.push("Must be a valid UUID.".into()),
                    },
                    _ => {}
                }
                if errs.is_empty() { Ok(json!(s)) } else { Err(errs) }
            }
            Kind::Integer { min, max } => {
                let s = match data {
                    Json::Number(n) => n.to_string(),
                    Json::String(s) if s.len() <= 1000 => s.trim().to_owned(),
                    Json::String(_) => return one("String value too large."),
                    _ => return one("A valid integer is required."),
                };
                // `re.sub(r"\.0*\s*$", "", s)` then `int()`.
                let s = match s.find('.') {
                    Some(i) if s[i + 1..].chars().all(|c| c == '0') => s[..i].to_owned(),
                    _ => s,
                };
                let Ok(v) = s.parse::<i64>() else { return one("A valid integer is required.") };
                let mut errs = Vec::new();
                if let Some(m) = max.filter(|m| v > *m) {
                    errs.push(format!("Ensure this value is less than or equal to {m}."));
                }
                if let Some(m) = min.filter(|m| v < *m) {
                    errs.push(format!("Ensure this value is greater than or equal to {m}."));
                }
                if errs.is_empty() { Ok(json!(v)) } else { Err(errs) }
            }
            Kind::Float { min, max } => {
                let v = match data {
                    Json::Number(n) => n.as_f64(),
                    Json::String(s) => s.trim().parse::<f64>().ok(),
                    _ => None,
                };
                let Some(v) = v else { return one("A valid number is required.") };
                let mut errs = Vec::new();
                if let Some(m) = max.filter(|m| v > *m) {
                    errs.push(format!("Ensure this value is less than or equal to {m}."));
                }
                if let Some(m) = min.filter(|m| v < *m) {
                    errs.push(format!("Ensure this value is greater than or equal to {m}."));
                }
                if errs.is_empty() { Ok(json!(v)) } else { Err(errs) }
            }
            Kind::Decimal { max_digits, decimal_places } => {
                let Some(s) = text(data) else { return one("A valid number is required.") };
                let s = s.trim();
                if s.len() > 1000 {
                    return one("String value too large.");
                }
                let Ok(mut d) = s.parse::<Decimal>().or_else(|_| Decimal::from_scientific(s)) else { return one("A valid number is required.") };
                // validate_precision
                let digits = d.mantissa().unsigned_abs().to_string().len() as u32;
                let scale = d.scale();
                let (total, places, whole) = if digits > scale { (digits, scale, digits - scale) } else { (scale, scale, 0) };
                if let Some(m) = max_digits.filter(|m| total > *m) {
                    return Err(vec![format!("Ensure that there are no more than {m} digits in total.")]);
                }
                if let Some(p) = decimal_places.filter(|p| places > *p) {
                    return Err(vec![format!("Ensure that there are no more than {p} decimal places.")]);
                }
                if let (Some(m), Some(p)) = (max_digits, decimal_places) {
                    if whole > m - p {
                        return Err(vec![format!("Ensure that there are no more than {} digits before the decimal point.", m - p)]);
                    }
                }
                if let Some(p) = decimal_places {
                    d.rescale(*p);
                }
                Ok(json!(d.to_string()))
            }
            Kind::DateTime => match data.as_str().and_then(|s| rango_core::utils::dateparse::parse_datetime(s.trim()).ok().flatten()) {
                Some(p) => {
                    let utc = p.naive - chrono::TimeDelta::seconds(i64::from(p.offset.map_or(0, |o| o.local_minus_utc())));
                    Ok(json!(utc.and_utc().format("%Y-%m-%dT%H:%M:%S%.fZ").to_string()))
                }
                None => one("Datetime has wrong format. Use one of these formats instead: YYYY-MM-DDThh:mm[:ss[.uuuuuu]][+HH:MM|-HH:MM|Z]."),
            },
            Kind::Date => match data.as_str().and_then(|s| rango_core::utils::dateparse::parse_date(s.trim()).ok().flatten()) {
                Some(d) => Ok(json!(d.to_string())),
                None => one("Date has wrong format. Use one of these formats instead: YYYY-MM-DD."),
            },
            Kind::Time => match data.as_str().and_then(|s| rango_core::utils::dateparse::parse_time(s.trim()).ok().flatten()) {
                Some(t) => Ok(json!(t.format("%H:%M:%S%.f").to_string())),
                None => one("Time has wrong format. Use one of these formats instead: hh:mm[:ss[.uuuuuu]]."),
            },
            Kind::Duration => match text(data).and_then(|s| rango_core::utils::dateparse::parse_duration(&s)) {
                Some(d) => Ok(json!(rango_core::utils::duration::duration_string(d))),
                None => one("Duration has wrong format. Use one of these formats instead: [DD] [HH:[MM:]]ss[.uuuuuu]."),
            },
            Kind::Choice(choices) => {
                let as_text = |j: &Json| text(j).unwrap_or_else(|| j.to_string());
                match choices.iter().find(|(v, _)| v == data || as_text(v) == as_text(data)) {
                    Some((v, _)) => Ok(v.clone()),
                    None => Err(vec![format!("\"{}\" is not a valid choice.", as_text(data))]),
                }
            }
            Kind::Json => Ok(data.clone()),
            Kind::List(child) => {
                let Json::Array(items) = data else { return Err(vec![format!("Expected a list of items but got type \"{}\".", pytype(data))]) };
                let mut out = Vec::new();
                for i in items {
                    out.push(child.to_internal_value(i)?);
                }
                Ok(Json::Array(out))
            }
            Kind::Related { many, .. } => {
                let check = |j: &Json| -> std::result::Result<Json, Vec<String>> {
                    match j {
                        Json::Bool(_) | Json::Array(_) | Json::Object(_) => Err(vec![format!("Incorrect type. Expected pk value, received {}.", pytype(j))]),
                        other => Ok(other.clone()),
                    }
                };
                if *many {
                    let Json::Array(items) = data else { return Err(vec![format!("Expected a list of items but got type \"{}\".", pytype(data))]) };
                    items.iter().map(check).collect::<std::result::Result<Vec<_>, _>>().map(Json::Array)
                } else {
                    check(data)
                }
            }
            Kind::Method(_) => Ok(Json::Null),
            Kind::Nested(s, many) => {
                let run = |j: &Json| s.run_validation(j, false).map(Json::Object).map_err(|e| vec![e.to_string()]);
                if *many {
                    let Json::Array(items) = data else { return Err(vec![format!("Expected a list of items but got type \"{}\".", pytype(data))]) };
                    items.iter().map(run).collect::<std::result::Result<Vec<_>, _>>().map(Json::Array)
                } else {
                    run(data)
                }
            }
        }
    }
}

/// `Serializer`: named fields plus an optional object-level `validate`.
#[derive(Clone)]
pub struct Serializer {
    pub name: String,
    pub fields: IndexMap<String, Field>,
    validate: Option<Arc<dyn Fn(&mut Map<String, Json>) -> std::result::Result<(), Json> + Send + Sync>>,
}

impl Serializer {
    pub fn new(name: &str) -> Self {
        Serializer { name: name.to_owned(), fields: IndexMap::new(), validate: None }
    }
    pub fn field(mut self, name: &str, f: Field) -> Self {
        self.fields.insert(name.to_owned(), f);
        self
    }
    /// Object-level `validate(attrs)`; return errors as `{"field": ["msg"]}` or `{"non_field_errors": [...]}`.
    pub fn validate(mut self, f: impl Fn(&mut Map<String, Json>) -> std::result::Result<(), Json> + Send + Sync + 'static) -> Self {
        self.validate = Some(Arc::new(f));
        self
    }

    /// Validate one field of `data`: `Ok(None)` = not provided and not needed.
    pub fn field_value(field: &Field, name: &str, data: &Map<String, Json>, partial: bool) -> std::result::Result<Option<Json>, Vec<String>> {
        if field.read_only {
            return Ok(None);
        }
        let Some(raw) = data.get(name) else {
            if partial {
                return Ok(None);
            }
            if let Some(d) = &field.default {
                return Ok(Some(d.clone()));
            }
            return if field.is_required() { Err(vec!["This field is required.".into()]) } else { Ok(None) };
        };
        if raw.is_null() {
            return if field.allow_null { Ok(Some(Json::Null)) } else { Err(vec!["This field may not be null.".into()]) };
        }
        let v = field.to_internal_value(raw)?;
        let errs: Vec<String> = field.validators.iter().filter_map(|f| f(&v).err()).collect();
        if errs.is_empty() { Ok(Some(v)) } else { Err(errs) }
    }

    pub(crate) fn not_a_dict(data: &Json) -> Json {
        json!({ crate::settings::non_field_errors_key(): [format!("Invalid data. Expected a dictionary, but got {}.", pytype(data))] })
    }

    pub(crate) fn run_object_validation(&self, attrs: &mut Map<String, Json>) -> std::result::Result<(), Json> {
        match &self.validate {
            Some(f) => f(attrs),
            None => Ok(()),
        }
    }

    /// `run_validation(data)` (`is_valid`): validated data, or the errors.
    pub fn run_validation(&self, data: &Json, partial: bool) -> std::result::Result<Map<String, Json>, Json> {
        let Json::Object(obj) = data else { return Err(Self::not_a_dict(data)) };
        let (mut out, mut errors) = (Map::new(), Map::new());
        for (name, f) in &self.fields {
            match Self::field_value(f, name, obj, partial) {
                Ok(Some(v)) => {
                    out.insert(f.source.clone().unwrap_or_else(|| name.clone()), v);
                }
                Ok(None) => {}
                Err(e) => {
                    errors.insert(name.clone(), json!(e));
                }
            }
        }
        if !errors.is_empty() {
            return Err(Json::Object(errors));
        }
        self.run_object_validation(&mut out)?;
        Ok(out)
    }

    /// `to_representation(instance)` where `instance` maps source names to values.
    pub fn to_representation(&self, instance: &Map<String, Json>) -> Json {
        let mut out = Map::new();
        for (name, f) in self.fields.iter().filter(|(_, f)| !f.write_only) {
            let raw = instance.get(f.source.as_deref().unwrap_or(name)).cloned().unwrap_or(Json::Null);
            let v = match &f.kind {
                Kind::Method(m) => m(instance),
                Kind::Nested(s, true) => Json::Array(raw.as_array().into_iter().flatten().filter_map(|x| x.as_object()).map(|o| s.to_representation(o)).collect()),
                Kind::Nested(s, false) => raw.as_object().map_or(Json::Null, |o| s.to_representation(o)),
                _ => raw,
            };
            out.insert(name.clone(), v);
        }
        Json::Object(out)
    }
}

fn capfirst(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

fn int_range(ty: FieldType) -> Option<(i64, i64)> {
    Some(match ty {
        FieldType::SmallInteger => (-32768, 32767),
        FieldType::Integer => (-2147483648, 2147483647),
        FieldType::BigInteger => (i64::MIN, i64::MAX),
        FieldType::PositiveSmallInteger => (0, 32767),
        FieldType::PositiveInteger => (0, 2147483647),
        FieldType::PositiveBigInteger => (0, i64::MAX),
        _ => return None,
    })
}

/// The serializer field the `ModelSerializer` builds for a model field.
pub fn field_for_model_field(meta: &ModelMeta, f: &MField) -> Result<Field> {
    let verbose = f.verbose_name.clone().unwrap_or_else(|| f.name.replace('_', " "));
    let mut out = match f.ty {
        FieldType::ForeignKey | FieldType::OneToOne | FieldType::ManyToMany => Field::related(&registry().target_of(meta, f)?.label(), f.ty == FieldType::ManyToMany),
        _ if !f.choices.is_empty() => Field::choice(f.choices.iter().map(|(v, l)| (to_json(f, v), l.clone()))),
        FieldType::Auto | FieldType::BigAuto | FieldType::SmallAuto => Field::integer(),
        FieldType::Boolean => Field::boolean(),
        FieldType::Char | FieldType::Text | FieldType::File | FieldType::Image | FieldType::GenericIpAddress => Field::char(),
        FieldType::Email => Field::email(),
        FieldType::Slug => Field::slug(),
        FieldType::Url => Field::url(),
        FieldType::Uuid => Field::uuid(),
        FieldType::Float => Field::float(),
        FieldType::Decimal => Field::decimal(f.max_digits, f.decimal_places),
        FieldType::DateTime => Field::datetime(),
        FieldType::Date => Field::date(),
        FieldType::Time => Field::time(),
        FieldType::Duration => Field::duration(),
        FieldType::Json => Field::json(),
        FieldType::Binary => Field::char().read_only(true),
        _ => Field::integer(),
    };
    if let (Kind::Integer { min, max }, Some((lo, hi))) = (&mut out.kind, int_range(f.ty)) {
        (*min, *max) = (Some(lo), Some(hi));
    }
    if let Kind::Char { max_length, allow_blank, .. } = &mut out.kind {
        *max_length = f.max_length.map(|n| n as usize);
        *allow_blank = f.blank;
    }
    out.read_only = f.primary_key && f.ty.is_auto() || !f.editable || f.auto_now || f.auto_now_add || out.read_only;
    out.allow_null = f.null;
    if f.default.is_some() || f.blank || f.null {
        out.required = Some(false);
    }
    out.label = Some(if f.primary_key && f.name == "id" { "ID".to_owned() } else { capfirst(&verbose) });
    if !f.help_text.is_empty() {
        out.help_text = Some(f.help_text.clone());
    }
    if f.unique && !f.primary_key {
        out.unique = Some(format!("{} with this {} already exists.", meta.verbose_name, verbose));
    }
    Ok(out)
}

/// `ModelSerializer`: fields derived from the model, `create`/`update` included.
pub struct ModelSerializer<M: Model> {
    pub serializer: Serializer,
    meta: Arc<ModelMeta>,
    _m: PhantomData<fn() -> M>,
}

impl<M: Model> Clone for ModelSerializer<M> {
    fn clone(&self) -> Self {
        Self { serializer: self.serializer.clone(), meta: Arc::clone(&self.meta), _m: PhantomData }
    }
}

impl<M: Model> ModelSerializer<M> {
    /// `fields = "__all__"`.
    pub fn all() -> Result<Self> {
        let meta = M::meta()?;
        let mut s = Serializer::new(&meta.object_name);
        for f in meta.fields.iter().filter(|f| f.concrete() || f.ty == FieldType::ManyToMany) {
            s.fields.insert(f.name.clone(), field_for_model_field(&meta, f)?);
        }
        Ok(Self { serializer: s, meta, _m: PhantomData })
    }

    /// `fields = [...]` (in this order).
    pub fn fields(names: &[&str]) -> Result<Self> {
        let mut all = Self::all()?;
        let mut picked = IndexMap::new();
        for n in names {
            let f = all.serializer.fields.shift_remove(*n).ok_or_else(|| rango_core::Error::improperly_configured(format!("Field name `{n}` is not valid for model `{}`.", all.meta.object_name)))?;
            picked.insert((*n).to_owned(), f);
        }
        all.serializer.fields = picked;
        Ok(all)
    }

    /// `exclude = [...]`.
    pub fn exclude(mut self, names: &[&str]) -> Self {
        self.serializer.fields.retain(|k, _| !names.contains(&k.as_str()));
        self
    }

    /// `read_only_fields = [...]`.
    pub fn read_only_fields(mut self, names: &[&str]) -> Self {
        for (k, f) in &mut self.serializer.fields {
            if names.contains(&k.as_str()) {
                f.read_only = true;
            }
        }
        self
    }

    /// Declare or replace a field (`extra_kwargs`, explicit serializer fields).
    pub fn field(mut self, name: &str, f: Field) -> Self {
        self.serializer.fields.insert(name.to_owned(), f);
        self
    }

    /// Object-level `validate`.
    pub fn validate(mut self, f: impl Fn(&mut Map<String, Json>) -> std::result::Result<(), Json> + Send + Sync + 'static) -> Self {
        self.serializer = self.serializer.validate(f);
        self
    }

    /// The component name in the OpenAPI document.
    pub fn name(mut self, name: &str) -> Self {
        self.serializer.name = name.to_owned();
        self
    }

    /// The instance as source-name -> JSON (many-to-many ids are read from the database).
    pub async fn instance_data(&self, db: &dyn Executor, obj: &M) -> Result<Map<String, Json>> {
        let data = obj.to_data();
        let pk = obj.pk_value();
        let mut out = Map::new();
        for f in &self.meta.fields {
            if f.concrete() {
                out.insert(f.name.clone(), to_json(f, data.get(&f.attname()).unwrap_or(&DbValue::Null)));
            } else if f.ty == FieldType::ManyToMany && self.serializer.fields.contains_key(&f.name) && !pk.is_null() {
                let target_pk = registry().target_of(&self.meta, f)?.pk().clone();
                let ids = related::current_ids(db, &M::label(), &f.name, &pk).await?;
                out.insert(f.name.clone(), Json::Array(ids.iter().map(|v| to_json(&target_pk, v)).collect()));
            }
        }
        Ok(out)
    }

    /// `serializer.data` for one instance.
    pub async fn represent(&self, db: &dyn Executor, obj: &M) -> Result<Json> {
        Ok(self.serializer.to_representation(&self.instance_data(db, obj).await?))
    }

    /// `is_valid()`: field validation, uniqueness and related-object existence; returns validated data keyed by field name.
    pub async fn validate_data(&self, db: &dyn Executor, data: &Json, instance: Option<&M>, partial: bool) -> Result<std::result::Result<Map<String, Json>, Json>> {
        let Json::Object(obj) = data else { return Ok(Err(Serializer::not_a_dict(data))) };
        let vendor = db.vendor();
        let (mut out, mut errors) = (Map::new(), Map::new());
        for (name, field) in &self.serializer.fields {
            let v = match Serializer::field_value(field, name, obj, partial) {
                Ok(Some(v)) => v,
                Ok(None) => continue,
                Err(e) => {
                    errors.insert(name.clone(), json!(e));
                    continue;
                }
            };
            let mf = self.meta.get_field(field.source.as_deref().unwrap_or(name));
            let mut errs: Vec<String> = Vec::new();
            if let (Some(msg), Some(mf), false) = (&field.unique, mf, v.is_null()) {
                let mut sql = format!("SELECT 1 FROM {} WHERE {} = %s", vendor.quote(&self.meta.db_table), vendor.quote(&mf.column()));
                let mut params = vec![prep_param(vendor, mf, from_json(mf, &v)?)];
                if let Some(i) = instance {
                    sql.push_str(&format!(" AND {} <> %s", vendor.quote(&self.meta.pk().column())));
                    params.push(prep_param(vendor, self.meta.pk(), i.pk_value()));
                }
                if !db.query(&format!("{sql} LIMIT 1"), &params).await?.is_empty() {
                    errs.push(msg.clone());
                }
            }
            if let (Kind::Related { model, .. }, false) = (&field.kind, v.is_null()) {
                let reg = registry();
                let target = reg.get_or_err(model)?;
                let pks: Vec<&Json> = match &v {
                    Json::Array(a) => a.iter().collect(),
                    one => vec![one],
                };
                for pk in pks {
                    let found = match from_json(target.pk(), pk) {
                        Ok(val) => !db.query(&format!("SELECT 1 FROM {} WHERE {} = %s LIMIT 1", vendor.quote(&target.db_table), vendor.quote(&target.pk().column())), &[prep_param(vendor, target.pk(), val)]).await.map(|r| r.is_empty()).unwrap_or(true),
                        Err(_) => false,
                    };
                    if !found {
                        errs.push(format!("Invalid pk \"{}\" - object does not exist.", text(pk).unwrap_or_else(|| pk.to_string())));
                    }
                }
            }
            if errs.is_empty() {
                out.insert(name.clone(), v);
            } else {
                errors.insert(name.clone(), json!(errs));
            }
        }
        if !errors.is_empty() {
            return Ok(Err(Json::Object(errors)));
        }
        if let Err(e) = self.serializer.run_object_validation(&mut out) {
            return Ok(Err(e));
        }
        Ok(Ok(out))
    }

    async fn apply(&self, db: &dyn Executor, mut data: IndexMap<String, DbValue>, validated: &Map<String, Json>) -> Result<M> {
        let mut m2m: Vec<(&MField, Vec<DbValue>)> = Vec::new();
        for (name, v) in validated {
            let source = self.serializer.fields.get(name).and_then(|f| f.source.clone()).unwrap_or_else(|| name.clone());
            let Some(mf) = self.meta.get_field(&source) else { continue };
            if mf.ty == FieldType::ManyToMany {
                let target_pk = registry().target_of(&self.meta, mf)?.pk().clone();
                m2m.push((mf, v.as_array().into_iter().flatten().map(|x| from_json(&target_pk, x)).collect::<Result<_>>()?));
            } else if mf.concrete() {
                data.insert(mf.attname(), from_json(mf, v)?);
            }
        }
        let mut obj = M::from_data(RowData { fields: data, ..RowData::default() })?;
        obj.save(db).await?;
        for (mf, ids) in m2m {
            related::set(db, &M::label(), &mf.name, &obj.pk_value(), &ids).await?;
        }
        Ok(obj)
    }

    /// `serializer.save()` on an existing instance (`update`).
    pub async fn update(&self, db: &dyn Executor, instance: &M, validated: &Map<String, Json>) -> Result<M> {
        self.apply(db, instance.to_data(), validated).await
    }
}

impl<M: Model + Default> ModelSerializer<M> {
    /// `serializer.save()` without an instance (`create`).
    pub async fn create(&self, db: &dyn Executor, validated: &Map<String, Json>) -> Result<M> {
        let blank = rango_forms::models::blank_instance::<M>(&self.meta)?;
        self.apply(db, blank.to_data(), validated).await
    }
}

#[allow(dead_code)]
fn _db(_: &Db) {}
