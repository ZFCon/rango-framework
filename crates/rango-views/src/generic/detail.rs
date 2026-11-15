//! `views.generic.detail.DetailView`.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_db::models::{Model, QuerySet};
use rango_db::{Db, Executor};
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, TemplateResponse, Value as T};
use rango_urls::ViewArgs;

use super::base::View;
use super::util::{arg_to_db, model_name_lower, model_value, not_found, url_kwargs_context};

pub type QuerySetHook<M> = Arc<dyn Fn(&HttpRequest) -> QuerySet<M> + Send + Sync>;
pub type ObjectContextHook<M> = Arc<dyn Fn(&HttpRequest, &ViewArgs, &M, &mut Context) + Send + Sync>;

/// `DetailView`: one object looked up by `pk` or `slug` from the URL.
pub struct DetailView<M: Model> {
    pub queryset: QuerySetHook<M>,
    pub using: String,
    pub template_name: Option<String>,
    pub template_name_suffix: String,
    pub context_object_name: Option<String>,
    pub pk_url_kwarg: String,
    pub slug_url_kwarg: String,
    pub slug_field: String,
    pub query_pk_and_slug: bool,
    pub extra_context: IndexMap<String, T>,
    context_hook: Option<ObjectContextHook<M>>,
}

impl<M: Model> Default for DetailView<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Model> DetailView<M> {
    pub fn new() -> Self {
        Self {
            queryset: Arc::new(|_| M::objects()),
            using: "default".into(),
            template_name: None,
            template_name_suffix: "_detail".into(),
            context_object_name: None,
            pk_url_kwarg: "pk".into(),
            slug_url_kwarg: "slug".into(),
            slug_field: "slug".into(),
            query_pk_and_slug: false,
            extra_context: IndexMap::new(),
            context_hook: None,
        }
    }
    pub fn template_name(mut self, n: &str) -> Self {
        self.template_name = Some(n.to_owned());
        self
    }
    pub fn context_object_name(mut self, n: &str) -> Self {
        self.context_object_name = Some(n.to_owned());
        self
    }
    pub fn pk_url_kwarg(mut self, n: &str) -> Self {
        self.pk_url_kwarg = n.to_owned();
        self
    }
    pub fn slug_field(mut self, n: &str) -> Self {
        self.slug_field = n.to_owned();
        self
    }
    pub fn slug_url_kwarg(mut self, n: &str) -> Self {
        self.slug_url_kwarg = n.to_owned();
        self
    }
    pub fn query_pk_and_slug(mut self, b: bool) -> Self {
        self.query_pk_and_slug = b;
        self
    }
    pub fn using(mut self, alias: &str) -> Self {
        self.using = alias.to_owned();
        self
    }
    /// `get_queryset()` override.
    pub fn queryset(mut self, f: impl Fn(&HttpRequest) -> QuerySet<M> + Send + Sync + 'static) -> Self {
        self.queryset = Arc::new(f);
        self
    }
    pub fn extra(mut self, key: &str, value: impl Into<T>) -> Self {
        self.extra_context.insert(key.to_owned(), value.into());
        self
    }
    /// `get_context_data()` override.
    pub fn context(mut self, f: impl Fn(&HttpRequest, &ViewArgs, &M, &mut Context) + Send + Sync + 'static) -> Self {
        self.context_hook = Some(Arc::new(f));
        self
    }

    /// `get_object()`.
    pub async fn get_object(&self, req: &HttpRequest, args: &ViewArgs, db: &dyn Executor) -> Result<M> {
        let mut qs = (self.queryset)(req);
        let pk = args.kwargs.get(&self.pk_url_kwarg);
        let slug = args.kwargs.get(&self.slug_url_kwarg);
        if pk.is_none() && slug.is_none() {
            return Err(Error::improperly_configured("Generic detail view must be called with either an object pk or a slug in the URLconf."));
        }
        if let Some(pk) = pk {
            qs = qs.filter_kw("pk", arg_to_db(pk));
        }
        if let Some(slug) = slug.filter(|_| pk.is_none() || self.query_pk_and_slug) {
            qs = qs.filter_kw(&self.slug_field, arg_to_db(slug));
        }
        match qs.get(db).await {
            Err(Error::ObjectDoesNotExist { .. }) => {
                let vn = M::meta().map(|m| m.verbose_name.clone()).unwrap_or_default();
                Err(not_found(format!("No {vn} found matching the query")))
            }
            other => other,
        }
    }

    pub(crate) fn object_context(&self, req: &HttpRequest, args: &ViewArgs, obj: &M) -> Result<Context> {
        let meta = M::meta()?;
        let mut ctx = Context::new();
        for (k, v) in url_kwargs_context(args) {
            ctx.set(&k, v);
        }
        let v = model_value(obj);
        ctx.set("object", v.clone());
        ctx.set(self.context_object_name.clone().unwrap_or_else(|| model_name_lower(&meta)).as_str(), v);
        for (k, v) in &self.extra_context {
            ctx.set(k, v.clone());
        }
        if let Some(h) = &self.context_hook {
            h(req, args, obj, &mut ctx);
        }
        Ok(ctx)
    }

    pub(crate) fn template_names(&self) -> Result<Vec<String>> {
        let meta = M::meta()?;
        Ok(match &self.template_name {
            Some(n) => vec![n.clone()],
            None => vec![format!("{}/{}{}.html", meta.app_label, model_name_lower(&meta), self.template_name_suffix)],
        })
    }
}

#[async_trait]
impl<M: Model> View for DetailView<M> {
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.using);
        let obj = self.get_object(&req, args, &db).await?;
        let ctx = self.object_context(&req, args, &obj)?;
        Ok(TemplateResponse::new(&req, self.template_names()?, ctx))
    }
}
