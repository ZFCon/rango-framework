//! `forms.forms`: `Form`.
//!
//! A [`Form`] value plays both roles of the class and instance: build it once with the
//! declarative builder (`Form::new("Contact").field(...)`), then bind per request with
//! [`Form::bound`].

use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;
use rango_core::exceptions::{NON_FIELD_ERRORS, ValidationError};
use rango_core::utils::translation::gettext;
use rango_template::Value as T;

use crate::boundfield::{BoundField, form_context};
use crate::fields::{Field, FieldKind};
use crate::renderers::{Renderer, get_default_renderer};
use crate::utils::{ErrorDict, ErrorList};
use crate::value::Value;
use crate::widgets::{Data, FileData, Media};

/// Mutable view handed to `clean_<field>` and `clean()` hooks.
pub struct FormClean<'a> {
    pub form: &'a Form,
    pub cleaned: &'a mut IndexMap<String, Value>,
    pub errors: &'a mut ErrorDict,
}

impl FormClean<'_> {
    /// `form.add_error(field, error)`; `field = None` targets the non-field errors.
    pub fn add_error(&mut self, field: Option<&str>, error: ValidationError) -> Result<(), String> {
        add_error(self.form, self.cleaned, self.errors, field, error)
    }
}

fn add_error(form: &Form, cleaned: &mut IndexMap<String, Value>, errors: &mut ErrorDict, field: Option<&str>, error: ValidationError) -> Result<(), String> {
    let by_field: IndexMap<String, Vec<ValidationError>> = match error {
        ValidationError::Dict(d) => {
            if field.is_some() {
                return Err("The argument `field` must be `None` when the `error` argument contains errors for multiple fields.".into());
            }
            d
        }
        other => [(field.unwrap_or(NON_FIELD_ERRORS).to_owned(), other.error_list())].into_iter().collect(),
    };
    for (name, list) in by_field {
        if !errors.items.contains_key(&name) {
            if name != NON_FIELD_ERRORS && !form.fields.contains_key(&name) {
                return Err(format!("'{}' has no field named '{name}'.", form.name));
            }
            let l = if name == NON_FIELD_ERRORS {
                ErrorList::new(Some("nonfield"), form.renderer.clone())
            } else {
                ErrorList::new(None, form.renderer.clone()).with_field_id(form.get(&name).map(|b| b.auto_id()))
            };
            errors.items.insert(name.clone(), l);
        }
        errors.items.get_mut(&name).expect("just inserted").extend(list);
        cleaned.shift_remove(&name);
    }
    Ok(())
}

/// `clean_<field>()`: receives the field's cleaned value, returns the (possibly changed) value.
pub type CleanFieldHook = Arc<dyn Fn(&mut FormClean, &Value) -> Result<Value, ValidationError> + Send + Sync>;
/// `clean()` / `_post_clean()`.
pub type CleanHook = Arc<dyn Fn(&mut FormClean) -> Result<(), ValidationError> + Send + Sync>;

#[derive(Clone, Default)]
pub(crate) struct CleanResult {
    pub errors: Option<ErrorDict>,
    pub cleaned: IndexMap<String, Value>,
}

/// A form: fields, data and configuration.
pub struct Form {
    pub name: String,
    pub fields: IndexMap<String, Field>,
    pub data: Data,
    pub files: FileData,
    pub is_bound: bool,
    /// `auto_id` pattern (`"id_%s"`); `None` disables ids.
    pub auto_id: Option<String>,
    pub prefix: Option<String>,
    pub initial: IndexMap<String, Value>,
    pub label_suffix: String,
    pub empty_permitted: bool,
    pub use_required_attribute: bool,
    pub renderer: Arc<dyn Renderer>,
    pub required_css_class: Option<String>,
    pub error_css_class: Option<String>,
    pub field_order: Option<Vec<String>>,
    clean_field_hooks: IndexMap<String, CleanFieldHook>,
    clean_hook: Option<CleanHook>,
    post_clean_hook: Option<CleanHook>,
    result: OnceLock<CleanResult>,
}

impl Clone for Form {
    fn clone(&self) -> Self {
        let result = OnceLock::new();
        if let Some(r) = self.result.get() {
            let _ = result.set(r.clone());
        }
        Form {
            name: self.name.clone(),
            fields: self.fields.clone(),
            data: self.data.clone(),
            files: self.files.clone(),
            is_bound: self.is_bound,
            auto_id: self.auto_id.clone(),
            prefix: self.prefix.clone(),
            initial: self.initial.clone(),
            label_suffix: self.label_suffix.clone(),
            empty_permitted: self.empty_permitted,
            use_required_attribute: self.use_required_attribute,
            renderer: self.renderer.clone(),
            required_css_class: self.required_css_class.clone(),
            error_css_class: self.error_css_class.clone(),
            field_order: self.field_order.clone(),
            clean_field_hooks: self.clean_field_hooks.clone(),
            clean_hook: self.clean_hook.clone(),
            post_clean_hook: self.post_clean_hook.clone(),
            result,
        }
    }
}

impl std::fmt::Debug for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let valid = match self.result.get() {
            None => "Unknown".to_owned(),
            Some(r) => (self.is_bound && r.errors.as_ref().is_none_or(ErrorDict::is_empty)).to_string(),
        };
        write!(f, "<{} bound={}, valid={}, fields=({})>", self.name, self.is_bound, if valid == "true" { "True" } else if valid == "false" { "False" } else { "Unknown" }, self.fields.keys().cloned().collect::<Vec<_>>().join(";"))
    }
}

impl Form {
    pub fn new(name: &str) -> Form {
        Form {
            name: name.to_owned(),
            fields: IndexMap::new(),
            data: Data::new(),
            files: FileData::new(),
            is_bound: false,
            auto_id: Some("id_%s".into()),
            prefix: None,
            initial: IndexMap::new(),
            label_suffix: gettext(":"),
            empty_permitted: false,
            use_required_attribute: true,
            renderer: get_default_renderer(),
            required_css_class: None,
            error_css_class: None,
            field_order: None,
            clean_field_hooks: IndexMap::new(),
            clean_hook: None,
            post_clean_hook: None,
            result: OnceLock::new(),
        }
    }

    // ---- declarative builder

    pub fn field(mut self, name: &str, field: Field) -> Self {
        self.fields.insert(name.to_owned(), field);
        self
    }

    /// `clean_<name>()`.
    pub fn clean_field(mut self, name: &str, f: impl Fn(&mut FormClean, &Value) -> Result<Value, ValidationError> + Send + Sync + 'static) -> Self {
        self.clean_field_hooks.insert(name.to_owned(), Arc::new(f));
        self
    }

    /// `clean()`: cross-field validation; mutate `cleaned`, or return a `ValidationError`.
    pub fn clean(mut self, f: impl Fn(&mut FormClean) -> Result<(), ValidationError> + Send + Sync + 'static) -> Self {
        self.clean_hook = Some(Arc::new(f));
        self
    }

    pub(crate) fn post_clean(mut self, f: CleanHook) -> Self {
        self.post_clean_hook = Some(f);
        self
    }

    pub fn prefix(mut self, p: &str) -> Self {
        self.prefix = Some(p.to_owned());
        self
    }
    pub fn auto_id(mut self, a: Option<&str>) -> Self {
        self.auto_id = a.map(str::to_owned);
        self
    }
    pub fn initial(mut self, initial: impl IntoIterator<Item = (String, Value)>) -> Self {
        self.initial = initial.into_iter().collect();
        self
    }
    pub fn label_suffix(mut self, s: &str) -> Self {
        self.label_suffix = s.to_owned();
        self
    }
    pub fn required_css_class(mut self, c: &str) -> Self {
        self.required_css_class = Some(c.to_owned());
        self
    }
    pub fn error_css_class(mut self, c: &str) -> Self {
        self.error_css_class = Some(c.to_owned());
        self
    }
    pub fn empty_permitted(mut self, e: bool) -> Self {
        self.empty_permitted = e;
        if e {
            self.use_required_attribute = false;
        }
        self
    }
    pub fn use_required_attribute(mut self, u: bool) -> Self {
        self.use_required_attribute = u;
        self
    }
    pub fn renderer(mut self, r: Arc<dyn Renderer>) -> Self {
        self.renderer = r;
        self
    }
    pub fn field_order(mut self, order: &[&str]) -> Self {
        self.field_order = Some(order.iter().map(|s| (*s).to_owned()).collect());
        self.order_fields();
        self
    }

    fn order_fields(&mut self) {
        let Some(order) = &self.field_order else { return };
        let mut fields = IndexMap::new();
        for k in order {
            if let Some(f) = self.fields.shift_remove(k) {
                fields.insert(k.clone(), f);
            }
        }
        fields.extend(std::mem::take(&mut self.fields));
        self.fields = fields;
    }

    /// Bind submitted data (`Form(data, files)`).
    pub fn bound(mut self, data: Data, files: FileData) -> Self {
        self.is_bound = true;
        self.data = data;
        self.files = files;
        self.result = OnceLock::new();
        self.order_fields();
        self
    }

    /// Bind from a parsed query/POST dictionary.
    pub fn bound_data(self, data: Data) -> Self {
        self.bound(data, FileData::new())
    }

    // ---- accessors

    pub fn add_prefix(&self, field_name: &str) -> String {
        match &self.prefix {
            Some(p) if !p.is_empty() => format!("{p}-{field_name}"),
            _ => field_name.to_owned(),
        }
    }

    pub fn add_initial_prefix(&self, field_name: &str) -> String {
        format!("initial-{}", self.add_prefix(field_name))
    }

    /// `form[name]`.
    pub fn get(&self, name: &str) -> Option<BoundField<'_>> {
        self.fields.get_key_value(name).map(|(k, f)| BoundField::new(self, f, k))
    }

    /// `iter(form)`.
    pub fn bound_fields(&self) -> Vec<BoundField<'_>> {
        self.fields.iter().map(|(k, f)| BoundField::new(self, f, k)).collect()
    }

    pub fn hidden_fields(&self) -> Vec<BoundField<'_>> {
        self.bound_fields().into_iter().filter(BoundField::is_hidden).collect()
    }

    pub fn visible_fields(&self) -> Vec<BoundField<'_>> {
        self.bound_fields().into_iter().filter(|b| !b.is_hidden()).collect()
    }

    pub(crate) fn widget_data_value(&self, widget: &crate::widgets::Widget, html_name: &str) -> Value {
        widget.value_from_datadict(&self.data, &self.files, html_name)
    }

    pub fn get_initial_for_field(&self, field: &Field, name: &str) -> Value {
        let v = self.initial.get(name).cloned().or_else(|| field.initial.clone()).unwrap_or(Value::None);
        if field.widget.supports_microseconds() {
            return v;
        }
        match v {
            Value::Time(t) => Value::Time(chrono::Timelike::with_nanosecond(&t, 0).unwrap_or(t)),
            Value::NaiveDateTime(d) => Value::NaiveDateTime(chrono::Timelike::with_nanosecond(&d, 0).unwrap_or(d)),
            Value::DateTime(d) => Value::DateTime(chrono::Timelike::with_nanosecond(&d, 0).unwrap_or(d)),
            other => other,
        }
    }

    pub fn is_multipart(&self) -> bool {
        self.fields.values().any(|f| f.widget.needs_multipart_form())
    }

    pub fn media(&self) -> Media {
        self.fields.values().fold(Media::default(), |m, f| m + f.widget.media())
    }

    pub fn changed_data(&self) -> Vec<String> {
        self.bound_fields().into_iter().filter(BoundField::has_changed).map(|b| b.name).collect()
    }

    pub fn has_changed(&self) -> bool {
        !self.changed_data().is_empty()
    }

    // ---- validation

    fn result(&self) -> &CleanResult {
        self.result.get_or_init(|| self.full_clean())
    }

    /// `form.errors`.
    pub fn errors(&self) -> &ErrorDict {
        self.result().errors.as_ref().expect("errors are always set")
    }

    /// `form.cleaned_data` (empty when unbound or invalid fields were dropped).
    pub fn cleaned_data(&self) -> &IndexMap<String, Value> {
        &self.result().cleaned
    }

    pub fn is_valid(&self) -> bool {
        self.is_bound && self.errors().is_empty()
    }

    pub fn non_field_errors(&self) -> ErrorList {
        self.errors().get(NON_FIELD_ERRORS).cloned().unwrap_or_else(|| ErrorList::new(Some("nonfield"), self.renderer.clone()))
    }

    /// `form.has_error(field, code)`.
    pub fn has_error(&self, field: &str, code: Option<&str>) -> bool {
        match self.errors().get(field) {
            None => false,
            Some(l) => code.is_none_or(|c| l.as_data().iter().any(|e| e.code() == Some(c))),
        }
    }

    /// `form.add_error(field, error)` after validation.
    pub fn add_error(&mut self, field: Option<&str>, error: ValidationError) -> Result<(), String> {
        self.result();
        let mut r = self.result.take().expect("initialized");
        let mut errors = r.errors.take().expect("errors set");
        let res = add_error(self, &mut r.cleaned, &mut errors, field, error);
        r.errors = Some(errors);
        let _ = self.result.set(r);
        res
    }

    fn full_clean(&self) -> CleanResult {
        let mut errors = ErrorDict::new(self.renderer.clone());
        let mut cleaned = IndexMap::new();
        if !self.is_bound {
            return CleanResult { errors: Some(errors), cleaned };
        }
        if self.empty_permitted && !self.has_changed() {
            return CleanResult { errors: Some(errors), cleaned };
        }
        // _clean_fields
        for bf in self.bound_fields() {
            let field = bf.field;
            let initial = bf.initial();
            let value = if field.disabled { initial.clone() } else { bf.data() };
            let is_file = matches!(field.kind, FieldKind::File { .. } | FieldKind::Image { .. });
            let result = if is_file { field.clean_with_initial(&value, &initial) } else { field.clean(&value) };
            match result {
                Ok(v) => {
                    cleaned.insert(bf.name.clone(), v.clone());
                    if let Some(hook) = self.clean_field_hooks.get(&bf.name) {
                        let mut ctx = FormClean { form: self, cleaned: &mut cleaned, errors: &mut errors };
                        match hook(&mut ctx, &v) {
                            Ok(nv) => {
                                cleaned.insert(bf.name.clone(), nv);
                            }
                            Err(e) => {
                                let _ = add_error(self, &mut cleaned, &mut errors, Some(&bf.name), e);
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = add_error(self, &mut cleaned, &mut errors, Some(&bf.name), e);
                }
            }
        }
        // _clean_form
        if let Some(hook) = &self.clean_hook {
            let mut ctx = FormClean { form: self, cleaned: &mut cleaned, errors: &mut errors };
            if let Err(e) = hook(&mut ctx) {
                let _ = add_error(self, &mut cleaned, &mut errors, None, e);
            }
        }
        // _post_clean
        if let Some(hook) = &self.post_clean_hook {
            let mut ctx = FormClean { form: self, cleaned: &mut cleaned, errors: &mut errors };
            if let Err(e) = hook(&mut ctx) {
                let _ = add_error(self, &mut cleaned, &mut errors, None, e);
            }
        }
        CleanResult { errors: Some(errors), cleaned }
    }

    // ---- rendering

    pub(crate) fn render_template(&self, template: &str, ctx: T) -> String {
        self.renderer.render(template, ctx).unwrap_or_else(|e| panic!("rendering {template}: {e}"))
    }

    fn render_with(&self, template: &str) -> String {
        self.render_template(template, T::Map(form_context(self)))
    }

    /// `str(form)`: the renderer's default template (`div.html`).
    pub fn render(&self) -> String {
        let t = self.renderer.form_template_name().to_owned();
        self.render_with(&t)
    }
    pub fn as_div(&self) -> String {
        self.render_with("rango/forms/div.html")
    }
    pub fn as_p(&self) -> String {
        self.render_with("rango/forms/p.html")
    }
    pub fn as_table(&self) -> String {
        self.render_with("rango/forms/table.html")
    }
    pub fn as_ul(&self) -> String {
        self.render_with("rango/forms/ul.html")
    }

    /// The form as a template value: `{{ form }}`, `{{ form.as_p }}`, `{{ form.name.errors }}`.
    pub fn to_template(&self) -> T {
        rango_template::Object::new(crate::boundfield::FormObject { form: Arc::new(self.clone()) })
    }
}

impl From<Form> for T {
    fn from(f: Form) -> T {
        f.to_template()
    }
}
