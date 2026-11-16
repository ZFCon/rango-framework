//! `forms.widgets`: widgets, `Media`, choices.
//!
//! A [`Widget`] is data (a [`WidgetKind`] plus attrs); rendering goes through the form
//! renderer's templates exactly as the does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, NaiveDateTime};
use indexmap::IndexMap;
use rango_core::conf::settings;
use rango_core::utils::translation::gettext;
use rango_core::utils::{datastructures::MultiValueDict, html::escape};
use rango_http::UploadedFile;
use rango_template::{Object, TemplateObject, Value as T};

use crate::renderers::{Renderer, get_default_renderer};
use crate::utils::{flatatt, to_current_timezone};
use crate::value::Value;

pub type Data = MultiValueDict<String>;
pub type FileData = MultiValueDict<UploadedFile>;

/// An HTML attribute value: text, or a boolean flag (`required`, `disabled`, ...).
#[derive(Debug, Clone, PartialEq)]
pub enum Attr {
    Str(String),
    Bool(bool),
}
pub type Attrs = IndexMap<String, Attr>;

impl From<&str> for Attr {
    fn from(s: &str) -> Self {
        Attr::Str(s.to_owned())
    }
}
impl From<String> for Attr {
    fn from(s: String) -> Self {
        Attr::Str(s)
    }
}
impl From<bool> for Attr {
    fn from(b: bool) -> Self {
        Attr::Bool(b)
    }
}
impl From<i64> for Attr {
    fn from(i: i64) -> Self {
        Attr::Str(i.to_string())
    }
}

fn attr_value(a: &Attr) -> T {
    match a {
        Attr::Str(s) => T::Str(s.clone()),
        Attr::Bool(b) => T::Bool(*b),
    }
}

pub(crate) fn attrs_value(a: &Attrs) -> T {
    T::Map(a.iter().map(|(k, v)| (k.clone(), attr_value(v))).collect())
}

/// `attrs! { "class" => "big", "required" => true }`.
#[macro_export]
macro_rules! attrs {
    ($($k:expr => $v:expr),* $(,)?) => {{
        let mut a = $crate::widgets::Attrs::new();
        $(a.insert($k.to_string(), $crate::widgets::Attr::from($v));)*
        a
    }};
}

/// One entry of `choices`: an option, or a named group of options (`optgroup`).
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Option(Value, String),
    Group(String, Vec<(Value, String)>),
}

impl Choice {
    pub fn option(value: impl Into<Value>, label: impl Into<String>) -> Self {
        Choice::Option(value.into(), label.into())
    }
}

/// `normalize_choices`: `[(value, label), ...]`.
pub fn choices<V: Into<Value>, L: Into<String>>(items: impl IntoIterator<Item = (V, L)>) -> Vec<Choice> {
    items.into_iter().map(|(v, l)| Choice::Option(v.into(), l.into())).collect()
}

/// `CheckboxInput.check_test`.
pub type CheckTest = Arc<dyn Fn(&Value) -> bool + Send + Sync>;

#[derive(Debug, Default)]
pub struct Flag(AtomicBool);
impl Clone for Flag {
    fn clone(&self) -> Self {
        Flag(AtomicBool::new(self.0.load(Ordering::Relaxed)))
    }
}

/// How a [`MultiWidget`](WidgetKind::Multi) turns one value into one value per subwidget.
#[derive(Clone)]
pub enum Decompress {
    /// `SplitDateTimeWidget.decompress`.
    SplitDateTime,
    Custom(Arc<dyn Fn(&Value) -> Vec<Value> + Send + Sync>),
}

#[derive(Clone)]
pub enum WidgetKind {
    Text,
    Number,
    Email,
    Url,
    Color,
    Search,
    Tel,
    Password { render_value: bool },
    Hidden,
    MultipleHidden,
    File { multiple: bool },
    ClearableFile { checked: Flag },
    Textarea,
    Date { format: Option<String> },
    DateTime { format: Option<String> },
    Time { format: Option<String> },
    Checkbox { check_test: Option<CheckTest> },
    Select,
    NullBooleanSelect,
    SelectMultiple,
    Radio,
    CheckboxSelectMultiple,
    Multi { widgets: Vec<Widget>, names: Vec<String>, decompress: Decompress, template: &'static str },
    SelectDate { years: Vec<i32>, months: Vec<(i32, String)>, none_values: [(String, String); 3] },
}

impl std::fmt::Debug for WidgetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.class_name())
    }
}

impl WidgetKind {
    /// The widget class name (`BoundField.widget_type` is derived from it).
    pub fn class_name(&self) -> String {
        match self {
            WidgetKind::Text => "TextInput",
            WidgetKind::Number => "NumberInput",
            WidgetKind::Email => "EmailInput",
            WidgetKind::Url => "URLInput",
            WidgetKind::Color => "ColorInput",
            WidgetKind::Search => "SearchInput",
            WidgetKind::Tel => "TelInput",
            WidgetKind::Password { .. } => "PasswordInput",
            WidgetKind::Hidden => "HiddenInput",
            WidgetKind::MultipleHidden => "MultipleHiddenInput",
            WidgetKind::File { .. } => "FileInput",
            WidgetKind::ClearableFile { .. } => "ClearableFileInput",
            WidgetKind::Textarea => "Textarea",
            WidgetKind::Date { .. } => "DateInput",
            WidgetKind::DateTime { .. } => "DateTimeInput",
            WidgetKind::Time { .. } => "TimeInput",
            WidgetKind::Checkbox { .. } => "CheckboxInput",
            WidgetKind::Select => "Select",
            WidgetKind::NullBooleanSelect => "NullBooleanSelect",
            WidgetKind::SelectMultiple => "SelectMultiple",
            WidgetKind::Radio => "RadioSelect",
            WidgetKind::CheckboxSelectMultiple => "CheckboxSelectMultiple",
            WidgetKind::Multi { template, .. } => match *template {
                "rango/forms/widgets/splitdatetime.html" => "SplitDateTimeWidget",
                "rango/forms/widgets/splithiddendatetime.html" => "SplitHiddenDateTimeWidget",
                _ => "MultiWidget",
            },
            WidgetKind::SelectDate { .. } => "SelectDateWidget",
        }
        .to_owned()
    }
}

/// A form widget.
#[derive(Debug, Clone)]
pub struct Widget {
    pub kind: WidgetKind,
    pub attrs: Attrs,
    pub is_required: bool,
    pub is_localized: bool,
    pub choices: Vec<Choice>,
    /// `Input.input_type` override (from `attrs["type"]` or a hidden variant).
    pub input_type: Option<String>,
    /// Replaces the class's `template_name`.
    pub template_name_override: Option<String>,
    pub media: Media,
}

macro_rules! ctor {
    ($($fn:ident => $kind:expr),* $(,)?) => {$(
        pub fn $fn() -> Widget { Widget::of($kind) }
    )*};
}

impl Widget {
    fn of(kind: WidgetKind) -> Widget {
        let mut w = Widget { kind, attrs: Attrs::new(), is_required: false, is_localized: false, choices: Vec::new(), input_type: None, template_name_override: None, media: Media::default() };
        match &w.kind {
            WidgetKind::Textarea => {
                w.attrs.insert("cols".into(), "40".into());
                w.attrs.insert("rows".into(), "10".into());
            }
            WidgetKind::File { multiple: true } => {
                w.attrs.insert("multiple".into(), true.into());
            }
            WidgetKind::NullBooleanSelect => {
                w.choices = choices([("unknown", gettext("Unknown")), ("true", gettext("Yes")), ("false", gettext("No"))]);
            }
            _ => {}
        }
        w
    }

    ctor!(text_input => WidgetKind::Text, number_input => WidgetKind::Number, email_input => WidgetKind::Email, url_input => WidgetKind::Url,
        color_input => WidgetKind::Color, search_input => WidgetKind::Search, tel_input => WidgetKind::Tel, hidden_input => WidgetKind::Hidden,
        multiple_hidden_input => WidgetKind::MultipleHidden, textarea => WidgetKind::Textarea, select => WidgetKind::Select,
        null_boolean_select => WidgetKind::NullBooleanSelect, select_multiple => WidgetKind::SelectMultiple, radio_select => WidgetKind::Radio,
        checkbox_select_multiple => WidgetKind::CheckboxSelectMultiple);

    pub fn password_input() -> Widget {
        Widget::of(WidgetKind::Password { render_value: false })
    }
    pub fn file_input() -> Widget {
        Widget::of(WidgetKind::File { multiple: false })
    }
    pub fn clearable_file_input() -> Widget {
        Widget::of(WidgetKind::ClearableFile { checked: Flag::default() })
    }
    pub fn date_input() -> Widget {
        Widget::of(WidgetKind::Date { format: None })
    }
    pub fn datetime_input() -> Widget {
        Widget::of(WidgetKind::DateTime { format: None })
    }
    pub fn time_input() -> Widget {
        Widget::of(WidgetKind::Time { format: None })
    }
    pub fn checkbox_input() -> Widget {
        Widget::of(WidgetKind::Checkbox { check_test: None })
    }

    /// `MultiWidget(widgets)` with names `_0`, `_1`, ...
    pub fn multi(widgets: Vec<Widget>, decompress: impl Fn(&Value) -> Vec<Value> + Send + Sync + 'static) -> Widget {
        let names = (0..widgets.len()).map(|i| format!("_{i}")).collect();
        Widget::of(WidgetKind::Multi { widgets, names, decompress: Decompress::Custom(Arc::new(decompress)), template: "rango/forms/widgets/multiwidget.html" })
    }

    fn split(hidden: bool) -> Widget {
        let (mut d, mut t) = (Widget::date_input(), Widget::time_input());
        if hidden {
            d.input_type = Some("hidden".into());
            t.input_type = Some("hidden".into());
        }
        let template = if hidden { "rango/forms/widgets/splithiddendatetime.html" } else { "rango/forms/widgets/splitdatetime.html" };
        Widget::of(WidgetKind::Multi { widgets: vec![d, t], names: vec!["_0".into(), "_1".into()], decompress: Decompress::SplitDateTime, template })
    }
    pub fn split_datetime() -> Widget {
        Widget::split(false)
    }
    pub fn split_hidden_datetime() -> Widget {
        Widget::split(true)
    }

    /// `SelectDateWidget(years=..., months=..., empty_label=...)`.
    pub fn select_date(years: Option<Vec<i32>>) -> Widget {
        let this_year = chrono::Utc::now().year();
        let years = years.unwrap_or_else(|| (this_year..this_year + 10).collect());
        let months = (1..=12).map(|m| (m, month_name(m))).collect();
        let none = ("".to_owned(), "---".to_owned());
        Widget::of(WidgetKind::SelectDate { years, months, none_values: [none.clone(), none.clone(), none] })
    }

    // ---- builder-style configuration

    pub fn with_attrs(mut self, attrs: Attrs) -> Self {
        for (k, v) in attrs {
            if k == "type" && self.is_input() {
                if let Attr::Str(s) = v {
                    self.input_type = Some(s);
                }
                continue;
            }
            self.attrs.insert(k, v);
        }
        self
    }

    pub fn with_choices(mut self, c: Vec<Choice>) -> Self {
        self.choices = c;
        self
    }

    pub fn with_format(mut self, f: &str) -> Self {
        match &mut self.kind {
            WidgetKind::Date { format } | WidgetKind::DateTime { format } | WidgetKind::Time { format } => *format = Some(f.to_owned()),
            _ => {}
        }
        self
    }

    pub fn render_value(mut self, rv: bool) -> Self {
        if let WidgetKind::Password { render_value } = &mut self.kind {
            *render_value = rv;
        }
        self
    }

    pub fn with_template(mut self, name: &str) -> Self {
        self.template_name_override = Some(name.to_owned());
        self
    }

    pub fn with_media(mut self, m: Media) -> Self {
        self.media = m;
        self
    }

    // ---- class-level properties

    fn is_input(&self) -> bool {
        matches!(
            self.kind,
            WidgetKind::Text | WidgetKind::Number | WidgetKind::Email | WidgetKind::Url | WidgetKind::Color | WidgetKind::Search | WidgetKind::Tel | WidgetKind::Password { .. } | WidgetKind::Hidden | WidgetKind::MultipleHidden | WidgetKind::File { .. } | WidgetKind::ClearableFile { .. } | WidgetKind::Date { .. } | WidgetKind::DateTime { .. } | WidgetKind::Time { .. } | WidgetKind::Checkbox { .. }
        )
    }

    /// `Input.input_type` (`None` for widgets without one).
    pub fn input_type(&self) -> Option<&str> {
        if let Some(t) = &self.input_type {
            return Some(t);
        }
        Some(match self.kind {
            WidgetKind::Text | WidgetKind::Date { .. } | WidgetKind::DateTime { .. } | WidgetKind::Time { .. } => "text",
            WidgetKind::Number => "number",
            WidgetKind::Email => "email",
            WidgetKind::Url => "url",
            WidgetKind::Color => "color",
            WidgetKind::Search => "search",
            WidgetKind::Tel => "tel",
            WidgetKind::Password { .. } => "password",
            WidgetKind::Hidden | WidgetKind::MultipleHidden => "hidden",
            WidgetKind::File { .. } | WidgetKind::ClearableFile { .. } => "file",
            WidgetKind::Checkbox { .. } => "checkbox",
            WidgetKind::Select | WidgetKind::NullBooleanSelect | WidgetKind::SelectMultiple | WidgetKind::SelectDate { .. } => "select",
            WidgetKind::Radio => "radio",
            WidgetKind::CheckboxSelectMultiple => "checkbox",
            _ => return None,
        })
    }

    pub fn is_hidden(&self) -> bool {
        match &self.kind {
            WidgetKind::Multi { widgets, .. } => widgets.iter().all(Widget::is_hidden),
            _ => self.input_type() == Some("hidden"),
        }
    }

    pub fn template_name(&self) -> String {
        if let Some(t) = &self.template_name_override {
            return t.clone();
        }
        match &self.kind {
            WidgetKind::Text => "rango/forms/widgets/text.html",
            WidgetKind::Number => "rango/forms/widgets/number.html",
            WidgetKind::Email => "rango/forms/widgets/email.html",
            WidgetKind::Url => "rango/forms/widgets/url.html",
            WidgetKind::Color => "rango/forms/widgets/color.html",
            WidgetKind::Search => "rango/forms/widgets/search.html",
            WidgetKind::Tel => "rango/forms/widgets/tel.html",
            WidgetKind::Password { .. } => "rango/forms/widgets/password.html",
            WidgetKind::Hidden => "rango/forms/widgets/hidden.html",
            WidgetKind::MultipleHidden => "rango/forms/widgets/multiple_hidden.html",
            WidgetKind::File { .. } => "rango/forms/widgets/file.html",
            WidgetKind::ClearableFile { .. } => "rango/forms/widgets/clearable_file_input.html",
            WidgetKind::Textarea => "rango/forms/widgets/textarea.html",
            WidgetKind::Date { .. } => "rango/forms/widgets/date.html",
            WidgetKind::DateTime { .. } => "rango/forms/widgets/datetime.html",
            WidgetKind::Time { .. } => "rango/forms/widgets/time.html",
            WidgetKind::Checkbox { .. } => "rango/forms/widgets/checkbox.html",
            WidgetKind::Select | WidgetKind::NullBooleanSelect | WidgetKind::SelectMultiple => "rango/forms/widgets/select.html",
            WidgetKind::Radio => "rango/forms/widgets/radio.html",
            WidgetKind::CheckboxSelectMultiple => "rango/forms/widgets/checkbox_select.html",
            WidgetKind::Multi { template, .. } => template,
            WidgetKind::SelectDate { .. } => "rango/forms/widgets/select_date.html",
        }
        .to_owned()
    }

    fn option_template_name(&self) -> &'static str {
        match self.kind {
            WidgetKind::Select | WidgetKind::NullBooleanSelect | WidgetKind::SelectMultiple => "rango/forms/widgets/select_option.html",
            WidgetKind::Radio => "rango/forms/widgets/radio_option.html",
            _ => "rango/forms/widgets/checkbox_option.html",
        }
    }

    pub fn is_choice_widget(&self) -> bool {
        matches!(self.kind, WidgetKind::Select | WidgetKind::NullBooleanSelect | WidgetKind::SelectMultiple | WidgetKind::Radio | WidgetKind::CheckboxSelectMultiple)
    }

    fn allow_multiple_selected(&self) -> bool {
        matches!(self.kind, WidgetKind::SelectMultiple | WidgetKind::CheckboxSelectMultiple) || matches!(self.kind, WidgetKind::File { multiple: true })
    }

    pub fn needs_multipart_form(&self) -> bool {
        match &self.kind {
            WidgetKind::File { .. } | WidgetKind::ClearableFile { .. } => true,
            WidgetKind::Multi { widgets, .. } => widgets.iter().any(Widget::needs_multipart_form),
            _ => false,
        }
    }

    pub fn supports_microseconds(&self) -> bool {
        !matches!(self.kind, WidgetKind::Date { .. } | WidgetKind::DateTime { .. } | WidgetKind::Time { .. })
            && !matches!(&self.kind, WidgetKind::Multi { template, .. } if template.contains("datetime"))
    }

    pub fn use_fieldset(&self) -> bool {
        matches!(self.kind, WidgetKind::Radio | WidgetKind::CheckboxSelectMultiple | WidgetKind::Multi { .. } | WidgetKind::SelectDate { .. })
    }

    /// `Widget.media`, including subwidgets.
    pub fn media(&self) -> Media {
        match &self.kind {
            WidgetKind::Multi { widgets, .. } => widgets.iter().fold(self.media.clone(), |m, w| m + w.media()),
            _ => self.media.clone(),
        }
    }

    pub fn build_attrs(&self, base: &Attrs, extra: Option<&Attrs>) -> Attrs {
        let mut a = base.clone();
        if let Some(e) = extra {
            for (k, v) in e {
                a.insert(k.clone(), v.clone());
            }
        }
        a
    }

    // ---- values

    fn date_format(&self) -> String {
        use rango_core::utils::formats::get_format_list;
        let (fmt, key) = match &self.kind {
            WidgetKind::Date { format } => (format, "DATE_INPUT_FORMATS"),
            WidgetKind::DateTime { format } => (format, "DATETIME_INPUT_FORMATS"),
            WidgetKind::Time { format } => (format, "TIME_INPUT_FORMATS"),
            _ => unreachable!("date_format on a non-temporal widget"),
        };
        fmt.clone().or_else(|| get_format_list(key).first().cloned()).unwrap_or_default()
    }

    /// `format_value(value)`: what the template shows (`None`, text, or a list of text).
    pub fn format_value(&self, value: &Value) -> T {
        match &self.kind {
            WidgetKind::File { .. } => T::None,
            WidgetKind::ClearableFile { .. } => T::None,
            WidgetKind::Checkbox { .. } => match value {
                Value::Bool(_) | Value::None => T::None,
                Value::Str(s) if s.is_empty() => T::None,
                v => T::Str(v.to_text()),
            },
            WidgetKind::MultipleHidden => match value {
                Value::None => T::List(vec![]),
                Value::List(l) => T::List(l.iter().map(|v| T::Str(v.to_text())).collect()),
                v => T::List(vec![T::Str(v.to_text())]),
            },
            WidgetKind::Date { .. } | WidgetKind::DateTime { .. } | WidgetKind::Time { .. } => match value {
                Value::None => T::None,
                Value::Str(s) if s.is_empty() => T::None,
                Value::Str(s) => T::Str(s.clone()),
                v => T::Str(strftime_value(v, &self.date_format())),
            },
            WidgetKind::NullBooleanSelect => T::List(vec![T::Str(
                match value {
                    Value::Bool(true) => "true",
                    Value::Bool(false) => "false",
                    Value::Str(s) if s == "true" || s == "2" => "true",
                    Value::Str(s) if s == "false" || s == "3" => "false",
                    _ => "unknown",
                }
                .into(),
            )]),
            WidgetKind::Select | WidgetKind::SelectMultiple | WidgetKind::Radio | WidgetKind::CheckboxSelectMultiple => match value {
                Value::None if self.allow_multiple_selected() => T::List(vec![]),
                Value::List(l) => T::List(l.iter().map(|v| T::Str(if matches!(v, Value::None) { String::new() } else { v.to_text() })).collect()),
                Value::None => T::List(vec![T::Str(String::new())]),
                v => T::List(vec![T::Str(v.to_text())]),
            },
            WidgetKind::SelectDate { .. } => self.select_date_parts(value),
            WidgetKind::Password { render_value: false } => T::None,
            _ => match value {
                Value::None => T::None,
                Value::Str(s) if s.is_empty() => T::None,
                v => T::Str(v.to_text()),
            },
        }
    }

    fn select_date_parts(&self, value: &Value) -> T {
        let (mut y, mut m, mut d): (Option<i64>, Option<i64>, Option<i64>) = (None, None, None);
        let mut set = |yy: i32, mm: u32, dd: u32| {
            y = Some(i64::from(yy));
            m = Some(i64::from(mm));
            d = Some(i64::from(dd));
        };
        match value {
            Value::Date(x) => set(x.year(), x.month(), chrono::Datelike::day(x)),
            Value::NaiveDateTime(x) => set(x.year(), x.month(), x.day()),
            Value::DateTime(x) => set(x.year(), x.month(), x.day()),
            Value::Str(s) => {
                let re = regex::Regex::new(r"^(\d{4}|0)-(\d\d?)-(\d\d?)$").expect("static regex");
                if let Some(c) = re.captures(s) {
                    let g = |i: usize| c[i].parse::<i64>().ok().filter(|v| *v != 0);
                    (y, m, d) = (g(1), g(2), g(3));
                } else if let Some(first) = rango_core::utils::formats::get_format_list("DATE_INPUT_FORMATS").first() {
                    if let Ok(x) = chrono::NaiveDate::parse_from_str(s, first) {
                        set(x.year(), x.month(), chrono::Datelike::day(&x));
                    }
                }
            }
            _ => {}
        }
        let f = |v: Option<i64>| v.map_or(T::None, T::Int);
        T::Map([("year".to_owned(), f(y)), ("month".to_owned(), f(m)), ("day".to_owned(), f(d))].into_iter().collect())
    }

    /// `get_context(name, value, attrs)["widget"]`.
    pub fn get_context(&self, name: &str, value: &Value, attrs: Option<&Attrs>) -> IndexMap<String, T> {
        // Per-kind adjustments that happen before the base context is built.
        let mut value = value.clone();
        let mut extra_attrs = attrs.cloned();
        match &self.kind {
            WidgetKind::Password { render_value: false } => value = Value::None,
            WidgetKind::Checkbox { check_test } => {
                let checked = match check_test {
                    Some(f) => f(&value),
                    None => !matches!(&value, Value::Bool(false) | Value::None) && !matches!(&value, Value::Str(s) if s.is_empty()),
                };
                if checked {
                    extra_attrs.get_or_insert_with(Attrs::new).insert("checked".into(), true.into());
                }
            }
            _ => {}
        }
        let mut final_attrs = self.build_attrs(&self.attrs, extra_attrs.as_ref());
        let formatted = self.format_value(&value);
        let mut w: IndexMap<String, T> = IndexMap::new();
        w.insert("name".into(), T::Str(name.to_owned()));
        w.insert("is_hidden".into(), T::Bool(self.is_hidden()));
        w.insert("required".into(), T::Bool(self.is_required));
        w.insert("value".into(), formatted.clone());

        match &self.kind {
            WidgetKind::ClearableFile { checked } => {
                let cb_name = format!("{name}-clear");
                let cb_id = format!("{cb_name}_id");
                let is_initial = matches!(&value, Value::Str(s) if !s.is_empty());
                final_attrs.entry("disabled".into()).or_insert(Attr::Bool(false));
                final_attrs.insert("checked".into(), Attr::Bool(checked.0.load(Ordering::Relaxed)));
                w.insert("checkbox_name".into(), T::Str(cb_name));
                w.insert("checkbox_id".into(), T::Str(cb_id));
                w.insert("is_initial".into(), T::Bool(is_initial));
                w.insert("input_text".into(), T::Str(gettext("Change")));
                w.insert("initial_text".into(), T::Str(gettext("Currently")));
                w.insert("clear_checkbox_label".into(), T::Str(gettext("Clear")));
                if is_initial {
                    w.insert("value".into(), T::Str(value.to_text()));
                }
            }
            WidgetKind::Multi { .. } => {
                return self.multi_context(name, &value, final_attrs, w);
            }
            WidgetKind::SelectDate { .. } => {
                return self.select_date_context(name, final_attrs, formatted, w);
            }
            _ => {}
        }

        if self.is_choice_widget() {
            if matches!(self.kind, WidgetKind::SelectMultiple) {
                final_attrs.insert("multiple".into(), true.into());
            }
            let selected = match &formatted {
                T::List(l) => l.iter().map(T::to_str).collect::<Vec<_>>(),
                _ => vec![],
            };
            w.insert("optgroups".into(), self.optgroups_value(name, &selected, attrs));
        }
        w.insert("attrs".into(), attrs_value(&final_attrs));
        w.insert("template_name".into(), T::Str(self.template_name()));
        if self.is_input() || matches!(self.kind, WidgetKind::MultipleHidden) {
            w.insert("type".into(), self.input_type().map_or(T::None, |t| T::Str(t.to_owned())));
        }
        if matches!(self.kind, WidgetKind::MultipleHidden) {
            let id = match final_attrs.get("id") {
                Some(Attr::Str(s)) => Some(s.clone()),
                _ => None,
            };
            let mut subs = Vec::new();
            if let T::List(vals) = &formatted {
                for (i, v) in vals.iter().enumerate() {
                    let mut a = final_attrs.clone();
                    if let Some(id) = &id {
                        a.insert("id".into(), format!("{id}_{i}").into());
                    }
                    let mut h = Widget::hidden_input();
                    h.is_required = self.is_required;
                    subs.push(T::Map(h.get_context(name, &Value::Str(v.to_str()), Some(&a))));
                }
            }
            w.insert("subwidgets".into(), T::List(subs));
        }
        w
    }

    fn multi_context(&self, name: &str, value: &Value, mut final_attrs: Attrs, mut w: IndexMap<String, T>) -> IndexMap<String, T> {
        let WidgetKind::Multi { widgets, names, decompress, .. } = &self.kind else { unreachable!() };
        let values: Vec<Value> = match value {
            Value::List(l) => l.clone(),
            v => match decompress {
                Decompress::SplitDateTime => split_decompress(v),
                Decompress::Custom(f) => f(v),
            },
        };
        let input_type = match final_attrs.shift_remove("type") {
            Some(Attr::Str(s)) => Some(s),
            _ => None,
        };
        let id = match final_attrs.get("id") {
            Some(Attr::Str(s)) => Some(s.clone()),
            _ => None,
        };
        let mut subs = Vec::new();
        for (i, (wn, widget)) in names.iter().zip(widgets).enumerate() {
            let mut widget = widget.clone();
            if let Some(t) = &input_type {
                widget.input_type = Some(t.clone());
            }
            if self.is_localized {
                widget.is_localized = true;
            }
            let mut a = final_attrs.clone();
            if let Some(id) = &id {
                a.insert("id".into(), format!("{id}_{i}").into());
            }
            let v = values.get(i).cloned().unwrap_or(Value::None);
            subs.push(T::Map(widget.get_context(&format!("{name}{wn}"), &v, Some(&a))));
        }
        w.insert("attrs".into(), attrs_value(&final_attrs));
        w.insert("template_name".into(), T::Str(self.template_name()));
        w.insert("subwidgets".into(), T::List(subs));
        w
    }

    fn select_date_context(&self, name: &str, final_attrs: Attrs, formatted: T, mut w: IndexMap<String, T>) -> IndexMap<String, T> {
        let WidgetKind::SelectDate { years, months, none_values } = &self.kind else { unreachable!() };
        let T::Map(parts) = &formatted else { unreachable!() };
        let part = |k: &str| parts.get(k).map_or(Value::None, |v| v.as_i64().map_or(Value::None, Value::Int));
        let sub = |field: &str, choices: Vec<Choice>, val: Value| {
            let field_name = format!("{name}_{field}");
            let mut a = final_attrs.clone();
            a.insert("id".into(), format!("id_{field_name}").into());
            let mut sel = Widget::select().with_choices(choices);
            sel.attrs = self.attrs.clone();
            T::Map(sel.get_context(&field_name, &val, Some(&a)))
        };
        let with_none = |mut c: Vec<Choice>, none: &(String, String)| {
            if !self.is_required {
                c.insert(0, Choice::Option(Value::Str(none.0.clone()), none.1.clone()));
            }
            c
        };
        let ychoices = with_none(years.iter().map(|y| Choice::Option(Value::Int(i64::from(*y)), y.to_string())).collect(), &none_values[0]);
        let mchoices = with_none(months.iter().map(|(i, n)| Choice::Option(Value::Int(i64::from(*i)), n.clone())).collect(), &none_values[1]);
        let dchoices = with_none((1..=31).map(|d| Choice::Option(Value::Int(d), d.to_string())).collect(), &none_values[2]);
        let mut ctx = IndexMap::new();
        ctx.insert("year", sub("year", ychoices, part("year")));
        ctx.insert("month", sub("month", mchoices, part("month")));
        ctx.insert("day", sub("day", dchoices, part("day")));
        let mut subs = Vec::new();
        for f in parse_date_fmt() {
            let T::Map(c) = &ctx[f] else { continue };
            subs.push(T::Map(c.clone()));
        }
        w.insert("attrs".into(), attrs_value(&final_attrs));
        w.insert("template_name".into(), T::Str(self.template_name()));
        w.insert("subwidgets".into(), T::List(subs));
        w
    }

    fn optgroups_value(&self, name: &str, selected: &[String], attrs: Option<&Attrs>) -> T {
        let mut groups = Vec::new();
        let mut has_selected = false;
        for (index, c) in self.choices.iter().enumerate() {
            let (group_name, items, mut subindex): (Option<String>, Vec<(Value, String)>, Option<usize>) = match c {
                Choice::Option(v, l) => (None, vec![(if matches!(v, Value::None) { Value::Str(String::new()) } else { v.clone() }, l.clone())], None),
                Choice::Group(g, items) => (Some(g.clone()), items.clone(), Some(0)),
            };
            let mut subgroup = Vec::new();
            for (sv, sl) in items {
                let sel = (!has_selected || self.allow_multiple_selected()) && selected.iter().any(|s| *s == sv.to_text());
                has_selected |= sel;
                subgroup.push(self.create_option(name, &sv, &sl, sel, index, subindex, attrs));
                if let Some(si) = subindex.as_mut() {
                    *si += 1;
                }
            }
            groups.push(T::List(vec![group_name.map_or(T::None, T::Str), T::List(subgroup), T::Int(index as i64)]));
        }
        T::List(groups)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_option(&self, name: &str, value: &Value, label: &str, selected: bool, index: usize, subindex: Option<usize>, attrs: Option<&Attrs>) -> T {
        let index_s = match subindex {
            None => index.to_string(),
            Some(si) => format!("{index}_{si}"),
        };
        let inherits = !matches!(self.kind, WidgetKind::Select | WidgetKind::NullBooleanSelect | WidgetKind::SelectMultiple);
        let mut option_attrs = if inherits { self.build_attrs(&self.attrs, attrs) } else { Attrs::new() };
        if selected {
            let key = if inherits { "checked" } else { "selected" };
            option_attrs.insert(key.into(), true.into());
        }
        if let Some(Attr::Str(id)) = option_attrs.get("id").cloned() {
            option_attrs.insert("id".into(), self.id_for_label_idx(&id, Some(&index_s)).into());
        }
        let value_t = match value {
            Value::Int(i) => T::Int(*i),
            Value::Str(s) => T::Str(s.clone()),
            other => T::Str(other.to_text()),
        };
        T::Map(
            [
                ("name", T::Str(name.to_owned())),
                ("value", value_t),
                ("label", T::Str(label.to_owned())),
                ("selected", T::Bool(selected)),
                ("index", T::Str(index_s)),
                ("attrs", attrs_value(&option_attrs)),
                ("type", self.input_type().map_or(T::None, |t| T::Str(t.to_owned()))),
                ("template_name", T::Str(self.option_template_name().to_owned())),
                ("wrap_label", T::Bool(true)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        )
    }

    /// `subwidgets(name, value, attrs)`: the widget's own context, or one per option.
    pub fn subwidgets(&self, name: &str, value: &Value, attrs: Option<&Attrs>) -> Vec<IndexMap<String, T>> {
        if self.is_choice_widget() {
            let formatted = self.format_value(value);
            let selected: Vec<String> = match &formatted {
                T::List(l) => l.iter().map(T::to_str).collect(),
                _ => vec![],
            };
            let T::List(groups) = self.optgroups_value(name, &selected, attrs) else { return vec![] };
            return groups
                .into_iter()
                .flat_map(|g| match g {
                    T::List(mut parts) => match parts.remove(1) {
                        T::List(opts) => opts.into_iter().filter_map(|o| if let T::Map(m) = o { Some(m) } else { None }).collect::<Vec<_>>(),
                        _ => vec![],
                    },
                    _ => vec![],
                })
                .collect();
        }
        vec![self.get_context(name, value, attrs)]
    }

    pub fn id_for_label(&self, id: &str) -> String {
        self.id_for_label_idx(id, None)
    }

    fn id_for_label_idx(&self, id: &str, index: Option<&str>) -> String {
        match &self.kind {
            WidgetKind::Select | WidgetKind::NullBooleanSelect | WidgetKind::SelectMultiple => id.to_owned(),
            WidgetKind::Radio | WidgetKind::CheckboxSelectMultiple => match index {
                None => String::new(),
                Some(i) if !id.is_empty() => format!("{id}_{i}"),
                Some(_) => id.to_owned(),
            },
            WidgetKind::Multi { .. } => String::new(),
            WidgetKind::SelectDate { .. } => format!("{id}_{}", parse_date_fmt().first().copied().unwrap_or("month")),
            _ => id.to_owned(),
        }
    }

    pub fn use_required_attribute(&self, initial: &Value) -> bool {
        let base = !self.is_hidden();
        match &self.kind {
            WidgetKind::File { .. } | WidgetKind::ClearableFile { .. } => base && !initial.truthy(),
            WidgetKind::Select | WidgetKind::NullBooleanSelect => base && self.choices.first().is_some_and(|c| matches!(c, Choice::Option(v, _) if v.is_empty())),
            WidgetKind::CheckboxSelectMultiple => false,
            _ => base,
        }
    }

    pub fn value_from_datadict(&self, data: &Data, files: &FileData, name: &str) -> Value {
        let get = |n: &str| data.get(n).map_or(Value::None, |s| Value::Str(s.clone()));
        match &self.kind {
            WidgetKind::MultipleHidden | WidgetKind::SelectMultiple | WidgetKind::CheckboxSelectMultiple => {
                let l = data.getlist(name);
                if l.is_empty() && !data.contains_key(name) { Value::None } else { Value::List(l.iter().cloned().map(Value::Str).collect()) }
            }
            WidgetKind::File { multiple } => {
                if *multiple {
                    let l = files.getlist(name);
                    if l.is_empty() { Value::None } else { Value::List(l.iter().cloned().map(Value::File).collect()) }
                } else {
                    files.get(name).cloned().map_or(Value::None, Value::File)
                }
            }
            WidgetKind::ClearableFile { checked } => {
                let upload = files.get(name).cloned().map_or(Value::None, Value::File);
                checked.0.store(data.contains_key(&format!("{name}-clear")), Ordering::Relaxed);
                if !self.is_required && Widget::checkbox_input().value_from_datadict(data, files, &format!("{name}-clear")) == Value::Bool(true) {
                    return if upload.truthy() { Value::Str("\u{0}FILE_INPUT_CONTRADICTION".into()) } else { Value::Cleared };
                }
                upload
            }
            WidgetKind::Checkbox { .. } => {
                let Some(v) = data.get(name) else { return Value::Bool(false) };
                Value::Bool(match v.to_lowercase().as_str() {
                    "true" => true,
                    "false" => false,
                    _ => !v.is_empty(),
                })
            }
            WidgetKind::NullBooleanSelect => match data.get(name).map(String::as_str) {
                Some("True" | "true" | "2") => Value::Bool(true),
                Some("False" | "false" | "3") => Value::Bool(false),
                _ => Value::None,
            },
            WidgetKind::Multi { widgets, names, .. } => Value::List(widgets.iter().zip(names).map(|(w, n)| w.value_from_datadict(data, files, &format!("{name}{n}"))).collect()),
            WidgetKind::SelectDate { .. } => {
                let (y, m, d) = (data.get(&format!("{name}_year")), data.get(&format!("{name}_month")), data.get(&format!("{name}_day")));
                if let (Some(y), Some(m), Some(d)) = (y, m, d) {
                    if y.is_empty() && m.is_empty() && d.is_empty() {
                        return Value::None;
                    }
                    let fmt = rango_core::utils::formats::get_format_list("DATE_INPUT_FORMATS").first().cloned().unwrap_or_default();
                    return match (y.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>()) {
                        (Ok(y), Ok(m), Ok(d)) => match chrono::NaiveDate::from_ymd_opt(y, m, d) {
                            Some(date) => Value::Str(strftime_value(&Value::Date(date), &fmt)),
                            None => Value::Str(format!("{y}-{m}-{d}")),
                        },
                        _ => {
                            let z = |s: &str| if s.is_empty() { "0".to_owned() } else { s.to_owned() };
                            Value::Str(format!("{}-{}-{}", z(y), z(m), z(d)))
                        }
                    };
                }
                get(name)
            }
            _ => get(name),
        }
    }

    pub fn value_omitted_from_data(&self, data: &Data, files: &FileData, name: &str) -> bool {
        match &self.kind {
            WidgetKind::File { .. } => !files.contains_key(name),
            WidgetKind::ClearableFile { .. } => !files.contains_key(name) && !data.contains_key(&format!("{name}-clear")),
            WidgetKind::Checkbox { .. } | WidgetKind::SelectMultiple | WidgetKind::CheckboxSelectMultiple => false,
            WidgetKind::Multi { widgets, names, .. } => widgets.iter().zip(names).all(|(w, n)| w.value_omitted_from_data(data, files, &format!("{name}{n}"))),
            WidgetKind::SelectDate { .. } => ["year", "month", "day"].iter().all(|p| !data.contains_key(&format!("{name}_{p}"))),
            _ => !data.contains_key(name),
        }
    }

    /// `MultiWidget.decompress` for `MultiValueField.clean` on disabled fields.
    pub fn decompress(&self, value: &Value) -> Vec<Value> {
        match &self.kind {
            WidgetKind::Multi { decompress: Decompress::SplitDateTime, .. } => split_decompress(value),
            WidgetKind::Multi { decompress: Decompress::Custom(f), .. } => f(value),
            _ => vec![],
        }
    }

    /// `widget.render(name, value, attrs, renderer)`.
    pub fn render(&self, name: &str, value: &Value, attrs: Option<&Attrs>, renderer: Option<&Arc<dyn Renderer>>) -> String {
        let ctx = T::Map([("widget".to_owned(), T::Map(self.get_context(name, value, attrs)))].into_iter().collect());
        self.render_template(&self.template_name(), ctx, renderer)
    }

    pub(crate) fn render_template(&self, template: &str, ctx: T, renderer: Option<&Arc<dyn Renderer>>) -> String {
        let r = renderer.cloned().unwrap_or_else(get_default_renderer);
        r.render(template, ctx).unwrap_or_else(|e| panic!("rendering {template}: {e}"))
    }

    pub fn is_file_contradiction(v: &Value) -> bool {
        matches!(v, Value::Str(s) if s == "\u{0}FILE_INPUT_CONTRADICTION")
    }
}

fn split_decompress(value: &Value) -> Vec<Value> {
    if !value.truthy() {
        return vec![Value::None, Value::None];
    }
    match to_current_timezone(value.clone()) {
        Value::NaiveDateTime(d) => vec![Value::Date(d.date()), Value::Time(d.time())],
        Value::DateTime(d) => vec![Value::Date(d.date_naive()), Value::Time(d.time())],
        _ => vec![Value::None, Value::None],
    }
}

/// `SelectDateWidget._parse_date_fmt()`: field order from `DATE_FORMAT`.
pub fn parse_date_fmt() -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut escaped = false;
    for ch in rango_core::utils::formats::get_format_str("DATE_FORMAT").chars() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if "Yy".contains(ch) {
            out.push("year");
        } else if "bEFMmNn".contains(ch) {
            out.push("month");
        } else if "dj".contains(ch) {
            out.push("day");
        }
    }
    out
}

/// English month names (`utils.dates.MONTHS`).
pub fn month_name(m: i32) -> String {
    const NAMES: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    gettext(NAMES[(m - 1) as usize])
}

/// `strftime` on a date/time/datetime value (`%f` is microseconds).
pub fn strftime_value(v: &Value, fmt: &str) -> String {
    let render = |nanos: u32, f: &dyn Fn(&str) -> String| f(&fmt.replace("%f", "\u{1}")).replace('\u{1}', &format!("{:06}", nanos / 1000));
    match v {
        Value::Date(d) => render(0, &|f| d.format(f).to_string()),
        Value::Time(t) => render(chrono::Timelike::nanosecond(t), &|f| t.format(f).to_string()),
        Value::NaiveDateTime(d) => render(chrono::Timelike::nanosecond(d), &|f| d.format(f).to_string()),
        Value::DateTime(d) => {
            let n: NaiveDateTime = to_naive(d);
            render(chrono::Timelike::nanosecond(&n), &|f| n.format(f).to_string())
        }
        other => other.to_text(),
    }
}

fn to_naive(d: &chrono::DateTime<chrono::Utc>) -> NaiveDateTime {
    match to_current_timezone(Value::DateTime(*d)) {
        Value::NaiveDateTime(n) => n,
        _ => d.naive_utc(),
    }
}

// ----------------------------------------------------------------------------- Media

/// A media asset path with optional attributes (`Script`).
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub path: String,
    pub attributes: Attrs,
}

impl From<&str> for Asset {
    fn from(p: &str) -> Self {
        Asset { path: p.to_owned(), attributes: Attrs::new() }
    }
}

/// `forms.Media`: CSS (by medium) and JS lists, merged with dependency order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Media {
    css_lists: Vec<IndexMap<String, Vec<Asset>>>,
    js_lists: Vec<Vec<Asset>>,
}

impl Media {
    pub fn new(css: IndexMap<String, Vec<&str>>, js: Vec<&str>) -> Media {
        let css: IndexMap<String, Vec<Asset>> = css.into_iter().map(|(k, v)| (k, v.into_iter().map(Asset::from).collect())).collect();
        Media { css_lists: if css.is_empty() { vec![] } else { vec![css] }, js_lists: if js.is_empty() { vec![] } else { vec![js.into_iter().map(Asset::from).collect()] } }
    }

    fn merge(lists: &[Vec<Asset>]) -> Vec<Asset> {
        // `Media.merge`: topological order of "a before b" constraints, ties by first appearance.
        let mut order: Vec<Asset> = Vec::new();
        let mut preds: Vec<Vec<usize>> = Vec::new();
        let mut index = |a: &Asset, order: &mut Vec<Asset>, preds: &mut Vec<Vec<usize>>| -> usize {
            order.iter().position(|x| x.path == a.path).unwrap_or_else(|| {
                order.push(a.clone());
                preds.push(Vec::new());
                order.len() - 1
            })
        };
        for l in lists.iter().filter(|l| !l.is_empty()) {
            let mut head = index(&l[0], &mut order, &mut preds);
            for item in &l[1..] {
                let i = index(item, &mut order, &mut preds);
                if head != i && !preds[i].contains(&head) {
                    preds[i].push(head);
                }
                head = i;
            }
        }
        let mut done = vec![false; order.len()];
        let mut out = Vec::new();
        while out.len() < order.len() {
            let ready: Vec<usize> = (0..order.len()).filter(|i| !done[*i] && preds[*i].iter().all(|p| done[*p])).collect();
            if ready.is_empty() {
                // cycle: fall back to first-seen order (the reference implementation warns here)
                let mut seen: Vec<Asset> = Vec::new();
                for a in lists.iter().flatten() {
                    if !seen.iter().any(|s| s.path == a.path) {
                        seen.push(a.clone());
                    }
                }
                return seen;
            }
            for i in ready {
                done[i] = true;
                out.push(order[i].clone());
            }
        }
        out
    }

    fn abs_path(path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") || path.starts_with('/') {
            return path.to_owned();
        }
        let base = settings().static_url.clone().unwrap_or_default();
        format!("{base}{}", rango_core::utils::http::quote(path, "/"))
    }

    pub fn render_js(&self) -> Vec<String> {
        Self::merge(&self.js_lists).iter().map(|a| format!("<script src=\"{}\"{}></script>", escape(&Self::abs_path(&a.path)), flatatt(&a.attributes))).collect()
    }

    pub fn render_css(&self) -> Vec<String> {
        let mut mediums: IndexMap<String, Vec<Vec<Asset>>> = IndexMap::new();
        for l in &self.css_lists {
            for (m, v) in l {
                mediums.entry(m.clone()).or_default().push(v.clone());
            }
        }
        let mut keys: Vec<String> = mediums.keys().cloned().collect();
        keys.sort();
        keys.iter().flat_map(|m| Self::merge(&mediums[m]).into_iter().map(move |a| format!("<link href=\"{}\" media=\"{}\" rel=\"stylesheet\">", escape(&Self::abs_path(&a.path)), escape(m)))).collect()
    }

    /// `str(media)`.
    pub fn render(&self) -> String {
        self.render_js_and_css()
    }

    fn render_js_and_css(&self) -> String {
        self.render_css().into_iter().chain(self.render_js()).collect::<Vec<_>>().join("\n")
    }
}

impl std::ops::Add for Media {
    type Output = Media;
    fn add(self, other: Media) -> Media {
        let mut c = self.clone();
        for i in other.css_lists {
            if !i.is_empty() && !self.css_lists.contains(&i) {
                c.css_lists.push(i);
            }
        }
        for i in other.js_lists {
            if !i.is_empty() && !self.js_lists.contains(&i) {
                c.js_lists.push(i);
            }
        }
        c
    }
}

impl TemplateObject for Media {
    fn get(&self, key: &str) -> Option<T> {
        match key {
            "render_js" => Some(T::Safe(self.render_js().join("\n"))),
            "render_css" => Some(T::Safe(self.render_css().join("\n"))),
            _ => None,
        }
    }
    fn display(&self) -> T {
        T::Safe(self.render())
    }
}

/// `BoundWidget`: one rendered sub-widget (a radio button, a checkbox...).
#[derive(Clone)]
pub struct BoundWidget {
    pub parent: Widget,
    pub data: IndexMap<String, T>,
    pub renderer: Arc<dyn Renderer>,
}

impl BoundWidget {
    pub fn tag(&self, wrap_label: bool) -> String {
        let mut d = self.data.clone();
        d.insert("wrap_label".into(), T::Bool(wrap_label));
        let template = match d.get("template_name") {
            Some(T::Str(t)) => t.clone(),
            _ => self.parent.template_name(),
        };
        self.parent.render_template(&template, T::Map([("widget".to_owned(), T::Map(d))].into_iter().collect()), Some(&self.renderer))
    }
    pub fn id_for_label(&self) -> Option<String> {
        match self.data.get("attrs") {
            Some(T::Map(a)) => a.get("id").map(T::to_str),
            _ => None,
        }
    }
    pub fn choice_label(&self) -> String {
        self.data.get("label").map(T::to_str).unwrap_or_default()
    }
}

impl TemplateObject for BoundWidget {
    fn get(&self, key: &str) -> Option<T> {
        match key {
            "tag" => Some(T::Safe(self.tag(false))),
            "id_for_label" => self.id_for_label().map(T::Str),
            "choice_label" => Some(T::Str(self.choice_label())),
            "data" => Some(T::Map(self.data.clone())),
            "template_name" => self.data.get("template_name").cloned(),
            other => self.data.get(other).cloned(),
        }
    }
    fn display(&self) -> T {
        T::Safe(self.tag(true))
    }
}

pub(crate) fn object<O: TemplateObject + 'static>(o: O) -> T {
    Object::new(o)
}
