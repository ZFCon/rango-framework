//! `template.loader`, `RequestContext`, `context_processors` and `TemplateResponse`.
//!
//! The global engine is built from `settings.TEMPLATES[0]` and rebuilt when that setting
//! (or `DEBUG`) changes, so `override_settings` works.

use std::any::Any;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use rango_core::conf::try_settings;
use rango_http::response::DeferredRender;
use rango_http::{HttpRequest, HttpResponse};

use crate::context::Context;
use crate::engine::{Engine, Template, TemplateError};
use crate::value::Value;

type Result<T> = std::result::Result<T, TemplateError>;

static ENGINE: Mutex<Option<(String, Arc<Engine>)>> = Mutex::new(None);

/// `engines["rango"]`: the engine for the current settings.
pub fn engine() -> Result<Arc<Engine>> {
    let s = try_settings().map_err(|e| TemplateError::Config(e.to_string()))?;
    if s.templates.is_empty() {
        return Err(TemplateError::Config("No template engine is configured: TEMPLATES is empty.".into()));
    }
    let key = format!("{}{}", s.debug, serde_json::to_string(&s.templates).unwrap_or_default());
    let mut slot = ENGINE.lock().expect("engine lock");
    if let Some((k, e)) = slot.as_ref() {
        if *k == key {
            return Ok(Arc::clone(e));
        }
    }
    let e = Arc::new(Engine::from_settings());
    *slot = Some((key, Arc::clone(&e)));
    Ok(e)
}

/// Template name(s) accepted by the loader functions (`str` or a list of names).
pub trait TemplateNames {
    fn names(&self) -> Vec<String>;
}
impl TemplateNames for &str {
    fn names(&self) -> Vec<String> {
        vec![(*self).to_owned()]
    }
}
impl TemplateNames for String {
    fn names(&self) -> Vec<String> {
        vec![self.clone()]
    }
}
impl<const N: usize> TemplateNames for [&str; N] {
    fn names(&self) -> Vec<String> {
        self.iter().map(|s| (*s).to_owned()).collect()
    }
}
impl TemplateNames for &[&str] {
    fn names(&self) -> Vec<String> {
        self.iter().map(|s| (*s).to_owned()).collect()
    }
}
impl TemplateNames for Vec<String> {
    fn names(&self) -> Vec<String> {
        self.clone()
    }
}

/// `loader.get_template(name)`.
pub fn get_template(name: &str) -> Result<Arc<Template>> {
    engine()?.get_template(name)
}

/// `loader.select_template([names])`.
pub fn select_template(names: &[&str]) -> Result<Arc<Template>> {
    engine()?.select_template(names)
}

/// `RequestContext`: user variables over the processors' output (csrf first, then `OPTIONS["context_processors"]`).
pub fn bind_request(ctx: &mut Context, request: &HttpRequest, engine: &Engine) -> Result<()> {
    let mut updates: Vec<(String, Value)> = Vec::new();
    let builtin = ["rango.template.context_processors.csrf"];
    for name in builtin.iter().copied().chain(engine.context_processors.iter().map(String::as_str)) {
        match registered_processor(name) {
            Some(f) => updates.extend(f(request)),
            None if builtin.contains(&name) => {} // csrf is provided by rango-middleware; absent when unlinked
            None => return Err(TemplateError::Config(format!("Context processor '{name}' is not registered. Use register_context_processor!."))),
        }
    }
    ctx.insert_below(updates);
    Ok(())
}

/// `render_to_string(template_name, context, request)`.
pub fn render_to_string(names: impl TemplateNames, mut context: Context, request: Option<&HttpRequest>) -> Result<String> {
    let engine = engine()?;
    let names = names.names();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let template = engine.select_template(&refs)?;
    if let Some(r) = request {
        bind_request(&mut context, r, &engine)?;
    }
    template.render(&engine, &mut context)
}

/// A context processor: `fn(request) -> extra variables`.
pub type ContextProcessor = fn(&HttpRequest) -> IndexMap<String, Value>;

pub struct ContextProcessorRegistration {
    pub name: &'static str,
    pub f: ContextProcessor,
}

inventory::collect!(ContextProcessorRegistration);

/// `register_context_processor!("polls.context_processors.site", site_processor)`.
#[macro_export]
macro_rules! register_context_processor {
    ($name:expr, $f:expr) => {
        $crate::inventory::submit! { $crate::loader::ContextProcessorRegistration { name: $name, f: $f } }
    };
}

fn registered_processor(name: &str) -> Option<ContextProcessor> {
    inventory::iter::<ContextProcessorRegistration>.into_iter().find(|r| r.name == name).map(|r| r.f)
}

/// `template.context_processors`.
mod processors {
    use super::*;
    use rango_core::conf::settings;

    fn s(v: impl Into<String>) -> Value {
        Value::Str(v.into())
    }

    fn map(pairs: Vec<(&str, Value)>) -> IndexMap<String, Value> {
        pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }

    pub fn debug(request: &HttpRequest) -> IndexMap<String, Value> {
        let st = settings();
        let ip = request.meta().get("REMOTE_ADDR").cloned().unwrap_or_default();
        if st.debug && st.internal_ips.contains(&ip) {
            map(vec![("debug", Value::Bool(true)), ("sql_queries", Value::List(vec![]))])
        } else {
            IndexMap::new()
        }
    }

    pub fn request(request: &HttpRequest) -> IndexMap<String, Value> {
        let lists = |q: &rango_http::querydict::QueryDict| Value::Map(q.lists().map(|(k, v)| (k.to_owned(), s(v.last().cloned().unwrap_or_default()))).collect());
        let mut m = IndexMap::new();
        m.insert("path".into(), s(request.path()));
        m.insert("path_info".into(), s(request.path_info()));
        m.insert("method".into(), s(request.method()));
        m.insert("scheme".into(), s(request.scheme()));
        m.insert("GET".into(), lists(request.query()));
        m.insert("COOKIES".into(), Value::Map(request.cookies().iter().map(|(k, v)| (k.clone(), s(v.clone()))).collect()));
        m.insert("META".into(), Value::Map(request.meta().iter().map(|(k, v)| (k.clone(), s(v.clone()))).collect()));
        m.insert("is_secure".into(), Value::Bool(request.is_secure()));
        m.insert("get_full_path".into(), s(request.get_full_path()));
        if let Ok(h) = request.get_host() {
            m.insert("get_host".into(), s(h));
        }
        if let Some(rm) = request.ext::<rango_urls::ResolverMatch>() {
            m.insert(
                "resolver_match".into(),
                Value::Map(
                    map(vec![
                        ("url_name", rm.url_name.clone().map_or(Value::None, Value::Str)),
                        ("view_name", s(rm.view_name())),
                        ("app_name", s(rm.app_name())),
                        ("namespace", s(rm.namespace())),
                        ("route", s(rm.route.clone())),
                    ]),
                ),
            );
        }
        map(vec![("request", Value::Map(m))])
    }

    pub fn static_url(_: &HttpRequest) -> IndexMap<String, Value> {
        map(vec![("STATIC_URL", s(settings().static_url.clone().unwrap_or_default()))])
    }

    pub fn media(_: &HttpRequest) -> IndexMap<String, Value> {
        map(vec![("MEDIA_URL", s(settings().media_url.clone()))])
    }

    pub fn tz(_: &HttpRequest) -> IndexMap<String, Value> {
        map(vec![("TIME_ZONE", s(settings().time_zone.clone()))])
    }
}

crate::register_context_processor!("rango.template.context_processors.debug", processors::debug);
crate::register_context_processor!("rango.template.context_processors.request", processors::request);
crate::register_context_processor!("rango.template.context_processors.static", processors::static_url);
crate::register_context_processor!("rango.template.context_processors.media", processors::media);
crate::register_context_processor!("rango.template.context_processors.tz", processors::tz);

/// `TemplateResponse`: renders after the middleware chain had a chance to change it.
pub struct TemplateResponse {
    pub template_names: Vec<String>,
    pub context_data: Option<Context>,
    pub request: Option<HttpRequest>,
}

impl TemplateResponse {
    /// `TemplateResponse(request, template, context)` as an [`HttpResponse`] with deferred content.
    pub fn new(request: &HttpRequest, names: impl TemplateNames, context: Context) -> HttpResponse {
        HttpResponse::new("").with_deferred(TemplateResponse { template_names: names.names(), context_data: Some(context), request: Some(request.clone()) })
    }
}

impl DeferredRender for TemplateResponse {
    fn render(&mut self, response: &mut HttpResponse) -> rango_core::Result<()> {
        let ctx = self.context_data.take().unwrap_or_default();
        let names: Vec<&str> = self.template_names.iter().map(String::as_str).collect();
        let html = render_to_string(names.as_slice(), ctx, self.request.as_ref())?;
        response.set_content(html);
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
