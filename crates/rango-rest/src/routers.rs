//! `SimpleRouter` and `DefaultRouter`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use rango_urls::{UrlEntry, View, re_path};
use serde_json::{Map, Value as Json};

use crate::views::{Response, ViewConfig, dispatch};
use crate::viewsets::{Route, ViewSet};

/// A routed viewset URL, kept so the OpenAPI generator can find it from the URL pattern's view name.
#[derive(Clone)]
pub struct Registered {
    pub viewset: Arc<dyn ViewSet>,
    pub route: Route,
    pub prefix: String,
    pub basename: String,
}

static REGISTRY: LazyLock<RwLock<HashMap<String, Registered>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// The viewset route behind a URL pattern's view name, if it is an API route.
pub fn lookup(view_name: &str) -> Option<Registered> {
    REGISTRY.read().expect("router registry").get(view_name).cloned()
}

/// `DefaultRouter` (with the API root view) / `SimpleRouter`.
pub struct Router {
    registry: Vec<(String, Arc<dyn ViewSet>, String)>,
    include_root_view: bool,
}

impl Router {
    /// `DefaultRouter()`.
    pub fn new() -> Self {
        Router { registry: vec![], include_root_view: true }
    }
    /// `SimpleRouter()`: no API root view.
    pub fn simple() -> Self {
        Router { registry: vec![], include_root_view: false }
    }

    /// `router.register(prefix, viewset)`; the basename defaults to the model name.
    pub fn register(&mut self, prefix: &str, viewset: impl ViewSet) -> &mut Self {
        let basename = viewset.basename();
        self.register_as(prefix, viewset, &basename)
    }

    /// `router.register(prefix, viewset, basename=...)`.
    pub fn register_as(&mut self, prefix: &str, viewset: impl ViewSet, basename: &str) -> &mut Self {
        self.registry.push((prefix.trim_matches('/').to_owned(), Arc::new(viewset), basename.to_owned()));
        self
    }

    /// `router.urls`: patterns to `include()`.
    pub fn urls(&self) -> Vec<UrlEntry> {
        let mut out: Vec<UrlEntry> = Vec::new();
        if self.include_root_view {
            let prefixes: Vec<String> = self.registry.iter().map(|(p, _, _)| p.clone()).collect();
            let view = View::new("rango.rest.api_root", move |http, _| {
                let prefixes = prefixes.clone();
                Box::pin(async move {
                    let cfg = ViewConfig::default();
                    Ok(dispatch(&cfg, http, &["GET"], "Api Root", |req| async move {
                        let base = req.http.build_absolute_uri(Some(req.http.path()))?;
                        Ok(Response::new(Json::Object(prefixes.iter().map(|p| (p.clone(), Json::String(format!("{base}{p}/")))).collect::<Map<_, _>>())))
                    })
                    .await)
                })
            })
            .with_flags(|f| f.csrf_exempt = true);
            out.push(re_path("^$", view).name("api-root").into());
        }
        for (prefix, viewset, basename) in &self.registry {
            // Extra actions first so `prefix/action/` is not swallowed by the detail route.
            let mut routes = viewset.routes();
            routes.sort_by_key(|r| (r.url_path.is_none(), r.detail));
            for route in routes {
                let regex = match (&route.url_path, route.detail) {
                    (None, false) => format!("^{prefix}/$"),
                    (None, true) => format!("^{prefix}/(?P<pk>[^/.]+)/$"),
                    (Some(p), false) => format!("^{prefix}/{p}/$"),
                    (Some(p), true) => format!("^{prefix}/(?P<pk>[^/.]+)/{p}/$"),
                };
                let name = match (&route.url_path, route.detail) {
                    (None, false) => format!("{basename}-list"),
                    (None, true) => format!("{basename}-detail"),
                    (Some(p), _) => format!("{basename}-{}", p.replace('_', "-")),
                };
                let key = format!("rango.rest.viewset:{basename}:{name}");
                REGISTRY.write().expect("router registry").insert(key.clone(), Registered { viewset: Arc::clone(viewset), route: route.clone(), prefix: prefix.clone(), basename: basename.clone() });
                let (vs, r) = (Arc::clone(viewset), route.clone());
                let display = format!("{} {}", vs.display_name(), if route.url_path.is_some() { "Action" } else if route.detail { "Instance" } else { "List" });
                let view = View::new(key, move |http, args| {
                    let (vs, r, display) = (Arc::clone(&vs), r.clone(), display.clone());
                    Box::pin(async move {
                        let methods: Vec<&str> = r.actions.iter().map(|(m, _)| *m).collect();
                        let method = if http.method() == "HEAD" { "GET".to_owned() } else { http.method().to_owned() };
                        let action = r.actions.iter().find(|(m, _)| *m == method).map(|(_, a)| a.clone()).unwrap_or_default();
                        let config = vs.config().clone();
                        let vs2 = Arc::clone(&vs);
                        Ok(dispatch(&config, http, &methods, &display, move |req| async move { vs2.call(&action, req, &args).await }).await)
                    })
                })
                .with_flags(|f| f.csrf_exempt = true);
                out.push(re_path(&regex, view).name(&name).into());
            }
        }
        out
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
