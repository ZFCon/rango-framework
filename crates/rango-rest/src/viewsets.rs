//! `ModelViewSet` and friends.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use rango_core::exceptions::{Error, Result};
use rango_db::models::meta::FieldType;
use rango_db::models::{Model, Q, QuerySet};
use rango_db::{Db, Value as DbValue};
use rango_urls::ViewArgs;
use serde_json::Value as Json;

use crate::auth::{Authentication, Permission, Throttle};
use crate::exceptions::ApiException;
use crate::pagination::Pagination;
use crate::serializers::ModelSerializer;
use crate::views::{ApiRequest, Response, ViewConfig};

/// One URL of a viewset: list or detail, optionally an extra action path.
#[derive(Debug, Clone)]
pub struct Route {
    pub detail: bool,
    /// Extra action path segment (`None` for the standard list/detail routes).
    pub url_path: Option<String>,
    /// `(HTTP method, action name)`.
    pub actions: Vec<(&'static str, String)>,
}

/// What the OpenAPI generator needs to know about a viewset.
#[derive(Clone)]
pub struct ViewSetSchema {
    pub serializer: crate::serializers::Serializer,
    pub pagination: Option<Pagination>,
    pub search: bool,
    pub ordering: bool,
    pub filterset_fields: Vec<String>,
    pub lookup_is_integer: bool,
    pub description: String,
}

/// A set of related views behind one router registration.
#[async_trait]
pub trait ViewSet: Send + Sync + 'static {
    fn config(&self) -> &ViewConfig;
    fn routes(&self) -> Vec<Route>;
    fn basename(&self) -> String;
    /// Human name used by `OPTIONS` (`"Article"`).
    fn display_name(&self) -> String;
    async fn call(&self, action: &str, request: ApiRequest, args: &ViewArgs) -> Result<Response>;
    fn schema(&self) -> ViewSetSchema;
}

type QuerySetHook<M> = Arc<dyn Fn(&ApiRequest) -> QuerySet<M> + Send + Sync>;
type ActionHandler<M> = Arc<dyn Fn(ApiRequest, Option<M>) -> BoxFuture<'static, Result<Response>> + Send + Sync>;

struct ExtraAction<M> {
    name: String,
    detail: bool,
    methods: Vec<&'static str>,
    handler: ActionHandler<M>,
}

/// `ModelViewSet`: list, create, retrieve, update, partial_update, destroy for one model.
pub struct ModelViewSet<M: Model + Default> {
    pub serializer: ModelSerializer<M>,
    queryset: QuerySetHook<M>,
    pub config: ViewConfig,
    /// `None` = `DEFAULT_PAGINATION_CLASS`; `Some(None)` = no pagination.
    pagination: Option<Option<Pagination>>,
    search_fields: Vec<String>,
    ordering_fields: Option<Vec<String>>,
    ordering_enabled: bool,
    ordering: Vec<String>,
    filterset_fields: Vec<String>,
    lookup_field: String,
    read_only: bool,
    using: String,
    description: String,
    extra: Vec<ExtraAction<M>>,
}

impl<M: Model + Default> ModelViewSet<M> {
    pub fn new(serializer: ModelSerializer<M>) -> Self {
        ModelViewSet {
            serializer,
            queryset: Arc::new(|_| M::objects()),
            config: ViewConfig::default(),
            pagination: None,
            search_fields: vec![],
            ordering_fields: None,
            ordering_enabled: false,
            ordering: vec![],
            filterset_fields: vec![],
            lookup_field: "pk".into(),
            read_only: false,
            using: "default".into(),
            description: String::new(),
            extra: vec![],
        }
    }

    /// `ReadOnlyModelViewSet`.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
    /// `get_queryset()`.
    pub fn queryset(mut self, f: impl Fn(&ApiRequest) -> QuerySet<M> + Send + Sync + 'static) -> Self {
        self.queryset = Arc::new(f);
        self
    }
    pub fn authentication(mut self, a: Vec<Authentication>) -> Self {
        self.config.authentication = Some(a);
        self
    }
    pub fn permissions(mut self, p: Vec<Permission>) -> Self {
        self.config.permissions = Some(p);
        self
    }
    pub fn throttles(mut self, t: Vec<Throttle>) -> Self {
        self.config.throttles = Some(t);
        self
    }
    /// `pagination_class` (`None` disables pagination for this viewset).
    pub fn pagination(mut self, p: Option<Pagination>) -> Self {
        self.pagination = Some(p);
        self
    }
    /// `SearchFilter` with `search_fields`.
    pub fn search_fields(mut self, f: &[&str]) -> Self {
        self.search_fields = f.iter().map(|s| (*s).to_owned()).collect();
        self
    }
    /// `OrderingFilter` with `ordering_fields`.
    pub fn ordering_fields(mut self, f: &[&str]) -> Self {
        self.ordering_fields = Some(f.iter().map(|s| (*s).to_owned()).collect());
        self.ordering_enabled = true;
        self
    }
    /// Default `ordering`.
    pub fn ordering(mut self, f: &[&str]) -> Self {
        self.ordering = f.iter().map(|s| (*s).to_owned()).collect();
        self
    }
    /// Exact-match query filters (`rango-filter`'s `filterset_fields`).
    pub fn filterset_fields(mut self, f: &[&str]) -> Self {
        self.filterset_fields = f.iter().map(|s| (*s).to_owned()).collect();
        self
    }
    pub fn lookup_field(mut self, f: &str) -> Self {
        self.lookup_field = f.to_owned();
        self
    }
    pub fn using(mut self, alias: &str) -> Self {
        self.using = alias.to_owned();
        self
    }
    /// Shown in the OpenAPI document (the view's docstring in the REST framework).
    pub fn description(mut self, d: &str) -> Self {
        self.description = d.to_owned();
        self
    }
    /// `@action(detail=..., methods=[...])`: `handler` gets the object for detail actions.
    pub fn action<F, Fut>(mut self, name: &str, detail: bool, methods: &[&'static str], handler: F) -> Self
    where
        F: Fn(ApiRequest, Option<M>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Response>> + Send + 'static,
    {
        self.extra.push(ExtraAction { name: name.to_owned(), detail, methods: methods.to_vec(), handler: Arc::new(move |r, o| Box::pin(handler(r, o))) });
        self
    }

    fn paginator(&self) -> Option<Pagination> {
        match &self.pagination {
            Some(p) => p.clone(),
            None => crate::settings::default_pagination(),
        }
    }

    fn filter_queryset(&self, req: &ApiRequest, mut qs: QuerySet<M>) -> QuerySet<M> {
        for f in &self.filterset_fields {
            if let Some(v) = req.query(f).filter(|v| !v.is_empty()) {
                // Query strings are text: coerce to the column's type.
                let ty = M::meta().ok().and_then(|m| m.get_field(f).map(|x| x.ty));
                let value = match ty {
                    Some(FieldType::Boolean) => DbValue::Bool(matches!(v.to_lowercase().as_str(), "true" | "1" | "yes" | "on")),
                    Some(t) if t.is_auto() || matches!(t, FieldType::Integer | FieldType::BigInteger | FieldType::SmallInteger | FieldType::PositiveInteger | FieldType::PositiveBigInteger | FieldType::PositiveSmallInteger | FieldType::ForeignKey) => v.parse().map_or_else(|_| DbValue::Str(v.to_owned()), DbValue::Int),
                    _ => DbValue::Str(v.to_owned()),
                };
                qs = qs.filter_kw(f, value);
            }
        }
        if !self.search_fields.is_empty() {
            let terms = req.query(&crate::settings::search_param()).unwrap_or("").replace(',', " ");
            for term in terms.split_whitespace() {
                let mut q: Option<Q> = None;
                for f in &self.search_fields {
                    let one = Q::lookup(&format!("{f}__icontains"), term);
                    q = Some(match q {
                        Some(prev) => prev | one,
                        None => one,
                    });
                }
                if let Some(q) = q {
                    qs = qs.filter_q(q);
                }
            }
        }
        let mut order: Vec<String> = Vec::new();
        if self.ordering_enabled {
            let allowed: Vec<String> = self.ordering_fields.clone().unwrap_or_else(|| self.serializer.serializer.fields.keys().cloned().collect());
            order = req.query(&crate::settings::ordering_param()).unwrap_or("").split(',').map(str::trim).filter(|t| allowed.iter().any(|a| a == t.trim_start_matches('-'))).map(str::to_owned).collect();
        }
        if order.is_empty() {
            order.clone_from(&self.ordering);
        }
        if !order.is_empty() {
            qs = qs.order_by(&order.iter().map(String::as_str).collect::<Vec<_>>());
        }
        qs
    }

    /// `get_object()`.
    async fn get_object(&self, req: &ApiRequest, args: &ViewArgs, db: &Db) -> Result<M> {
        let meta = M::meta()?;
        let missing = || Error::Http404(format!("No {} matches the given query.", meta.object_name));
        let raw = args.kwargs.get("pk").map(ToString::to_string).ok_or_else(missing)?;
        let field = if self.lookup_field == "pk" { Some(meta.pk()) } else { meta.get_field(&self.lookup_field) };
        let is_int = field.is_some_and(|f| f.ty.is_auto() || matches!(f.ty, FieldType::Integer | FieldType::BigInteger | FieldType::SmallInteger | FieldType::PositiveInteger | FieldType::PositiveBigInteger | FieldType::PositiveSmallInteger));
        let value = if is_int { DbValue::Int(raw.parse().map_err(|_| missing())?) } else { DbValue::Str(raw) };
        match (self.queryset)(req).filter_kw(&self.lookup_field, value).get(db).await {
            Err(Error::ObjectDoesNotExist { .. }) => Err(missing()),
            other => other,
        }
    }

    async fn validated(&self, db: &Db, req: &ApiRequest, instance: Option<&M>, partial: bool) -> Result<serde_json::Map<String, Json>> {
        match self.serializer.validate_data(db, &req.data, instance, partial).await? {
            Ok(v) => Ok(v),
            Err(errors) => Err(ApiException::validation(errors).into()),
        }
    }
}

#[async_trait]
impl<M: Model + Default> ViewSet for ModelViewSet<M> {
    fn config(&self) -> &ViewConfig {
        &self.config
    }

    fn basename(&self) -> String {
        M::meta().map(|m| m.object_name.to_lowercase()).unwrap_or_default()
    }

    fn display_name(&self) -> String {
        M::meta().map(|m| m.object_name.clone()).unwrap_or_default()
    }

    fn routes(&self) -> Vec<Route> {
        let s = |x: &str| x.to_owned();
        let (mut list, mut detail) = (vec![("GET", s("list"))], vec![("GET", s("retrieve"))]);
        if !self.read_only {
            list.push(("POST", s("create")));
            detail.extend([("PUT", s("update")), ("PATCH", s("partial_update")), ("DELETE", s("destroy"))]);
        }
        let mut routes = vec![Route { detail: false, url_path: None, actions: list }, Route { detail: true, url_path: None, actions: detail }];
        for a in &self.extra {
            routes.push(Route { detail: a.detail, url_path: Some(a.name.clone()), actions: a.methods.iter().map(|m| (*m, a.name.clone())).collect() });
        }
        routes
    }

    async fn call(&self, action: &str, req: ApiRequest, args: &ViewArgs) -> Result<Response> {
        let db = Db::using(&self.using);
        match action {
            "list" => {
                let qs = self.filter_queryset(&req, (self.queryset)(&req));
                match self.paginator() {
                    Some(p) => {
                        let count = qs.count(&db).await?;
                        let w = p.window(&req.http, count)?;
                        let rows = qs.slice(w.offset, Some(w.offset + w.limit)).fetch(&db).await?;
                        let mut out = Vec::with_capacity(rows.len());
                        for r in &rows {
                            out.push(self.serializer.represent(&db, r).await?);
                        }
                        Ok(Response::new(Pagination::envelope(count, &w, out)))
                    }
                    None => {
                        let rows = qs.fetch(&db).await?;
                        let mut out = Vec::with_capacity(rows.len());
                        for r in &rows {
                            out.push(self.serializer.represent(&db, r).await?);
                        }
                        Ok(Response::new(Json::Array(out)))
                    }
                }
            }
            "create" => {
                let v = self.validated(&db, &req, None, false).await?;
                let obj = self.serializer.create(&db, &v).await?;
                Ok(Response::created(self.serializer.represent(&db, &obj).await?))
            }
            "retrieve" => {
                let obj = self.get_object(&req, args, &db).await?;
                Ok(Response::new(self.serializer.represent(&db, &obj).await?))
            }
            "update" | "partial_update" => {
                let obj = self.get_object(&req, args, &db).await?;
                let v = self.validated(&db, &req, Some(&obj), action == "partial_update").await?;
                let obj = self.serializer.update(&db, &obj, &v).await?;
                Ok(Response::new(self.serializer.represent(&db, &obj).await?))
            }
            "destroy" => {
                let obj = self.get_object(&req, args, &db).await?;
                rango_db::models::deletion::delete_pks(&db, &M::label(), vec![obj.pk_value()]).await?;
                Ok(Response::no_content())
            }
            other => {
                let a = self.extra.iter().find(|a| a.name == other).ok_or_else(|| Error::Http404(String::new()))?;
                let obj = if a.detail { Some(self.get_object(&req, args, &db).await?) } else { None };
                (a.handler)(req, obj).await
            }
        }
    }

    fn schema(&self) -> ViewSetSchema {
        let lookup_is_integer = M::meta().is_ok_and(|m| self.lookup_field == "pk" && (m.pk().ty.is_auto() || matches!(m.pk().ty, FieldType::Integer | FieldType::BigInteger)));
        ViewSetSchema {
            serializer: self.serializer.serializer.clone(),
            pagination: self.paginator(),
            search: !self.search_fields.is_empty(),
            ordering: self.ordering_enabled,
            filterset_fields: self.filterset_fields.clone(),
            lookup_is_integer,
            description: self.description.clone(),
        }
    }
}
