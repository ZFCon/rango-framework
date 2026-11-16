//! `views.generic.edit`: `FormView`, `CreateView`, `UpdateView`, `DeleteView`.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_db::models::Model;
use rango_db::{Db, Executor};
use rango_forms::{Form, ModelForm, ModelFormOptions};
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, TemplateResponse, Value as T};
use rango_urls::ViewArgs;

use super::base::View;
use super::detail::DetailView;
use super::util::{file_data, model_name_lower, model_value, post_data, url_kwargs_context};

type SuccessUrl = Arc<dyn Fn(&HttpRequest) -> Result<String> + Send + Sync>;
type FormValid = Arc<dyn Fn(HttpRequest, Form) -> BoxFuture<'static, Result<HttpResponse>> + Send + Sync>;

fn redirect_to(url: &str) -> Result<HttpResponse> {
    HttpResponse::redirect(url)
}

fn no_url() -> Error {
    Error::improperly_configured("No URL to redirect to. Provide a success_url.")
}

/// `FormView`: show a form, validate on POST, redirect on success.
pub struct FormView {
    form: Arc<dyn Fn(&HttpRequest) -> Form + Send + Sync>,
    pub template_name: String,
    success_url: Option<SuccessUrl>,
    form_valid: Option<FormValid>,
    pub extra_context: IndexMap<String, T>,
}

impl FormView {
    /// `form_class`: builds a fresh unbound form per request.
    pub fn new(template_name: &str, form: impl Fn(&HttpRequest) -> Form + Send + Sync + 'static) -> Self {
        Self { form: Arc::new(form), template_name: template_name.to_owned(), success_url: None, form_valid: None, extra_context: IndexMap::new() }
    }
    pub fn success_url(mut self, url: &str) -> Self {
        let url = url.to_owned();
        self.success_url = Some(Arc::new(move |_| Ok(url.clone())));
        self
    }
    /// `get_success_url()` override.
    pub fn success_url_with(mut self, f: impl Fn(&HttpRequest) -> Result<String> + Send + Sync + 'static) -> Self {
        self.success_url = Some(Arc::new(f));
        self
    }
    /// `form_valid()` override (default: redirect to the success URL).
    pub fn form_valid(mut self, f: impl Fn(HttpRequest, Form) -> BoxFuture<'static, Result<HttpResponse>> + Send + Sync + 'static) -> Self {
        self.form_valid = Some(Arc::new(f));
        self
    }
    pub fn extra(mut self, key: &str, value: impl Into<T>) -> Self {
        self.extra_context.insert(key.to_owned(), value.into());
        self
    }

    fn render(&self, req: &HttpRequest, args: &ViewArgs, form: &Form) -> HttpResponse {
        let mut ctx = Context::new();
        for (k, v) in url_kwargs_context(args) {
            ctx.set(&k, v);
        }
        for (k, v) in &self.extra_context {
            ctx.set(k, v.clone());
        }
        ctx.set("form", form.to_template());
        TemplateResponse::new(req, self.template_name.as_str(), ctx)
    }
}

#[async_trait]
impl View for FormView {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "PUT", "OPTIONS"]
    }
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        Ok(self.render(&req, args, &(self.form)(&req)))
    }
    async fn post(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let form = (self.form)(&req).bound(post_data(&req).await?, file_data(&req).await?);
        if !form.is_valid() {
            return Ok(self.render(&req, args, &form));
        }
        if let Some(f) = &self.form_valid {
            return f(req, form).await;
        }
        let url = self.success_url.as_ref().ok_or_else(no_url)?(&req)?;
        redirect_to(&url)
    }
    async fn put(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.post(req, args).await
    }
}

type ObjectUrl<M> = Arc<dyn Fn(&M) -> String + Send + Sync>;

/// Shared config of the model form views.
struct ModelEdit<M: Model> {
    detail: DetailView<M>,
    opts: ModelFormOptions,
    success_url: Option<SuccessUrl>,
    absolute_url: Option<ObjectUrl<M>>,
}

impl<M: Model> ModelEdit<M> {
    /// Called before saving: with neither a `success_url` nor an object URL the save would succeed and then fail.
    fn ensure_target(&self) -> Result<()> {
        if self.success_url.is_none() && self.absolute_url.is_none() {
            return Err(Error::improperly_configured("No URL to redirect to.  Either provide a url or define a get_absolute_url method on the Model."));
        }
        Ok(())
    }
    fn success(&self, req: &HttpRequest, obj: &M) -> Result<String> {
        if let Some(f) = &self.success_url {
            return f(req);
        }
        match &self.absolute_url {
            Some(f) => Ok(f(obj)),
            None => Err(Error::improperly_configured("No URL to redirect to.  Either provide a url or define a get_absolute_url method on the Model.")),
        }
    }
}

macro_rules! edit_builders {
    () => {
        pub fn template_name(mut self, n: &str) -> Self {
            self.inner.detail.template_name = Some(n.to_owned());
            self
        }
        pub fn success_url(mut self, url: &str) -> Self {
            let url = url.to_owned();
            self.inner.success_url = Some(Arc::new(move |_| Ok(url.clone())));
            self
        }
        pub fn success_url_with(mut self, f: impl Fn(&HttpRequest) -> Result<String> + Send + Sync + 'static) -> Self {
            self.inner.success_url = Some(Arc::new(f));
            self
        }
        /// `Model.get_absolute_url()`, used when no `success_url` is set.
        pub fn absolute_url(mut self, f: impl Fn(&M) -> String + Send + Sync + 'static) -> Self {
            self.inner.absolute_url = Some(Arc::new(f));
            self
        }
        pub fn using(mut self, alias: &str) -> Self {
            self.inner.detail.using = alias.to_owned();
            self
        }
        pub fn extra(mut self, key: &str, value: impl Into<T>) -> Self {
            self.inner.detail.extra_context.insert(key.to_owned(), value.into());
            self
        }
        pub fn queryset(mut self, f: impl Fn(&HttpRequest) -> rango_db::models::QuerySet<M> + Send + Sync + 'static) -> Self {
            self.inner.detail.queryset = Arc::new(f);
            self
        }
        pub fn pk_url_kwarg(mut self, n: &str) -> Self {
            self.inner.detail.pk_url_kwarg = n.to_owned();
            self
        }
    };
}

fn form_names<M: Model>(detail: &DetailView<M>) -> Result<Vec<String>> {
    let meta = M::meta()?;
    Ok(match &detail.template_name {
        Some(n) => vec![n.clone()],
        None => vec![format!("{}/{}_form.html", meta.app_label, model_name_lower(&meta))],
    })
}

fn form_response<M: Model>(req: &HttpRequest, args: &ViewArgs, detail: &DetailView<M>, form: &Form, obj: Option<&M>) -> Result<HttpResponse> {
    let mut ctx = Context::new();
    for (k, v) in url_kwargs_context(args) {
        ctx.set(&k, v);
    }
    if let Some(o) = obj {
        let v = model_value(o);
        ctx.set("object", v.clone());
        let meta = M::meta()?;
        ctx.set(detail.context_object_name.clone().unwrap_or_else(|| model_name_lower(&meta)).as_str(), v);
    }
    for (k, v) in &detail.extra_context {
        ctx.set(k, v.clone());
    }
    ctx.set("form", form.to_template());
    Ok(TemplateResponse::new(req, form_names(detail)?, ctx))
}

/// `CreateView`.
pub struct CreateView<M: Model + Default> {
    inner: ModelEdit<M>,
}

impl<M: Model + Default> CreateView<M> {
    pub fn new(opts: ModelFormOptions) -> Self {
        Self { inner: ModelEdit { detail: DetailView::new(), opts, success_url: None, absolute_url: None } }
    }
    edit_builders!();
}

#[async_trait]
impl<M: Model + Default> View for CreateView<M> {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "PUT", "OPTIONS"]
    }
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.inner.detail.using);
        let form = ModelForm::<M>::new(&db, self.inner.opts.clone()).await?;
        form_response(&req, args, &self.inner.detail, form.form(), None)
    }
    async fn post(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.inner.detail.using);
        let mut form = ModelForm::<M>::new(&db, self.inner.opts.clone()).await?.bound(post_data(&req).await?, file_data(&req).await?);
        if !form.is_valid(&db).await? {
            return form_response(&req, args, &self.inner.detail, form.form(), None);
        }
        self.inner.ensure_target()?;
        let obj = form.save(&db).await?;
        redirect_to(&self.inner.success(&req, obj)?)
    }
    async fn put(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.post(req, args).await
    }
}

/// `UpdateView`.
pub struct UpdateView<M: Model> {
    inner: ModelEdit<M>,
}

impl<M: Model> UpdateView<M> {
    pub fn new(opts: ModelFormOptions) -> Self {
        Self { inner: ModelEdit { detail: DetailView::new(), opts, success_url: None, absolute_url: None } }
    }
    edit_builders!();
}

#[async_trait]
impl<M: Model> View for UpdateView<M> {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "PUT", "OPTIONS"]
    }
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.inner.detail.using);
        let obj = self.inner.detail.get_object(&req, args, &db).await?;
        let form = ModelForm::<M>::edit(&db, obj, self.inner.opts.clone()).await?;
        form_response(&req, args, &self.inner.detail, form.form(), Some(&form.instance))
    }
    async fn post(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.inner.detail.using);
        let obj = self.inner.detail.get_object(&req, args, &db).await?;
        let mut form = ModelForm::<M>::edit(&db, obj, self.inner.opts.clone()).await?.bound(post_data(&req).await?, file_data(&req).await?);
        if !form.is_valid(&db).await? {
            return form_response(&req, args, &self.inner.detail, form.form(), Some(&form.instance));
        }
        self.inner.ensure_target()?;
        let obj = form.save(&db).await?;
        redirect_to(&self.inner.success(&req, obj)?)
    }
    async fn put(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.post(req, args).await
    }
}

/// `DeleteView`: confirmation page on GET, delete on POST/DELETE.
pub struct DeleteView<M: Model> {
    detail: DetailView<M>,
    success_url: Option<SuccessUrl>,
}

impl<M: Model> DeleteView<M> {
    pub fn new() -> Self {
        let mut detail = DetailView::new();
        detail.template_name_suffix = "_confirm_delete".into();
        Self { detail, success_url: None }
    }
    pub fn template_name(mut self, n: &str) -> Self {
        self.detail.template_name = Some(n.to_owned());
        self
    }
    pub fn success_url(mut self, url: &str) -> Self {
        let url = url.to_owned();
        self.success_url = Some(Arc::new(move |_| Ok(url.clone())));
        self
    }
    pub fn using(mut self, alias: &str) -> Self {
        self.detail.using = alias.to_owned();
        self
    }
    pub fn queryset(mut self, f: impl Fn(&HttpRequest) -> rango_db::models::QuerySet<M> + Send + Sync + 'static) -> Self {
        self.detail.queryset = Arc::new(f);
        self
    }
}

impl<M: Model> Default for DeleteView<M> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<M: Model> View for DeleteView<M> {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "DELETE", "OPTIONS"]
    }
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.detail.using);
        let obj = self.detail.get_object(&req, args, &db).await?;
        let ctx = self.detail.object_context(&req, args, &obj)?;
        Ok(TemplateResponse::new(&req, self.detail.template_names()?, ctx))
    }
    async fn post(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.detail.using);
        let obj = self.detail.get_object(&req, args, &db).await?;
        // Resolve the redirect first: a missing success_url must not leave the row deleted.
        let url = self.success_url.as_ref().ok_or_else(|| Error::improperly_configured("No URL to redirect to. Provide a success_url."))?(&req)?;
        let pk = obj.pk_value();
        rango_db::models::deletion::delete_pks(&db as &dyn Executor, &M::label(), vec![pk]).await?;
        redirect_to(&url)
    }
    async fn delete(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        self.post(req, args).await
    }
}

