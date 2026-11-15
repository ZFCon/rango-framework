//! `contrib.redirects`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::{Error, Result};
use rango_db::models::{ForeignKey, Model};
use rango_db::Db;
use rango_http::{HttpRequest, HttpResponse};
use rango_middleware::handlers::Middleware;

use crate::sites::Site;

/// `Redirect` (table `rango_redirect`).
#[derive(Clone, Debug, Default, PartialEq, Model)]
#[model(crate = "rango_db", app_label = "redirects", db_table = "rango_redirect", ordering = ["old_path"], unique_together = [["site", "old_path"]], verbose_name = "redirect", verbose_name_plural = "redirects")]
pub struct Redirect {
    #[field(primary_key, auto)]
    pub id: Option<i64>,
    pub site: ForeignKey<Site>,
    #[field(max_length = 200, db_index, verbose_name = "redirect from", help_text = "This should be an absolute path, excluding the domain name. Example: “/events/search/”.")]
    pub old_path: String,
    #[field(max_length = 200, blank, verbose_name = "redirect to", help_text = "This can be either an absolute path (as above) or a full URL starting with a scheme such as “https://”.")]
    pub new_path: String,
}

/// `RedirectFallbackMiddleware`: turn 404s into the configured redirect (301) or 410.
#[derive(Default)]
pub struct RedirectFallbackMiddleware;

#[async_trait]
impl Middleware for RedirectFallbackMiddleware {
    async fn process_response(&self, request: &HttpRequest, response: HttpResponse) -> Result<HttpResponse> {
        if response.status_code != 404 {
            return Ok(response);
        }
        let db = Db::default_db();
        let site = crate::sites::get_current(&db, Some(request)).await?;
        let full = request.get_full_path();
        let mut candidates = vec![full.clone()];
        if settings().append_slash && !request.path().ends_with('/') {
            candidates.push(full.replacen(request.path(), &format!("{}/", request.path()), 1));
        }
        for old in candidates {
            match Redirect::objects().filter_kw("site", site.id).filter_kw("old_path", old).get(&db).await {
                Ok(r) if r.new_path.is_empty() => return Ok(HttpResponse::gone("")),
                Ok(r) => return HttpResponse::permanent_redirect(&r.new_path),
                Err(Error::ObjectDoesNotExist { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(response)
    }
}

rango_middleware::register_middleware!("rango.contrib.redirects.middleware.RedirectFallbackMiddleware", || Ok(Arc::new(RedirectFallbackMiddleware)));
rango_core::register_app!(rango_core::apps::AppConfig::new("rango.contrib.redirects"));
rango_db::register_initial_migration!("redirects", [("sites", "0001_initial")]);
