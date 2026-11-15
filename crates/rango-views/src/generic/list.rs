//! `views.generic.list.ListView` (with pagination).

use std::ops::Range;
use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use rango_core::exceptions::Result;
use rango_core::paginator::{Page, PageSource, Paginator};
use rango_db::models::{Model, QuerySet};
use rango_db::Db;
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, Object, TemplateObject, TemplateResponse, Value as T};
use rango_urls::ViewArgs;

use super::base::View;
use super::detail::QuerySetHook;
use super::util::{model_name_lower, model_value, not_found, url_kwargs_context};

pub type ListContextHook<M> = Arc<dyn Fn(&HttpRequest, &ViewArgs, &[M], &mut Context) + Send + Sync>;

/// A queryset (with its connection) as a [`PageSource`].
pub struct QuerySource<M: Model> {
    pub qs: QuerySet<M>,
    pub db: Db,
}

impl<M: Model> PageSource for QuerySource<M> {
    type Item = M;

    async fn count(&self) -> Result<usize> {
        Ok(self.qs.count(&self.db).await? as usize)
    }

    async fn slice(&self, range: Range<usize>) -> Result<Vec<M>> {
        self.qs.clone().slice(range.start, Some(range.end)).fetch(&self.db).await
    }
}

/// `page_obj` / `paginator` as seen from templates.
struct PageObject {
    number: usize,
    count: usize,
    num_pages: usize,
    per_page: usize,
    len: usize,
    start: usize,
    end: usize,
}

impl TemplateObject for PageObject {
    fn get(&self, key: &str) -> Option<T> {
        let int = |n: usize| T::Int(n as i64);
        Some(match key {
            "number" => int(self.number),
            "has_next" => T::Bool(self.number < self.num_pages),
            "has_previous" => T::Bool(self.number > 1),
            "has_other_pages" => T::Bool(self.num_pages > 1),
            "next_page_number" if self.number < self.num_pages => int(self.number + 1),
            "previous_page_number" if self.number > 1 => int(self.number - 1),
            "start_index" => int(self.start),
            "end_index" => int(self.end),
            "num_pages" => int(self.num_pages),
            "count" => int(self.count),
            "per_page" => int(self.per_page),
            "page_range" => T::List((1..=self.num_pages).map(int).collect()),
            "paginator" => Object::new(PageObject { ..*self }),
            _ => return None,
        })
    }
    fn display(&self) -> T {
        T::Str(format!("<Page {} of {}>", self.number, self.num_pages))
    }
    fn iterate(&self) -> Option<Vec<T>> {
        Some((0..self.len).map(|i| T::Int(i as i64)).collect())
    }
}

impl Clone for PageObject {
    fn clone(&self) -> Self {
        *self
    }
}
impl Copy for PageObject {}

/// `ListView`.
pub struct ListView<M: Model> {
    pub queryset: QuerySetHook<M>,
    pub using: String,
    pub template_name: Option<String>,
    pub template_name_suffix: String,
    pub context_object_name: Option<String>,
    pub paginate_by: Option<usize>,
    pub paginate_orphans: usize,
    pub page_kwarg: String,
    pub allow_empty: bool,
    pub ordering: Option<Vec<String>>,
    pub extra_context: IndexMap<String, T>,
    context_hook: Option<ListContextHook<M>>,
}

impl<M: Model> Default for ListView<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Model> ListView<M> {
    pub fn new() -> Self {
        Self {
            queryset: Arc::new(|_| M::objects()),
            using: "default".into(),
            template_name: None,
            template_name_suffix: "_list".into(),
            context_object_name: None,
            paginate_by: None,
            paginate_orphans: 0,
            page_kwarg: "page".into(),
            allow_empty: true,
            ordering: None,
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
    pub fn paginate_by(mut self, n: usize) -> Self {
        self.paginate_by = Some(n);
        self
    }
    pub fn paginate_orphans(mut self, n: usize) -> Self {
        self.paginate_orphans = n;
        self
    }
    pub fn page_kwarg(mut self, n: &str) -> Self {
        self.page_kwarg = n.to_owned();
        self
    }
    pub fn allow_empty(mut self, b: bool) -> Self {
        self.allow_empty = b;
        self
    }
    pub fn ordering<I: IntoIterator<Item = S>, S: Into<String>>(mut self, o: I) -> Self {
        self.ordering = Some(o.into_iter().map(Into::into).collect());
        self
    }
    pub fn using(mut self, alias: &str) -> Self {
        self.using = alias.to_owned();
        self
    }
    pub fn queryset(mut self, f: impl Fn(&HttpRequest) -> QuerySet<M> + Send + Sync + 'static) -> Self {
        self.queryset = Arc::new(f);
        self
    }
    pub fn extra(mut self, key: &str, value: impl Into<T>) -> Self {
        self.extra_context.insert(key.to_owned(), value.into());
        self
    }
    pub fn context(mut self, f: impl Fn(&HttpRequest, &ViewArgs, &[M], &mut Context) + Send + Sync + 'static) -> Self {
        self.context_hook = Some(Arc::new(f));
        self
    }
}

#[async_trait]
impl<M: Model> View for ListView<M> {
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.using);
        let meta = M::meta()?;
        let mut qs = (self.queryset)(&req);
        if let Some(o) = &self.ordering {
            qs = qs.order_by(&o.iter().map(String::as_str).collect::<Vec<_>>());
        }
        let mut ctx = Context::new();
        for (k, v) in url_kwargs_context(args) {
            ctx.set(&k, v);
        }
        let name = self.context_object_name.clone().unwrap_or_else(|| format!("{}_list", model_name_lower(&meta)));
        let objects: Vec<M>;
        if let Some(per_page) = self.paginate_by {
            let paginator = Paginator::new(QuerySource { qs, db: db.clone() }, per_page).orphans(self.paginate_orphans).allow_empty_first_page(self.allow_empty);
            let page_arg = args.kwargs.get(&self.page_kwarg).map(ToString::to_string).or_else(|| req.query().get(&self.page_kwarg).map(str::to_owned)).unwrap_or_else(|| "1".into());
            let number = if page_arg == "last" {
                paginator.num_pages().await? as i64
            } else {
                page_arg.parse::<i64>().map_err(|_| not_found("Page is not “last”, nor can it be converted to an int."))?
            };
            let page: Page<M> = paginator.page(number).await?.map_err(|e| not_found(format!("Invalid page ({page_arg}): {e}")))?;
            let po = PageObject { number: page.number, count: page.count, num_pages: page.num_pages, per_page: page.per_page, len: page.object_list.len(), start: page.start_index(), end: page.end_index() };
            ctx.set("paginator", Object::new(po));
            ctx.set("page_obj", Object::new(po));
            ctx.set("is_paginated", po.num_pages > 1);
            objects = page.object_list;
        } else {
            objects = qs.fetch(&db).await?;
            if objects.is_empty() && !self.allow_empty {
                return Err(not_found("Empty list and “ListView.allow_empty” is False."));
            }
            ctx.set("paginator", T::None);
            ctx.set("page_obj", T::None);
            ctx.set("is_paginated", false);
        }
        let list = T::List(objects.iter().map(model_value).collect());
        ctx.set("object_list", list.clone());
        ctx.set(name.as_str(), list);
        for (k, v) in &self.extra_context {
            ctx.set(k, v.clone());
        }
        if let Some(h) = &self.context_hook {
            h(&req, args, &objects, &mut ctx);
        }
        let names = match &self.template_name {
            Some(n) => vec![n.clone()],
            None => vec![format!("{}/{}{}.html", meta.app_label, model_name_lower(&meta), self.template_name_suffix)],
        };
        Ok(TemplateResponse::new(&req, names, ctx))
    }
}
