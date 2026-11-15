//! `Engine`, `Template`, loaders and libraries (`template.engine`, `library`, `loaders`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use indexmap::IndexMap;

use crate::context::Context;
use crate::nodes::Node;
use crate::parser::{FilterExpr, Parser};
use crate::value::Value;

/// `TemplateSyntaxError` / `TemplateDoesNotExist` / render errors.
#[derive(Debug, Clone, thiserror::Error)]
pub enum TemplateError {
    #[error("{0}")]
    Syntax(String),
    #[error("{0}")]
    DoesNotExist(String),
    #[error("{0}")]
    Render(String),
    /// `ImproperlyConfigured`: no usable `TEMPLATES` setting.
    #[error("{0}")]
    Config(String),
}

impl TemplateError {
    pub(crate) fn at_line(self, line: usize) -> Self {
        match self {
            TemplateError::Syntax(m) if !m.contains(" line ") => TemplateError::Syntax(format!("{m} (line {line})")),
            other => other,
        }
    }
}

impl From<TemplateError> for rango_core::Error {
    fn from(e: TemplateError) -> Self {
        if let TemplateError::Config(m) = &e {
            return rango_core::Error::improperly_configured(m.clone());
        }
        rango_core::Error::Other(Box::new(e))
    }
}

pub type FilterFn = fn(&Value, Option<&Value>, bool) -> Result<Value, TemplateError>;

/// A registered filter and its flags.
#[derive(Clone, Copy)]
pub struct Filter {
    pub f: FilterFn,
    /// Output is safe if the input was (`is_safe=True`).
    pub is_safe: bool,
    pub needs_autoescape: bool,
}

pub type TagCompile = Arc<dyn Fn(&mut Parser<'_>, Vec<String>) -> Result<Node, TemplateError> + Send + Sync>;
pub type SimpleTag = Arc<dyn Fn(&mut Context, Vec<Value>, IndexMap<String, Value>) -> Result<Value, TemplateError> + Send + Sync>;

/// A tag library (`template.Library`).
#[derive(Default, Clone)]
pub struct Library {
    pub tags: HashMap<String, TagCompile>,
    pub filters: HashMap<String, Filter>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// `@register.filter`.
    pub fn filter(&mut self, name: &str, f: FilterFn, is_safe: bool, needs_autoescape: bool) -> &mut Self {
        self.filters.insert(name.to_owned(), Filter { f, is_safe, needs_autoescape });
        self
    }

    /// `@register.tag`: a custom compile function.
    pub fn tag(&mut self, name: &str, compile: TagCompile) -> &mut Self {
        self.tags.insert(name.to_owned(), compile);
        self
    }

    /// `@register.simple_tag`: `{% name arg key=value as var %}`.
    pub fn simple_tag(&mut self, name: &str, f: SimpleTag) -> &mut Self {
        let tag_name = name.to_owned();
        self.tag(
            name,
            Arc::new(move |_p, bits| crate::tags::compile_simple_tag(&tag_name, bits, f.clone())),
        )
    }

    /// `@register.inclusion_tag(template)`.
    pub fn inclusion_tag(&mut self, name: &str, template: &str, f: Arc<dyn Fn(&mut Context, Vec<Value>, IndexMap<String, Value>) -> Result<IndexMap<String, Value>, TemplateError> + Send + Sync>) -> &mut Self {
        let tpl = template.to_owned();
        let tag_name = name.to_owned();
        self.tag(name, Arc::new(move |_p, bits| crate::tags::compile_inclusion_tag(&tag_name, bits, &tpl, f.clone())))
    }
}

/// Link-time registration of a library under its `{% load %}` name.
pub struct LibraryRegistration {
    pub name: &'static str,
    pub build: fn() -> Library,
}

inventory::collect!(LibraryRegistration);

/// `register_library!("polls_tags", || { let mut l = Library::new(); ...; l })`.
#[macro_export]
macro_rules! register_library {
    ($name:expr, $build:expr) => {
        $crate::inventory::submit! { $crate::engine::LibraryRegistration { name: $name, build: $build } }
    };
}

/// A compiled template.
#[derive(Debug)]
pub struct Template {
    pub name: String,
    pub nodes: Vec<Node>,
}

#[derive(Clone)]
enum Source {
    /// In-memory templates (`locmem.Loader`).
    Memory(Arc<RwLock<HashMap<String, String>>>),
}

/// The template engine.
pub struct Engine {
    pub dirs: Vec<PathBuf>,
    pub app_dirs: Vec<PathBuf>,
    pub autoescape: bool,
    pub string_if_invalid: String,
    pub debug: bool,
    pub extra_libraries: Vec<(String, Arc<Library>)>,
    /// `OPTIONS["context_processors"]` (registered names, see `register_context_processor!`).
    pub context_processors: Vec<String>,
    builtins: Vec<Arc<Library>>,
    sources: Vec<Source>,
    cache: Mutex<HashMap<String, Arc<Template>>>,
    pub cache_enabled: bool,
}

static BUILTINS: LazyLock<Vec<Arc<Library>>> = LazyLock::new(|| vec![Arc::new(crate::tags::default_tags()), Arc::new(crate::filters::default_filters())]);

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            dirs: Vec::new(),
            app_dirs: Vec::new(),
            autoescape: true,
            string_if_invalid: String::new(),
            debug: false,
            extra_libraries: Vec::new(),
            context_processors: Vec::new(),
            builtins: BUILTINS.clone(),
            sources: Vec::new(),
            cache: Mutex::new(HashMap::new()),
            cache_enabled: false,
        }
    }

    /// Engine configured from `settings.TEMPLATES[0]` (`DIRS`, `APP_DIRS`, `OPTIONS`).
    pub fn from_settings() -> Engine {
        let mut e = Engine::new();
        let Ok(s) = rango_core::conf::try_settings() else { return e };
        e.debug = s.debug;
        e.cache_enabled = !s.debug;
        if let Some(cfg) = s.templates.first() {
            e.dirs = cfg.get("DIRS").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(PathBuf::from)).collect()).unwrap_or_default();
            if cfg.get("APP_DIRS").and_then(|v| v.as_bool()).unwrap_or(false) {
                if let Ok(apps) = rango_core::apps::apps() {
                    e.app_dirs = apps.get_app_configs().filter_map(|c| c.path.as_ref().map(|p| p.join("templates"))).collect();
                }
            }
            if let Some(opts) = cfg.get("OPTIONS") {
                if let Some(a) = opts.get("autoescape").and_then(|v| v.as_bool()) {
                    e.autoescape = a;
                }
                if let Some(v) = opts.get("string_if_invalid").and_then(|v| v.as_str()) {
                    e.string_if_invalid = v.to_owned();
                }
                if let Some(d) = opts.get("debug").and_then(|v| v.as_bool()) {
                    e.debug = d;
                }
                let names = |k: &str| -> Vec<String> {
                    opts.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
                };
                e.context_processors = names("context_processors");
                // `builtins`: libraries whose tags/filters are always loaded.
                for n in names("builtins") {
                    if let Some(l) = e.library(&n) {
                        e.builtins.push(l);
                    }
                }
                // `libraries`: `{alias: registered library name}` for `{% load alias %}`.
                if let Some(m) = opts.get("libraries").and_then(|v| v.as_object()) {
                    for (alias, target) in m {
                        if let Some(l) = target.as_str().and_then(|t| e.library(t)) {
                            e.extra_libraries.push((alias.clone(), l));
                        }
                    }
                }
            }
        }
        e
    }

    /// Add an in-memory template source; returns a handle for adding templates.
    pub fn locmem(&mut self, templates: &[(&str, &str)]) -> &mut Self {
        let map: HashMap<String, String> = templates.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        self.sources.push(Source::Memory(Arc::new(RwLock::new(map))));
        self
    }

    /// Libraries available to `{% load %}`: registered ones plus engine extras.
    pub fn library(&self, name: &str) -> Option<Arc<Library>> {
        if let Some((_, l)) = self.extra_libraries.iter().find(|(n, _)| n == name) {
            return Some(Arc::clone(l));
        }
        if let Some(r) = inventory::iter::<LibraryRegistration>.into_iter().find(|r| r.name == name) {
            return Some(Arc::new((r.build)()));
        }
        crate::tags::builtin_extra_library(name).map(Arc::new)
    }

    pub(crate) fn builtins(&self) -> &[Arc<Library>] {
        &self.builtins
    }

    fn find_source(&self, name: &str) -> Option<String> {
        for s in &self.sources {
            let Source::Memory(m) = s;
            if let Some(t) = m.read().expect("locmem lock").get(name) {
                return Some(t.clone());
            }
        }
        for dir in self.dirs.iter().chain(&self.app_dirs) {
            let Ok(p) = rango_core::utils::os::safe_join(dir, &[name]) else { continue };
            if let Ok(t) = std::fs::read_to_string(&p) {
                return Some(t);
            }
        }
        None
    }

    /// `engine.get_template(name)`.
    pub fn get_template(&self, name: &str) -> Result<Arc<Template>, TemplateError> {
        if self.cache_enabled {
            if let Some(t) = self.cache.lock().expect("cache lock").get(name) {
                return Ok(Arc::clone(t));
            }
        }
        let src = self.find_source(name).ok_or_else(|| TemplateError::DoesNotExist(name.to_owned()))?;
        let t = Arc::new(self.compile(&src, name)?);
        if self.cache_enabled {
            self.cache.lock().expect("cache lock").insert(name.to_owned(), Arc::clone(&t));
        }
        Ok(t)
    }

    /// `engine.select_template([names])`.
    pub fn select_template(&self, names: &[&str]) -> Result<Arc<Template>, TemplateError> {
        for n in names {
            match self.get_template(n) {
                Err(TemplateError::DoesNotExist(_)) => continue,
                other => return other,
            }
        }
        Err(TemplateError::DoesNotExist(names.join(", ")))
    }

    /// `engine.from_string(source)`.
    pub fn from_string(&self, src: &str) -> Result<Template, TemplateError> {
        self.compile(src, "<unknown source>")
    }

    fn compile(&self, src: &str, name: &str) -> Result<Template, TemplateError> {
        let lookup = |n: &str| self.library(n);
        let mut parser = Parser::new(src, self.builtins.clone(), &lookup);
        let (nodes, _) = parser.parse(&[])?;
        Ok(Template { name: name.to_owned(), nodes })
    }

    /// `render_to_string(name, context)`.
    pub fn render_to_string(&self, name: &str, ctx: &mut Context) -> Result<String, TemplateError> {
        self.get_template(name)?.render(self, ctx)
    }
}

impl Template {
    /// `template.render(context)`.
    pub fn render(&self, engine: &Engine, ctx: &mut Context) -> Result<String, TemplateError> {
        ctx.autoescape = engine.autoescape;
        ctx.render.used_templates.push(self.name.clone());
        let mut out = String::new();
        crate::nodes::render_nodes(&self.nodes, ctx, engine, &mut out)?;
        Ok(out)
    }
}

pub(crate) fn resolve_template_arg(
    engine: &Engine,
    ctx: &mut Context,
    expr: &FilterExpr,
) -> Result<Arc<Template>, TemplateError> {
    let v = crate::nodes::resolve_expr(expr, ctx, engine)?;
    engine.get_template(&v.to_str())
}
