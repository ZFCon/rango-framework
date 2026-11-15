//! `forms.models`: `ModelForm`, `ModelChoiceField`, model form sets.
//!
//! Database access is explicit: choices are loaded when the form is built
//! (`ModelForm::new(&db, ..).await`), uniqueness is checked in `is_valid(&db).await`.

use std::sync::Arc;

use indexmap::IndexMap;
use rango_core::exceptions::{Error, NON_FIELD_ERRORS, Result, ValidationError};
use rango_core::utils::translation::gettext;
use rango_db::models::base::RowData;
use rango_db::models::fields_db::prep_param;
use rango_db::models::meta::{Default_, FieldType, ModelMeta, registry};
use rango_db::models::{Model, related};
use rango_db::{Executor, Value as DbValue};

use crate::fields::Field;
use crate::forms::Form;
use crate::value::Value;
use crate::widgets::{Choice, Data, FileData, Widget};

type MField = rango_db::models::meta::Field;

pub fn capfirst(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// `Meta` of a `ModelForm`: which fields, and per-field overrides.
#[derive(Clone, Default)]
pub struct ModelFormOptions {
    /// `fields` (`None` = `"__all__"`).
    pub fields: Option<Vec<String>>,
    pub exclude: Vec<String>,
    pub widgets: IndexMap<String, Widget>,
    pub labels: IndexMap<String, String>,
    pub help_texts: IndexMap<String, String>,
    pub error_messages: IndexMap<String, IndexMap<String, String>>,
    /// Replace the generated field entirely (the declared fields / `field_classes`).
    pub overrides: IndexMap<String, Field>,
    /// Restrict a relation field's choices (`queryset=`/`limit_choices_to`).
    pub choices: IndexMap<String, Vec<Choice>>,
}

impl ModelFormOptions {
    pub fn all() -> Self {
        Self::default()
    }
    pub fn fields<I: IntoIterator<Item = S>, S: Into<String>>(fields: I) -> Self {
        Self { fields: Some(fields.into_iter().map(Into::into).collect()), ..Self::default() }
    }
    pub fn exclude<I: IntoIterator<Item = S>, S: Into<String>>(mut self, ex: I) -> Self {
        self.exclude = ex.into_iter().map(Into::into).collect();
        self
    }
    pub fn widget(mut self, field: &str, w: Widget) -> Self {
        self.widgets.insert(field.to_owned(), w);
        self
    }
    pub fn label(mut self, field: &str, l: &str) -> Self {
        self.labels.insert(field.to_owned(), l.to_owned());
        self
    }
    pub fn help_text(mut self, field: &str, l: &str) -> Self {
        self.help_texts.insert(field.to_owned(), l.to_owned());
        self
    }
    pub fn error_message(mut self, field: &str, code: &str, msg: &str) -> Self {
        self.error_messages.entry(field.to_owned()).or_default().insert(code.to_owned(), msg.to_owned());
        self
    }
    pub fn field_override(mut self, field: &str, f: Field) -> Self {
        self.overrides.insert(field.to_owned(), f);
        self
    }
    pub fn choices(mut self, field: &str, c: Vec<Choice>) -> Self {
        self.choices.insert(field.to_owned(), c);
        self
    }
}

fn int_range(ty: FieldType) -> Option<(i64, i64)> {
    Some(match ty {
        FieldType::SmallInteger | FieldType::SmallAuto => (-32768, 32767),
        FieldType::Integer | FieldType::Auto => (-2147483648, 2147483647),
        FieldType::BigInteger | FieldType::BigAuto => (i64::MIN, i64::MAX),
        FieldType::PositiveSmallInteger => (0, 32767),
        FieldType::PositiveInteger => (0, 2147483647),
        FieldType::PositiveBigInteger => (0, i64::MAX),
        _ => return None,
    })
}

fn default_value(d: &Default_) -> Value {
    match d {
        Default_::Value(v) => v.clone().into(),
        Default_::Callable(f) => f().into(),
    }
}

/// Load `(pk, label)` rows of a model as choices (`ModelChoiceIterator`).
pub async fn load_choices(db: &dyn Executor, target: &ModelMeta) -> Result<Vec<Choice>> {
    let vendor = db.vendor();
    let pk = target.pk();
    let label_col = target.str_field.as_ref().and_then(|n| target.get_field(n)).map(MField::column);
    let order: Vec<String> = target
        .ordering
        .iter()
        .filter_map(|o| {
            let (name, desc) = o.strip_prefix('-').map_or((o.as_str(), false), |n| (n, true));
            target.get_field(name).filter(|f| f.concrete()).map(|f| format!("{}{}", vendor.quote(&f.column()), if desc { " DESC" } else { "" }))
        })
        .collect();
    let mut sql = format!("SELECT {}{} FROM {}", vendor.quote(&pk.column()), label_col.as_ref().map_or(String::new(), |c| format!(", {}", vendor.quote(c))), vendor.quote(&target.db_table));
    sql.push_str(&format!(" ORDER BY {}", if order.is_empty() { vendor.quote(&pk.column()) } else { order.join(", ") }));
    let rows = db.query(&sql, &[]).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let mut it = r.values.into_iter();
            let key: Value = it.next().unwrap_or(DbValue::Null).into();
            let label = if label_col.is_some() { Value::from(it.next().unwrap_or(DbValue::Null)).to_text() } else { format!("{} object ({})", target.object_name, key.to_text()) };
            Choice::Option(key, label)
        })
        .collect())
}

/// `Field.formfield()`: the form field for a model field (`None` for non-editable fields).
pub async fn formfield(db: &dyn Executor, meta: &ModelMeta, f: &MField, opts: &ModelFormOptions) -> Result<Option<Field>> {
    if !f.editable || f.ty.is_auto() {
        return Ok(None);
    }
    if let Some(o) = opts.overrides.get(&f.name) {
        return Ok(Some(o.clone()));
    }
    let required = !f.blank;
    let mut ff = match f.ty {
        FieldType::ForeignKey | FieldType::OneToOne | FieldType::ManyToMany => {
            let multiple = f.ty == FieldType::ManyToMany;
            let target = registry().target_of(meta, f)?.clone();
            let mut choices = match opts.choices.get(&f.name) {
                Some(c) => c.clone(),
                None => load_choices(db, &target).await?,
            };
            let mut fld = Field::model_choice(multiple);
            if !multiple && (!required || f.default.is_none()) {
                choices.insert(0, Choice::option("", "---------"));
            }
            fld.set_choices(choices);
            fld
        }
        _ if !f.choices.is_empty() => {
            let mut choices: Vec<Choice> = f.choices.iter().map(|(v, l)| Choice::Option(v.clone().into(), l.clone())).collect();
            if f.blank || f.default.is_none() {
                choices.insert(0, Choice::option("", "---------"));
            }
            let int = matches!(f.ty, FieldType::Integer | FieldType::BigInteger | FieldType::SmallInteger | FieldType::PositiveInteger | FieldType::PositiveBigInteger | FieldType::PositiveSmallInteger);
            let null = f.null;
            let mut fld = Field::typed_choice(choices, move |s| if int { s.parse::<i64>().map(Value::Int).map_err(|_| ()) } else { Ok(Value::Str(s.to_owned())) });
            if let crate::fields::FieldKind::TypedChoice { empty_value, .. } = &mut fld.kind {
                *empty_value = if null { Value::None } else { Value::Str(String::new()) };
            }
            fld
        }
        FieldType::Char => {
            let mut fld = Field::char();
            if let Some(n) = f.max_length {
                fld = fld.max_length(n as usize);
            }
            if f.null {
                fld = fld.empty_value(Value::None);
            }
            fld
        }
        FieldType::Text => {
            let mut fld = Field::char().widget(Widget::textarea());
            if let Some(n) = f.max_length {
                fld = fld.max_length(n as usize);
            }
            if f.null {
                fld = fld.empty_value(Value::None);
            }
            fld
        }
        FieldType::Boolean => {
            if f.null { Field::null_boolean() } else { Field::boolean() }
        }
        FieldType::Float => Field::float(),
        FieldType::Decimal => {
            let mut fld = Field::decimal();
            if let Some(d) = f.max_digits {
                fld = fld.max_digits(d);
            }
            if let Some(d) = f.decimal_places {
                fld = fld.decimal_places(d);
            }
            fld
        }
        FieldType::Date => Field::date(),
        FieldType::DateTime => Field::datetime(),
        FieldType::Time => Field::time(),
        FieldType::Duration => Field::duration(),
        FieldType::Email => {
            let mut fld = Field::email();
            if let Some(n) = f.max_length {
                fld = fld.max_length(n as usize);
            }
            fld
        }
        FieldType::Url => {
            let mut fld = Field::url();
            if let Some(n) = f.max_length {
                fld = fld.max_length(n as usize);
            }
            fld
        }
        FieldType::Slug => {
            let mut fld = Field::slug();
            if let Some(n) = f.max_length {
                fld = fld.max_length(n as usize);
            }
            fld
        }
        FieldType::Uuid => Field::uuid(),
        FieldType::Json => Field::json(),
        FieldType::GenericIpAddress => Field::generic_ip_address("both", false),
        FieldType::File => Field::file(),
        FieldType::Image => Field::image(),
        FieldType::Binary => return Ok(None),
        _ => {
            let mut fld = Field::integer();
            if let Some((lo, hi)) = int_range(f.ty) {
                fld = fld.min_value(lo).max_value(hi);
            }
            fld
        }
    };
    ff = ff.required(required && f.ty != FieldType::Boolean).label(capfirst(&f.verbose_name.clone().unwrap_or_else(|| f.name.replace('_', " "))));
    if !f.help_text.is_empty() {
        ff = ff.help_text(f.help_text.clone());
    }
    if let Some(d) = &f.default {
        ff = ff.initial(default_value(d));
        if matches!(d, Default_::Callable(_)) {
            ff = ff.show_hidden_initial(true);
        }
    }
    if let Some(w) = opts.widgets.get(&f.name) {
        ff = ff.widget(w.clone());
    }
    if let Some(l) = opts.labels.get(&f.name) {
        ff = ff.label(l.clone());
    }
    if let Some(h) = opts.help_texts.get(&f.name) {
        ff = ff.help_text(h.clone());
    }
    for (code, msg) in opts.error_messages.get(&f.name).into_iter().flatten() {
        ff = ff.error_message(code, msg.clone());
    }
    Ok(Some(ff))
}

/// `fields_for_model(model, fields, exclude, ...)`.
pub async fn fields_for_model(db: &dyn Executor, meta: &ModelMeta, opts: &ModelFormOptions) -> Result<IndexMap<String, Field>> {
    let mut out = IndexMap::new();
    for f in meta.fields.iter().filter(|f| f.concrete() || f.ty == FieldType::ManyToMany) {
        if opts.fields.as_ref().is_some_and(|l| !l.contains(&f.name)) || opts.exclude.contains(&f.name) {
            continue;
        }
        if let Some(ff) = formfield(db, meta, f, opts).await? {
            out.insert(f.name.clone(), ff);
        }
    }
    if let Some(order) = &opts.fields {
        let mut ordered = IndexMap::new();
        for n in order {
            if let Some(f) = out.shift_remove(n) {
                ordered.insert(n.clone(), f);
            }
        }
        ordered.extend(out);
        out = ordered;
    }
    Ok(out)
}

/// A form bound to (or creating) a model instance.
pub struct ModelForm<M: Model> {
    pub form: Form,
    pub instance: M,
    pub is_new: bool,
    meta: Arc<ModelMeta>,
    model_clean: Option<Arc<dyn Fn(&mut M) -> std::result::Result<(), ValidationError> + Send + Sync>>,
    unique_checked: bool,
}

pub fn blank_instance<M: Model + Default>(meta: &ModelMeta) -> Result<M> {
    let inst = M::default();
    let mut data = inst.to_data();
    for f in meta.fields.iter().filter(|f| f.concrete()) {
        if let Some(d) = &f.default {
            data.insert(f.attname(), match d {
                Default_::Value(v) => v.clone(),
                Default_::Callable(c) => c(),
            });
        }
    }
    M::from_data(RowData { fields: data, ..RowData::default() })
}

impl<M: Model + Default> ModelForm<M> {
    /// `ModelForm()`: an add form (choices are loaded from `db`).
    pub async fn new(db: &dyn Executor, opts: ModelFormOptions) -> Result<Self> {
        let meta = M::meta()?;
        let instance = blank_instance::<M>(&meta)?;
        Self::build(db, opts, instance, true, &meta).await
    }
}

impl<M: Model> ModelForm<M> {
    /// `ModelForm(instance=obj)`: a change form, initial values from the instance.
    pub async fn edit(db: &dyn Executor, instance: M, opts: ModelFormOptions) -> Result<Self> {
        let meta = M::meta()?;
        Self::build(db, opts, instance, false, &meta).await
    }

    async fn build(db: &dyn Executor, opts: ModelFormOptions, instance: M, is_new: bool, meta: &Arc<ModelMeta>) -> Result<Self> {
        let fields = fields_for_model(db, meta, &opts).await?;
        let mut form = Form::new(&format!("{}Form", meta.object_name));
        let mut initial: IndexMap<String, Value> = IndexMap::new();
        let data = instance.to_data();
        let pk = instance.pk_value();
        for (name, f) in fields {
            if let Some(mf) = meta.get_field(&name) {
                if mf.ty == FieldType::ManyToMany {
                    if !pk.is_null() {
                        let ids = related::current_ids(db, &M::label(), &name, &pk).await?;
                        initial.insert(name.clone(), Value::List(ids.into_iter().map(Value::from).collect()));
                    }
                } else if !is_new {
                    if let Some(v) = data.get(&mf.attname()) {
                        initial.insert(name.clone(), if mf.ty == FieldType::Json { Value::Json(match v { DbValue::Json(j) => j.clone(), _ => serde_json::Value::Null }) } else { v.clone().into() });
                    }
                }
            }
            form = form.field(&name, f);
        }
        form = form.initial(initial);
        Ok(ModelForm { form, instance, is_new, meta: Arc::clone(meta), model_clean: None, unique_checked: false })
    }

    /// Model-level `clean()`: runs on the instance built from the form's data.
    pub fn model_clean(mut self, f: impl Fn(&mut M) -> std::result::Result<(), ValidationError> + Send + Sync + 'static) -> Self {
        self.model_clean = Some(Arc::new(f));
        self
    }

    /// Replace the inner form (e.g. to add `clean_<field>` hooks or change `prefix`).
    pub fn map_form(mut self, f: impl FnOnce(Form) -> Form) -> Self {
        self.form = f(self.form);
        self
    }

    /// Bind submitted data.
    pub fn bound(mut self, data: Data, files: FileData) -> Self {
        self.form = self.form.bound(data, files);
        self.unique_checked = false;
        self
    }

    pub fn form(&self) -> &Form {
        &self.form
    }

    /// `construct_instance(form, instance)`: copy cleaned values into a fresh instance.
    pub fn construct_instance(&self) -> Result<M> {
        construct_instance::<M>(&self.meta, &self.form, self.instance.to_data())
    }

    /// `validate_unique()` against the database.
    async fn validate_unique(&mut self, db: &dyn Executor, candidate: &M) -> Result<()> {
        let vendor = db.vendor();
        let data = candidate.to_data();
        let pk = candidate.pk_value();
        let mut checks: Vec<Vec<&MField>> = Vec::new();
        for f in self.meta.fields.iter().filter(|f| f.concrete() && f.unique && !f.primary_key) {
            checks.push(vec![f]);
        }
        for group in &self.meta.unique_together {
            let fs: Vec<&MField> = group.iter().filter_map(|n| self.meta.get_field(n)).collect();
            if fs.len() == group.len() {
                checks.push(fs);
            }
        }
        let model_name = capfirst(&self.meta.verbose_name);
        for check in checks {
            let cleaned = self.form.cleaned_data();
            if check.iter().any(|f| !cleaned.contains_key(&f.name) || data.get(&f.attname()).is_none_or(DbValue::is_null)) {
                continue;
            }
            let mut sql = format!("SELECT 1 FROM {} WHERE ", vendor.quote(&self.meta.db_table));
            let mut params = Vec::new();
            let conds: Vec<String> = check
                .iter()
                .map(|f| {
                    params.push(prep_param(vendor, f, data[&f.attname()].clone()));
                    format!("{} = %s", vendor.quote(&f.column()))
                })
                .collect();
            sql.push_str(&conds.join(" AND "));
            if !self.is_new && !pk.is_null() {
                sql.push_str(&format!(" AND {} <> %s", vendor.quote(&self.meta.pk().column())));
                params.push(prep_param(vendor, self.meta.pk(), pk.clone()));
            }
            sql.push_str(" LIMIT 1");
            if db.query(&sql, &params).await?.is_empty() {
                continue;
            }
            let label = |f: &MField| capfirst(&f.verbose_name.clone().unwrap_or_else(|| f.name.replace('_', " ")));
            if check.len() == 1 {
                let e = ValidationError::new(gettext("%(model_name)s with this %(field_label)s already exists.")).with_code("unique").with_params([("model_name", model_name.clone()), ("field_label", label(check[0]))]);
                self.form.add_error(Some(&check[0].name), e).map_err(|m| Error::Other(m.into()))?;
            } else {
                let labels: Vec<String> = check.iter().map(|f| label(f)).collect();
                let text = text_list(&labels, &gettext("and"));
                let e = ValidationError::new(gettext("%(model_name)s with this %(field_labels)s already exists.")).with_code("unique_together").with_params([("model_name", model_name.clone()), ("field_labels", text)]);
                self.form.add_error(Some(NON_FIELD_ERRORS), e).map_err(|m| Error::Other(m.into()))?;
            }
        }
        Ok(())
    }

    /// `form.is_valid()` including model validation and uniqueness (needs the database).
    pub async fn is_valid(&mut self, db: &dyn Executor) -> Result<bool> {
        if !self.form.is_valid() {
            return Ok(false);
        }
        if !self.unique_checked {
            self.unique_checked = true;
            let mut candidate = self.construct_instance()?;
            if let Some(h) = &self.model_clean {
                if let Err(e) = h(&mut candidate) {
                    self.form.add_error(None, e).map_err(|m| Error::Other(m.into()))?;
                }
            }
            self.validate_unique(db, &candidate).await?;
            self.instance = candidate;
        }
        Ok(self.form.errors().is_empty())
    }

    /// `form.save(commit=True)`: INSERT/UPDATE the instance and set many-to-many relations.
    pub async fn save(&mut self, db: &dyn Executor) -> Result<&M> {
        if !self.is_valid(db).await? {
            let action = if self.is_new { "created" } else { "changed" };
            return Err(Error::Other(format!("The {} could not be {action} because the data didn't validate.", self.meta.object_name).into()));
        }
        let mut data = self.instance.to_data();
        // Uploaded files go to storage first; the column then holds the stored name.
        for f in self.meta.fields.iter().filter(|f| matches!(f.ty, FieldType::File | FieldType::Image) && self.form.fields.contains_key(&f.name)) {
            match self.form.cleaned_data().get(&f.name) {
                Some(Value::File(up)) => {
                    let name = rango_core::files::get_valid_filename(up.name.rsplit(['/', '\\']).next().unwrap_or(&up.name))?;
                    let dest = match &f.upload_to {
                        Some(dir) => format!("{}/{name}", dir.trim_end_matches('/')),
                        None => name,
                    };
                    let saved = rango_core::files::default_storage().save(&dest, &up.read().map_err(|e| Error::Other(Box::new(e)))?).await?;
                    data.insert(f.attname(), DbValue::Str(saved));
                }
                Some(Value::Cleared) => {
                    data.insert(f.attname(), DbValue::Str(String::new()));
                }
                _ => {}
            }
        }
        let mut inst = construct_instance::<M>(&self.meta, &self.form, data)?;
        inst.save(db).await?;
        let pk = inst.pk_value();
        for f in self.meta.many_to_many() {
            if let Some(Value::List(items)) = self.form.cleaned_data().get(&f.name) {
                let ids: Vec<DbValue> = items.iter().cloned().map(Into::into).collect();
                related::set(db, &M::label(), &f.name, &pk, &ids).await?;
            }
        }
        self.instance = inst;
        self.is_new = false;
        Ok(&self.instance)
    }
}

/// `construct_instance(form, instance, ...)` on instance data keyed by attname.
pub fn construct_instance<M: Model>(meta: &ModelMeta, form: &Form, mut data: IndexMap<String, DbValue>) -> Result<M> {
    let cleaned = form.cleaned_data();
    for f in meta.fields.iter().filter(|f| f.concrete()) {
        if !f.editable || f.ty.is_auto() || !form.fields.contains_key(&f.name) || matches!(f.ty, FieldType::File | FieldType::Image) {
            continue;
        }
        let Some(v) = cleaned.get(&f.name) else { continue };
        // Keep model defaults for omitted values (e.g. unchecked-by-omission fields).
        let bf = &form.fields[&f.name];
        if f.default.is_some() && bf.widget.value_omitted_from_data(&form.data, &form.files, &form.add_prefix(&f.name)) && v.is_empty() {
            continue;
        }
        data.insert(f.attname(), v.clone().into());
    }
    M::from_data(RowData { fields: data, ..RowData::default() })
}

/// the `get_text_list(["a", "b", "c"], "and")` → `"a, b and c"`.
pub fn text_list(items: &[String], last_word: &str) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} {last_word} {}", items[..n - 1].join(", "), items[n - 1]),
    }
}


// ----------------------------------------------------------------------------- model form sets

/// `modelformset_factory` / `inlineformset_factory`: a [`FormSet`](crate::FormSet) of model forms.
pub struct ModelFormSet<M: Model> {
    pub formset: crate::formsets::FormSet,
    meta: Arc<ModelMeta>,
    instances: Vec<M>,
    /// `(fk field name, parent pk)` for inline form sets.
    inline: Option<(String, DbValue)>,
}

impl<M: Model + Default + Clone> ModelFormSet<M> {
    /// `modelformset_factory(Model, ...)(queryset=...)`: forms for `instances`, plus `extra` blank ones.
    pub async fn new(db: &dyn Executor, instances: Vec<M>, opts: ModelFormOptions, extra: usize) -> Result<Self> {
        Self::build(db, instances, opts, extra, None).await
    }

    /// `inlineformset_factory(Parent, Model, fk_name=...)(instance=parent)`: children of `parent_pk` via `fk_name`.
    pub async fn inline(db: &dyn Executor, parent_pk: DbValue, fk_name: &str, children: Vec<M>, mut opts: ModelFormOptions, extra: usize) -> Result<Self> {
        opts.exclude.push(fk_name.to_owned());
        Self::build(db, children, opts, extra, Some((fk_name.to_owned(), parent_pk))).await
    }

    async fn build(db: &dyn Executor, instances: Vec<M>, opts: ModelFormOptions, extra: usize, inline: Option<(String, DbValue)>) -> Result<Self> {
        let meta = M::meta()?;
        let fields = fields_for_model(db, &meta, &opts).await?;
        let mut proto = Form::new(&format!("{}Form", meta.object_name));
        for (n, f) in fields {
            proto = proto.field(&n, f);
        }
        // The hidden primary-key field ties each submitted form to its instance.
        let pk_name = meta.pk().name.clone();
        let pk_choices: Vec<Choice> = instances.iter().map(|i| Choice::Option(i.pk_value().into(), String::new())).collect();
        let mut pk_field = Field::model_choice(false).required(false).widget(Widget::hidden_input());
        pk_field.set_choices(pk_choices);
        let proto = proto.field(&pk_name, pk_field);
        let initial = instances
            .iter()
            .map(|i| {
                let data = i.to_data();
                let mut m: IndexMap<String, Value> = IndexMap::new();
                for (name, _) in proto.fields.iter() {
                    if let Some(mf) = meta.get_field(name).filter(|f| f.concrete()) {
                        if let Some(v) = data.get(&mf.attname()) {
                            m.insert(name.clone(), if mf.ty == FieldType::Json { Value::Json(match v { DbValue::Json(j) => j.clone(), _ => serde_json::Value::Null }) } else { v.clone().into() });
                        }
                    }
                }
                m
            })
            .collect();
        let formset = crate::formsets::FormSet::new(proto).extra(extra).with_initial(initial);
        Ok(ModelFormSet { formset, meta, instances, inline })
    }

    pub fn bound(mut self, data: Data, files: FileData) -> Self {
        self.formset = self.formset.bound(data, files);
        self
    }

    /// `formset.save()`: create new objects, update changed ones, delete the ones marked for deletion.
    pub async fn save(&self, db: &dyn Executor) -> Result<Vec<M>> {
        if !self.formset.is_valid() {
            return Err(Error::Other("The formset could not be saved because the data didn't validate.".into()));
        }
        let pk_name = self.meta.pk().name.clone();
        let deleted: Vec<usize> = self.formset.deleted_forms().iter().filter_map(|d| self.formset.forms().iter().position(|f| std::ptr::eq(f, *d))).collect();
        let ic = self.formset.initial_form_count();
        let mut saved = Vec::new();
        for (i, form) in self.formset.forms().iter().enumerate() {
            let existing = form.cleaned_data().get(&pk_name).filter(|v| !v.is_empty()).and_then(|k| self.instances.iter().find(|inst| Value::from(inst.pk_value()) == *k));
            if deleted.contains(&i) {
                if let Some(inst) = existing {
                    M::objects().filter_q(rango_db::models::Q::lookup("pk", inst.pk_value())).delete(db).await?;
                }
                continue;
            }
            if i >= ic && !form.has_changed() {
                continue;
            }
            if existing.is_some() && !form.has_changed() {
                continue;
            }
            let base = match existing {
                Some(inst) => inst.to_data(),
                None => blank_instance::<M>(&self.meta)?.to_data(),
            };
            let mut data = base;
            if let Some((fk, parent)) = &self.inline {
                if let Some(f) = self.meta.get_field(fk) {
                    data.insert(f.attname(), parent.clone());
                }
            }
            let mut inst: M = construct_instance(&self.meta, form, data)?;
            inst.save(db).await?;
            let pk = inst.pk_value();
            for f in self.meta.many_to_many() {
                if let Some(Value::List(items)) = form.cleaned_data().get(&f.name) {
                    let ids: Vec<DbValue> = items.iter().cloned().map(Into::into).collect();
                    related::set(db, &M::label(), &f.name, &pk, &ids).await?;
                }
            }
            saved.push(inst);
        }
        Ok(saved)
    }
}
