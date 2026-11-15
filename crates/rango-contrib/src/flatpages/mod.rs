//! `contrib.flatpages`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::exceptions::{Error, Result};
use rango_db::models::{ManyToMany, Model};
use rango_db::Db;
use rango_http::{HttpRequest, HttpResponse};
use rango_middleware::handlers::Middleware;
use rango_template::{Context, TemplateResponse, Value as T};

use crate::sites::Site;

/// `FlatPage` (table `rango_flatpage`).
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "flatpages", db_table = "rango_flatpage", ordering = ["url"], verbose_name = "flat page", verbose_name_plural = "flat pages", str = "url")]
pub struct FlatPage {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    #[field(max_length = 100, db_index, verbose_name = "URL")]
    pub url: String,
    #[field(max_length = 200, verbose_name = "title")]
    pub title: String,
    #[field(blank, verbose_name = "content")]
    pub content: String,
    #[field(default = false, verbose_name = "enable comments")]
    pub enable_comments: bool,
    #[field(max_length = 70, blank, verbose_name = "template name", help_text = "Example: “flatpages/contact_page.html”. If this isn’t provided, the system will use “flatpages/default.html”.")]
    pub template_name: String,
    #[field(default = false, verbose_name = "registration required", help_text = "If this is checked, only logged-in users will be able to view the page.")]
    pub registration_required: bool,
    pub sites: ManyToMany<Site>,
}

/// `flatpage(request, url)`: render the page at `url` (404 when there is none for this site).
pub async fn flatpage(request: &HttpRequest, url: &str) -> Result<HttpResponse> {
    let db = Db::default_db();
    let site = crate::sites::get_current(&db, Some(request)).await?;
    let mut url = if url.starts_with('/') { url.to_owned() } else { format!("/{url}") };
    let page = match find(&db, site.id, &url).await? {
        Some(p) => p,
        None if !url.ends_with('/') && rango_core::conf::settings().append_slash => {
            url.push('/');
            match find(&db, site.id, &url).await? {
                Some(_) => return HttpResponse::permanent_redirect(&format!("{}/", request.path())),
                None => return Err(Error::Http404("No FlatPage matches the given query.".into())),
            }
        }
        None => return Err(Error::Http404("No FlatPage matches the given query.".into())),
    };
    if page.registration_required && !request.user_is_authenticated() {
        return crate::auth::redirect_to_login(request.path(), None, crate::auth::REDIRECT_FIELD_NAME);
    }
    let names = if page.template_name.is_empty() { vec!["flatpages/default.html".to_owned()] } else { vec![page.template_name.clone(), "flatpages/default.html".to_owned()] };
    let mut ctx = Context::new();
    ctx.set("flatpage", T::Map([("url", page.url.clone()), ("title", page.title.clone())].into_iter().map(|(k, v)| (k.to_owned(), T::Str(v))).chain([("content".to_owned(), T::Safe(page.content.clone())), ("enable_comments".to_owned(), T::Bool(page.enable_comments))]).collect()));
    Ok(TemplateResponse::new(request, names, ctx))
}

async fn find(db: &Db, site_id: Option<i64>, url: &str) -> Result<Option<FlatPage>> {
    match FlatPage::objects().filter_kw("url", url).filter_kw("sites__id", site_id).get(db).await {
        Ok(p) => Ok(Some(p)),
        Err(Error::ObjectDoesNotExist { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

trait AuthExt {
    fn user_is_authenticated(&self) -> bool;
}

impl AuthExt for HttpRequest {
    fn user_is_authenticated(&self) -> bool {
        use crate::auth::RequestUserExt;
        self.user().is_authenticated()
    }
}

/// `FlatpageFallbackMiddleware`: serve a flat page when a 404 would be returned.
#[derive(Default)]
pub struct FlatpageFallbackMiddleware;

#[async_trait]
impl Middleware for FlatpageFallbackMiddleware {
    async fn process_response(&self, request: &HttpRequest, response: HttpResponse) -> Result<HttpResponse> {
        if response.status_code != 404 {
            return Ok(response);
        }
        match flatpage(request, request.path_info()).await {
            Ok(mut r) => {
                r.render_deferred()?;
                Ok(r)
            }
            Err(Error::Http404(_)) => Ok(response),
            Err(e) => Err(e),
        }
    }
}

rango_middleware::register_middleware!("rango.contrib.flatpages.middleware.FlatpageFallbackMiddleware", || Ok(Arc::new(FlatpageFallbackMiddleware)));
rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.flatpages"));
rango_db::register_initial_migration!("flatpages", [("sites", "0001_initial")]);
