//! `contrib.admin` (a compact admin): site, `ModelAdmin`, change lists with search / filters /
//! ordering / pagination, add / change / delete through `ModelForm`, login and permission checks.
//!
//! Not implemented yet: inlines, actions other than delete, `LogEntry` history, date hierarchy,
//! autocomplete widgets, admin docs.

use std::marker::PhantomData;
use std::sync::{Arc, LazyLock, RwLock};

use async_trait::async_trait;
use indexmap::IndexMap;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_db::models::fields_db::prep_param;
use rango_db::models::meta::{FieldType, ModelMeta, registry};
use rango_db::models::{Model, deletion};
use rango_db::{Db, Executor, Value as DbValue};
use rango_forms::{Data, FileData, Form, ModelForm, ModelFormOptions};
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, Engine, Value as T};
use rango_urls::{ViewArgs, path};
use rango_views::generic::View;

use crate::auth::{self, CurrentUser, Credentials, RequestUserExt};
use crate::messages;

static EMBEDDED: LazyLock<Engine> = LazyLock::new(|| {
    let mut e = Engine::new();
    e.locmem(&[
        ("admin/base.html", include_str!("../../templates/admin/base.html")),
        ("admin/login.html", include_str!("../../templates/admin/login.html")),
        ("admin/index.html", include_str!("../../templates/admin/index.html")),
        ("admin/change_list.html", include_str!("../../templates/admin/change_list.html")),
        ("admin/change_form.html", include_str!("../../templates/admin/change_form.html")),
        ("admin/delete_confirmation.html", include_str!("../../templates/admin/delete_confirmation.html")),
    ]);
    e
});

use rango_views::debug::THEME_CSS;

/// `ModelAdmin` options.
#[derive(Clone)]
pub struct ModelAdmin {
    /// Columns of the change list (field names, or `"__str__"`).
    pub list_display: Vec<String>,
    pub list_filter: Vec<String>,
    pub search_fields: Vec<String>,
    pub ordering: Vec<String>,
    pub list_per_page: usize,
    /// Form fields (`None` = all editable).
    pub fields: Option<Vec<String>>,
    pub exclude: Vec<String>,
}

impl Default for ModelAdmin {
    fn default() -> Self {
        Self { list_display: vec!["__str__".into()], list_filter: vec![], search_fields: vec![], ordering: vec![], list_per_page: 100, fields: None, exclude: vec![] }
    }
}

impl ModelAdmin {
    pub fn list_display<I: IntoIterator<Item = S>, S: Into<String>>(mut self, v: I) -> Self {
        self.list_display = v.into_iter().map(Into::into).collect();
        self
    }
    pub fn list_filter<I: IntoIterator<Item = S>, S: Into<String>>(mut self, v: I) -> Self {
        self.list_filter = v.into_iter().map(Into::into).collect();
        self
    }
    pub fn search_fields<I: IntoIterator<Item = S>, S: Into<String>>(mut self, v: I) -> Self {
        self.search_fields = v.into_iter().map(Into::into).collect();
        self
    }
    pub fn ordering<I: IntoIterator<Item = S>, S: Into<String>>(mut self, v: I) -> Self {
        self.ordering = v.into_iter().map(Into::into).collect();
        self
    }
    pub fn fields<I: IntoIterator<Item = S>, S: Into<String>>(mut self, v: I) -> Self {
        self.fields = Some(v.into_iter().map(Into::into).collect());
        self
    }
    pub fn exclude<I: IntoIterator<Item = S>, S: Into<String>>(mut self, v: I) -> Self {
        self.exclude = v.into_iter().map(Into::into).collect();
        self
    }
    pub fn list_per_page(mut self, n: usize) -> Self {
        self.list_per_page = n;
        self
    }
}

/// Result of showing/saving a form.
struct FormFlow {
    form: Form,
    saved: Option<String>,
    object: Option<String>,
}

#[async_trait]
trait ModelOps: Send + Sync {
    async fn form_flow(&self, db: &Db, pk: Option<DbValue>, post: Option<(Data, FileData)>, opts: ModelFormOptions) -> Result<Option<FormFlow>>;
}

struct TypedOps<M>(PhantomData<fn() -> M>);

fn object_str(meta: &ModelMeta, data: &IndexMap<String, DbValue>, pk: &DbValue) -> String {
    meta.str_field.as_ref().and_then(|n| meta.get_field(n)).and_then(|f| data.get(&f.attname())).map_or_else(|| format!("{} object ({})", meta.object_name, cell(pk)), cell)
}

#[async_trait]
impl<M: Model + Default> ModelOps for TypedOps<M> {
    async fn form_flow(&self, db: &Db, pk: Option<DbValue>, post: Option<(Data, FileData)>, opts: ModelFormOptions) -> Result<Option<FormFlow>> {
        let meta = M::meta()?;
        let mut mf = match pk {
            Some(pk) => match M::objects().filter_kw("pk", pk).get(db).await {
                Ok(o) => ModelForm::<M>::edit(db, o, opts).await?,
                Err(Error::ObjectDoesNotExist { .. }) => return Ok(None),
                Err(e) => return Err(e),
            },
            None => ModelForm::<M>::new(db, opts).await?,
        };
        let object = (!mf.is_new).then(|| object_str(&meta, &mf.instance.to_data(), &mf.instance.pk_value()));
        let Some((data, files)) = post else { return Ok(Some(FormFlow { form: mf.form, saved: None, object })) };
        mf = mf.bound(data, files);
        if mf.is_valid(db).await? {
            let saved = mf.save(db).await?;
            let s = object_str(&meta, &saved.to_data(), &saved.pk_value());
            return Ok(Some(FormFlow { form: mf.form, saved: Some(s), object }));
        }
        Ok(Some(FormFlow { form: mf.form, saved: None, object }))
    }
}

struct Registered {
    meta: Arc<ModelMeta>,
    admin: ModelAdmin,
    ops: Arc<dyn ModelOps>,
}

/// `AdminSite`.
pub struct AdminSite {
    /// URL prefix the site is mounted at, e.g. `/admin/`.
    pub prefix: String,
    pub site_header: String,
    pub site_title: String,
    pub index_title: String,
    models: RwLock<IndexMap<String, Arc<Registered>>>,
}

impl AdminSite {
    pub fn new(prefix: &str) -> Arc<Self> {
        Arc::new(Self { prefix: prefix.to_owned(), site_header: "Rango administration".into(), site_title: "Rango site admin".into(), index_title: "Site administration".into(), models: RwLock::new(IndexMap::new()) })
    }

    /// `site.register(Model, ModelAdmin)`.
    pub fn register<M: Model + Default>(&self, admin: ModelAdmin) -> Result<()> {
        let meta = M::meta()?;
        self.models.write().expect("admin models").insert(meta.label_lower(), Arc::new(Registered { meta, admin, ops: Arc::new(TypedOps::<M>(PhantomData)) }));
        Ok(())
    }

    fn get(&self, app: &str, model: &str) -> Option<Arc<Registered>> {
        self.models.read().expect("admin models").get(&format!("{app}.{model}").to_lowercase()).cloned()
    }

    /// `site.urls`: patterns to `include()` under [`prefix`](Self::prefix).
    pub fn urls(self: &Arc<Self>) -> Vec<rango_urls::UrlEntry> {
        let v = |r: Route| AdminView { site: Arc::clone(self), route: r }.as_view();
        rango_urls::urlpatterns![
            path("", v(Route::Index)).name("index"),
            path("login/", v(Route::Login)).name("login"),
            path("logout/", v(Route::Logout)).name("logout"),
            path("<str:app>/<str:model>/", v(Route::List)).name("changelist"),
            path("<str:app>/<str:model>/add/", v(Route::Add)).name("add"),
            path("<str:app>/<str:model>/<str:pk>/change/", v(Route::Change)).name("change"),
            path("<str:app>/<str:model>/<str:pk>/delete/", v(Route::Delete)).name("delete"),
        ]
    }
}

#[derive(Clone, Copy)]
enum Route {
    Index,
    Login,
    Logout,
    List,
    Add,
    Change,
    Delete,
}

struct AdminView {
    site: Arc<AdminSite>,
    route: Route,
}

fn cell(v: &DbValue) -> String {
    match v {
        DbValue::Null => "-".into(),
        DbValue::Bool(true) => "Yes".into(),
        DbValue::Bool(false) => "No".into(),
        DbValue::Int(i) => i.to_string(),
        DbValue::Float(f) => f.to_string(),
        DbValue::Decimal(d) => d.to_string(),
        DbValue::Str(s) => s.clone(),
        DbValue::Date(d) => d.to_string(),
        DbValue::Time(t) => t.to_string(),
        DbValue::DateTime(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(),
        DbValue::NaiveDateTime(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(),
        DbValue::Uuid(u) => u.to_string(),
        other => format!("{other:?}"),
    }
}

fn qs_url(base: &str, params: &[(&str, String)]) -> String {
    let q: Vec<String> = params.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{}={}", rango_core::utils::http::quote_plus(k), rango_core::utils::http::quote(v, "/"))).collect();
    if q.is_empty() { base.to_owned() } else { format!("{base}?{}", q.join("&")) }
}

impl AdminView {
    fn base_context(&self, req: &HttpRequest, title: &str) -> Context {
        let mut c = Context::new();
        c.set("title", title);
        c.set("site_title", self.site.site_title.clone());
        c.set("site_header", self.site.site_header.clone());
        c.set("index_title", self.site.index_title.clone());
        c.set("admin_url", self.site.prefix.clone());
        c.set("theme_css", T::Safe(THEME_CSS.to_owned()));
        c.set("messages", messages::get_messages(req).map_or(T::None, rango_template::Object::new));
        c
    }

    async fn render(&self, req: &HttpRequest, name: &str, mut ctx: Context) -> Result<HttpResponse> {
        // A project may override any admin template; otherwise the bundled one is used.
        let project = rango_template::loader::engine().ok().and_then(|e| e.get_template(name).ok().map(|t| (e, t)));
        let html = match project {
            Some((engine, t)) => {
                rango_template::loader::bind_request(&mut ctx, req, &engine)?;
                t.render(&engine, &mut ctx)?
            }
            None => {
                rango_template::loader::bind_request(&mut ctx, req, &EMBEDDED)?;
                EMBEDDED.get_template(name)?.render(&EMBEDDED, &mut ctx)?
            }
        };
        Ok(HttpResponse::new(html))
    }

    async fn can(&self, user: &CurrentUser, db: &Db, r: &Registered, action: &str) -> Result<bool> {
        let perm = format!("{}.{action}_{}", r.meta.app_label, r.meta.object_name.to_lowercase());
        if user.has_perm(db, &perm).await? {
            return Ok(true);
        }
        // `change` implies `view`.
        Ok(action == "view" && user.has_perm(db, &format!("{}.change_{}", r.meta.app_label, r.meta.object_name.to_lowercase())).await?)
    }

    fn login_redirect(&self, req: &HttpRequest) -> Result<HttpResponse> {
        HttpResponse::redirect(&qs_url(&format!("{}login/", self.site.prefix), &[("next", req.get_full_path())]))
    }

    async fn dispatch_route(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let post = req.method() == "POST";
        let db = Db::default_db();
        match self.route {
            Route::Login => return self.login(&req, &db, post).await,
            Route::Logout => {
                if !post {
                    return Ok(HttpResponse::not_allowed(&["POST"], ""));
                }
                auth::logout(&req).await?;
                return HttpResponse::redirect(&format!("{}login/", self.site.prefix));
            }
            _ => {}
        }
        let user = req.user();
        if !user.is_staff().await {
            return self.login_redirect(&req);
        }
        let arg = |k: &str| args.kwargs.get(k).map(ToString::to_string).unwrap_or_default();
        if matches!(self.route, Route::Index) {
            return self.index(&req, &db, &user).await;
        }
        let reg = self.site.get(&arg("app"), &arg("model")).ok_or_else(|| Error::Http404(format!("'{}.{}' is not registered in the admin", arg("app"), arg("model"))))?;
        match self.route {
            Route::List => {
                if !self.can(&user, &db, &reg, "view").await? {
                    return Err(Error::PermissionDenied(String::new()));
                }
                self.changelist(&req, &db, &reg).await
            }
            Route::Add | Route::Change => {
                let adding = matches!(self.route, Route::Add);
                if !self.can(&user, &db, &reg, if adding { "add" } else { "change" }).await? {
                    return Err(Error::PermissionDenied(String::new()));
                }
                let pk = if adding { None } else { Some(parse_pk(&reg.meta, &arg("pk"))) };
                self.form_view(&req, &db, &reg, pk, post).await
            }
            Route::Delete => {
                if !self.can(&user, &db, &reg, "delete").await? {
                    return Err(Error::PermissionDenied(String::new()));
                }
                self.delete(&req, &db, &reg, parse_pk(&reg.meta, &arg("pk")), post).await
            }
            _ => unreachable!(),
        }
    }

    async fn login(&self, req: &HttpRequest, db: &Db, post: bool) -> Result<HttpResponse> {
        let mut ctx = self.base_context(req, "Log in");
        let next = req.query().get("next").map(str::to_owned).or_else(|| None).unwrap_or_default();
        if post {
            let p = req.post().await?;
            let creds = Credentials::new(p.get("username").unwrap_or(""), p.get("password").unwrap_or(""));
            let next_post = p.get("next").unwrap_or("").to_owned();
            if let Some((user, backend)) = auth::authenticate(db, &creds).await? {
                if user.is_staff() && user.is_active() {
                    auth::login(req, db, user, Some(&backend)).await?;
                    return HttpResponse::redirect(&if next_post.starts_with('/') { next_post } else { self.site.prefix.clone() });
                }
            }
            ctx.set("error", "Please enter the correct username and password for a staff account. Note that both fields may be case-sensitive.");
        }
        ctx.set("next", next);
        self.render(req, "admin/login.html", ctx).await
    }

    async fn index(&self, req: &HttpRequest, db: &Db, user: &CurrentUser) -> Result<HttpResponse> {
        let mut apps: IndexMap<String, Vec<T>> = IndexMap::new();
        let regs: Vec<Arc<Registered>> = self.site.models.read().expect("admin models").values().cloned().collect();
        for r in regs {
            if !self.can(user, db, &r, "view").await? && !self.can(user, db, &r, "add").await? {
                continue;
            }
            let base = format!("{}{}/{}/", self.site.prefix, r.meta.app_label, r.meta.object_name.to_lowercase());
            let add = self.can(user, db, &r, "add").await?;
            apps.entry(r.meta.app_label.clone()).or_default().push(T::Map(
                [("name", T::Str(crate::admin::capfirst(&r.meta.verbose_name_plural))), ("url", T::Str(base.clone())), ("add_url", if add { T::Str(format!("{base}add/")) } else { T::None })].into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            ));
        }
        let mut ctx = self.base_context(req, &self.site.index_title);
        ctx.set("user_name", user.username().await);
        ctx.set("app_list", T::List(apps.into_iter().map(|(a, m)| T::Map([("name".to_owned(), T::Str(crate::admin::capfirst(&a))), ("models".to_owned(), T::List(m))].into_iter().collect())).collect()));
        self.render(req, "admin/index.html", ctx).await
    }

    async fn changelist(&self, req: &HttpRequest, db: &Db, r: &Registered) -> Result<HttpResponse> {
        let vendor = db.vendor();
        let m = &r.meta;
        let base = format!("{}{}/{}/", self.site.prefix, m.app_label, m.object_name.to_lowercase());
        let q = req.query();
        let (search, page, order) = (q.get("q").unwrap_or("").to_owned(), q.get("p").and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1), q.get("o").unwrap_or("").to_owned());
        let cols: Vec<String> = r.admin.list_display.clone();
        // WHERE
        let (mut conds, mut params) = (Vec::new(), Vec::new());
        let mut active: Vec<(String, String)> = Vec::new();
        for f in &r.admin.list_filter {
            if let (Some(v), Some(field)) = (q.get(f), m.get_field(f)) {
                if !v.is_empty() {
                    conds.push(format!("{} = %s", vendor.quote(&field.column())));
                    params.push(prep_param(vendor, field, filter_value(field.ty, v)));
                    active.push((f.clone(), v.to_owned()));
                }
            }
        }
        if !search.is_empty() && !r.admin.search_fields.is_empty() {
            let likes: Vec<String> = r.admin.search_fields.iter().filter_map(|n| m.get_field(n)).map(|f| format!("LOWER({}) LIKE %s", vendor.quote(&f.column()))).collect();
            if !likes.is_empty() {
                params.extend(std::iter::repeat_n(DbValue::Str(format!("%{}%", search.to_lowercase())), likes.len()));
                conds.push(format!("({})", likes.join(" OR ")));
            }
        }
        let where_sql = if conds.is_empty() { String::new() } else { format!(" WHERE {}", conds.join(" AND ")) };
        let table = vendor.quote(&m.db_table);
        let count = match db.query(&format!("SELECT COUNT(*) FROM {table}{where_sql}"), &params).await?.first().and_then(|r| r.values.first()) {
            Some(DbValue::Int(n)) => *n as usize,
            _ => 0,
        };
        // ORDER
        let order_spec = if !order.is_empty() { vec![order.clone()] } else if !r.admin.ordering.is_empty() { r.admin.ordering.clone() } else if !m.ordering.is_empty() { m.ordering.clone() } else { vec!["-pk".into()] };
        let order_sql: Vec<String> = order_spec
            .iter()
            .filter_map(|o| {
                let (name, desc) = o.strip_prefix('-').map_or((o.as_str(), false), |n| (n, true));
                let f = if name == "pk" { Some(m.pk()) } else { m.get_field(name).filter(|f| f.concrete()) }?;
                Some(format!("{}{}", vendor.quote(&f.column()), if desc { " DESC" } else { "" }))
            })
            .collect();
        // SELECT: pk, str field, then each displayed field
        let str_col = m.str_field.as_ref().and_then(|n| m.get_field(n));
        let mut select: Vec<&rango_db::models::meta::Field> = vec![m.pk()];
        if let Some(f) = str_col {
            select.push(f);
        }
        let disp: Vec<Option<&rango_db::models::meta::Field>> = cols.iter().map(|c| m.get_field(c).filter(|f| f.concrete())).collect();
        select.extend(disp.iter().flatten().copied());
        let per = r.admin.list_per_page.max(1);
        let sql = format!("SELECT {} FROM {table}{where_sql} ORDER BY {} LIMIT {per} OFFSET {}", select.iter().map(|f| vendor.quote(&f.column())).collect::<Vec<_>>().join(", "), if order_sql.is_empty() { vendor.quote(&m.pk().column()) } else { order_sql.join(", ") }, (page - 1) * per);
        let rows = db.query(&sql, &params).await?;
        let offset = 1 + usize::from(str_col.is_some());
        let mut out_rows = Vec::new();
        for row in &rows {
            let pk = rango_db::models::fields_db::from_db(vendor, m.pk(), row.values[0].clone())?;
            let strv = str_col.map(|_| cell(&row.values[1])).unwrap_or_else(|| format!("{} object ({})", m.object_name, cell(&pk)));
            let mut di = 0;
            let cells: Vec<T> = cols
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if c == "__str__" || disp[i].is_none() {
                        T::Str(strv.clone())
                    } else {
                        let v = &row.values[offset + di];
                        di += 1;
                        let _ = i;
                        T::Str(cell(v))
                    }
                })
                .collect();
            out_rows.push(T::Map([("cells".to_owned(), T::List(cells)), ("change_url".to_owned(), T::Str(format!("{base}{}/change/", cell(&pk))))].into_iter().collect()));
        }
        let keep = |skip: &str| -> Vec<(&str, String)> { active.iter().filter(|(k, _)| k != skip).map(|(k, v)| (k.as_str(), v.clone())).collect() };
        let headers: Vec<T> = cols
            .iter()
            .map(|c| {
                let label = if c == "__str__" { capfirst(&m.verbose_name) } else { capfirst(&c.replace('_', " ")) };
                let next = if order == *c { format!("-{c}") } else { c.clone() };
                let mut p: Vec<(&str, String)> = active.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                p.push(("q", search.clone()));
                p.push(("o", if c == "__str__" { String::new() } else { next }));
                T::Map([("label".to_owned(), T::Str(label)), ("url".to_owned(), T::Str(qs_url(&base, &p)))].into_iter().collect())
            })
            .collect();
        let mut filters = Vec::new();
        for f in &r.admin.list_filter {
            let Some(field) = m.get_field(f) else { continue };
            let vals = db.query(&format!("SELECT DISTINCT {c} FROM {table} ORDER BY {c}", c = vendor.quote(&field.column())), &[]).await?;
            let cur = q.get(f).unwrap_or("").to_owned();
            let url_for = |v: &str| {
                let mut p = keep(f);
                p.push((f.as_str(), v.to_owned()));
                p.push(("q", search.clone()));
                qs_url(&base, &p)
            };
            let mut choices = vec![T::Map([("label".to_owned(), T::Str("All".into())), ("url".to_owned(), T::Str(url_for(""))), ("selected".to_owned(), T::Bool(cur.is_empty()))].into_iter().collect())];
            for v in vals.into_iter().filter_map(|r| r.values.into_iter().next()).filter(|v| !v.is_null()) {
                let raw = match &v {
                    DbValue::Bool(b) => i64::from(*b).to_string(),
                    other => cell(other),
                };
                choices.push(T::Map([("label".to_owned(), T::Str(cell(&v))), ("url".to_owned(), T::Str(url_for(&raw))), ("selected".to_owned(), T::Bool(cur == raw))].into_iter().collect()));
            }
            filters.push(T::Map([("title".to_owned(), T::Str(f.replace('_', " "))), ("choices".to_owned(), T::List(choices))].into_iter().collect()));
        }
        let num_pages = count.div_ceil(per).max(1);
        let page_url = |p: usize| {
            let mut v: Vec<(&str, String)> = active.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            v.push(("q", search.clone()));
            v.push(("o", order.clone()));
            v.push(("p", p.to_string()));
            qs_url(&base, &v)
        };
        let user = req.user();
        let mut ctx = self.base_context(req, &capfirst(&m.verbose_name_plural));
        ctx.set("user_name", user.username().await);
        ctx.set("app_name", capfirst(&m.app_label));
        ctx.set("verbose_name", m.verbose_name.clone());
        ctx.set("verbose_name_plural", m.verbose_name_plural.clone());
        ctx.set("add_url", if self.can(&user, db, r, "add").await? { T::Str(format!("{base}add/")) } else { T::None });
        ctx.set("search_fields", T::Bool(!r.admin.search_fields.is_empty()));
        ctx.set("query", search.clone());
        ctx.set("count", count as i64);
        ctx.set("headers", T::List(headers));
        ctx.set("rows", T::List(out_rows));
        ctx.set("filters", T::List(filters));
        ctx.set("page", page as i64);
        ctx.set("num_pages", num_pages as i64);
        ctx.set("prev_url", if page > 1 { T::Str(page_url(page - 1)) } else { T::None });
        ctx.set("next_url", if page < num_pages { T::Str(page_url(page + 1)) } else { T::None });
        self.render(req, "admin/change_list.html", ctx).await
    }

    fn form_options(r: &Registered) -> ModelFormOptions {
        ModelFormOptions { fields: r.admin.fields.clone(), exclude: r.admin.exclude.clone(), ..ModelFormOptions::default() }
    }

    async fn form_view(&self, req: &HttpRequest, db: &Db, r: &Registered, pk: Option<DbValue>, post: bool) -> Result<HttpResponse> {
        let m = &r.meta;
        let base = format!("{}{}/{}/", self.site.prefix, m.app_label, m.object_name.to_lowercase());
        let data = if post { Some((Data::from_lists(req.post().await?.lists().map(|(k, v)| (k.to_owned(), v.to_vec()))), req.files().await?.clone())) } else { None };
        let Some(flow) = r.ops.form_flow(db, pk.clone(), data, Self::form_options(r)).await? else { return Err(Error::Http404(format!("{} with ID “{}” doesn’t exist. Perhaps it was deleted?", capfirst(&m.verbose_name), pk.as_ref().map_or_else(String::new, cell)))) };
        if let Some(obj) = &flow.saved {
            let verb = if pk.is_none() { "added" } else { "changed" };
            messages::add_message(req, messages::SUCCESS, &format!("The {} “{obj}” was {verb} successfully.", m.verbose_name), "", true)?;
            return HttpResponse::redirect(&base);
        }
        let adding = pk.is_none();
        let mut ctx = self.base_context(req, &if adding { format!("Add {}", m.verbose_name) } else { format!("Change {}", m.verbose_name) });
        ctx.set("user_name", req.user().username().await);
        ctx.set("app_name", capfirst(&m.app_label));
        ctx.set("verbose_name_plural", m.verbose_name_plural.clone());
        ctx.set("list_url", base.clone());
        ctx.set("form", flow.form.to_template());
        if let Some(pk) = &pk {
            if self.can(&req.user(), db, r, "delete").await? {
                ctx.set("delete_url", format!("{base}{}/delete/", cell(pk)));
            }
        }
        self.render(req, "admin/change_form.html", ctx).await
    }

    async fn delete(&self, req: &HttpRequest, db: &Db, r: &Registered, pk: DbValue, post: bool) -> Result<HttpResponse> {
        let m = &r.meta;
        let base = format!("{}{}/{}/", self.site.prefix, m.app_label, m.object_name.to_lowercase());
        let Some(flow) = r.ops.form_flow(db, Some(pk.clone()), None, Self::form_options(r)).await? else { return Err(Error::Http404("No such object".into())) };
        let name = flow.object.unwrap_or_default();
        if post {
            deletion::delete_pks(db as &dyn Executor, &m.label(), vec![pk]).await?;
            messages::add_message(req, messages::SUCCESS, &format!("The {} “{name}” was deleted successfully.", m.verbose_name), "", true)?;
            return HttpResponse::redirect(&base);
        }
        let mut ctx = self.base_context(req, "Are you sure?");
        ctx.set("user_name", req.user().username().await);
        ctx.set("app_name", capfirst(&m.app_label));
        ctx.set("verbose_name", m.verbose_name.clone());
        ctx.set("verbose_name_plural", m.verbose_name_plural.clone());
        ctx.set("list_url", base);
        ctx.set("object", name);
        self.render(req, "admin/delete_confirmation.html", ctx).await
    }
}

fn parse_pk(m: &ModelMeta, raw: &str) -> DbValue {
    match m.pk().ty {
        FieldType::Uuid => raw.parse().map_or_else(|_| DbValue::Str(raw.to_owned()), DbValue::Uuid),
        t if t.is_auto() || matches!(t, FieldType::Integer | FieldType::BigInteger | FieldType::SmallInteger) => raw.parse().map_or_else(|_| DbValue::Str(raw.to_owned()), DbValue::Int),
        _ => DbValue::Str(raw.to_owned()),
    }
}

fn filter_value(ty: FieldType, v: &str) -> DbValue {
    match ty {
        FieldType::Boolean => DbValue::Bool(v == "1" || v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true")),
        t if t.is_auto() || matches!(t, FieldType::Integer | FieldType::BigInteger | FieldType::SmallInteger | FieldType::ForeignKey | FieldType::OneToOne | FieldType::PositiveInteger) => v.parse().map_or_else(|_| DbValue::Str(v.to_owned()), DbValue::Int),
        _ => DbValue::Str(v.to_owned()),
    }
}

pub fn capfirst(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
}

#[async_trait]
impl View for AdminView {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "OPTIONS"]
    }
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.dispatch_route(req, args).await
    }
    async fn post(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.dispatch_route(req, args).await
    }
}

/// Create the add/change/delete/view permissions of an app's models (`create_permissions`).
pub async fn create_permissions(db: &Db, app_label: &str) -> Result<usize> {
    use crate::auth::models::Permission;
    let mut n = 0;
    for m in registry().models().filter(|m| m.app_label == app_label && !m.abstract_ && m.proxy_for.is_none()) {
        let ct = crate::contenttypes::get_for_model(db, &m.label()).await?;
        let mut wanted: Vec<(String, String)> = ["add", "change", "delete", "view"].iter().map(|a| (format!("{a}_{}", m.object_name.to_lowercase()), format!("Can {a} {}", m.verbose_name))).collect();
        wanted.extend(m.permissions.iter().cloned());
        for (code, name) in wanted {
            if !Permission::objects().filter_kw("content_type", ct.id).filter_kw("codename", code.clone()).exists(db).await? {
                Permission::objects().create(db, Permission { id: None, name, content_type: rango_db::models::ForeignKey::new(ct.id), codename: code }).await?;
                n += 1;
            }
        }
    }
    Ok(n)
}

#[allow(dead_code)]
fn _s() -> bool {
    settings().debug
}

rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.admin"));
