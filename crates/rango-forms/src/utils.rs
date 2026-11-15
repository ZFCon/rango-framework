//! `forms.utils`: `flatatt`, `ErrorList`, `ErrorDict`, timezone helpers.

use std::sync::Arc;

use indexmap::IndexMap;
use rango_core::exceptions::ValidationError;
use rango_core::utils::html::escape;
use rango_template::{Object, TemplateObject, Value as T};

use crate::renderers::Renderer;
use crate::widgets::{Attr, Attrs};

/// `pretty_name("first_name") == "First name"`.
pub fn pretty_name(name: &str) -> String {
    let s = name.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// `flatatt(attrs)`: ` key="value"` pairs sorted by key, then boolean attributes.
pub fn flatatt(attrs: &Attrs) -> String {
    let mut kv: Vec<(&String, &String)> = Vec::new();
    let mut flags: Vec<&String> = Vec::new();
    for (k, v) in attrs {
        match v {
            Attr::Bool(true) => flags.push(k),
            Attr::Bool(false) => {}
            Attr::Str(s) => kv.push((k, s)),
        }
    }
    kv.sort();
    flags.sort();
    let mut out = String::new();
    for (k, v) in kv {
        out.push_str(&format!(" {}=\"{}\"", escape(k), escape(v)));
    }
    for k in flags {
        out.push_str(&format!(" {}", escape(k)));
    }
    out
}

/// `forms.utils.ErrorList`: a list of [`ValidationError`]s that renders as `<ul class="errorlist">`.
#[derive(Clone)]
pub struct ErrorList {
    data: Vec<ValidationError>,
    pub error_class: String,
    pub renderer: Arc<dyn Renderer>,
    pub field_id: Option<String>,
}

impl std::fmt::Debug for ErrorList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.messages())
    }
}

impl ErrorList {
    /// `ErrorList(error_class=..., renderer=...)`.
    pub fn new(error_class: Option<&str>, renderer: Arc<dyn Renderer>) -> Self {
        let error_class = match error_class {
            None => "errorlist".to_owned(),
            Some(c) => format!("errorlist {c}"),
        };
        Self { data: Vec::new(), error_class, renderer, field_id: None }
    }

    pub fn with_field_id(mut self, id: Option<String>) -> Self {
        self.field_id = id;
        self
    }

    pub fn extend(&mut self, errors: impl IntoIterator<Item = ValidationError>) {
        self.data.extend(errors);
    }

    pub fn push(&mut self, e: ValidationError) {
        self.data.push(e);
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// `as_data()`: the underlying `ValidationError`s (with codes and params).
    pub fn as_data(&self) -> &[ValidationError] {
        &self.data
    }

    /// The formatted messages (`list(error_list)`).
    pub fn messages(&self) -> Vec<String> {
        self.data.iter().flat_map(ValidationError::messages).collect()
    }

    pub fn get_json_data(&self, escape_html: bool) -> serde_json::Value {
        serde_json::Value::Array(
            self.data
                .iter()
                .map(|e| {
                    let m = e.messages().into_iter().next().unwrap_or_default();
                    serde_json::json!({"message": if escape_html { escape(&m).into_string() } else { m }, "code": e.code().unwrap_or("")})
                })
                .collect(),
        )
    }

    pub fn as_json(&self, escape_html: bool) -> String {
        rango_core::utils::json::dumps(&self.get_json_data(escape_html), false).unwrap_or_default()
    }

    fn context(&self) -> T {
        let mut m = IndexMap::new();
        m.insert("errors".to_owned(), Object::new(self.clone()));
        m.insert("error_class".to_owned(), T::Str(self.error_class.clone()));
        T::Map(m)
    }

    fn render_with(&self, template: &str) -> String {
        self.renderer.render(template, self.context()).unwrap_or_default()
    }

    /// `str(error_list)`: the default template (a `<ul>`).
    pub fn render(&self) -> String {
        self.render_with("rango/forms/errors/list/default.html")
    }

    pub fn as_ul(&self) -> String {
        self.render_with("rango/forms/errors/list/ul.html")
    }

    pub fn as_text(&self) -> String {
        self.render_with("rango/forms/errors/list/text.txt")
    }
}

impl TemplateObject for ErrorList {
    fn get(&self, key: &str) -> Option<T> {
        Some(match key {
            "field_id" => self.field_id.clone().map_or(T::None, T::Str),
            "error_class" => T::Str(self.error_class.clone()),
            "as_ul" => T::Safe(self.as_ul()),
            "as_text" => T::Safe(self.as_text()),
            "as_json" => T::Safe(self.as_json(false)),
            _ => return None,
        })
    }
    fn display(&self) -> T {
        T::Safe(self.render())
    }
    fn iterate(&self) -> Option<Vec<T>> {
        Some(self.messages().into_iter().map(T::Str).collect())
    }
    fn truthy(&self) -> bool {
        !self.data.is_empty()
    }
}

/// `forms.utils.ErrorDict`: field name -> [`ErrorList`].
#[derive(Clone)]
pub struct ErrorDict {
    pub items: IndexMap<String, ErrorList>,
    pub renderer: Arc<dyn Renderer>,
}

impl std::fmt::Debug for ErrorDict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.items.iter().map(|(k, v)| (k, v.messages()))).finish()
    }
}

impl ErrorDict {
    pub fn new(renderer: Arc<dyn Renderer>) -> Self {
        Self { items: IndexMap::new(), renderer }
    }

    pub fn get(&self, field: &str) -> Option<&ErrorList> {
        self.items.get(field)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains_key(&self, field: &str) -> bool {
        self.items.contains_key(field)
    }

    pub fn as_data(&self) -> IndexMap<&str, &[ValidationError]> {
        self.items.iter().map(|(k, v)| (k.as_str(), v.as_data())).collect()
    }

    pub fn get_json_data(&self, escape_html: bool) -> serde_json::Value {
        serde_json::Value::Object(self.items.iter().map(|(k, v)| (k.clone(), v.get_json_data(escape_html))).collect())
    }

    pub fn as_json(&self, escape_html: bool) -> String {
        rango_core::utils::json::dumps(&self.get_json_data(escape_html), false).unwrap_or_default()
    }

    fn render_with(&self, template: &str) -> String {
        let items = T::List(self.items.iter().map(|(k, v)| T::List(vec![T::Str(k.clone()), Object::new(v.clone())])).collect());
        let ctx = T::Map([("errors".to_owned(), items), ("error_class".to_owned(), T::Str("errorlist".into()))].into_iter().collect());
        self.renderer.render(template, ctx).unwrap_or_default()
    }

    pub fn render(&self) -> String {
        self.render_with("rango/forms/errors/dict/default.html")
    }

    pub fn as_ul(&self) -> String {
        self.render_with("rango/forms/errors/dict/ul.html")
    }

    pub fn as_text(&self) -> String {
        self.render_with("rango/forms/errors/dict/text.txt")
    }
}

impl TemplateObject for ErrorDict {
    fn get(&self, key: &str) -> Option<T> {
        match key {
            "as_ul" => Some(T::Safe(self.as_ul())),
            "as_text" => Some(T::Safe(self.as_text())),
            "as_json" => Some(T::Safe(self.as_json(false))),
            k => self.items.get(k).map(|l| Object::new(l.clone())),
        }
    }
    fn display(&self) -> T {
        T::Safe(self.render())
    }
    fn iterate(&self) -> Option<Vec<T>> {
        Some(self.items.keys().cloned().map(T::Str).collect())
    }
    fn truthy(&self) -> bool {
        !self.items.is_empty()
    }
}

/// `from_current_timezone(naive)`: make a naive datetime aware (when `USE_TZ`).
pub fn from_current_timezone(value: crate::Value) -> Result<crate::Value, ValidationError> {
    use crate::Value;
    let Value::NaiveDateTime(n) = value else { return Ok(value) };
    if !rango_core::conf::settings().use_tz {
        return Ok(Value::NaiveDateTime(n));
    }
    let tz = rango_core::utils::timezone::get_current_timezone().map_err(|e| ValidationError::new(e.to_string()))?;
    match chrono::TimeZone::from_local_datetime(&tz, &n) {
        chrono::LocalResult::Single(dt) => Ok(Value::DateTime(dt.with_timezone(&chrono::Utc))),
        _ => Err(ValidationError::new(rango_core::utils::translation::gettext(
            "%(datetime)s couldn’t be interpreted in time zone %(current_timezone)s; it may be ambiguous or it may not exist.",
        ))
        .with_code("ambiguous_timezone")
        .with_params([("datetime", n.to_string()), ("current_timezone", tz.to_string())])),
    }
}

/// `to_current_timezone(aware)`: naive local time (when `USE_TZ`).
pub fn to_current_timezone(value: crate::Value) -> crate::Value {
    use crate::Value;
    match value {
        Value::DateTime(d) if rango_core::conf::settings().use_tz => rango_core::utils::timezone::get_current_timezone().map_or(Value::DateTime(d), |tz| Value::NaiveDateTime(rango_core::utils::timezone::make_naive(&d, tz))),
        other => other,
    }
}
