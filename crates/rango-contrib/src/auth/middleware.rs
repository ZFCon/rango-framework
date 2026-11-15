//! `contrib.auth.middleware`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::exceptions::Result;
use rango_db::Db;
use rango_http::{HttpRequest, HttpResponse};
use rango_middleware::handlers::Middleware;

use super::{CurrentUser, RequestUserExt, get_user, redirect_to_login};
use crate::sessions::RequestSessionExt;

/// `AuthenticationMiddleware`: sets `request.user` (needs `SessionMiddleware` before it).
#[derive(Default)]
pub struct AuthenticationMiddleware;

#[async_trait]
impl Middleware for AuthenticationMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        if request.try_session().is_none() {
            return Err(rango_core::Error::improperly_configured("The Rango authentication middleware requires session middleware to be installed. Edit your MIDDLEWARE setting to insert 'rango.contrib.sessions.middleware.SessionMiddleware' before 'rango.contrib.auth.middleware.AuthenticationMiddleware'."));
        }
        let user = get_user(request, &Db::default_db()).await?;
        request.set_user(user);
        Ok(None)
    }
}

rango_middleware::register_middleware!("rango.contrib.auth.middleware.AuthenticationMiddleware", || Ok(Arc::new(AuthenticationMiddleware)));

/// `LoginRequiredMiddleware`: every view requires login unless marked `login_required_exempt`.
#[derive(Default)]
pub struct LoginRequiredMiddleware;

#[async_trait]
impl Middleware for LoginRequiredMiddleware {
    async fn process_view(&self, request: &HttpRequest, view: &rango_urls::View, _args: &rango_urls::ViewArgs) -> Result<Option<HttpResponse>> {
        if view.flags.login_required_exempt || request.user().is_authenticated() {
            return Ok(None);
        }
        Ok(Some(redirect_to_login(&request.get_full_path(), None, super::REDIRECT_FIELD_NAME)?))
    }
}

rango_middleware::register_middleware!("rango.contrib.auth.middleware.LoginRequiredMiddleware", || Ok(Arc::new(LoginRequiredMiddleware)));

#[allow(dead_code)]
fn _anon() -> CurrentUser {
    CurrentUser::default()
}
