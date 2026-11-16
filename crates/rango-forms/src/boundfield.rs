//! `forms.boundfield`: a field bound to a form.

use std::sync::Arc;

use indexmap::IndexMap;
use rango_template::{Object, TemplateObject, Value as T};

use crate::fields::Field;
use crate::forms::Form;
use crate::utils::{ErrorList, pretty_name};
use crate::value::Value;
use crate::widgets::{Attr, Attrs, BoundWidget, Widget, WidgetKind};

/// `BoundField`: `form[name]`.
#[derive(Clone)]
pub struct BoundField<'a> {
    pub form: &'a Form,
    pub field: &'a Field,
    pub name: String,
    pub html_name: String,
    pub html_initial_name: String,
    pub html_initial_id: String,
    pub label: String,
    pub help_text: String,
}

impl<'a> BoundField<'a> {
    pub(crate) fn new(form: &'a Form, field: &'a Field, name: &str) -> Self {
        let html_name = form.add_prefix(name);
        let mut bf = BoundField {
            form,
            field,
            name: name.to_owned(),
            html_initial_name: form.add_initial_prefix(name),
            html_initial_id: String::new(),
            label: field.label.clone().unwrap_or_else(|| pretty_name(name)),
            help_text: field.help_text.clone(),
            html_name,
        };
        bf.html_initial_id = form.add_initial_prefix(&bf.auto_id());
        bf
    }

    /// `auto_id`: `id_<html_name>` per the form's `auto_id` pattern, or `""`.
    pub fn auto_id(&self) -> String {
        match &self.form.auto_id {
            Some(a) if a.contains("%s") => a.replacen("%s", &self.html_name, 1),
            Some(a) if !a.is_empty() => self.html_name.clone(),
            _ => String::new(),
        }
    }

    pub fn errors(&self) -> ErrorList {
        self.form.errors().get(&self.name).cloned().unwrap_or_else(|| ErrorList::new(None, self.form.renderer.clone()))
    }

    pub fn is_hidden(&self) -> bool {
        self.field.widget.is_hidden()
    }

    pub fn use_fieldset(&self) -> bool {
        self.field.widget.use_fieldset()
    }

    /// `initial`.
    pub fn initial(&self) -> Value {
        self.form.get_initial_for_field(self.field, &self.name)
    }

    /// `data`: the raw submitted value for this field.
    pub fn data(&self) -> Value {
        self.form.widget_data_value(&self.field.widget, &self.html_name)
    }

    /// `value()`: what the widget shows.
    pub fn value(&self) -> Value {
        let mut data = self.initial();
        if self.form.is_bound {
            data = self.field.bound_data(&self.data(), &data);
        }
        self.field.prepare_value(&data)
    }

    pub fn aria_describedby(&self) -> Option<String> {
        if matches!(self.field.widget.attrs.get("aria-describedby"), Some(Attr::Str(s)) if !s.is_empty()) {
            return None;
        }
        let mut parts = Vec::new();
        let id = self.auto_id();
        if !id.is_empty() && !self.is_hidden() {
            if !self.help_text.is_empty() {
                parts.push(format!("{id}_helptext"));
            }
            if !self.errors().is_empty() {
                parts.push(format!("{id}_error"));
            }
        }
        let s = parts.join(" ");
        if s.is_empty() { None } else { Some(s) }
    }

    /// `build_widget_attrs(attrs, widget)`: `required`, `disabled`, `aria-*`.
    pub fn build_widget_attrs(&self, attrs: &Attrs, widget: Option<&Widget>) -> Attrs {
        let widget = widget.unwrap_or(&self.field.widget);
        let mut attrs = attrs.clone();
        if widget.use_required_attribute(&self.initial()) && self.field.required && self.form.use_required_attribute {
            attrs.insert("required".into(), true.into());
        }
        if self.field.disabled {
            attrs.insert("disabled".into(), true.into());
        }
        if !widget.is_hidden() && !self.errors().is_empty() {
            attrs.insert("aria-invalid".into(), "true".into());
        }
        let has_desc = matches!(attrs.get("aria-describedby"), Some(Attr::Str(s)) if !s.is_empty());
        if !has_desc && !self.use_fieldset() {
            if let Some(d) = self.aria_describedby() {
                attrs.insert("aria-describedby".into(), d.into());
            }
        }
        attrs
    }

    /// `as_widget(widget=None, attrs=None, only_initial=False)`.
    pub fn as_widget(&self, widget: Option<&Widget>, attrs: Option<&Attrs>, only_initial: bool) -> String {
        let mut owned;
        let widget = match widget {
            Some(w) => w,
            None => {
                owned = self.field.widget.clone();
                &{
                    if self.field.localize {
                        owned.is_localized = true;
                    }
                    owned
                }
            }
        };
        let mut attrs = self.build_widget_attrs(&attrs.cloned().unwrap_or_default(), Some(widget));
        let id = self.auto_id();
        if !id.is_empty() && !widget.attrs.contains_key("id") {
            attrs.entry("id".into()).or_insert_with(|| (if only_initial { self.html_initial_id.clone() } else { id }).into());
        }
        let value = if only_initial && self.form.data.contains_key(&self.html_initial_name) {
            self.form.widget_data_value(&self.field.hidden_widget(), &self.html_initial_name)
        } else {
            self.value()
        };
        widget.render(if only_initial { &self.html_initial_name } else { &self.html_name }, &value, Some(&attrs), Some(&self.form.renderer))
    }

    pub fn as_text(&self, attrs: Option<&Attrs>) -> String {
        self.as_widget(Some(&Widget::text_input()), attrs, false)
    }

    pub fn as_textarea(&self, attrs: Option<&Attrs>) -> String {
        self.as_widget(Some(&Widget::textarea()), attrs, false)
    }

    pub fn as_hidden(&self, attrs: Option<&Attrs>, only_initial: bool) -> String {
        self.as_widget(Some(&self.field.hidden_widget()), attrs, only_initial)
    }

    pub fn has_changed(&self) -> bool {
        let initial = if self.field.show_hidden_initial {
            let v = self.form.widget_data_value(&self.field.hidden_widget(), &self.html_initial_name);
            match self.field.to_clean(&v) {
                Ok(v) => v,
                Err(_) => return true,
            }
        } else {
            self.initial()
        };
        self.field.has_changed(&initial, &self.data())
    }

    /// `id_for_label`.
    pub fn id_for_label(&self) -> String {
        let id = match self.field.widget.attrs.get("id") {
            Some(Attr::Str(s)) if !s.is_empty() => s.clone(),
            _ => self.auto_id(),
        };
        self.field.widget.id_for_label(&id)
    }

    pub fn widget_type(&self) -> String {
        let n = self.field.widget.kind.class_name().to_lowercase();
        let n = n.strip_suffix("widget").unwrap_or(&n).to_owned();
        n.strip_suffix("input").unwrap_or(&n).to_owned()
    }

    pub fn css_classes(&self, extra: Option<&str>) -> String {
        let mut classes: Vec<String> = Vec::new();
        let mut add = |c: &str| {
            if !c.is_empty() && !classes.iter().any(|x| x == c) {
                classes.push(c.to_owned());
            }
        };
        for c in extra.unwrap_or("").split_whitespace() {
            add(c);
        }
        if !self.errors().is_empty() {
            if let Some(c) = &self.form.error_css_class {
                add(c);
            }
        }
        if self.field.required {
            if let Some(c) = &self.form.required_css_class {
                add(c);
            }
        }
        classes.join(" ")
    }

    /// `label_tag(contents, attrs, label_suffix, tag)`.
    pub fn label_tag(&self, contents: Option<&str>, attrs: Option<&Attrs>, label_suffix: Option<&str>, tag: Option<&str>) -> String {
        let contents = contents.map_or_else(|| self.label.clone(), str::to_owned);
        let suffix = label_suffix.map(str::to_owned).or_else(|| self.field.label_suffix.clone()).unwrap_or_else(|| self.form.label_suffix.clone());
        // `format_html("{}{}", contents, suffix)` yields safe (already escaped) HTML.
        let label = if !suffix.is_empty() && !contents.is_empty() && !":?.!".contains(contents.chars().last().unwrap_or(' ')) {
            T::Safe(format!("{}{}", rango_core::utils::html::escape(&contents), rango_core::utils::html::escape(&suffix)))
        } else {
            T::Str(contents)
        };
        let id = match self.field.widget.attrs.get("id") {
            Some(Attr::Str(s)) if !s.is_empty() => s.clone(),
            _ => self.auto_id(),
        };
        let mut attrs = attrs.cloned();
        if !id.is_empty() {
            let for_id = self.field.widget.id_for_label(&id);
            if !for_id.is_empty() {
                let mut a = attrs.take().unwrap_or_default();
                a.insert("for".into(), for_id.into());
                attrs = Some(a);
            }
            if self.field.required {
                if let Some(rc) = &self.form.required_css_class {
                    let mut a = attrs.take().unwrap_or_default();
                    match a.get("class") {
                        Some(Attr::Str(c)) => {
                            let nc = format!("{c} {rc}");
                            a.insert("class".into(), nc.into());
                        }
                        _ => {
                            a.insert("class".into(), rc.clone().into());
                        }
                    }
                    attrs = Some(a);
                }
            }
        }
        let ctx = T::Map(
            [
                ("field".to_owned(), self.to_template()),
                ("label".to_owned(), label),
                ("attrs".to_owned(), attrs.as_ref().map_or(T::None, crate::widgets::attrs_value)),
                ("use_tag".to_owned(), T::Bool(!id.is_empty())),
                ("tag".to_owned(), T::Str(tag.unwrap_or("label").to_owned())),
            ]
            .into_iter()
            .collect(),
        );
        self.form.render_template("rango/forms/label.html", ctx)
    }

    pub fn legend_tag(&self, contents: Option<&str>, attrs: Option<&Attrs>, label_suffix: Option<&str>) -> String {
        self.label_tag(contents, attrs, label_suffix, Some("legend"))
    }

    /// `subwidgets`: one entry per option for choice widgets.
    pub fn subwidgets(&self) -> Vec<BoundWidget> {
        let id = match self.field.widget.attrs.get("id") {
            Some(Attr::Str(s)) if !s.is_empty() => s.clone(),
            _ => self.auto_id(),
        };
        let mut attrs = Attrs::new();
        if !id.is_empty() {
            attrs.insert("id".into(), id.into());
        }
        let attrs = self.build_widget_attrs(&attrs, None);
        self.field.widget.subwidgets(&self.html_name, &self.value(), Some(&attrs)).into_iter().map(|data| BoundWidget { parent: self.field.widget.clone(), data, renderer: self.form.renderer.clone() }).collect()
    }

    /// `str(bound_field)`.
    pub fn render(&self) -> String {
        if self.field.show_hidden_initial {
            return self.as_widget(None, None, false) + &self.as_hidden(None, true);
        }
        self.as_widget(None, None, false)
    }

    /// `as_field_group()`: label, help text, errors and widget through `field.html`.
    pub fn as_field_group(&self) -> String {
        let template = self.field.template_name.clone().unwrap_or_else(|| self.form.renderer.field_template_name().to_owned());
        let ctx = T::Map([("field".to_owned(), self.to_template())].into_iter().collect());
        self.form.render_template(&template, ctx)
    }

    /// A template-friendly snapshot (`Object`) of this bound field.
    pub fn to_template(&self) -> T {
        Object::new(BoundFieldObject { form: Arc::new(self.form.clone()), name: self.name.clone() })
    }
}

/// Owning, template-facing view of a [`BoundField`].
pub struct BoundFieldObject {
    pub form: Arc<Form>,
    pub name: String,
}

impl BoundFieldObject {
    fn bf(&self) -> Option<BoundField<'_>> {
        self.form.get(&self.name)
    }
}

impl TemplateObject for BoundFieldObject {
    fn get(&self, key: &str) -> Option<T> {
        let bf = self.bf()?;
        let s = |x: String| T::Str(x);
        Some(match key {
            "name" => s(bf.name.clone()),
            "html_name" => s(bf.html_name.clone()),
            "html_initial_name" => s(bf.html_initial_name.clone()),
            "html_initial_id" => s(bf.html_initial_id.clone()),
            "label" => s(bf.label.clone()),
            "help_text" => T::Safe(bf.help_text.clone()),
            "errors" => Object::new(bf.errors()),
            "is_hidden" => T::Bool(bf.is_hidden()),
            "auto_id" => s(bf.auto_id()),
            "id_for_label" => s(bf.id_for_label()),
            "use_fieldset" => T::Bool(bf.use_fieldset()),
            "label_tag" => T::Safe(bf.label_tag(None, None, None, None)),
            "legend_tag" => T::Safe(bf.legend_tag(None, None, None)),
            "css_classes" => s(bf.css_classes(None)),
            "as_widget" => T::Safe(bf.as_widget(None, None, false)),
            "as_text" => T::Safe(bf.as_text(None)),
            "as_textarea" => T::Safe(bf.as_textarea(None)),
            "as_hidden" => T::Safe(bf.as_hidden(None, false)),
            "as_field_group" => T::Safe(bf.as_field_group()),
            "aria_describedby" => bf.aria_describedby().map_or(T::None, s),
            "widget_type" => s(bf.widget_type()),
            "value" => value_to_template(&bf.value()),
            "initial" => value_to_template(&bf.initial()),
            "data" => value_to_template(&bf.data()),
            "field" => Object::new(FieldObject { field: bf.field.clone() }),
            "form" => Object::new(FormObject { form: Arc::clone(&self.form) }),
            "subwidgets" => T::List(bf.subwidgets().into_iter().map(Object::new).collect()),
            _ => return None,
        })
    }

    fn display(&self) -> T {
        self.bf().map_or(T::Str(String::new()), |b| T::Safe(b.render()))
    }

    fn iterate(&self) -> Option<Vec<T>> {
        Some(self.bf()?.subwidgets().into_iter().map(Object::new).collect())
    }
}

/// `field.required`, `field.widget`, ... as seen from templates.
pub struct FieldObject {
    pub field: Field,
}

impl TemplateObject for FieldObject {
    fn get(&self, key: &str) -> Option<T> {
        let f = &self.field;
        Some(match key {
            "required" => T::Bool(f.required),
            "disabled" => T::Bool(f.disabled),
            "label" => f.label.clone().map_or(T::None, T::Str),
            "help_text" => T::Str(f.help_text.clone()),
            "show_hidden_initial" => T::Bool(f.show_hidden_initial),
            "max_length" => f.max_length.map_or(T::None, |n| T::Int(n as i64)),
            "widget" => Object::new(WidgetObject { widget: f.widget.clone() }),
            _ => return None,
        })
    }
    fn display(&self) -> T {
        T::Str(format!("<{:?}>", self.field.kind))
    }
}

pub struct WidgetObject {
    pub widget: Widget,
}

impl TemplateObject for WidgetObject {
    fn get(&self, key: &str) -> Option<T> {
        let w = &self.widget;
        Some(match key {
            "is_hidden" => T::Bool(w.is_hidden()),
            "is_required" => T::Bool(w.is_required),
            "needs_multipart_form" => T::Bool(w.needs_multipart_form()),
            "attrs" => crate::widgets::attrs_value(&w.attrs),
            "input_type" => w.input_type().map_or(T::None, |t| T::Str(t.to_owned())),
            "template_name" => T::Str(w.template_name()),
            "use_fieldset" => T::Bool(w.use_fieldset()),
            "is_multi" => T::Bool(matches!(w.kind, WidgetKind::Multi { .. })),
            _ => return None,
        })
    }
}

/// `form` as seen from templates (`form.as_p`, `form.errors`, `{{ form }}`, iteration).
pub struct FormObject {
    pub form: Arc<Form>,
}

impl TemplateObject for FormObject {
    fn get(&self, key: &str) -> Option<T> {
        let f = &self.form;
        Some(match key {
            "as_p" => T::Safe(f.as_p()),
            "as_table" => T::Safe(f.as_table()),
            "as_ul" => T::Safe(f.as_ul()),
            "as_div" => T::Safe(f.as_div()),
            "errors" => Object::new(f.errors().clone()),
            "non_field_errors" => Object::new(f.non_field_errors()),
            "is_bound" => T::Bool(f.is_bound),
            "is_valid" => T::Bool(f.is_valid()),
            "is_multipart" => T::Bool(f.is_multipart()),
            "media" => Object::new(f.media()),
            "hidden_fields" => T::List(f.hidden_fields().iter().map(BoundField::to_template).collect()),
            "visible_fields" => T::List(f.visible_fields().iter().map(BoundField::to_template).collect()),
            "prefix" => f.prefix.clone().map_or(T::None, T::Str),
            "auto_id" => f.auto_id.clone().map_or(T::Bool(false), T::Str),
            "cleaned_data" => T::Map(f.cleaned_data().iter().map(|(k, v)| (k.clone(), value_to_template(v))).collect()),
            name => return f.get(name).map(|b| b.to_template()),
        })
    }
    fn display(&self) -> T {
        T::Safe(self.form.render())
    }
    fn iterate(&self) -> Option<Vec<T>> {
        Some(self.form.bound_fields().iter().map(BoundField::to_template).collect())
    }
}

/// A form [`Value`] as a template value.
pub fn value_to_template(v: &Value) -> T {
    match v {
        Value::None | Value::Cleared => T::None,
        Value::Bool(b) => T::Bool(*b),
        Value::Int(i) => T::Int(*i),
        Value::Float(f) => T::Float(*f),
        Value::Decimal(d) => T::Decimal(*d),
        Value::Str(s) => T::Str(s.clone()),
        Value::Date(d) => T::Date(*d),
        Value::Time(t) => T::Time(*t),
        Value::NaiveDateTime(d) => T::DateTime(*d, None),
        Value::DateTime(d) => T::DateTime(d.naive_utc(), Some(0)),
        Value::List(l) => T::List(l.iter().map(value_to_template).collect()),
        Value::Json(j) => T::from(j.clone()),
        other => T::Str(other.to_text()),
    }
}

/// Build the template context of `Form.get_context()`.
pub(crate) fn form_context(form: &Form) -> IndexMap<String, T> {
    let mut fields = Vec::new();
    let mut hidden = Vec::new();
    let mut top_errors = form.non_field_errors();
    for bf in form.bound_fields() {
        if bf.is_hidden() {
            for e in bf.errors().messages() {
                top_errors.push(rango_core::exceptions::ValidationError::new(
                    rango_core::utils::translation::gettext("(Hidden field %(name)s) %(error)s").replace("%(name)s", &bf.name).replace("%(error)s", &e),
                ));
            }
            hidden.push(bf.to_template());
        } else {
            fields.push(T::List(vec![bf.to_template(), Object::new(bf.errors())]));
        }
    }
    let mut m = IndexMap::new();
    m.insert("form".to_owned(), Object::new(FormObject { form: Arc::new(form.clone()) }));
    m.insert("fields".to_owned(), T::List(fields));
    m.insert("hidden_fields".to_owned(), T::List(hidden));
    m.insert("errors".to_owned(), Object::new(top_errors));
    m
}
