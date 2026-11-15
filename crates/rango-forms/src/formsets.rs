//! `forms.formsets`: several forms of one kind on a page.
//!
//! A [`FormSet`] value is both the `formset_factory(...)` class and an instance: configure it
//! once, then call [`FormSet::bound`] / [`FormSet::with_initial`] per request.

use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;
use rango_core::exceptions::ValidationError;
use rango_core::utils::translation::{gettext, ngettext};
use rango_template::{Object, TemplateObject, Value as T};

use crate::fields::Field;
use crate::forms::Form;
use crate::renderers::{Renderer, get_default_renderer};
use crate::utils::{ErrorDict, ErrorList};
use crate::value::Value;
use crate::widgets::{Data, FileData, Media, Widget};

pub const TOTAL_FORM_COUNT: &str = "TOTAL_FORMS";
pub const INITIAL_FORM_COUNT: &str = "INITIAL_FORMS";
pub const MIN_NUM_FORM_COUNT: &str = "MIN_NUM_FORMS";
pub const MAX_NUM_FORM_COUNT: &str = "MAX_NUM_FORMS";
pub const ORDERING_FIELD_NAME: &str = "ORDER";
pub const DELETION_FIELD_NAME: &str = "DELETE";
pub const DEFAULT_MIN_NUM: i64 = 0;
pub const DEFAULT_MAX_NUM: i64 = 1000;

type FormHook = Arc<dyn Fn(Form, Option<usize>) -> Form + Send + Sync>;
type FormSetClean = Arc<dyn Fn(&FormSet) -> Result<(), ValidationError> + Send + Sync>;

#[derive(Clone)]
struct Cleaned {
    errors: Vec<ErrorDict>,
    non_form: ErrorList,
}

/// `BaseFormSet` + `formset_factory` options.
pub struct FormSet {
    pub form: Form,
    pub extra: usize,
    pub can_order: bool,
    pub can_delete: bool,
    pub can_delete_extra: bool,
    pub min_num: i64,
    pub max_num: i64,
    pub absolute_max: i64,
    pub validate_min: bool,
    pub validate_max: bool,
    pub prefix: String,
    pub auto_id: Option<String>,
    pub is_bound: bool,
    pub data: Data,
    pub files: FileData,
    pub initial: Vec<IndexMap<String, Value>>,
    pub renderer: Arc<dyn Renderer>,
    pub error_messages: IndexMap<String, String>,
    form_hook: Option<FormHook>,
    clean_hook: Option<FormSetClean>,
    forms: OnceLock<Vec<Form>>,
    management: OnceLock<Form>,
    cleaned: OnceLock<Cleaned>,
}

impl Clone for FormSet {
    fn clone(&self) -> Self {
        FormSet {
            form: self.form.clone(),
            extra: self.extra,
            can_order: self.can_order,
            can_delete: self.can_delete,
            can_delete_extra: self.can_delete_extra,
            min_num: self.min_num,
            max_num: self.max_num,
            absolute_max: self.absolute_max,
            validate_min: self.validate_min,
            validate_max: self.validate_max,
            prefix: self.prefix.clone(),
            auto_id: self.auto_id.clone(),
            is_bound: self.is_bound,
            data: self.data.clone(),
            files: self.files.clone(),
            initial: self.initial.clone(),
            renderer: self.renderer.clone(),
            error_messages: self.error_messages.clone(),
            form_hook: self.form_hook.clone(),
            clean_hook: self.clean_hook.clone(),
            forms: OnceLock::new(),
            management: OnceLock::new(),
            cleaned: OnceLock::new(),
        }
    }
}

fn hidden_int(required: bool) -> Field {
    Field::integer().required(required).widget(Widget::hidden_input())
}

impl FormSet {
    /// `formset_factory(form)`.
    pub fn new(form: Form) -> FormSet {
        let mut error_messages = IndexMap::new();
        error_messages.insert(
            "missing_management_form".to_owned(),
            gettext("ManagementForm data is missing or has been tampered with. Missing fields: %(field_names)s. You may need to file a bug report if the issue persists."),
        );
        FormSet {
            form,
            extra: 1,
            can_order: false,
            can_delete: false,
            can_delete_extra: true,
            min_num: DEFAULT_MIN_NUM,
            max_num: DEFAULT_MAX_NUM,
            absolute_max: DEFAULT_MAX_NUM + DEFAULT_MAX_NUM,
            validate_min: false,
            validate_max: false,
            prefix: "form".into(),
            auto_id: Some("id_%s".into()),
            is_bound: false,
            data: Data::new(),
            files: FileData::new(),
            initial: Vec::new(),
            renderer: get_default_renderer(),
            error_messages,
            form_hook: None,
            clean_hook: None,
            forms: OnceLock::new(),
            management: OnceLock::new(),
            cleaned: OnceLock::new(),
        }
    }

    pub fn extra(mut self, n: usize) -> Self {
        self.extra = n;
        self
    }
    pub fn can_order(mut self, b: bool) -> Self {
        self.can_order = b;
        self
    }
    pub fn can_delete(mut self, b: bool) -> Self {
        self.can_delete = b;
        self
    }
    pub fn can_delete_extra(mut self, b: bool) -> Self {
        self.can_delete_extra = b;
        self
    }
    pub fn min_num(mut self, n: i64) -> Self {
        self.min_num = n;
        self
    }
    /// `max_num`; `absolute_max` follows (`max_num + 1000`) unless set explicitly afterwards.
    pub fn max_num(mut self, n: i64) -> Self {
        self.max_num = n;
        self.absolute_max = n + DEFAULT_MAX_NUM;
        self
    }
    pub fn absolute_max(mut self, n: i64) -> Self {
        self.absolute_max = n;
        self
    }
    pub fn validate_min(mut self, b: bool) -> Self {
        self.validate_min = b;
        self
    }
    pub fn validate_max(mut self, b: bool) -> Self {
        self.validate_max = b;
        self
    }
    pub fn prefix(mut self, p: &str) -> Self {
        self.prefix = p.to_owned();
        self
    }
    pub fn auto_id(mut self, a: Option<&str>) -> Self {
        self.auto_id = a.map(str::to_owned);
        self
    }
    /// `get_form_kwargs(index)`: adjust every constructed form (`index` is `None` for `empty_form`).
    pub fn form_hook(mut self, f: impl Fn(Form, Option<usize>) -> Form + Send + Sync + 'static) -> Self {
        self.form_hook = Some(Arc::new(f));
        self
    }
    /// `formset.clean()`.
    pub fn clean(mut self, f: impl Fn(&FormSet) -> Result<(), ValidationError> + Send + Sync + 'static) -> Self {
        self.clean_hook = Some(Arc::new(f));
        self
    }
    pub fn with_initial(mut self, initial: Vec<IndexMap<String, Value>>) -> Self {
        self.initial = initial;
        self.reset();
        self
    }
    pub fn bound(mut self, data: Data, files: FileData) -> Self {
        self.is_bound = true;
        self.data = data;
        self.files = files;
        self.reset();
        self
    }

    fn reset(&mut self) {
        self.forms = OnceLock::new();
        self.management = OnceLock::new();
        self.cleaned = OnceLock::new();
    }

    // ---- management form

    /// The management form (`TOTAL_FORMS`, `INITIAL_FORMS`, `MIN_NUM_FORMS`, `MAX_NUM_FORMS`).
    pub fn management_form(&self) -> &Form {
        self.management.get_or_init(|| {
            let mut f = Form::new("ManagementForm")
                .field(TOTAL_FORM_COUNT, hidden_int(true))
                .field(INITIAL_FORM_COUNT, hidden_int(true))
                .field(MIN_NUM_FORM_COUNT, hidden_int(false))
                .field(MAX_NUM_FORM_COUNT, hidden_int(false))
                .clean(|c| {
                    c.cleaned.entry(TOTAL_FORM_COUNT.into()).or_insert(Value::Int(0));
                    c.cleaned.entry(INITIAL_FORM_COUNT.into()).or_insert(Value::Int(0));
                    Ok(())
                })
                .prefix(&self.prefix);
            f.auto_id = self.auto_id.clone();
            f.renderer = self.renderer.clone();
            if self.is_bound {
                f.bound(self.data.clone(), FileData::new())
            } else {
                f.initial = [
                    (TOTAL_FORM_COUNT, Value::Int(self.total_form_count() as i64)),
                    (INITIAL_FORM_COUNT, Value::Int(self.initial_form_count() as i64)),
                    (MIN_NUM_FORM_COUNT, Value::Int(self.min_num)),
                    (MAX_NUM_FORM_COUNT, Value::Int(self.max_num)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect();
                f
            }
        })
    }

    fn mgmt_value(&self, name: &str) -> i64 {
        self.management_form().cleaned_data().get(name).and_then(Value::as_i64).unwrap_or(0)
    }

    pub fn total_form_count(&self) -> usize {
        if self.is_bound {
            return self.mgmt_value(TOTAL_FORM_COUNT).min(self.absolute_max).max(0) as usize;
        }
        let initial = self.initial_form_count() as i64;
        let mut total = initial.max(self.min_num) + self.extra as i64;
        if initial > self.max_num && self.max_num >= 0 {
            total = initial;
        } else if total > self.max_num && self.max_num >= 0 {
            total = self.max_num;
        }
        total as usize
    }

    pub fn initial_form_count(&self) -> usize {
        if self.is_bound { self.mgmt_value(INITIAL_FORM_COUNT).max(0) as usize } else { self.initial.len() }
    }

    // ---- forms

    pub fn add_prefix(&self, index: impl std::fmt::Display) -> String {
        format!("{}-{index}", self.prefix)
    }

    fn construct_form(&self, i: Option<usize>) -> Form {
        let mut form = self.form.clone();
        form.auto_id = self.auto_id.clone();
        form.use_required_attribute = false;
        form.renderer = self.renderer.clone();
        match i {
            Some(i) => {
                form.prefix = Some(self.add_prefix(i));
                if let Some(init) = self.initial.get(i) {
                    form.initial = init.clone();
                }
                if i >= self.initial_form_count() && (i as i64) >= self.min_num {
                    form.empty_permitted = true;
                }
                if self.is_bound {
                    form = form.bound(self.data.clone(), self.files.clone());
                }
            }
            None => {
                form.prefix = Some(self.add_prefix("__prefix__"));
                form.empty_permitted = true;
            }
        }
        if let Some(h) = &self.form_hook {
            form = h(form, i);
        }
        self.add_fields(&mut form, i);
        form
    }

    /// `forms`.
    pub fn forms(&self) -> &[Form] {
        self.forms.get_or_init(|| (0..self.total_form_count()).map(|i| self.construct_form(Some(i))).collect())
    }

    pub fn initial_forms(&self) -> &[Form] {
        &self.forms()[..self.initial_form_count().min(self.forms().len())]
    }

    pub fn extra_forms(&self) -> &[Form] {
        &self.forms()[self.initial_form_count().min(self.forms().len())..]
    }

    /// `empty_form`: the template for JavaScript-added forms (prefix `__prefix__`).
    pub fn empty_form(&self) -> Form {
        self.construct_form(None)
    }

    fn add_fields(&self, form: &mut Form, index: Option<usize>) {
        let initial_count = self.initial_form_count();
        if self.can_order {
            let mut f = Field::integer().label(gettext("Order")).required(false).widget(Widget::number_input());
            if let Some(i) = index.filter(|i| *i < initial_count) {
                f = f.initial(i as i64 + 1);
            }
            form.fields.insert(ORDERING_FIELD_NAME.into(), f);
        }
        if self.can_delete && (self.can_delete_extra || index.is_some_and(|i| i < initial_count)) {
            form.fields.insert(DELETION_FIELD_NAME.into(), Field::boolean().label(gettext("Delete")).required(false));
        }
    }

    fn should_delete(&self, form: &Form) -> bool {
        form.cleaned_data().get(DELETION_FIELD_NAME).is_some_and(Value::truthy)
    }

    /// `cleaned_data`: per form (`None` when the formset is invalid).
    pub fn cleaned_data(&self) -> Option<Vec<&IndexMap<String, Value>>> {
        self.is_valid().then(|| self.forms().iter().map(Form::cleaned_data).collect())
    }

    /// `deleted_forms`.
    pub fn deleted_forms(&self) -> Vec<&Form> {
        if !self.is_valid() || !self.can_delete {
            return vec![];
        }
        let ic = self.initial_form_count();
        self.forms().iter().enumerate().filter(|(i, f)| !(*i >= ic && !f.has_changed()) && self.should_delete(f)).map(|(_, f)| f).collect()
    }

    /// `ordered_forms` (`None` unless valid and `can_order`).
    pub fn ordered_forms(&self) -> Option<Vec<&Form>> {
        if !self.is_valid() || !self.can_order {
            return None;
        }
        let ic = self.initial_form_count();
        let mut order: Vec<(usize, Option<i64>)> = Vec::new();
        for (i, f) in self.forms().iter().enumerate() {
            if i >= ic && !f.has_changed() {
                continue;
            }
            if self.can_delete && self.should_delete(f) {
                continue;
            }
            order.push((i, f.cleaned_data().get(ORDERING_FIELD_NAME).and_then(Value::as_i64)));
        }
        order.sort_by_key(|(_, o)| o.map_or((1, 0), |v| (0, v)));
        Some(order.into_iter().map(|(i, _)| &self.forms()[i]).collect())
    }

    // ---- validation

    fn clean_all(&self) -> &Cleaned {
        self.cleaned.get_or_init(|| {
            let mut non_form = ErrorList::new(Some("nonform"), self.renderer.clone());
            let mut errors = Vec::new();
            if !self.is_bound {
                return Cleaned { errors, non_form };
            }
            if !self.management_form().is_valid() {
                let names = self.management_form().errors().items.keys().map(|k| self.management_form().add_prefix(k)).collect::<Vec<_>>().join(", ");
                non_form.push(ValidationError::new(self.error_messages["missing_management_form"].clone()).with_code("missing_management_form").with_params([("field_names", names)]));
            }
            let mut empty_forms = 0usize;
            for (i, form) in self.forms().iter().enumerate() {
                if !form.has_changed() && i >= self.initial_form_count() {
                    empty_forms += 1;
                }
                let form_errors = form.errors().clone();
                if self.can_delete && self.should_delete(form) {
                    continue;
                }
                errors.push(form_errors);
            }
            let result: Result<(), ValidationError> = (|| {
                let deleted = self.deleted_forms_unchecked();
                let total = self.total_form_count() as i64;
                if (self.validate_max && total - deleted > self.max_num) || self.mgmt_value(TOTAL_FORM_COUNT) > self.absolute_max {
                    let msg = ngettext("Please submit at most %(num)d form.", "Please submit at most %(num)d forms.", self.max_num.unsigned_abs()).replace("%(num)d", &self.max_num.to_string());
                    return Err(ValidationError::new(msg).with_code("too_many_forms"));
                }
                if self.validate_min && total - deleted - (empty_forms as i64) < self.min_num {
                    let msg = ngettext("Please submit at least %(num)d form.", "Please submit at least %(num)d forms.", self.min_num.unsigned_abs()).replace("%(num)d", &self.min_num.to_string());
                    return Err(ValidationError::new(msg).with_code("too_few_forms"));
                }
                if let Some(h) = &self.clean_hook {
                    h(self)?;
                }
                Ok(())
            })();
            if let Err(e) = result {
                non_form = ErrorList::new(Some("nonform"), self.renderer.clone());
                non_form.extend(e.error_list());
            }
            Cleaned { errors, non_form }
        })
    }

    /// `len(deleted_forms)` computed without requiring validity (used while cleaning).
    fn deleted_forms_unchecked(&self) -> i64 {
        if !self.can_delete {
            return 0;
        }
        let ic = self.initial_form_count();
        self.forms().iter().enumerate().filter(|(i, f)| !(*i >= ic && !f.has_changed()) && self.should_delete(f)).count() as i64
    }

    pub fn non_form_errors(&self) -> &ErrorList {
        &self.clean_all().non_form
    }

    /// `errors`: one `ErrorDict` per (non-deleted) form.
    pub fn errors(&self) -> &[ErrorDict] {
        &self.clean_all().errors
    }

    pub fn total_error_count(&self) -> usize {
        self.non_form_errors().len() + self.errors().iter().map(|e| e.items.values().map(ErrorList::len).sum::<usize>()).sum::<usize>()
    }

    pub fn is_valid(&self) -> bool {
        if !self.is_bound {
            return false;
        }
        self.clean_all();
        let forms_valid = self.forms().iter().filter(|f| !(self.can_delete && self.should_delete(f))).all(Form::is_valid);
        forms_valid && self.non_form_errors().is_empty()
    }

    pub fn has_changed(&self) -> bool {
        self.forms().iter().any(Form::has_changed)
    }

    pub fn is_multipart(&self) -> bool {
        self.forms().first().map_or_else(|| self.empty_form().is_multipart(), Form::is_multipart)
    }

    pub fn media(&self) -> Media {
        self.forms().first().map_or_else(|| self.empty_form().media(), Form::media)
    }

    // ---- rendering

    fn render_with(&self, template: &str) -> String {
        let ctx = T::Map([("formset".to_owned(), self.to_template())].into_iter().collect());
        self.renderer.render(template, ctx).unwrap_or_else(|e| panic!("rendering {template}: {e}"))
    }

    pub fn render(&self) -> String {
        let t = self.renderer.formset_template_name().to_owned();
        self.render_with(&t)
    }
    pub fn as_div(&self) -> String {
        self.render_with("rango/forms/formsets/div.html")
    }
    pub fn as_p(&self) -> String {
        self.render_with("rango/forms/formsets/p.html")
    }
    pub fn as_table(&self) -> String {
        self.render_with("rango/forms/formsets/table.html")
    }
    pub fn as_ul(&self) -> String {
        self.render_with("rango/forms/formsets/ul.html")
    }

    pub fn to_template(&self) -> T {
        Object::new(FormSetObject { formset: Arc::new(self.clone()) })
    }
}

/// `all_valid(formsets)`.
pub fn all_valid(formsets: &[&FormSet]) -> bool {
    formsets.iter().all(|f| f.is_valid())
}

struct FormSetObject {
    formset: Arc<FormSet>,
}

impl TemplateObject for FormSetObject {
    fn get(&self, key: &str) -> Option<T> {
        let f = &self.formset;
        Some(match key {
            "management_form" => f.management_form().to_template(),
            "forms" => T::List(f.forms().iter().map(Form::to_template).collect()),
            "empty_form" => f.empty_form().to_template(),
            "non_form_errors" => Object::new(f.non_form_errors().clone()),
            "errors" => T::List(f.errors().iter().map(|e| Object::new(e.clone())).collect()),
            "is_bound" => T::Bool(f.is_bound),
            "is_valid" => T::Bool(f.is_valid()),
            "is_multipart" => T::Bool(f.is_multipart()),
            "media" => Object::new(f.media()),
            "total_form_count" => T::Int(f.total_form_count() as i64),
            "initial_form_count" => T::Int(f.initial_form_count() as i64),
            "can_delete" => T::Bool(f.can_delete),
            "can_order" => T::Bool(f.can_order),
            "as_div" => T::Safe(f.as_div()),
            "as_p" => T::Safe(f.as_p()),
            "as_table" => T::Safe(f.as_table()),
            "as_ul" => T::Safe(f.as_ul()),
            _ => return None,
        })
    }
    fn display(&self) -> T {
        T::Safe(self.formset.render())
    }
    fn iterate(&self) -> Option<Vec<T>> {
        Some(self.formset.forms().iter().map(Form::to_template).collect())
    }
}
