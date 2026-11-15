//! Model metadata: fields, relations, `ModelMeta` (the `Options`) and the model registry.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::value::Value;

/// the concrete field classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldType {
    Auto,
    BigAuto,
    SmallAuto,
    Boolean,
    Char,
    Text,
    Integer,
    BigInteger,
    SmallInteger,
    PositiveInteger,
    PositiveBigInteger,
    PositiveSmallInteger,
    Float,
    Decimal,
    Date,
    DateTime,
    Time,
    Duration,
    Email,
    Url,
    Slug,
    Uuid,
    Binary,
    Json,
    GenericIpAddress,
    File,
    Image,
    ForeignKey,
    OneToOne,
    ManyToMany,
}

impl FieldType {
    /// The class name as the reference implementation writes it in migrations (`models.CharField`).
    pub fn class_name(self) -> &'static str {
        match self {
            FieldType::Auto => "AutoField",
            FieldType::BigAuto => "BigAutoField",
            FieldType::SmallAuto => "SmallAutoField",
            FieldType::Boolean => "BooleanField",
            FieldType::Char => "CharField",
            FieldType::Text => "TextField",
            FieldType::Integer => "IntegerField",
            FieldType::BigInteger => "BigIntegerField",
            FieldType::SmallInteger => "SmallIntegerField",
            FieldType::PositiveInteger => "PositiveIntegerField",
            FieldType::PositiveBigInteger => "PositiveBigIntegerField",
            FieldType::PositiveSmallInteger => "PositiveSmallIntegerField",
            FieldType::Float => "FloatField",
            FieldType::Decimal => "DecimalField",
            FieldType::Date => "DateField",
            FieldType::DateTime => "DateTimeField",
            FieldType::Time => "TimeField",
            FieldType::Duration => "DurationField",
            FieldType::Email => "EmailField",
            FieldType::Url => "URLField",
            FieldType::Slug => "SlugField",
            FieldType::Uuid => "UUIDField",
            FieldType::Binary => "BinaryField",
            FieldType::Json => "JSONField",
            FieldType::GenericIpAddress => "GenericIPAddressField",
            FieldType::File => "FileField",
            FieldType::Image => "ImageField",
            FieldType::ForeignKey => "ForeignKey",
            FieldType::OneToOne => "OneToOneField",
            FieldType::ManyToMany => "ManyToManyField",
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, FieldType::Auto | FieldType::BigAuto | FieldType::SmallAuto)
    }

    pub fn is_relation(self) -> bool {
        matches!(self, FieldType::ForeignKey | FieldType::OneToOne | FieldType::ManyToMany)
    }

    pub fn is_string(self) -> bool {
        matches!(self, FieldType::Char | FieldType::Text | FieldType::Email | FieldType::Url | FieldType::Slug | FieldType::GenericIpAddress | FieldType::File | FieldType::Image)
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            FieldType::Auto | FieldType::BigAuto | FieldType::SmallAuto | FieldType::Integer | FieldType::BigInteger | FieldType::SmallInteger | FieldType::PositiveInteger | FieldType::PositiveBigInteger | FieldType::PositiveSmallInteger
        )
    }
}

/// `on_delete=` behaviours.
#[derive(Clone, Debug, PartialEq)]
pub enum OnDelete {
    Cascade,
    Protect,
    Restrict,
    SetNull,
    SetDefault,
    /// `SET(value)`.
    SetValue(Value),
    DoNothing,
}

/// A field default.
#[derive(Clone, Debug)]
pub enum Default_ {
    Value(Value),
    /// Evaluated per instance (`default=uuid.uuid4`, `timezone.now`).
    Callable(fn() -> Value),
}

impl PartialEq for Default_ {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Default_::Value(a), Default_::Value(b)) => a == b,
            (Default_::Callable(a), Default_::Callable(b)) => std::ptr::fn_addr_eq(*a, *b),
            _ => false,
        }
    }
}

/// Relation details of a ForeignKey/OneToOne/ManyToMany.
#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    /// `"app_label.ModelName"` or `"self"`.
    pub to: String,
    pub on_delete: OnDelete,
    pub related_name: Option<String>,
    pub related_query_name: Option<String>,
    /// Target field name (default: the target's primary key).
    pub to_field: Option<String>,
    /// Explicit through model label for ManyToMany.
    pub through: Option<String>,
    pub db_constraint: bool,
    pub symmetrical: Option<bool>,
    pub parent_link: bool,
    pub limit_choices_to: Option<Q2>,
}

/// Placeholder so `Relation` stays `PartialEq`/`Debug` (limit_choices_to is data for forms).
#[derive(Clone, Debug, PartialEq)]
pub struct Q2(pub String);

/// A model field (the `Field`).
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: FieldType,
    /// Column name (`db_column`), default derived from the name.
    pub db_column: Option<String>,
    pub null: bool,
    pub blank: bool,
    pub unique: bool,
    pub db_index: bool,
    pub primary_key: bool,
    pub editable: bool,
    pub default: Option<Default_>,
    pub max_length: Option<u32>,
    pub max_digits: Option<u32>,
    pub decimal_places: Option<u32>,
    pub auto_now: bool,
    pub auto_now_add: bool,
    pub choices: Vec<(Value, String)>,
    pub verbose_name: Option<String>,
    pub help_text: String,
    /// `FileField(upload_to=...)`: directory (under the storage root) uploads are saved to.
    pub upload_to: Option<String>,
    pub db_comment: Option<String>,
    pub db_default: Option<Value>,
    pub unique_for_date: Option<String>,
    pub rel: Option<Relation>,
    /// Validators by name (resolved by forms/validation layer).
    pub validators: Vec<String>,
}

impl Field {
    pub fn new(name: &str, ty: FieldType) -> Field {
        Field {
            name: name.to_owned(),
            ty,
            db_column: None,
            null: false,
            blank: false,
            unique: false,
            db_index: false,
            primary_key: false,
            editable: !ty.is_auto(),
            default: None,
            max_length: None,
            max_digits: None,
            decimal_places: None,
            auto_now: false,
            auto_now_add: false,
            choices: Vec::new(),
            verbose_name: None,
            help_text: String::new(),
            upload_to: None,
            db_comment: None,
            db_default: None,
            unique_for_date: None,
            rel: None,
            validators: Vec::new(),
        }
    }

    pub fn auto(name: &str) -> Field {
        Field { primary_key: true, ..Field::new(name, FieldType::Auto) }
    }
    pub fn big_auto(name: &str) -> Field {
        Field { primary_key: true, ..Field::new(name, FieldType::BigAuto) }
    }
    pub fn char(name: &str, max_length: u32) -> Field {
        Field { max_length: Some(max_length), ..Field::new(name, FieldType::Char) }
    }
    pub fn text(name: &str) -> Field {
        Field::new(name, FieldType::Text)
    }
    pub fn integer(name: &str) -> Field {
        Field::new(name, FieldType::Integer)
    }
    pub fn big_integer(name: &str) -> Field {
        Field::new(name, FieldType::BigInteger)
    }
    pub fn boolean(name: &str) -> Field {
        Field::new(name, FieldType::Boolean)
    }
    pub fn float(name: &str) -> Field {
        Field::new(name, FieldType::Float)
    }
    pub fn decimal(name: &str, max_digits: u32, decimal_places: u32) -> Field {
        Field { max_digits: Some(max_digits), decimal_places: Some(decimal_places), ..Field::new(name, FieldType::Decimal) }
    }
    pub fn date(name: &str) -> Field {
        Field::new(name, FieldType::Date)
    }
    pub fn datetime(name: &str) -> Field {
        Field::new(name, FieldType::DateTime)
    }
    pub fn time(name: &str) -> Field {
        Field::new(name, FieldType::Time)
    }
    pub fn uuid(name: &str) -> Field {
        Field::new(name, FieldType::Uuid)
    }
    pub fn json(name: &str) -> Field {
        Field::new(name, FieldType::Json)
    }
    pub fn slug(name: &str, max_length: u32) -> Field {
        Field { max_length: Some(max_length), db_index: true, ..Field::new(name, FieldType::Slug) }
    }
    pub fn email(name: &str) -> Field {
        Field { max_length: Some(254), ..Field::new(name, FieldType::Email) }
    }

    fn relation(name: &str, ty: FieldType, to: &str, on_delete: OnDelete) -> Field {
        Field {
            rel: Some(Relation {
                to: to.to_owned(),
                on_delete,
                related_name: None,
                related_query_name: None,
                to_field: None,
                through: None,
                db_constraint: true,
                symmetrical: None,
                parent_link: false,
                limit_choices_to: None,
            }),
            ..Field::new(name, ty)
        }
    }

    pub fn foreign_key(name: &str, to: &str, on_delete: OnDelete) -> Field {
        Field { db_index: true, ..Field::relation(name, FieldType::ForeignKey, to, on_delete) }
    }
    pub fn one_to_one(name: &str, to: &str, on_delete: OnDelete) -> Field {
        Field { unique: true, ..Field::relation(name, FieldType::OneToOne, to, on_delete) }
    }
    pub fn many_to_many(name: &str, to: &str) -> Field {
        Field::relation(name, FieldType::ManyToMany, to, OnDelete::Cascade)
    }

    // ---- builder-style modifiers
    pub fn null(mut self) -> Self {
        self.null = true;
        self
    }
    pub fn blank(mut self) -> Self {
        self.blank = true;
        self
    }
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }
    pub fn db_index(mut self) -> Self {
        self.db_index = true;
        self
    }
    pub fn primary_key(mut self) -> Self {
        self.primary_key = true;
        self
    }
    pub fn default(mut self, v: impl crate::value::ToValue) -> Self {
        self.default = Some(Default_::Value(v.to_value()));
        self
    }
    pub fn default_fn(mut self, f: fn() -> Value) -> Self {
        self.default = Some(Default_::Callable(f));
        self
    }
    pub fn auto_now(mut self) -> Self {
        self.auto_now = true;
        self.editable = false;
        self.blank = true;
        self
    }
    pub fn auto_now_add(mut self) -> Self {
        self.auto_now_add = true;
        self.editable = false;
        self.blank = true;
        self
    }
    pub fn db_column(mut self, c: &str) -> Self {
        self.db_column = Some(c.to_owned());
        self
    }
    pub fn verbose_name(mut self, v: &str) -> Self {
        self.verbose_name = Some(v.to_owned());
        self
    }
    pub fn upload_to(mut self, v: &str) -> Self {
        self.upload_to = Some(v.to_owned());
        self
    }
    pub fn help_text(mut self, v: &str) -> Self {
        self.help_text = v.to_owned();
        self
    }
    pub fn choices(mut self, c: Vec<(Value, String)>) -> Self {
        self.choices = c;
        self
    }
    pub fn related_name(mut self, n: &str) -> Self {
        if let Some(r) = &mut self.rel {
            r.related_name = Some(n.to_owned());
        }
        self
    }
    pub fn related_query_name(mut self, n: &str) -> Self {
        if let Some(r) = &mut self.rel {
            r.related_query_name = Some(n.to_owned());
        }
        self
    }
    pub fn to_field(mut self, n: &str) -> Self {
        if let Some(r) = &mut self.rel {
            r.to_field = Some(n.to_owned());
        }
        self
    }
    pub fn through(mut self, label: &str) -> Self {
        if let Some(r) = &mut self.rel {
            r.through = Some(label.to_owned());
        }
        self
    }
    pub fn db_constraint(mut self, v: bool) -> Self {
        if let Some(r) = &mut self.rel {
            r.db_constraint = v;
        }
        self
    }

    // ---- derived properties
    pub fn is_relation(&self) -> bool {
        self.ty.is_relation()
    }

    /// Apply several attribute changes (used by generated migrations).
    pub fn with(mut self, f: impl FnOnce(&mut Field)) -> Self {
        f(&mut self);
        self
    }
    pub fn many_to_many_(&self) -> bool {
        self.ty == FieldType::ManyToMany
    }
    /// Does the field have a column on its model's table?
    pub fn concrete(&self) -> bool {
        self.ty != FieldType::ManyToMany
    }
    /// `attname`: `question_id` for a foreign key named `question`.
    pub fn attname(&self) -> String {
        match self.ty {
            FieldType::ForeignKey | FieldType::OneToOne => format!("{}_id", self.name),
            _ => self.name.clone(),
        }
    }
    /// Column name.
    pub fn column(&self) -> String {
        self.db_column.clone().unwrap_or_else(|| self.attname())
    }
    pub fn verbose(&self) -> String {
        self.verbose_name.clone().unwrap_or_else(|| self.name.replace('_', " "))
    }
}

/// An index declared in `Meta.indexes`.
#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub name: Option<String>,
    /// Field names, `-name` for descending.
    pub fields: Vec<String>,
    pub condition: Option<Q2>,
    pub include: Vec<String>,
}

/// A constraint declared in `Meta.constraints`.
#[derive(Clone, Debug, PartialEq)]
pub enum Constraint {
    Unique { name: String, fields: Vec<String>, condition: Option<Q2> },
    Check { name: String, condition: Q2 },
}

/// the `Options` (`Model._meta`).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMeta {
    pub app_label: String,
    pub object_name: String,
    pub db_table: String,
    pub fields: Vec<Field>,
    pub ordering: Vec<String>,
    pub unique_together: Vec<Vec<String>>,
    pub index_together: Vec<Vec<String>>,
    pub indexes: Vec<Index>,
    pub constraints: Vec<Constraint>,
    pub verbose_name: String,
    pub verbose_name_plural: String,
    pub abstract_: bool,
    /// Label of the concrete model a proxy stands for.
    pub proxy_for: Option<String>,
    pub managed: bool,
    pub get_latest_by: Vec<String>,
    pub default_related_name: Option<String>,
    pub db_table_comment: Option<String>,
    /// Multi-table inheritance: label of the parent model.
    pub parent: Option<String>,
    pub select_on_save: bool,
    pub permissions: Vec<(String, String)>,
    /// `(file, line)` where the model is declared (keeps generated migrations in declaration order).
    pub source: Option<(String, u32)>,
    /// Field whose value is the instance's text (`__str__`), used for form choices and the admin.
    pub str_field: Option<String>,
}

impl ModelMeta {
    pub fn new(app_label: &str, object_name: &str) -> ModelMeta {
        let lower = object_name.to_lowercase();
        ModelMeta {
            app_label: app_label.to_owned(),
            object_name: object_name.to_owned(),
            db_table: format!("{app_label}_{lower}"),
            fields: Vec::new(),
            ordering: Vec::new(),
            unique_together: Vec::new(),
            index_together: Vec::new(),
            indexes: Vec::new(),
            constraints: Vec::new(),
            verbose_name: camel_to_spaces(object_name),
            verbose_name_plural: format!("{}s", camel_to_spaces(object_name)),
            abstract_: false,
            proxy_for: None,
            managed: true,
            get_latest_by: Vec::new(),
            default_related_name: None,
            db_table_comment: None,
            parent: None,
            select_on_save: false,
            permissions: Vec::new(),
            source: None,
            str_field: None,
        }
    }

    pub fn field(mut self, f: Field) -> Self {
        self.fields.push(f);
        self
    }
    pub fn db_table(mut self, t: &str) -> Self {
        self.db_table = t.to_owned();
        self
    }
    pub fn ordering(mut self, o: &[&str]) -> Self {
        self.ordering = o.iter().map(|s| (*s).to_owned()).collect();
        self
    }
    pub fn unique_together(mut self, f: &[&str]) -> Self {
        self.unique_together.push(f.iter().map(|s| (*s).to_owned()).collect());
        self
    }
    pub fn verbose_name(mut self, v: &str, plural: &str) -> Self {
        self.verbose_name = v.to_owned();
        self.verbose_name_plural = plural.to_owned();
        self
    }

    /// `label`: `app_label.ModelName`.
    pub fn label(&self) -> String {
        format!("{}.{}", self.app_label, self.object_name)
    }
    /// `label_lower`.
    pub fn label_lower(&self) -> String {
        format!("{}.{}", self.app_label, self.object_name.to_lowercase())
    }

    /// Fields with a column on this model's table.
    pub fn concrete_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| f.concrete())
    }

    pub fn many_to_many(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| f.many_to_many_())
    }

    /// Lookup by field name or attname (`question` / `question_id`).
    pub fn get_field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name || (f.concrete() && f.attname() == name))
    }

    pub fn pk(&self) -> &Field {
        self.fields.iter().find(|f| f.primary_key).expect("model has a primary key")
    }
}

fn camel_to_spaces(s: &str) -> String {
    rango_core::utils::text::camel_case_to_spaces(s)
}

/// Link-time registration of a model (what `#[derive(Model)]` emits).
pub struct ModelRegistration {
    pub build: fn() -> ModelMeta,
}

inventory::collect!(ModelRegistration);

/// A reverse relation (`_meta.related_objects`).
#[derive(Clone, Debug)]
pub struct RelatedObject {
    /// Label of the model that declares the relation field.
    pub model: String,
    pub field: String,
    pub ty: FieldType,
    /// Attribute on the target model (`choice_set` / `profile`).
    pub accessor_name: String,
    /// Name usable in lookups (`choice`).
    pub query_name: String,
}

/// All known models, with relations resolved.
#[derive(Default, Debug)]
pub struct Registry {
    models: IndexMap<String, Arc<ModelMeta>>,
    related: HashMap<String, Vec<RelatedObject>>,
}

static EXTRA: RwLock<Vec<ModelMeta>> = RwLock::new(Vec::new());
static GLOBAL: LazyLock<RwLock<Option<Arc<Registry>>>> = LazyLock::new(|| RwLock::new(None));

/// Register a model at runtime (tests, dynamic models). Must happen before the first [`registry`] call.
pub fn register_model(meta: ModelMeta) {
    EXTRA.write().expect("registry lock").push(meta);
}

/// The global registry (built from `inventory` + runtime registrations on first use).
pub fn registry() -> Arc<Registry> {
    if let Some(r) = GLOBAL.read().expect("registry lock").as_ref() {
        return Arc::clone(r);
    }
    let mut metas: Vec<ModelMeta> = inventory::iter::<ModelRegistration>.into_iter().map(|r| (r.build)()).collect();
    metas.extend(EXTRA.read().expect("registry lock").iter().cloned());
    let reg = Arc::new(Registry::new(metas).unwrap_or_else(|e| panic!("{e}")));
    *GLOBAL.write().expect("registry lock") = Some(Arc::clone(&reg));
    reg
}

/// Reset the global registry (tests).
#[doc(hidden)]
pub fn __reset_registry() {
    *GLOBAL.write().expect("registry lock") = None;
}

impl Registry {
    /// Build a registry: add implicit primary keys, auto-created M2M through models, reverse relations.
    pub fn new(metas: Vec<ModelMeta>) -> Result<Registry> {
        let mut reg = Registry::default();
        let mut all: Vec<ModelMeta> = Vec::new();
        for mut m in metas {
            if m.abstract_ {
                continue;
            }
            if !m.fields.iter().any(|f| f.primary_key) && m.proxy_for.is_none() {
                m.fields.insert(0, Field::auto("id"));
            }
            all.push(m);
        }
        // Auto-created through models for M2M fields without an explicit through.
        let mut throughs = Vec::new();
        for m in &all {
            for f in m.many_to_many() {
                let rel = f.rel.as_ref().expect("m2m has relation");
                if rel.through.is_some() {
                    continue;
                }
                let target_label = if rel.to == "self" { m.label() } else { rel.to.clone() };
                let (from_name, to_name) = if target_label.eq_ignore_ascii_case(&m.label()) {
                    (format!("from_{}", m.object_name.to_lowercase()), format!("to_{}", m.object_name.to_lowercase()))
                } else {
                    (m.object_name.to_lowercase(), target_label.rsplit('.').next().unwrap_or("").to_lowercase())
                };
                let through_name = format!("{}_{}", m.object_name, f.name);
                let mut t = ModelMeta::new(&m.app_label, &through_name);
                t.db_table = format!("{}_{}", m.db_table, f.name);
                t.fields = vec![
                    Field::auto("id"),
                    Field::foreign_key(&from_name, &m.label(), OnDelete::Cascade).db_index(),
                    Field::foreign_key(&to_name, &target_label, OnDelete::Cascade).db_index(),
                ];
                t.unique_together = vec![vec![from_name, to_name]];
                t.verbose_name = format!("{} {}", m.verbose_name, f.verbose());
                t.verbose_name_plural = t.verbose_name.clone();
                throughs.push(t);
            }
        }
        all.extend(throughs);
        for m in all {
            reg.models.insert(m.label_lower(), Arc::new(m));
        }
        // Reverse relations.
        let snapshot: Vec<Arc<ModelMeta>> = reg.models.values().cloned().collect();
        for m in &snapshot {
            for f in m.fields.iter().filter(|f| f.is_relation()) {
                let rel = f.rel.as_ref().expect("relation");
                let target = if rel.to == "self" { m.label() } else { rel.to.clone() };
                let key = target.to_lowercase();
                if !reg.models.contains_key(&key) {
                    return Err(Error::improperly_configured(format!(
                        "Field {}.{} references model '{}' which is not installed, or is abstract.",
                        m.label(),
                        f.name,
                        rel.to
                    )));
                }
                let lower = m.object_name.to_lowercase();
                let default_related = m.default_related_name.clone().map(|d| d.replace("%(model_name)s", &lower).replace("%(app_label)s", &m.app_label));
                let accessor = match (&rel.related_name, f.ty) {
                    (Some(n), _) => n.clone(),
                    (None, _) if default_related.is_some() => default_related.clone().expect("checked"),
                    (None, FieldType::OneToOne) => lower.clone(),
                    (None, _) => format!("{lower}_set"),
                };
                let query_name = rel.related_query_name.clone().or_else(|| rel.related_name.clone()).or(default_related).unwrap_or_else(|| lower.clone());
                if rel.related_name.as_deref().is_some_and(|n| n.ends_with('+')) {
                    continue;
                }
                reg.related.entry(key).or_default().push(RelatedObject {
                    model: m.label(),
                    field: f.name.clone(),
                    ty: f.ty,
                    accessor_name: accessor,
                    query_name,
                });
            }
        }
        Ok(reg)
    }

    pub fn models(&self) -> impl Iterator<Item = &Arc<ModelMeta>> {
        self.models.values()
    }

    /// `apps.get_model("app.Model")` (case-insensitive model name).
    pub fn get(&self, label: &str) -> Option<&Arc<ModelMeta>> {
        self.models.get(&label.to_lowercase())
    }

    pub fn get_or_err(&self, label: &str) -> Result<&Arc<ModelMeta>> {
        self.get(label).ok_or_else(|| Error::LookupError(format!("No installed app with label '{}'.", label)))
    }

    /// Resolve a relation's target (`"self"` is relative to `from`).
    pub fn target_of(&self, from: &ModelMeta, f: &Field) -> Result<&Arc<ModelMeta>> {
        let rel = f.rel.as_ref().ok_or_else(|| Error::FieldError(format!("{} is not a relation", f.name)))?;
        let label = if rel.to == "self" { from.label() } else { rel.to.clone() };
        self.get_or_err(&label)
    }

    /// Reverse relations pointing at `label`.
    pub fn related_objects(&self, label: &str) -> &[RelatedObject] {
        self.related.get(&label.to_lowercase()).map_or(&[], Vec::as_slice)
    }

    /// The through model of M2M field `f` on `m`.
    pub fn through_of(&self, m: &ModelMeta, f: &Field) -> Result<&Arc<ModelMeta>> {
        let rel = f.rel.as_ref().ok_or_else(|| Error::FieldError("not a relation".into()))?;
        match &rel.through {
            Some(t) => self.get_or_err(t),
            None => self.get_or_err(&format!("{}.{}_{}", m.app_label, m.object_name, f.name)),
        }
    }
}
