//! `forms.fields`: validation and cleaning of submitted data.

use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use indexmap::IndexMap;
use rango_core::conf::settings;
use rango_core::exceptions::ValidationError;
use rango_core::utils::translation::{gettext, ngettext};
use regex::Regex;
use rust_decimal::Decimal;

use crate::utils::{from_current_timezone, to_current_timezone};
use crate::value::Value;
use crate::widgets::{Attr, Attrs, Choice, Data, FileData, Widget, WidgetKind};

type VResult<T> = Result<T, ValidationError>;

/// A validator over a cleaned value (`field.validators`).
pub type FieldValidator = Arc<dyn Fn(&Value) -> VResult<()> + Send + Sync>;

/// `TypedChoiceField.coerce`.
pub type Coerce = Arc<dyn Fn(&str) -> Result<Value, ()> + Send + Sync>;

/// Built-in validators lifted to [`FieldValidator`].
pub mod validators {
    use super::*;
    use rango_core::validators as v;
    use rango_core::validators::Validator;

    fn text(value: &Value) -> String {
        value.to_text()
    }

    pub fn max_length(n: usize) -> FieldValidator {
        let val = v::MaxLengthValidator::new(n);
        Arc::new(move |x| val.validate(&text(x)))
    }
    pub fn min_length(n: usize) -> FieldValidator {
        let val = v::MinLengthValidator::new(n);
        Arc::new(move |x| val.validate(&text(x)))
    }
    pub fn prohibit_null_characters() -> FieldValidator {
        let val = v::ProhibitNullCharactersValidator::default();
        Arc::new(move |x| val.validate(&text(x)))
    }
    pub fn regex(pattern: &str) -> FieldValidator {
        let val = v::RegexValidator::new(pattern).expect("valid regex");
        Arc::new(move |x| val.validate(&text(x)))
    }
    pub fn email() -> FieldValidator {
        Arc::new(|x| v::validate_email(&text(x)))
    }
    pub fn slug() -> FieldValidator {
        Arc::new(|x| v::validate_slug(&text(x)))
    }
    pub fn unicode_slug() -> FieldValidator {
        Arc::new(|x| v::validate_unicode_slug(&text(x)))
    }
    pub fn url() -> FieldValidator {
        let val = v::UrlValidator::new().expect("url validator");
        Arc::new(move |x| val.validate(&text(x)))
    }
    pub fn ip(protocol: &str, unpack: bool) -> FieldValidator {
        let f = v::ip_address_validators(protocol, unpack).expect("valid protocol");
        Arc::new(move |x| f(&text(x)))
    }
    pub fn decimal(max_digits: Option<u32>, decimal_places: Option<u32>) -> FieldValidator {
        let val = v::DecimalValidator::new(max_digits, decimal_places);
        Arc::new(move |x| match x {
            Value::Decimal(d) => val.validate(d),
            _ => Ok(()),
        })
    }
    pub fn file_extension(allowed: &[&str]) -> FieldValidator {
        let val = v::FileExtensionValidator::new(Some(allowed));
        Arc::new(move |x| match x {
            Value::File(f) => val.validate(&f.name),
            _ => Ok(()),
        })
    }

    fn num(x: &Value) -> Option<f64> {
        match x {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Decimal(d) => d.to_string().parse().ok(),
            _ => None,
        }
    }

    fn limit_err(msg: &str, code: &str, limit: &Value, value: &Value) -> ValidationError {
        ValidationError::new(gettext(msg)).with_code(code).with_params([("limit_value", limit.to_text()), ("show_value", value.to_text()), ("value", value.to_text())])
    }

    pub fn max_value(limit: Value) -> FieldValidator {
        Arc::new(move |x| match (num(x), num(&limit)) {
            (Some(a), Some(b)) if a > b => Err(limit_err("Ensure this value is less than or equal to %(limit_value)s.", "max_value", &limit, x)),
            _ => Ok(()),
        })
    }
    pub fn min_value(limit: Value) -> FieldValidator {
        Arc::new(move |x| match (num(x), num(&limit)) {
            (Some(a), Some(b)) if a < b => Err(limit_err("Ensure this value is greater than or equal to %(limit_value)s.", "min_value", &limit, x)),
            _ => Ok(()),
        })
    }
    pub fn step_value(step: f64, offset: Option<f64>) -> FieldValidator {
        let mut val = v::StepValueValidator::new(step);
        if let Some(o) = offset {
            val = val.with_offset(o);
        }
        Arc::new(move |x| match num(x) {
            Some(n) => val.validate(&n),
            None => Ok(()),
        })
    }
}

#[derive(Clone)]
pub enum FieldKind {
    Char { strip: bool, empty_value: Value },
    Integer { max_value: Option<Value>, min_value: Option<Value>, step_size: Option<f64> },
    Float { max_value: Option<Value>, min_value: Option<Value>, step_size: Option<f64> },
    Decimal { max_value: Option<Value>, min_value: Option<Value>, max_digits: Option<u32>, decimal_places: Option<u32> },
    Date { input_formats: Option<Vec<String>> },
    Time { input_formats: Option<Vec<String>> },
    DateTime { input_formats: Option<Vec<String>> },
    Duration,
    Regex { strip: bool },
    Email,
    File { max_length: Option<usize>, allow_empty_file: bool },
    Image { max_length: Option<usize>, allow_empty_file: bool },
    Url { assume_scheme: String },
    Boolean,
    NullBoolean,
    Choice,
    TypedChoice { coerce: Coerce, empty_value: Value },
    MultipleChoice,
    TypedMultipleChoice { coerce: Coerce, empty_value: Value },
    Combo { fields: Vec<Field> },
    MultiValue { fields: Vec<Field>, require_all_fields: bool, compress: Arc<dyn Fn(&[Value]) -> VResult<Value> + Send + Sync> },
    SplitDateTime { fields: Vec<Field> },
    FilePath,
    GenericIpAddress { unpack_ipv4: bool },
    Slug,
    Uuid,
    Json,
    /// `ModelChoiceField`/`ModelMultipleChoiceField`: choices are loaded from the queryset (`models::load_choices`).
    ModelChoice { multiple: bool },
}

impl std::fmt::Debug for FieldKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FieldKind")
    }
}

/// A form field: definition plus its constructor options.
#[derive(Clone)]
pub struct Field {
    pub kind: FieldKind,
    pub required: bool,
    pub label: Option<String>,
    pub initial: Option<Value>,
    pub help_text: String,
    pub error_messages: IndexMap<String, String>,
    pub validators: Vec<FieldValidator>,
    pub widget: Widget,
    pub disabled: bool,
    pub show_hidden_initial: bool,
    pub localize: bool,
    pub label_suffix: Option<String>,
    pub template_name: Option<String>,
    /// `choices` of choice fields (also kept on the widget).
    pub choices: Vec<Choice>,
    /// `max_length`/`min_length` of the char-like fields (they drive `maxlength`/`minlength` attributes).
    pub max_length: Option<usize>,
    pub min_length: Option<usize>,
    /// Attributes `widget_attrs()` derived from the field (re-derived when options change).
    derived_attrs: Attrs,
}

impl std::fmt::Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Field({:?}, required={}, label={:?})", self.kind, self.required, self.label)
    }
}

fn msgs(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), gettext(v))).collect()
}

impl Field {
    fn base(kind: FieldKind, widget: Widget, defaults: IndexMap<String, String>) -> Field {
        let mut error_messages = msgs(&[("required", "This field is required.")]);
        error_messages.extend(defaults);
        let mut f = Field {
            kind,
            required: true,
            label: None,
            initial: None,
            help_text: String::new(),
            error_messages,
            validators: Vec::new(),
            widget,
            disabled: false,
            show_hidden_initial: false,
            localize: false,
            label_suffix: None,
            template_name: None,
            choices: Vec::new(),
            max_length: None,
            min_length: None,
            derived_attrs: Attrs::new(),
        };
        f.apply_widget_attrs();
        f.widget.is_required = true;
        f
    }

    // ------------------------------------------------------------------ constructors

    pub fn char() -> Field {
        let mut f = Field::base(FieldKind::Char { strip: true, empty_value: Value::Str(String::new()) }, Widget::text_input(), IndexMap::new());
        f.validators.push(validators::prohibit_null_characters());
        f
    }
    pub fn integer() -> Field {
        Field::base(FieldKind::Integer { max_value: None, min_value: None, step_size: None }, Widget::number_input(), msgs(&[("invalid", "Enter a whole number.")]))
    }
    pub fn float() -> Field {
        Field::base(FieldKind::Float { max_value: None, min_value: None, step_size: None }, Widget::number_input(), msgs(&[("invalid", "Enter a number.")]))
    }
    pub fn decimal() -> Field {
        Field::base(FieldKind::Decimal { max_value: None, min_value: None, max_digits: None, decimal_places: None }, Widget::number_input(), msgs(&[("invalid", "Enter a number.")]))
    }
    pub fn date() -> Field {
        Field::base(FieldKind::Date { input_formats: None }, Widget::date_input(), msgs(&[("invalid", "Enter a valid date.")]))
    }
    pub fn time() -> Field {
        Field::base(FieldKind::Time { input_formats: None }, Widget::time_input(), msgs(&[("invalid", "Enter a valid time.")]))
    }
    pub fn datetime() -> Field {
        Field::base(FieldKind::DateTime { input_formats: None }, Widget::datetime_input(), msgs(&[("invalid", "Enter a valid date/time.")]))
    }
    pub fn duration() -> Field {
        Field::base(FieldKind::Duration, Widget::text_input(), msgs(&[("invalid", "Enter a valid duration."), ("overflow", "The number of days must be between {min_days} and {max_days}.")]))
    }
    pub fn regex(pattern: &str) -> Field {
        let mut f = Field::base(FieldKind::Regex { strip: false }, Widget::text_input(), IndexMap::new());
        f.validators.push(validators::prohibit_null_characters());
        f.validators.push(validators::regex(pattern));
        f
    }
    pub fn email() -> Field {
        let mut f = Field::base(FieldKind::Email, Widget::email_input(), IndexMap::new());
        f.validators.push(validators::email());
        f.validators.push(validators::max_length(320));
        f.validators.push(validators::prohibit_null_characters());
        f.set_max_length(320);
        f
    }
    pub fn file() -> Field {
        Field::base(FieldKind::File { max_length: None, allow_empty_file: false }, Widget::clearable_file_input(), file_messages())
    }
    pub fn image() -> Field {
        let mut m = file_messages();
        m.insert("invalid_image".into(), gettext("Upload a valid image. The file you uploaded was either not an image or a corrupted image."));
        let mut f = Field::base(FieldKind::Image { max_length: None, allow_empty_file: false }, Widget::clearable_file_input(), m);
        f.validators.push(validators::file_extension(&["bmp", "dib", "gif", "tif", "tiff", "jfif", "jpe", "jpg", "jpeg", "pbm", "pgm", "ppm", "pnm", "png", "apng", "blp", "bufr", "cur", "pcx", "dcx", "dds", "ps", "eps", "fit", "fits", "fli", "flc", "ftc", "ftu", "gbr", "grib", "h5", "hdf", "icns", "ico", "im", "iim", "jp2", "j2k", "jpc", "jpf", "jpx", "j2c", "mic", "mpg", "mpeg", "mpo", "msp", "palm", "pcd", "pdf", "pxr", "psd", "qoi", "bw", "rgb", "rgba", "sgi", "ras", "tga", "icb", "vda", "vst", "webp", "wmf", "emf", "xbm", "xpm"]));
        f.widget.attrs.entry("accept".into()).or_insert("image/*".into());
        f
    }
    pub fn url() -> Field {
        let scheme = if settings().forms_urlfield_assume_https { "https" } else { "http" };
        let mut f = Field::base(FieldKind::Url { assume_scheme: scheme.into() }, Widget::url_input(), msgs(&[("invalid", "Enter a valid URL.")]));
        f.validators.push(validators::url());
        f.validators.push(validators::prohibit_null_characters());
        f
    }
    pub fn boolean() -> Field {
        Field::base(FieldKind::Boolean, Widget::checkbox_input(), IndexMap::new())
    }
    pub fn null_boolean() -> Field {
        Field::base(FieldKind::NullBoolean, Widget::null_boolean_select(), IndexMap::new())
    }
    pub fn choice(choices: Vec<Choice>) -> Field {
        let mut f = Field::base(FieldKind::Choice, Widget::select(), msgs(&[("invalid_choice", "Select a valid choice. %(value)s is not one of the available choices.")]));
        f.set_choices(choices);
        f
    }
    pub fn typed_choice(choices: Vec<Choice>, coerce: impl Fn(&str) -> Result<Value, ()> + Send + Sync + 'static) -> Field {
        let mut f = Field::choice(choices);
        f.kind = FieldKind::TypedChoice { coerce: Arc::new(coerce), empty_value: Value::Str(String::new()) };
        f
    }
    pub fn multiple_choice(choices: Vec<Choice>) -> Field {
        let mut f = Field::base(
            FieldKind::MultipleChoice,
            Widget::select_multiple(),
            msgs(&[("invalid_choice", "Select a valid choice. %(value)s is not one of the available choices."), ("invalid_list", "Enter a list of values.")]),
        );
        f.set_choices(choices);
        f
    }
    pub fn typed_multiple_choice(choices: Vec<Choice>, coerce: impl Fn(&str) -> Result<Value, ()> + Send + Sync + 'static) -> Field {
        let mut f = Field::multiple_choice(choices);
        f.kind = FieldKind::TypedMultipleChoice { coerce: Arc::new(coerce), empty_value: Value::List(vec![]) };
        f
    }
    pub fn combo(fields: Vec<Field>) -> Field {
        let fields = fields.into_iter().map(|mut f| {
            f.required = false;
            f
        });
        Field::base(FieldKind::Combo { fields: fields.collect() }, Widget::text_input(), IndexMap::new())
    }
    pub fn multi_value(fields: Vec<Field>, widget: Widget, compress: impl Fn(&[Value]) -> VResult<Value> + Send + Sync + 'static) -> Field {
        let mut f = Field::base(
            FieldKind::MultiValue { fields: vec![], require_all_fields: true, compress: Arc::new(compress) },
            widget,
            msgs(&[("invalid", "Enter a list of values."), ("incomplete", "Enter a complete value.")]),
        );
        f.set_subfields(fields);
        f
    }
    pub fn split_datetime() -> Field {
        let invalid_date = gettext("Enter a valid date.");
        let invalid_time = gettext("Enter a valid time.");
        let mut d = Field::date();
        d.error_messages.insert("invalid".into(), invalid_date.clone());
        let mut t = Field::time();
        t.error_messages.insert("invalid".into(), invalid_time.clone());
        let mut f = Field::base(
            FieldKind::SplitDateTime { fields: vec![] },
            Widget::split_datetime(),
            msgs(&[("invalid", "Enter a list of values."), ("incomplete", "Enter a complete value."), ("invalid_date", "Enter a valid date."), ("invalid_time", "Enter a valid time.")]),
        );
        f.set_subfields(vec![d, t]);
        f
    }
    pub fn file_path(path: &str, match_: Option<&str>, recursive: bool, allow_files: bool, allow_folders: bool, required: bool) -> Field {
        let mut f = Field::base(FieldKind::FilePath, Widget::select(), msgs(&[("invalid_choice", "Select a valid choice. %(value)s is not one of the available choices.")]));
        f.required = required;
        f.widget.is_required = required;
        let mut choices = if required { vec![] } else { vec![Choice::option("", "---------")] };
        let re = match_.map(|m| Regex::new(m).expect("valid regex"));
        let ok = |name: &str| re.as_ref().is_none_or(|r| r.is_match(name));
        let mut found: Vec<(String, String)> = Vec::new();
        if recursive {
            fn walk(dir: &std::path::Path, base: &str, files: bool, dirs: bool, ok: &dyn Fn(&str) -> bool, out: &mut Vec<(String, String)>) {
                let Ok(rd) = std::fs::read_dir(dir) else { return };
                let mut entries: Vec<_> = rd.flatten().collect();
                entries.sort_by_key(|e| e.file_name());
                for e in &entries {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let p = e.path();
                    if p.is_file() && files && ok(&name) {
                        let full = p.to_string_lossy().into_owned();
                        out.push((full.clone(), full.replacen(base, "", 1)));
                    }
                }
                for e in &entries {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let p = e.path();
                    if p.is_dir() && name != "__pycache__" {
                        if dirs && ok(&name) {
                            let full = p.to_string_lossy().into_owned();
                            out.push((full.clone(), full.replacen(base, "", 1)));
                        }
                        walk(&p, base, files, dirs, ok, out);
                    }
                }
            }
            walk(std::path::Path::new(path), path, allow_files, allow_folders, &ok, &mut found);
        } else if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == "__pycache__" {
                    continue;
                }
                let p = e.path();
                if ((allow_files && p.is_file()) || (allow_folders && p.is_dir())) && ok(&name) {
                    found.push((p.to_string_lossy().into_owned(), name));
                }
            }
            found.sort_by(|a, b| a.1.cmp(&b.1));
        }
        choices.extend(found.into_iter().map(|(v, l)| Choice::option(v, l)));
        f.set_choices(choices);
        f
    }
    pub fn generic_ip_address(protocol: &str, unpack_ipv4: bool) -> Field {
        let mut f = Field::base(FieldKind::GenericIpAddress { unpack_ipv4 }, Widget::text_input(), IndexMap::new());
        f.validators.push(validators::ip(protocol, unpack_ipv4));
        f.validators.push(validators::max_length(39));
        f.validators.push(validators::prohibit_null_characters());
        f.set_max_length(39);
        f
    }
    pub fn slug() -> Field {
        let mut f = Field::base(FieldKind::Slug, Widget::text_input(), IndexMap::new());
        f.validators.push(validators::slug());
        f.validators.push(validators::prohibit_null_characters());
        f
    }
    pub fn uuid() -> Field {
        let mut f = Field::base(FieldKind::Uuid, Widget::text_input(), msgs(&[("invalid", "Enter a valid UUID.")]));
        f.validators.push(validators::prohibit_null_characters());
        f
    }
    pub fn json() -> Field {
        let mut f = Field::base(FieldKind::Json, Widget::textarea(), msgs(&[("invalid", "Enter a valid JSON.")]));
        f.validators.push(validators::prohibit_null_characters());
        f
    }
    pub fn model_choice(multiple: bool) -> Field {
        let mut m = msgs(&[("invalid_choice", if multiple { "Select a valid choice. %(value)s is not one of the available choices." } else { "Select a valid choice. That choice is not one of the available choices." })]);
        if multiple {
            m.extend(msgs(&[("invalid_list", "Enter a list of values."), ("invalid_pk_value", "“%(pk)s” is not a valid value.")]));
        }
        Field::base(FieldKind::ModelChoice { multiple }, if multiple { Widget::select_multiple() } else { Widget::select() }, m)
    }

    // ------------------------------------------------------------------ builder options

    pub fn required(mut self, r: bool) -> Self {
        self.required = r;
        self.widget.is_required = r;
        self
    }
    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = Some(l.into());
        self
    }
    pub fn initial(mut self, v: impl Into<Value>) -> Self {
        self.initial = Some(v.into());
        self
    }
    pub fn help_text(mut self, t: impl Into<String>) -> Self {
        self.help_text = t.into();
        self
    }
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
    pub fn label_suffix(mut self, s: impl Into<String>) -> Self {
        self.label_suffix = Some(s.into());
        self
    }
    pub fn show_hidden_initial(mut self, s: bool) -> Self {
        self.show_hidden_initial = s;
        self
    }
    pub fn localize(mut self, l: bool) -> Self {
        self.localize = l;
        self.widget.is_localized = l;
        self
    }
    pub fn error_message(mut self, code: &str, msg: impl Into<String>) -> Self {
        self.error_messages.insert(code.to_owned(), msg.into());
        self
    }
    pub fn validator(mut self, v: impl Fn(&Value) -> VResult<()> + Send + Sync + 'static) -> Self {
        self.validators.push(Arc::new(v));
        self
    }
    /// Replace the widget (`widget=...`); keeps `required` and the field's own widget attrs.
    pub fn widget(mut self, w: Widget) -> Self {
        self.widget = w;
        self.widget.is_required = self.required;
        self.apply_widget_attrs();
        if !self.choices.is_empty() && self.widget.is_choice_widget() {
            self.widget.choices = self.choices.clone();
        }
        self
    }
    pub fn widget_attrs(mut self, a: Attrs) -> Self {
        for (k, v) in a {
            self.widget.attrs.insert(k, v);
        }
        self
    }
    pub fn template_name(mut self, t: &str) -> Self {
        self.template_name = Some(t.to_owned());
        self
    }

    pub fn max_length(mut self, n: usize) -> Self {
        self.set_max_length(n);
        self.validators.push(validators::max_length(n));
        self
    }
    pub fn min_length(mut self, n: usize) -> Self {
        self.min_length = Some(n);
        self.apply_widget_attrs();
        self.validators.push(validators::min_length(n));
        self
    }
    fn set_max_length(&mut self, n: usize) {
        match &mut self.kind {
            FieldKind::File { max_length, .. } | FieldKind::Image { max_length, .. } => *max_length = Some(n),
            _ => self.max_length = Some(n),
        }
        self.apply_widget_attrs();
    }
    pub fn strip(mut self, s: bool) -> Self {
        if let FieldKind::Char { strip, .. } | FieldKind::Regex { strip } = &mut self.kind {
            *strip = s;
        }
        self
    }
    /// `CharField(empty_value=...)`: what an empty submission cleans to (`""` by default; `Value::None` for nullable columns).
    pub fn empty_value(mut self, v: impl Into<Value>) -> Self {
        if let FieldKind::Char { empty_value, .. } = &mut self.kind {
            *empty_value = v.into();
        }
        self
    }
    pub fn max_value(mut self, v: impl Into<Value>) -> Self {
        let v = v.into();
        match &mut self.kind {
            FieldKind::Integer { max_value, .. } | FieldKind::Float { max_value, .. } | FieldKind::Decimal { max_value, .. } => *max_value = Some(v.clone()),
            _ => {}
        }
        self.apply_widget_attrs();
        self.validators.push(validators::max_value(v));
        self
    }
    pub fn min_value(mut self, v: impl Into<Value>) -> Self {
        let v = v.into();
        match &mut self.kind {
            FieldKind::Integer { min_value, .. } | FieldKind::Float { min_value, .. } | FieldKind::Decimal { min_value, .. } => *min_value = Some(v.clone()),
            _ => {}
        }
        self.apply_widget_attrs();
        self.validators.push(validators::min_value(v));
        self
    }
    pub fn step_size(mut self, s: f64) -> Self {
        let offset = match &self.kind {
            FieldKind::Integer { min_value, .. } | FieldKind::Float { min_value, .. } => min_value.as_ref().and_then(|v| match v {
                Value::Int(i) => Some(*i as f64),
                Value::Float(f) => Some(*f),
                _ => None,
            }),
            _ => None,
        };
        if let FieldKind::Integer { step_size, .. } | FieldKind::Float { step_size, .. } = &mut self.kind {
            *step_size = Some(s);
        }
        self.apply_widget_attrs();
        self.validators.push(validators::step_value(s, offset));
        self
    }
    pub fn max_digits(mut self, n: u32) -> Self {
        if let FieldKind::Decimal { max_digits, decimal_places, .. } = &mut self.kind {
            *max_digits = Some(n);
            let dp = *decimal_places;
            self.validators.push(validators::decimal(Some(n), dp));
        }
        self
    }
    pub fn decimal_places(mut self, n: u32) -> Self {
        if let FieldKind::Decimal { decimal_places, max_digits, .. } = &mut self.kind {
            *decimal_places = Some(n);
            let md = *max_digits;
            self.validators.push(validators::decimal(md, Some(n)));
        }
        self.apply_widget_attrs();
        self
    }
    pub fn input_formats(mut self, f: &[&str]) -> Self {
        if let FieldKind::Date { input_formats } | FieldKind::Time { input_formats } | FieldKind::DateTime { input_formats } = &mut self.kind {
            *input_formats = Some(f.iter().map(|s| (*s).to_owned()).collect());
        }
        self
    }
    pub fn assume_scheme(mut self, s: &str) -> Self {
        if let FieldKind::Url { assume_scheme } = &mut self.kind {
            *assume_scheme = s.to_owned();
        }
        self
    }
    pub fn allow_empty_file(mut self, a: bool) -> Self {
        if let FieldKind::File { allow_empty_file, .. } | FieldKind::Image { allow_empty_file, .. } = &mut self.kind {
            *allow_empty_file = a;
        }
        self
    }
    pub fn require_all_fields(mut self, r: bool) -> Self {
        let fields = match &mut self.kind {
            FieldKind::MultiValue { require_all_fields, fields, .. } => {
                *require_all_fields = r;
                fields.clone()
            }
            _ => return self,
        };
        self.set_subfields(fields);
        self
    }

    pub fn set_choices(&mut self, c: Vec<Choice>) {
        self.choices = c.clone();
        self.widget.choices = c;
    }

    fn set_subfields(&mut self, mut fields: Vec<Field>) {
        let incomplete = self.error_messages.get("incomplete").cloned().unwrap_or_default();
        let (disabled, all) = (self.disabled, matches!(&self.kind, FieldKind::MultiValue { require_all_fields: true, .. } | FieldKind::SplitDateTime { .. }));
        for f in &mut fields {
            f.error_messages.entry("incomplete".into()).or_insert_with(|| incomplete.clone());
            if disabled {
                f.disabled = true;
            }
            if all {
                f.required = false;
            }
        }
        match &mut self.kind {
            FieldKind::MultiValue { fields: slot, .. } | FieldKind::SplitDateTime { fields: slot } => *slot = fields,
            _ => {}
        }
    }

    /// `widget_attrs(widget)`: derive HTML attributes (maxlength, min, max, step, accept) from the field.
    fn apply_widget_attrs(&mut self) {
        // Drop what an earlier pass derived (unless the user overrode it), then derive afresh.
        for (k, v) in std::mem::take(&mut self.derived_attrs) {
            if self.widget.attrs.get(&k) == Some(&v) {
                self.widget.attrs.shift_remove(&k);
            }
        }
        let hidden = self.widget.is_hidden();
        let is_number = matches!(self.widget.kind, WidgetKind::Number);
        let mut extra: Attrs = Attrs::new();
        if self.is_char_like() {
            if let (Some(n), false) = (self.max_length, hidden) {
                extra.insert("maxlength".into(), n.to_string().into());
            }
            if let (Some(n), false) = (self.min_length, hidden) {
                extra.insert("minlength".into(), n.to_string().into());
            }
        }
        match &self.kind {
            FieldKind::Integer { max_value, min_value, step_size } | FieldKind::Float { max_value, min_value, step_size } => {
                if is_number {
                    if let Some(v) = min_value {
                        extra.insert("min".into(), v.to_text().into());
                    }
                    if let Some(v) = max_value {
                        extra.insert("max".into(), v.to_text().into());
                    }
                    if let Some(s) = step_size {
                        extra.insert("step".into(), fmt_f(*s).into());
                    }
                    if matches!(self.kind, FieldKind::Float { .. }) && !extra.contains_key("step") {
                        extra.insert("step".into(), "any".into());
                    }
                }
            }
            FieldKind::Decimal { max_value, min_value, decimal_places, .. } => {
                if is_number {
                    if let Some(v) = min_value {
                        extra.insert("min".into(), v.to_text().into());
                    }
                    if let Some(v) = max_value {
                        extra.insert("max".into(), v.to_text().into());
                    }
                    {
                        let step = match decimal_places {
                            Some(0) => "1".to_owned(),
                            Some(n) => format!("0.{}1", "0".repeat(*n as usize - 1)),
                            None => "any".to_owned(),
                        };
                        extra.insert("step".into(), step.into());
                    }
                }
            }
            _ => {}
        }
        for (k, v) in extra {
            if !self.widget.attrs.contains_key(&k) {
                self.widget.attrs.insert(k.clone(), v.clone());
                self.derived_attrs.insert(k, v);
            }
        }
    }

    // ------------------------------------------------------------------ behaviour

    fn is_char_like(&self) -> bool {
        matches!(self.kind, FieldKind::Char { .. } | FieldKind::Regex { .. } | FieldKind::Email | FieldKind::Url { .. } | FieldKind::GenericIpAddress { .. } | FieldKind::Slug | FieldKind::Uuid | FieldKind::Json)
    }

    fn strips(&self) -> bool {
        match &self.kind {
            FieldKind::Char { strip, .. } | FieldKind::Regex { strip } => *strip,
            _ => true,
        }
    }

    fn err(&self, code: &str) -> ValidationError {
        ValidationError::new(self.error_messages.get(code).cloned().unwrap_or_default()).with_code(code)
    }

    fn input_formats_list(&self) -> Vec<String> {
        use rango_core::utils::formats::get_format_list as fl;
        match &self.kind {
            FieldKind::Date { input_formats } => input_formats.clone().unwrap_or_else(|| fl("DATE_INPUT_FORMATS")),
            FieldKind::Time { input_formats } => input_formats.clone().unwrap_or_else(|| fl("TIME_INPUT_FORMATS")),
            FieldKind::DateTime { input_formats } => input_formats.clone().unwrap_or_else(|| fl("DATETIME_INPUT_FORMATS").into_iter().chain(fl("DATE_INPUT_FORMATS")).collect()),
            _ => vec![],
        }
    }

    /// `prepare_value(value)`.
    pub fn prepare_value(&self, value: &Value) -> Value {
        match (&self.kind, value) {
            (FieldKind::DateTime { .. }, Value::DateTime(_)) => to_current_timezone(value.clone()),
            (FieldKind::Duration, Value::Duration(d)) => Value::Str(rango_core::utils::duration::duration_string(*d)),
            (FieldKind::Uuid, Value::Uuid(u)) => Value::Str(u.to_string()),
            (FieldKind::Json, Value::Str(s)) if s.starts_with('\u{0}') => Value::Str(s[1..].to_owned()),
            (FieldKind::Json, v) => Value::Str(match v {
                Value::Json(j) => rango_core::utils::json::dumps_with(j, false, false).unwrap_or_default(),
                Value::None => "null".into(),
                other => rango_core::utils::json::dumps_with(&to_json(other), false, false).unwrap_or_default(),
            }),
            _ => value.clone(),
        }
    }

    /// `to_clean(value)`.
    pub fn to_clean(&self, value: &Value) -> VResult<Value> {
        match &self.kind {
            k if self.is_char_like() && !matches!(k, FieldKind::Uuid | FieldKind::Json | FieldKind::Url { .. } | FieldKind::GenericIpAddress { .. }) => Ok(self.char_to_clean(value)),
            FieldKind::Url { assume_scheme } => {
                let v = self.char_to_clean(value);
                let Value::Str(s) = &v else { return Ok(v) };
                if s.is_empty() {
                    return Ok(v);
                }
                let (scheme, sep) = match s.split_once(':') {
                    Some((a, _)) => (a, true),
                    None => ("", false),
                };
                let bad = !sep || scheme.is_empty() || !scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) || scheme.contains('/');
                if bad {
                    return Ok(Value::Str(if s.starts_with("//") { format!("{assume_scheme}:{s}") } else { format!("{assume_scheme}://{s}") }));
                }
                Ok(v)
            }
            FieldKind::GenericIpAddress { unpack_ipv4 } => {
                if value.is_empty() {
                    return Ok(Value::Str(String::new()));
                }
                let s = value.to_text().trim().to_owned();
                if s.contains(':') {
                    return rango_core::utils::ipv6::clean_ipv6_address(&s, *unpack_ipv4, None, self.max_length.unwrap_or(39)).map(Value::Str);
                }
                Ok(Value::Str(s))
            }
            FieldKind::Uuid => {
                let v = self.char_to_clean(value);
                if v.is_empty() {
                    return Ok(Value::None);
                }
                if let Value::Uuid(_) = v {
                    return Ok(v);
                }
                uuid::Uuid::parse_str(&v.to_text()).map(Value::Uuid).map_err(|_| self.err("invalid"))
            }
            FieldKind::Json => {
                if self.disabled {
                    return Ok(value.clone());
                }
                if value.is_empty() {
                    return Ok(Value::None);
                }
                let Value::Str(s) = value else { return Ok(value.clone()) };
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(serde_json::Value::String(t)) => Ok(Value::Json(serde_json::Value::String(t))),
                    Ok(j) => Ok(Value::Json(j)),
                    Err(_) => Err(self.err("invalid").with_params([("value", s.clone())])),
                }
            }
            FieldKind::Integer { .. } => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                if let Value::Int(_) = value {
                    return Ok(value.clone());
                }
                parse_int(&value.to_text()).map(Value::Int).ok_or_else(|| self.err("invalid"))
            }
            FieldKind::Float { .. } => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                if let Value::Float(_) = value {
                    return Ok(value.clone());
                }
                let t = value.to_text();
                parse_float(t.trim()).map(Value::Float).ok_or_else(|| self.err("invalid"))
            }
            FieldKind::Decimal { .. } => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                if let Value::Decimal(_) = value {
                    return Ok(value.clone());
                }
                let t = value.to_text();
                let t = t.trim();
                t.parse::<Decimal>().or_else(|_| Decimal::from_scientific(t)).map(Value::Decimal).map_err(|_| self.err("invalid"))
            }
            FieldKind::Date { .. } => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                match value {
                    Value::Date(_) => return Ok(value.clone()),
                    Value::NaiveDateTime(d) => return Ok(Value::Date(d.date())),
                    Value::DateTime(d) => return Ok(Value::Date(d.date_naive())),
                    _ => {}
                }
                let s = value.to_text();
                for f in self.input_formats_list() {
                    if let Ok(d) = NaiveDate::parse_from_str(s.trim(), &py_fmt(&f)) {
                        return Ok(Value::Date(d));
                    }
                }
                Err(self.err("invalid"))
            }
            FieldKind::Time { .. } => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                if let Value::Time(_) = value {
                    return Ok(value.clone());
                }
                let s = value.to_text();
                for f in self.input_formats_list() {
                    if let Ok(t) = NaiveTime::parse_from_str(s.trim(), &py_fmt(&f)) {
                        return Ok(Value::Time(t));
                    }
                }
                Err(self.err("invalid"))
            }
            FieldKind::DateTime { .. } => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                match value {
                    Value::NaiveDateTime(_) | Value::DateTime(_) => return from_current_timezone(value.clone()),
                    Value::Date(d) => return from_current_timezone(Value::NaiveDateTime(d.and_hms_opt(0, 0, 0).expect("midnight"))),
                    _ => {}
                }
                let s = value.to_text();
                let s = s.trim();
                match rango_core::utils::dateparse::parse_datetime(s) {
                    Err(_) => return Err(self.err("invalid")),
                    Ok(Some(p)) => {
                        return match p.offset {
                            Some(off) => Ok(Value::DateTime((p.naive - chrono::TimeDelta::seconds(i64::from(off.local_minus_utc()))).and_utc())),
                            None => from_current_timezone(Value::NaiveDateTime(p.naive)),
                        };
                    }
                    Ok(None) => {}
                }
                for f in self.input_formats_list() {
                    let f = py_fmt(&f);
                    if let Ok(d) = NaiveDateTime::parse_from_str(s, &f) {
                        return from_current_timezone(Value::NaiveDateTime(d));
                    }
                    if let Ok(d) = NaiveDate::parse_from_str(s, &f) {
                        return from_current_timezone(Value::NaiveDateTime(d.and_hms_opt(0, 0, 0).expect("midnight")));
                    }
                }
                Err(self.err("invalid"))
            }
            FieldKind::Duration => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                if let Value::Duration(_) = value {
                    return Ok(value.clone());
                }
                rango_core::utils::dateparse::parse_duration(&value.to_text()).map(Value::Duration).ok_or_else(|| self.err("invalid"))
            }
            FieldKind::File { max_length, allow_empty_file } | FieldKind::Image { max_length, allow_empty_file } => {
                if value.is_empty() {
                    return Ok(Value::None);
                }
                let Value::File(f) = value else { return Err(self.err("invalid")) };
                if let Some(m) = max_length {
                    if f.name.chars().count() > *m {
                        let msg = ngettext("Ensure this filename has at most %(max)d character (it has %(length)d).", "Ensure this filename has at most %(max)d characters (it has %(length)d).", *m as u64);
                        let msg = self.error_messages.get("max_length").cloned().filter(|m| !m.starts_with("Ensure this filename")).unwrap_or(msg);
                        return Err(ValidationError::new(msg).with_code("max_length").with_params([("max", m.to_string()), ("length", f.name.chars().count().to_string())]));
                    }
                }
                if f.name.is_empty() {
                    return Err(self.err("invalid"));
                }
                if !allow_empty_file && f.size == 0 {
                    return Err(self.err("empty"));
                }
                if matches!(self.kind, FieldKind::Image { .. }) {
                    let bytes = f.read().unwrap_or_default();
                    if !looks_like_image(&bytes) {
                        return Err(self.err("invalid_image"));
                    }
                }
                Ok(value.clone())
            }
            FieldKind::Boolean => Ok(match value {
                Value::Str(s) if matches!(s.to_lowercase().as_str(), "false" | "0") => Value::Bool(false),
                v => Value::Bool(v.truthy()),
            }),
            FieldKind::NullBoolean => Ok(match value {
                Value::Bool(b) => Value::Bool(*b),
                Value::Str(s) if matches!(s.as_str(), "True" | "true" | "1") => Value::Bool(true),
                Value::Str(s) if matches!(s.as_str(), "False" | "false" | "0") => Value::Bool(false),
                Value::Int(1) => Value::Bool(true),
                Value::Int(0) => Value::Bool(false),
                _ => Value::None,
            }),
            FieldKind::Choice | FieldKind::TypedChoice { .. } | FieldKind::FilePath => Ok(if value.is_empty() { Value::Str(String::new()) } else { Value::Str(value.to_text()) }),
            FieldKind::MultipleChoice | FieldKind::TypedMultipleChoice { .. } => match value {
                Value::List(l) => Ok(Value::List(l.iter().map(|v| Value::Str(v.to_text())).collect())),
                v if !v.truthy() => Ok(Value::List(vec![])),
                _ => Err(self.err("invalid_list")),
            },
            FieldKind::ModelChoice { multiple } => {
                let key_of = |text: &str| self.choices.iter().find_map(|c| match c {
                    Choice::Option(k, _) if !matches!(k, Value::Str(s) if s.is_empty()) && k.to_text() == text => Some(k.clone()),
                    _ => None,
                });
                if *multiple {
                    let items: Vec<Value> = match value {
                        Value::List(l) => l.clone(),
                        v if !v.truthy() => return Ok(Value::List(vec![])),
                        _ => return Err(self.err("invalid_list")),
                    };
                    let mut keys = Vec::new();
                    for v in items {
                        match key_of(&v.to_text()) {
                            Some(k) => keys.push(k),
                            None => return Err(self.err("invalid_choice").with_params([("value", v.to_text())])),
                        }
                    }
                    Ok(Value::List(keys))
                } else if value.is_empty() {
                    Ok(Value::None)
                } else {
                    key_of(&value.to_text()).ok_or_else(|| self.err("invalid_choice").with_params([("value", value.to_text())]))
                }
            }
            _ => Ok(value.clone()),
        }
    }

    fn char_to_clean(&self, value: &Value) -> Value {
        if value.is_empty() {
            return self.empty_string();
        }
        let s = value.to_text();
        let s = if self.strips() { s.trim().to_owned() } else { s };
        if s.is_empty() { self.empty_string() } else { Value::Str(s) }
    }

    fn empty_string(&self) -> Value {
        match &self.kind {
            FieldKind::Char { empty_value, .. } => empty_value.clone(),
            _ => Value::Str(String::new()),
        }
    }

    /// `valid_value(value)` for choice fields.
    pub fn valid_value(&self, value: &str) -> bool {
        let matches = |k: &Value| k.to_text() == value;
        self.choices.iter().any(|c| match c {
            Choice::Option(k, _) => matches(k),
            Choice::Group(_, items) => items.iter().any(|(k, _)| matches(k)),
        })
    }

    /// `validate(value)`.
    pub fn validate(&self, value: &Value) -> VResult<()> {
        match &self.kind {
            FieldKind::Boolean => {
                if !value.truthy() && self.required {
                    return Err(self.err("required"));
                }
                Ok(())
            }
            FieldKind::NullBoolean | FieldKind::MultiValue { .. } | FieldKind::SplitDateTime { .. } => Ok(()),
            FieldKind::Float { .. } => {
                self.base_validate(value)?;
                match value {
                    Value::Float(f) if !f.is_finite() => Err(self.err("invalid")),
                    _ => Ok(()),
                }
            }
            FieldKind::Choice | FieldKind::TypedChoice { .. } | FieldKind::FilePath => {
                self.base_validate(value)?;
                if value.truthy() && !self.valid_value(&value.to_text()) {
                    return Err(self.err("invalid_choice").with_params([("value", value.to_text())]));
                }
                Ok(())
            }
            FieldKind::MultipleChoice => {
                if self.required && !value.truthy() {
                    return Err(self.err("required"));
                }
                if let Value::List(l) = value {
                    for v in l {
                        if !self.valid_value(&v.to_text()) {
                            return Err(self.err("invalid_choice").with_params([("value", v.to_text())]));
                        }
                    }
                }
                Ok(())
            }
            FieldKind::TypedMultipleChoice { empty_value, .. } => {
                if value != empty_value {
                    if self.required && !value.truthy() {
                        return Err(self.err("required"));
                    }
                    if let Value::List(l) = value {
                        for v in l {
                            if !self.valid_value(&v.to_text()) {
                                return Err(self.err("invalid_choice").with_params([("value", v.to_text())]));
                            }
                        }
                    }
                } else if self.required {
                    return Err(self.err("required"));
                }
                Ok(())
            }
            FieldKind::ModelChoice { multiple } => {
                if *multiple {
                    if self.required && !value.truthy() {
                        return Err(self.err("required"));
                    }
                    return Ok(());
                }
                self.base_validate(value)
            }
            _ => self.base_validate(value),
        }
    }

    fn base_validate(&self, value: &Value) -> VResult<()> {
        if value.is_empty() && self.required { Err(self.err("required")) } else { Ok(()) }
    }

    /// `run_validators(value)`: collect every failure (messages overridden by `error_messages[code]`).
    pub fn run_validators(&self, value: &Value) -> VResult<()> {
        if value.is_empty() {
            return Ok(());
        }
        let mut errors = Vec::new();
        for v in &self.validators {
            if let Err(e) = v(value) {
                for mut one in e.error_list() {
                    if let ValidationError::Single { message, code: Some(c), .. } = &mut one {
                        if let Some(m) = self.error_messages.get(c.as_str()) {
                            message.clone_from(m);
                        }
                    }
                    errors.push(one);
                }
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(ValidationError::List(errors)) }
    }

    /// `clean(value)`.
    pub fn clean(&self, value: &Value) -> VResult<Value> {
        self.clean_with_initial(value, &Value::None)
    }

    pub fn clean_with_initial(&self, value: &Value, initial: &Value) -> VResult<Value> {
        match &self.kind {
            FieldKind::File { .. } | FieldKind::Image { .. } => {
                if Widget::is_file_contradiction(value) {
                    return Err(self.err("contradiction"));
                }
                let mut data = value.clone();
                if matches!(data, Value::Cleared) {
                    if !self.required {
                        return Ok(Value::Cleared);
                    }
                    data = Value::None;
                }
                if !data.truthy() && initial.truthy() {
                    return Ok(initial.clone());
                }
                self.standard_clean(&data)
            }
            FieldKind::TypedChoice { coerce, empty_value } => {
                let v = self.standard_clean(value)?;
                if v == *empty_value || v.is_empty() {
                    return Ok(empty_value.clone());
                }
                coerce(&v.to_text()).map_err(|()| self.err("invalid_choice").with_params([("value", v.to_text())]))
            }
            FieldKind::TypedMultipleChoice { coerce, empty_value } => {
                let v = self.standard_clean(value)?;
                if v == *empty_value || v.is_empty() {
                    return Ok(empty_value.clone());
                }
                let Value::List(items) = &v else { return Ok(v) };
                let mut out = Vec::new();
                for c in items {
                    out.push(coerce(&c.to_text()).map_err(|()| self.err("invalid_choice").with_params([("value", c.to_text())]))?);
                }
                Ok(Value::List(out))
            }
            FieldKind::Combo { fields } => {
                self.standard_clean(value)?;
                let mut v = value.clone();
                for f in fields {
                    v = f.clean(&v)?;
                }
                Ok(v)
            }
            FieldKind::MultiValue { fields, require_all_fields, compress } => self.clean_multi(value, fields, *require_all_fields, &|d| compress(d)),
            FieldKind::SplitDateTime { fields } => self.clean_multi(value, fields, true, &|d| self.split_compress(d)),
            _ => self.standard_clean(value),
        }
    }

    fn standard_clean(&self, value: &Value) -> VResult<Value> {
        let v = self.to_clean(value)?;
        self.validate(&v)?;
        self.run_validators(&v)?;
        Ok(v)
    }

    fn split_compress(&self, data: &[Value]) -> VResult<Value> {
        if data.is_empty() {
            return Ok(Value::None);
        }
        if data[0].is_empty() {
            return Err(self.err("invalid_date"));
        }
        if data[1].is_empty() {
            return Err(self.err("invalid_time"));
        }
        match (&data[0], &data[1]) {
            (Value::Date(d), Value::Time(t)) => from_current_timezone(Value::NaiveDateTime(d.and_time(*t))),
            _ => Ok(Value::None),
        }
    }

    fn clean_multi(&self, value: &Value, fields: &[Field], require_all: bool, compress: &dyn Fn(&[Value]) -> VResult<Value>) -> VResult<Value> {
        let mut value = value.clone();
        if self.disabled && !matches!(value, Value::List(_)) {
            value = Value::List(self.widget.decompress(&value));
        }
        let items: Vec<Value> = match &value {
            Value::List(l) => l.clone(),
            v if !v.truthy() => vec![],
            _ => return Err(self.err("invalid")),
        };
        if items.iter().all(Value::is_empty) {
            return if self.required { Err(self.err("required")) } else { compress(&[]) };
        }
        let mut clean_data = Vec::new();
        let mut errors: Vec<ValidationError> = Vec::new();
        for (i, field) in fields.iter().enumerate() {
            let fv = items.get(i).cloned().unwrap_or(Value::None);
            if fv.is_empty() {
                if require_all {
                    if self.required {
                        return Err(self.err("required"));
                    }
                } else if field.required {
                    let inc = field.error_messages.get("incomplete").cloned().unwrap_or_default();
                    if !errors.iter().any(|e| e.messages().first() == Some(&inc)) {
                        errors.push(ValidationError::new(inc).with_code("incomplete"));
                    }
                    continue;
                }
            }
            match field.clean(&fv) {
                Ok(v) => clean_data.push(v),
                Err(e) => {
                    for one in e.error_list() {
                        if !errors.contains(&one) {
                            errors.push(one);
                        }
                    }
                }
            }
        }
        if !errors.is_empty() {
            return Err(ValidationError::List(errors));
        }
        let out = compress(&clean_data)?;
        self.validate(&out)?;
        self.run_validators(&out)?;
        Ok(out)
    }

    /// `bound_data(data, initial)`.
    pub fn bound_data(&self, data: &Value, initial: &Value) -> Value {
        match &self.kind {
            _ if self.disabled => initial.clone(),
            FieldKind::File { .. } | FieldKind::Image { .. } => initial.clone(),
            FieldKind::Json => match data {
                Value::None => Value::None,
                Value::Str(s) => match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(j) => Value::Json(j),
                    Err(_) => Value::Str(format!("\u{0}{s}")),
                },
                other => other.clone(),
            },
            _ => data.clone(),
        }
    }

    /// `has_changed(initial, data)`.
    pub fn has_changed(&self, initial: &Value, data: &Value) -> bool {
        if self.disabled {
            return false;
        }
        match &self.kind {
            FieldKind::File { .. } | FieldKind::Image { .. } => !matches!(data, Value::None),
            FieldKind::Boolean | FieldKind::NullBoolean => self.to_clean(initial).ok() != self.to_clean(data).ok(),
            FieldKind::MultipleChoice | FieldKind::TypedMultipleChoice { .. } | FieldKind::ModelChoice { multiple: true } => {
                let set = |v: &Value| -> Vec<String> {
                    let mut s: Vec<String> = match v {
                        Value::List(l) => l.iter().map(Value::to_text).collect(),
                        _ => vec![],
                    };
                    s.sort();
                    s.dedup();
                    s
                };
                let (a, b) = (set(initial), set(data));
                let len = |v: &Value| if let Value::List(l) = v { l.len() } else { 0 };
                len(initial) != len(data) || a != b
            }
            FieldKind::TypedChoice { coerce, empty_value } => {
                let c = |v: &Value| -> Option<Value> {
                    if v == empty_value || v.is_empty() {
                        return Some(empty_value.clone());
                    }
                    coerce(&v.to_text()).ok()
                };
                match self.to_clean(data) {
                    Ok(d) => c(&d) != c(initial),
                    Err(_) => true,
                }
            }
            FieldKind::MultiValue { fields, .. } | FieldKind::SplitDateTime { fields } => {
                let data_list: Vec<Value> = match data {
                    Value::List(l) => l.clone(),
                    _ => vec![],
                };
                let init: Vec<Value> = match initial {
                    Value::None => vec![Value::Str(String::new()); data_list.len()],
                    Value::List(l) => l.clone(),
                    v => self.widget.decompress(v),
                };
                for ((f, i), d) in fields.iter().zip(init).zip(&data_list) {
                    match f.to_clean(&i) {
                        Ok(i) => {
                            if f.has_changed(&i, d) {
                                return true;
                            }
                        }
                        Err(_) => return true,
                    }
                }
                false
            }
            FieldKind::Json => {
                let to_j = |v: &Value| match v {
                    Value::Json(j) => Some(j.clone()),
                    Value::None => Some(serde_json::Value::Null),
                    Value::Str(s) => serde_json::from_str(s).ok(),
                    _ => None,
                };
                match self.to_clean(data) {
                    Err(_) => true,
                    Ok(d) => {
                        let dj = to_j(&d).unwrap_or(serde_json::Value::Null);
                        let ij = to_j(initial).unwrap_or(serde_json::Value::Null);
                        let dumps = |j: &serde_json::Value| rango_core::utils::json::dumps_with(j, false, false).unwrap_or_default();
                        dumps(&ij) != dumps(&dj)
                    }
                }
            }
            _ => match self.to_clean(data) {
                Err(_) => true,
                Ok(d) => {
                    let norm = |v: &Value| if matches!(v, Value::None) { Value::Str(String::new()) } else { v.clone() };
                    norm(initial) != norm(&d)
                }
            },
        }
    }

    pub fn hidden_widget(&self) -> Widget {
        match &self.kind {
            FieldKind::MultipleChoice | FieldKind::TypedMultipleChoice { .. } | FieldKind::ModelChoice { multiple: true } => Widget::multiple_hidden_input(),
            FieldKind::SplitDateTime { .. } => Widget::split_hidden_datetime(),
            _ => Widget::hidden_input(),
        }
    }

    /// `widget.value_from_datadict` for this field.
    pub fn value_from_datadict(&self, data: &Data, files: &FileData, name: &str) -> Value {
        self.widget.value_from_datadict(data, files, name)
    }

    pub fn max_len(&self) -> Option<usize> {
        self.max_length
    }

    /// Mark the widget attrs for `required`; used by `BoundField`.
    pub fn widget_attr(&self, name: &str) -> Option<&Attr> {
        self.widget.attrs.get(name)
    }
}

fn fmt_f(f: f64) -> String {
    if f.fract() == 0.0 { format!("{}", f as i64) } else { f.to_string() }
}

fn file_messages() -> IndexMap<String, String> {
    let mut m = msgs(&[
        ("invalid", "No file was submitted. Check the encoding type on the form."),
        ("missing", "No file was submitted."),
        ("empty", "The submitted file is empty."),
        ("contradiction", "Please either submit a file or check the clear checkbox, not both."),
    ]);
    m.insert("max_length".into(), ngettext("Ensure this filename has at most %(max)d character (it has %(length)d).", "Ensure this filename has at most %(max)d characters (it has %(length)d).", 1));
    m
}

/// `int(str)`: optional sign, digits with single underscores; trailing `.0*` is stripped first.
fn parse_int(s: &str) -> Option<i64> {
    let s = s.trim();
    let s = match s.find('.') {
        Some(i) if s[i + 1..].chars().all(|c| c == '0') => &s[..i],
        _ => s,
    };
    let (neg, digits) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") || !digits.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return None;
    }
    let n: i64 = digits.replace('_', "").parse().ok()?;
    Some(if neg { -n } else { n })
}

fn parse_float(s: &str) -> Option<f64> {
    if s.is_empty() || s.starts_with('_') || s.ends_with('_') || s.contains("__") {
        return None;
    }
    s.replace('_', "").parse().ok()
}

/// strptime formats are chrono-compatible except `%f` (microseconds, 1–6 digits).
fn py_fmt(f: &str) -> String {
    f.replace("%f", "%6f")
}

/// Magic-number check standing in for PIL's `Image.verify()`.
fn looks_like_image(b: &[u8]) -> bool {
    b.starts_with(b"\x89PNG\r\n\x1a\n") || b.starts_with(&[0xff, 0xd8, 0xff]) || b.starts_with(b"GIF8") || b.starts_with(b"BM") || (b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP") || b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") || b.starts_with(&[0, 0, 1, 0])
}

pub(crate) fn to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::None | Value::Cleared => serde_json::Value::Null,
        Value::Bool(b) => (*b).into(),
        Value::Int(i) => (*i).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f).map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::Json(j) => j.clone(),
        Value::List(l) => serde_json::Value::Array(l.iter().map(to_json).collect()),
        other => other.to_text().into(),
    }
}

macro_rules! field_classes {
    ($($name:ident => $ctor:ident),* $(,)?) => {$(
        #[doc = concat!("the `forms.", stringify!($name), "`: `", stringify!($name), "::new()` builds a [`Field`].")]
        pub struct $name;
        impl $name { pub fn new() -> Field { Field::$ctor() } }
    )*};
}
field_classes!(CharField => char, IntegerField => integer, FloatField => float, DecimalField => decimal, DateField => date, TimeField => time,
    DateTimeField => datetime, DurationField => duration, EmailField => email, FileField => file, ImageField => image, URLField => url,
    BooleanField => boolean, NullBooleanField => null_boolean, SplitDateTimeField => split_datetime, SlugField => slug, UUIDField => uuid, JSONField => json);

pub struct ChoiceField;
impl ChoiceField {
    pub fn new(choices: Vec<Choice>) -> Field {
        Field::choice(choices)
    }
}
pub struct MultipleChoiceField;
impl MultipleChoiceField {
    pub fn new(choices: Vec<Choice>) -> Field {
        Field::multiple_choice(choices)
    }
}
pub struct RegexField;
impl RegexField {
    pub fn new(pattern: &str) -> Field {
        Field::regex(pattern)
    }
}
