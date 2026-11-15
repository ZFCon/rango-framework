//! `urls.resolvers` and `urls.base`: patterns, resolvers,
//! `resolve()` and `reverse()`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, OnceLock, RwLock};

use fancy_regex::Regex;
use indexmap::IndexMap;
use rango_core::conf::try_settings;
use rango_core::exceptions::{Error, Result};
use rango_core::utils::encoding::iri_to_uri;
use rango_core::utils::http::{RFC3986_SUBDELIMS, escape_leading_slashes, quote};
use rango_core::utils::regex_helper::normalize;

use crate::converters::{PathConverter, get_converter};
use crate::view::{Arg, IntoView, View, ViewArgs};

type Converters = IndexMap<String, Arc<dyn PathConverter>>;
type Captured = (String, Vec<Arg>, IndexMap<String, Arg>);

// ---------------------------------------------------------------- patterns

/// `RoutePattern` / `RegexPattern`.
pub struct Pattern {
    /// The route as the user wrote it (`str(pattern)`).
    pub describe: String,
    regex: Regex,
    regex_src: String,
    is_route: bool,
    pub name: Option<String>,
    pub converters: Converters,
}

fn config_error(msg: String) -> Error {
    Error::ImproperlyConfigured(msg)
}

impl Pattern {
    /// `RoutePattern(route, is_endpoint=...)`.
    pub fn route(route: &str, is_endpoint: bool) -> Result<Self> {
        static PARAM: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"<(?:(?P<converter>[^>:]+):)?(?P<parameter>[^>]+)>").expect("valid regex"));
        let mut parts = String::from("^");
        let mut converters = Converters::new();
        let mut prev = 0;
        for m in PARAM.captures_iter(route) {
            let m = m.map_err(|e| config_error(e.to_string()))?;
            let whole = m.get(0).expect("group 0");
            if whole.as_str().contains(char::is_whitespace) {
                return Err(config_error(format!("URL route '{route}' cannot contain whitespace in angle brackets <…>.")));
            }
            let conv_name = m.name("converter").map_or("str", |c| c.as_str());
            let param = &m["parameter"];
            if param.is_empty() || !param.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') || !param.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(config_error(format!(
                    "URL route '{route}' uses parameter name '{param}' which isn't a valid identifier."
                )));
            }
            let conv = get_converter(conv_name)
                .ok_or_else(|| config_error(format!("URL route '{route}' uses invalid converter '{conv_name}'.")))?;
            parts.push_str(&fancy_regex::escape(&route[prev..whole.start()]));
            prev = whole.end();
            parts.push_str(&format!("(?P<{param}>{})", conv.regex()));
            converters.insert(param.to_owned(), conv);
        }
        parts.push_str(&fancy_regex::escape(&route[prev..]));
        if is_endpoint {
            parts.push_str(r"\z");
        }
        let regex = Regex::new(&parts).map_err(|e| config_error(format!("invalid route {route:?}: {e}")))?;
        Ok(Self { describe: route.to_owned(), regex, regex_src: parts, is_route: true, name: None, converters })
    }

    /// `RegexPattern(regex, is_endpoint=...)`.
    pub fn regex(src: &str, is_endpoint: bool) -> Result<Self> {
        let _ = is_endpoint;
        let compiled = src.replace("\\Z", "\\z");
        let regex = Regex::new(&compiled).map_err(|e| config_error(format!("invalid regex {src:?}: {e}")))?;
        Ok(Self { describe: src.to_owned(), regex, regex_src: compiled, is_route: false, name: None, converters: Converters::new() })
    }

    /// Source of the compiled regex (what `reverse()` reasons about).
    pub fn regex_source(&self) -> &str {
        &self.regex_src
    }

    /// `pattern.match(path)`: remaining path, positional args and keyword args.
    pub fn match_path(&self, path: &str) -> Option<Captured> {
        let caps = self.regex.captures(path).ok()??;
        let end = caps.get(0)?.end();
        let mut kwargs = IndexMap::new();
        for name in self.regex.capture_names().flatten() {
            if let Some(m) = caps.name(name) {
                let value = m.as_str();
                let arg = match self.converters.get(name) {
                    Some(c) => c.to_python(value)?,
                    None => Arg::Str(value.to_owned()),
                };
                kwargs.insert(name.to_owned(), arg);
            }
        }
        let args = if kwargs.is_empty() && !self.is_route {
            (1..caps.len()).filter_map(|i| caps.get(i)).map(|m| Arg::Str(m.as_str().to_owned())).collect()
        } else {
            Vec::new()
        };
        Some((path[end..].to_owned(), args, kwargs))
    }
}

// ---------------------------------------------------------------- resolver match

/// `ResolverMatch`.
#[derive(Clone, Debug)]
pub struct ResolverMatch {
    pub func: View,
    pub args: Vec<Arg>,
    pub kwargs: IndexMap<String, Arg>,
    pub url_name: Option<String>,
    pub app_names: Vec<String>,
    pub namespaces: Vec<String>,
    pub route: String,
    pub tried: Vec<Vec<String>>,
    pub captured_kwargs: IndexMap<String, Arg>,
    pub extra_kwargs: IndexMap<String, Arg>,
}

impl ResolverMatch {
    pub fn app_name(&self) -> String {
        self.app_names.join(":")
    }
    pub fn namespace(&self) -> String {
        self.namespaces.join(":")
    }
    /// `view_name`: `ns:url_name` or the view's name.
    pub fn view_name(&self) -> String {
        let mut parts = self.namespaces.clone();
        parts.push(self.url_name.clone().unwrap_or_else(|| self.func.name.to_string()));
        parts.join(":")
    }
    pub fn view_args(&self) -> ViewArgs {
        ViewArgs { args: self.args.clone(), kwargs: self.kwargs.clone() }
    }
}

/// `Resolver404`.
#[derive(Debug, Clone, thiserror::Error)]
#[error("Resolver404: {path}")]
pub struct Resolver404 {
    pub path: String,
    /// Patterns tried, outermost first (for the debug 404 page).
    pub tried: Vec<Vec<String>>,
}

impl From<Resolver404> for Error {
    fn from(e: Resolver404) -> Self {
        Error::Http404(format!("Resolver404: {}", e.path))
    }
}

// ---------------------------------------------------------------- entries

pub struct UrlPattern {
    pub pattern: Pattern,
    pub callback: View,
    pub default_args: IndexMap<String, Arg>,
    pub name: Option<String>,
}

impl UrlPattern {
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self.pattern.name = Some(name.to_owned());
        self
    }

    /// Extra keyword arguments passed to the view (`path(route, view, kwargs)`).
    pub fn kwargs(mut self, kwargs: impl IntoIterator<Item = (&'static str, Arg)>) -> Self {
        self.default_args = kwargs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        self
    }

    fn resolve(&self, path: &str) -> Option<ResolverMatch> {
        let (_, args, mut kwargs) = self.pattern.match_path(path)?;
        let captured = kwargs.clone();
        kwargs.extend(self.default_args.clone());
        Some(ResolverMatch {
            func: self.callback.clone(),
            args,
            kwargs,
            url_name: self.name.clone(),
            app_names: Vec::new(),
            namespaces: Vec::new(),
            route: self.pattern.describe.clone(),
            tried: Vec::new(),
            captured_kwargs: captured,
            extra_kwargs: self.default_args.clone(),
        })
    }
}

/// Where a resolver's patterns come from.
pub enum UrlConf {
    Inline(Vec<UrlEntry>),
    /// Name registered with [`register_urlconf!`](crate::register_urlconf).
    Registered(String),
}

pub struct UrlResolver {
    pub pattern: Pattern,
    urlconf: UrlConf,
    pub default_kwargs: IndexMap<String, Arg>,
    pub app_name: Option<String>,
    pub namespace: Option<String>,
    built: OnceLock<Vec<UrlEntry>>,
    populated: OnceLock<Populated>,
}

pub enum UrlEntry {
    Pattern(Arc<UrlPattern>),
    Resolver(Arc<UrlResolver>),
}

impl From<UrlPattern> for UrlEntry {
    fn from(p: UrlPattern) -> Self {
        Self::Pattern(Arc::new(p))
    }
}

impl From<UrlResolver> for UrlEntry {
    fn from(r: UrlResolver) -> Self {
        Self::Resolver(Arc::new(r))
    }
}

impl UrlEntry {
    fn pattern(&self) -> &Pattern {
        match self {
            Self::Pattern(p) => &p.pattern,
            Self::Resolver(r) => &r.pattern,
        }
    }
}

/// `urlpatterns![ path(...), re_path(...), ... ]`.
#[macro_export]
macro_rules! urlpatterns {
    ($($e:expr),* $(,)?) => { ::std::vec![$($crate::UrlEntry::from($e)),*] };
}

/// Link-time registration of a URLconf module (what `include("polls.urls")` and
/// `ROOT_URLCONF` name).
pub struct UrlConfRegistration {
    pub name: &'static str,
    pub app_name: Option<&'static str>,
    pub build: fn() -> Vec<UrlEntry>,
}

inventory::collect!(UrlConfRegistration);

/// `register_urlconf!("polls.urls", || urlpatterns![...])` or with `app_name = "polls"`.
#[macro_export]
macro_rules! register_urlconf {
    ($name:literal, app_name = $app:literal, $build:expr) => {
        $crate::inventory::submit! { $crate::UrlConfRegistration { name: $name, app_name: Some($app), build: $build } }
    };
    ($name:literal, $build:expr) => {
        $crate::inventory::submit! { $crate::UrlConfRegistration { name: $name, app_name: None, build: $build } }
    };
}

fn find_urlconf(name: &str) -> Option<&'static UrlConfRegistration> {
    inventory::iter::<UrlConfRegistration>.into_iter().find(|r| r.name == name)
}

impl UrlResolver {
    pub fn new(pattern: Pattern, urlconf: UrlConf) -> Self {
        Self {
            pattern,
            urlconf,
            default_kwargs: IndexMap::new(),
            app_name: None,
            namespace: None,
            built: OnceLock::new(),
            populated: OnceLock::new(),
        }
    }

    pub fn kwargs(mut self, kwargs: impl IntoIterator<Item = (&'static str, Arg)>) -> Self {
        self.default_kwargs = kwargs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        self
    }

    /// `url_patterns`.
    pub fn url_patterns(&self) -> Result<&[UrlEntry]> {
        let name = match &self.urlconf {
            UrlConf::Inline(v) => return Ok(v),
            UrlConf::Registered(name) => name,
        };
        if let Some(b) = self.built.get() {
            return Ok(b);
        }
        let reg = find_urlconf(name)
            .ok_or_else(|| Error::improperly_configured(format!("The URLconf '{name}' is not registered. Use register_urlconf!.")))?;
        Ok(self.built.get_or_init(|| (reg.build)()))
    }

    /// `resolve(path)`.
    pub fn resolve(&self, path: &str) -> std::result::Result<ResolverMatch, Resolver404> {
        let nf = |tried| Resolver404 { path: path.to_owned(), tried };
        let Some((new_path, args, captured)) = self.pattern.match_path(path) else { return Err(nf(vec![])) };
        let mut kwargs = captured;
        kwargs.extend(self.default_kwargs.clone());
        let mut tried: Vec<Vec<String>> = Vec::new();
        let patterns = self.url_patterns().map_err(|_| nf(vec![]))?;
        for entry in patterns {
            let sub = match entry {
                UrlEntry::Pattern(p) => p.resolve(&new_path).ok_or(None),
                UrlEntry::Resolver(r) => r.resolve(&new_path).map_err(Some),
            };
            match sub {
                Ok(m) => {
                    let mut dict = kwargs.clone();
                    dict.extend(self.default_kwargs.clone());
                    dict.extend(m.kwargs.clone());
                    let sub_args = if dict.is_empty() { args.iter().chain(&m.args).cloned().collect() } else { m.args.clone() };
                    let current_route = match entry {
                        UrlEntry::Pattern(_) => String::new(),
                        UrlEntry::Resolver(r) => r.pattern.describe.clone(),
                    };
                    let mut t = tried.clone();
                    t.extend(m.tried.iter().cloned());
                    let mut extra = self.default_kwargs.clone();
                    extra.extend(m.extra_kwargs.clone());
                    return Ok(ResolverMatch {
                        func: m.func,
                        args: sub_args,
                        kwargs: dict,
                        url_name: m.url_name,
                        app_names: self.app_name.iter().cloned().chain(m.app_names).collect(),
                        namespaces: self.namespace.iter().cloned().chain(m.namespaces).collect(),
                        route: join_route(&current_route, &m.route),
                        tried: t,
                        captured_kwargs: m.captured_kwargs,
                        extra_kwargs: extra,
                    });
                }
                Err(Some(e)) => {
                    for t in e.tried {
                        let mut row = vec![entry.pattern().describe.clone()];
                        row.extend(t);
                        tried.push(row);
                    }
                    if tried.is_empty() {
                        tried.push(vec![entry.pattern().describe.clone()]);
                    }
                }
                Err(None) => tried.push(vec![entry.pattern().describe.clone()]),
            }
        }
        Err(Resolver404 { path: new_path, tried })
    }

    fn populate(&self) -> &Populated {
        self.populated.get_or_init(|| {
            let mut p = Populated::default();
            let Ok(patterns) = self.url_patterns() else { return p };
            for entry in patterns.iter().rev() {
                let src = entry.pattern().regex_source();
                let p_pattern = src.strip_prefix('^').unwrap_or(src).to_owned();
                match entry {
                    UrlEntry::Pattern(up) => {
                        let bits = normalize(src).unwrap_or_default();
                        let poss = Possibility {
                            bits,
                            pattern: p_pattern,
                            defaults: up.default_args.clone(),
                            converters: up.pattern.converters.clone(),
                        };
                        p.lookups.entry(up.callback.name.to_string()).or_default().push(poss.clone());
                        if let Some(n) = &up.name {
                            p.lookups.entry(n.clone()).or_default().push(poss);
                        }
                    }
                    UrlEntry::Resolver(r) => {
                        let sub = r.populate();
                        if let Some(app) = &r.app_name {
                            p.apps.entry(app.clone()).or_default().push(r.namespace.clone().unwrap_or_default());
                            p.namespaces.insert(r.namespace.clone().unwrap_or_default(), (p_pattern.clone(), Arc::clone(r), r.pattern.converters.clone()));
                        } else {
                            for (name, list) in &sub.lookups {
                                for poss in list {
                                    let combined = format!("{p_pattern}{}", poss.pattern);
                                    let mut defaults = poss.defaults.clone();
                                    defaults.extend(r.default_kwargs.clone());
                                    let mut convs = self.pattern.converters.clone();
                                    convs.extend(r.pattern.converters.clone());
                                    convs.extend(poss.converters.clone());
                                    p.lookups.entry(name.clone()).or_default().push(Possibility {
                                        bits: normalize(&combined).unwrap_or_default(),
                                        pattern: combined,
                                        defaults,
                                        converters: convs,
                                    });
                                }
                            }
                            for (ns, (prefix, sub_r, convs)) in &sub.namespaces {
                                let mut c = convs.clone();
                                c.extend(r.pattern.converters.clone());
                                p.namespaces.insert(ns.clone(), (format!("{p_pattern}{prefix}"), Arc::clone(sub_r), c));
                            }
                            for (app, list) in &sub.apps {
                                p.apps.entry(app.clone()).or_default().extend(list.clone());
                            }
                        }
                    }
                }
            }
            p
        })
    }

    /// `_reverse_with_prefix()`.
    pub fn reverse_with_prefix(
        &self,
        lookup: &str,
        prefix: &str,
        args: &[String],
        kwargs: &IndexMap<String, String>,
    ) -> Result<String> {
        if !args.is_empty() && !kwargs.is_empty() {
            return Err(Error::Other("Don't mix *args and **kwargs in call to reverse()!".into()));
        }
        let populated = self.populate();
        let possibilities = populated.lookups.get(lookup).cloned().unwrap_or_default();
        for poss in &possibilities {
            for (result, params) in &poss.bits {
                let subs: IndexMap<String, String> = if !args.is_empty() {
                    if args.len() != params.len() {
                        continue;
                    }
                    params.iter().cloned().zip(args.iter().cloned()).collect()
                } else {
                    let mut diff: Vec<&String> = kwargs.keys().filter(|k| !params.contains(k)).chain(params.iter().filter(|p| !kwargs.contains_key(*p))).collect();
                    diff.retain(|k| !poss.defaults.contains_key(*k));
                    if !diff.is_empty() {
                        continue;
                    }
                    let mut ok = true;
                    for (k, v) in &poss.defaults {
                        if params.contains(k) {
                            continue;
                        }
                        if kwargs.get(k).is_some_and(|kv| *kv != v.to_string()) {
                            ok = false;
                            break;
                        }
                    }
                    if !ok {
                        continue;
                    }
                    kwargs.clone()
                };
                let mut text_subs = IndexMap::new();
                let mut matches = true;
                for (k, v) in &subs {
                    match poss.converters.get(k) {
                        Some(c) => match c.to_url(v) {
                            Some(s) => {
                                text_subs.insert(k.clone(), s);
                            }
                            None => {
                                matches = false;
                                break;
                            }
                        },
                        None => {
                            text_subs.insert(k.clone(), v.clone());
                        }
                    }
                }
                if !matches {
                    continue;
                }
                let mut candidate = prefix.to_owned();
                candidate.push_str(&percent_format(result, &text_subs));
                let check = format!("^{}{}", fancy_regex::escape(prefix), poss.pattern);
                if Regex::new(&check).ok().and_then(|r| r.is_match(&candidate).ok()).unwrap_or(false) {
                    let url = quote(&candidate, &format!("{RFC3986_SUBDELIMS}/~:@"));
                    return Ok(escape_leading_slashes(&url));
                }
            }
        }
        let patterns: Vec<&str> = possibilities.iter().map(|p| p.pattern.as_str()).collect();
        Err(Error::NoReverseMatch(if patterns.is_empty() {
            format!("Reverse for '{lookup}' not found. '{lookup}' is not a valid view function or pattern name.")
        } else {
            let arg_msg = if !args.is_empty() {
                format!("arguments '{args:?}'")
            } else if !kwargs.is_empty() {
                format!("keyword arguments '{kwargs:?}'")
            } else {
                "no arguments".into()
            };
            format!("Reverse for '{lookup}' with {arg_msg} not found. {} pattern(s) tried: {patterns:?}", patterns.len())
        }))
    }
}

/// Substitute `%(name)s` placeholders (Python `%` formatting with a dict).
fn percent_format(template: &str, subs: &IndexMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("%(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        if let Some(j) = after.find(")s") {
            out.push_str(subs.get(&after[..j]).map_or("", String::as_str));
            rest = &after[j + 2..];
        } else {
            out.push_str("%(");
            rest = after;
        }
    }
    out.push_str(rest);
    out.replace("%%", "%")
}

fn join_route(route1: &str, route2: &str) -> String {
    if route1.is_empty() {
        return route2.to_owned();
    }
    format!("{route1}{}", route2.strip_prefix('^').unwrap_or(route2))
}

#[derive(Clone)]
struct Possibility {
    bits: Vec<(String, Vec<String>)>,
    pattern: String,
    defaults: IndexMap<String, Arg>,
    converters: Converters,
}

#[derive(Default)]
struct Populated {
    lookups: IndexMap<String, Vec<Possibility>>,
    /// namespace -> (prefix pattern, resolver, converters)
    namespaces: IndexMap<String, (String, Arc<UrlResolver>, Converters)>,
    apps: IndexMap<String, Vec<String>>,
}

// ---------------------------------------------------------------- builders

/// What `path()` accepts as its second argument: a view or an [`Include`].
pub trait PathTarget<Marker> {
    type Output: Into<UrlEntry>;
    fn build(self, route: &str, regex: bool) -> Self::Output;
}

impl<M, T: IntoView<M>> PathTarget<M> for T {
    type Output = UrlPattern;
    fn build(self, route: &str, regex: bool) -> UrlPattern {
        let pattern = if regex { Pattern::regex(route, true) } else { Pattern::route(route, true) }.unwrap_or_else(|e| panic!("{e}"));
        UrlPattern { pattern, callback: self.into_view(), default_args: IndexMap::new(), name: None }
    }
}

pub struct IncludeMarker;

impl PathTarget<IncludeMarker> for Include {
    type Output = UrlResolver;
    fn build(self, route: &str, regex: bool) -> UrlResolver {
        let pattern = if regex { Pattern::regex(route, false) } else { Pattern::route(route, false) }.unwrap_or_else(|e| panic!("{e}"));
        let mut r = UrlResolver::new(pattern, self.conf);
        r.app_name = self.app_name;
        r.namespace = self.namespace;
        r
    }
}

/// `path(route, view_or_include)`.
pub fn path<M, T: PathTarget<M>>(route: &str, target: T) -> T::Output {
    target.build(route, false)
}

/// `re_path(regex, view_or_include)`.
pub fn re_path<M, T: PathTarget<M>>(route: &str, target: T) -> T::Output {
    target.build(route, true)
}

/// `include(...)`.
pub struct Include {
    conf: UrlConf,
    app_name: Option<String>,
    namespace: Option<String>,
}

/// Argument of [`include`]: a registered URLconf name or an inline list.
pub enum IncludeArg {
    Registered(String),
    Inline(Vec<UrlEntry>),
}

impl From<&str> for IncludeArg {
    fn from(s: &str) -> Self {
        Self::Registered(s.to_owned())
    }
}

impl From<Vec<UrlEntry>> for IncludeArg {
    fn from(v: Vec<UrlEntry>) -> Self {
        Self::Inline(v)
    }
}

pub fn include(arg: impl Into<IncludeArg>) -> Include {
    match arg.into() {
        IncludeArg::Registered(name) => {
            let app_name = find_urlconf(&name).and_then(|r| r.app_name).map(str::to_owned);
            Include { conf: UrlConf::Registered(name), namespace: app_name.clone(), app_name }
        }
        IncludeArg::Inline(v) => {
            Include { conf: UrlConf::Inline(v), app_name: None, namespace: None }
        }
    }
}

impl Include {
    /// `include((patterns, app_name))`.
    pub fn app_name(mut self, app_name: &str) -> Self {
        self.app_name = Some(app_name.to_owned());
        if self.namespace.is_none() {
            self.namespace = Some(app_name.to_owned());
        }
        self
    }

    /// `include(..., namespace=...)`; needs an app name.
    pub fn namespace(mut self, namespace: &str) -> Self {
        self.namespace = Some(namespace.to_owned());
        self
    }
}

// ---------------------------------------------------------------- root resolver, script prefix, urlconf

static RESOLVERS: LazyLock<RwLock<HashMap<String, Arc<UrlResolver>>>> = LazyLock::new(|| RwLock::new(HashMap::new()));
static SCRIPT_PREFIX: RwLock<String> = RwLock::new(String::new());

tokio::task_local! {
    static PREFIX: String;
    static URLCONF: String;
}

/// `get_resolver(urlconf)`: cached root resolver for `urlconf` (default `ROOT_URLCONF`).
pub fn get_resolver(urlconf: Option<&str>) -> Result<Arc<UrlResolver>> {
    let name = match urlconf {
        Some(n) => n.to_owned(),
        None => match get_urlconf() {
            Some(n) => n,
            None => try_settings()?.root_urlconf.clone().ok_or_else(|| Error::improperly_configured("The ROOT_URLCONF setting is not configured."))?,
        },
    };
    if let Some(r) = RESOLVERS.read().expect("resolvers lock").get(&name) {
        return Ok(Arc::clone(r));
    }
    let r = Arc::new(UrlResolver::new(Pattern::regex("^/", false)?, UrlConf::Registered(name.clone())));
    RESOLVERS.write().expect("resolvers lock").insert(name, Arc::clone(&r));
    Ok(r)
}

/// `clear_url_caches()`.
pub fn clear_url_caches() {
    RESOLVERS.write().expect("resolvers lock").clear();
}

/// `set_script_prefix()` (process-wide default).
pub fn set_script_prefix(prefix: &str) {
    let p = if prefix.ends_with('/') { prefix.to_owned() } else { format!("{prefix}/") };
    *SCRIPT_PREFIX.write().expect("prefix lock") = p;
}

/// `get_script_prefix()`: the request-scoped prefix if set, else the process-wide one.
pub fn get_script_prefix() -> String {
    if let Ok(p) = PREFIX.try_with(Clone::clone) {
        return p;
    }
    let p = SCRIPT_PREFIX.read().expect("prefix lock").clone();
    if p.is_empty() { "/".into() } else { p }
}

/// Run `fut` with a request-scoped script prefix.
pub async fn with_script_prefix<F: Future>(prefix: &str, fut: F) -> F::Output {
    let p = if prefix.ends_with('/') { prefix.to_owned() } else { format!("{prefix}/") };
    PREFIX.scope(p, fut).await
}

/// `get_urlconf()`: the request-scoped URLconf override, if any.
pub fn get_urlconf() -> Option<String> {
    URLCONF.try_with(Clone::clone).ok()
}

/// Run `fut` with `request.urlconf` set to `urlconf`.
pub async fn with_urlconf<F: Future>(urlconf: &str, fut: F) -> F::Output {
    URLCONF.scope(urlconf.to_owned(), fut).await
}

/// `resolve(path, urlconf=None)`.
pub fn resolve(path: &str, urlconf: Option<&str>) -> Result<std::result::Result<ResolverMatch, Resolver404>> {
    Ok(get_resolver(urlconf)?.resolve(path))
}

/// `is_valid_path(path)` returning the match (what `CommonMiddleware` needs).
pub fn valid_match(path: &str, urlconf: Option<&str>) -> Option<ResolverMatch> {
    resolve(path, urlconf).ok().and_then(Result::ok)
}

/// Per-request URLconf override (`request.urlconf`), attached with `HttpRequest::set_ext`.
#[derive(Clone, Debug)]
pub struct RequestUrlconf(pub String);

/// `is_valid_path(path)`.
pub fn is_valid_path(path: &str, urlconf: Option<&str>) -> bool {
    resolve(path, urlconf).is_ok_and(|r| r.is_ok())
}

/// Arguments of [`reverse`].
#[derive(Default, Clone, Debug)]
pub struct Reverse {
    pub args: Vec<String>,
    pub kwargs: IndexMap<String, String>,
    pub urlconf: Option<String>,
    pub current_app: Option<String>,
}

impl Reverse {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn args<I: IntoIterator<Item = T>, T: ToString>(mut self, args: I) -> Self {
        self.args = args.into_iter().map(|a| a.to_string()).collect();
        self
    }
    pub fn kwarg(mut self, key: &str, value: impl ToString) -> Self {
        self.kwargs.insert(key.to_owned(), value.to_string());
        self
    }
    pub fn urlconf(mut self, urlconf: &str) -> Self {
        self.urlconf = Some(urlconf.to_owned());
        self
    }
    pub fn current_app(mut self, app: &str) -> Self {
        self.current_app = Some(app.to_owned());
        self
    }
}

/// `reverse(viewname, args=..., kwargs=...)`; `viewname` may be `"namespace:name"`.
pub fn reverse(viewname: &str, r: Reverse) -> Result<String> {
    let resolver = get_resolver(r.urlconf.as_deref())?;
    let prefix = get_script_prefix();
    let mut parts: Vec<&str> = viewname.split(':').collect();
    let view = parts.pop().unwrap_or("");
    let mut current_path: Option<Vec<String>> = r.current_app.as_ref().map(|c| c.split(':').rev().map(str::to_owned).collect());
    let mut resolver = resolver;
    let mut resolved_path: Vec<String> = Vec::new();
    let mut ns_pattern = String::new();
    let mut ns_converters = Converters::new();
    for ns in parts {
        let current_ns = current_path.as_mut().and_then(Vec::pop);
        let mut ns = ns.to_owned();
        if let Some(app_list) = resolver.populate().apps.get(&ns) {
            if current_ns.as_ref().is_some_and(|c| app_list.contains(c)) {
                ns = current_ns.clone().expect("checked");
            } else if !app_list.contains(&ns) {
                ns = app_list[0].clone();
            }
        }
        if Some(&ns) != current_ns.as_ref() {
            current_path = None;
        }
        let found = resolver.populate().namespaces.get(&ns).cloned();
        match found {
            Some((extra, sub, convs)) => {
                resolved_path.push(ns);
                ns_pattern.push_str(&extra);
                ns_converters.extend(convs);
                resolver = sub;
            }
            None => {
                return Err(Error::NoReverseMatch(if resolved_path.is_empty() {
                    format!("'{ns}' is not a registered namespace")
                } else {
                    format!("'{ns}' is not a registered namespace inside '{}'", resolved_path.join(":"))
                }));
            }
        }
    }
    let url = if ns_pattern.is_empty() {
        resolver.reverse_with_prefix(view, &prefix, &r.args, &r.kwargs)?
    } else {
        // get_ns_resolver(): wrap the namespace's patterns under the accumulated prefix.
        let mut pat = Pattern::regex(&ns_pattern, false)?;
        pat.converters = ns_converters;
        let inner = UrlResolver::new(pat, UrlConf::Inline(Vec::new()));
        let populated = resolver.populate();
        let mut synthetic = Populated::default();
        for (name, list) in &populated.lookups {
            for poss in list {
                let combined = format!("{ns_pattern}{}", poss.pattern);
                let mut convs = inner.pattern.converters.clone();
                convs.extend(poss.converters.clone());
                synthetic.lookups.entry(name.clone()).or_default().push(Possibility {
                    bits: normalize(&combined).unwrap_or_default(),
                    pattern: combined,
                    defaults: poss.defaults.clone(),
                    converters: convs,
                });
            }
        }
        let wrapper = UrlResolver::new(Pattern::regex("^/", false)?, UrlConf::Inline(Vec::new()));
        let _ = wrapper.populated.set(synthetic);
        wrapper.reverse_with_prefix(view, &prefix, &r.args, &r.kwargs)?
    };
    Ok(iri_to_uri(&url))
}

/// `reverse!("polls:detail", 1, 2)` / `reverse!("polls:detail", id = 1)` / `reverse!("home")`.
#[macro_export]
macro_rules! reverse {
    ($name:expr) => { $crate::reverse($name, $crate::Reverse::new()) };
    ($name:expr, $($k:ident = $v:expr),+ $(,)?) => {
        $crate::reverse($name, $crate::Reverse::new()$(.kwarg(stringify!($k), $v))+)
    };
    ($name:expr, $($a:expr),+ $(,)?) => {
        $crate::reverse($name, $crate::Reverse::new().args([$($a.to_string()),+]))
    };
}
