//! `contrib.auth.views` (login, logout, password change).

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_db::Db;
use rango_forms::Form;
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, TemplateResponse};
use rango_urls::ViewArgs;
use rango_views::generic::View;

use super::forms::{authenticate_form, authentication_form, password_change_form, post_data, save_password_change};
use super::{REDIRECT_FIELD_NAME, RequestUserExt, login, logout, redirect_to_login, update_session_auth_hash};

fn next_url(req: &HttpRequest, field: &str, post: Option<&rango_http::QueryDict>) -> String {
    let raw = post.and_then(|p| p.get(field).map(str::to_owned)).or_else(|| req.query().get(field).map(str::to_owned)).unwrap_or_default();
    // `url_has_allowed_host_and_scheme`
    let host = req.get_host().unwrap_or_default();
    if !raw.is_empty() && rango_core::utils::http::url_has_allowed_host_and_scheme(&raw, &std::collections::HashSet::from([host]), req.is_secure()) { raw } else { String::new() }
}

/// `LoginView`.
pub struct LoginView {
    pub template_name: String,
    pub redirect_field_name: String,
}

impl Default for LoginView {
    fn default() -> Self {
        Self { template_name: "registration/login.html".into(), redirect_field_name: REDIRECT_FIELD_NAME.into() }
    }
}

impl LoginView {
    fn render(&self, req: &HttpRequest, form: &Form) -> HttpResponse {
        let mut ctx = Context::new();
        ctx.set("form", form.to_template());
        ctx.set(self.redirect_field_name.as_str(), next_url(req, &self.redirect_field_name, None));
        TemplateResponse::new(req, self.template_name.as_str(), ctx)
    }
}

#[async_trait]
impl View for LoginView {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "OPTIONS"]
    }
    async fn get(&self, req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        Ok(self.render(&req, &authentication_form()))
    }
    async fn post(&self, req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::default_db();
        let mut form = authentication_form().bound(post_data(&req).await?, Default::default());
        match authenticate_form(&db, &mut form).await? {
            Some((user, backend)) => {
                login(&req, &db, user, Some(&backend)).await?;
                let next = next_url(&req, &self.redirect_field_name, Some(req.post().await?));
                HttpResponse::redirect(&if next.is_empty() { settings().login_redirect_url.clone() } else { next })
            }
            None => Ok(self.render(&req, &form)),
        }
    }
}

/// `LogoutView` (POST logs out; GET is not allowed since the reference).
#[derive(Default)]
pub struct LogoutView {
    pub template_name: Option<String>,
}

#[async_trait]
impl View for LogoutView {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["POST", "OPTIONS"]
    }
    async fn post(&self, req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        let next = next_url(&req, REDIRECT_FIELD_NAME, Some(req.post().await?));
        logout(&req).await?;
        if !next.is_empty() {
            return HttpResponse::redirect(&next);
        }
        if let Some(url) = &settings().logout_redirect_url {
            return HttpResponse::redirect(url);
        }
        Ok(TemplateResponse::new(&req, self.template_name.clone().unwrap_or_else(|| "registration/logged_out.html".into()), Context::new()))
    }
}

/// `PasswordChangeView`: needs a logged-in user; redirects to `success_url` afterwards.
pub struct PasswordChangeView {
    pub template_name: String,
    pub success_url: String,
}

impl Default for PasswordChangeView {
    fn default() -> Self {
        Self { template_name: "registration/password_change_form.html".into(), success_url: "/password_change/done/".into() }
    }
}

#[async_trait]
impl View for PasswordChangeView {
    fn allowed_methods(&self) -> Vec<&'static str> {
        vec!["GET", "HEAD", "POST", "OPTIONS"]
    }
    async fn get(&self, req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        if req.user().is_anonymous() {
            return redirect_to_login(&req.get_full_path(), None, REDIRECT_FIELD_NAME);
        }
        let mut ctx = Context::new();
        ctx.set("form", password_change_form().to_template());
        Ok(TemplateResponse::new(&req, self.template_name.as_str(), ctx))
    }
    async fn post(&self, req: HttpRequest, _args: &ViewArgs) -> Result<HttpResponse> {
        let Some(user) = req.user().0 else { return redirect_to_login(&req.get_full_path(), None, REDIRECT_FIELD_NAME) };
        let _ = req.post().await?;
        let db = Db::default_db();
        let mut form = password_change_form().bound(post_data(&req).await?, Default::default());
        let saved = save_password_change(&db, &mut form, &mut **user.lock().await).await?;
        if saved {
            update_session_auth_hash(&req).await?;
            return HttpResponse::redirect(&self.success_url);
        }
        let mut ctx = Context::new();
        ctx.set("form", form.to_template());
        Ok(TemplateResponse::new(&req, self.template_name.as_str(), ctx))
    }
}
