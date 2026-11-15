//! `contrib.auth.decorators`: wrap a view so it checks the logged-in user first.

use std::sync::Arc;

use rango_core::exceptions::{Error, Result};
use rango_db::Db;
use rango_http::{HttpRequest, HttpResponse};
use rango_urls::{IntoView, View};

use super::{CurrentUser, RequestUserExt, redirect_to_login};

type Test = Arc<dyn Fn(CurrentUser, Db) -> futures_util::future::BoxFuture<'static, Result<bool>> + Send + Sync>;

/// `user_passes_test(test, login_url, redirect_field_name)(view)`.
pub fn user_passes_test<M>(view: impl IntoView<M>, login_url: Option<&str>, field: &str, test: Test) -> View {
    let (login_url, field) = (login_url.map(str::to_owned), field.to_owned());
    view.into_view().wrap(move |inner, req: HttpRequest, args| {
        let (test, login_url, field) = (Arc::clone(&test), login_url.clone(), field.clone());
        Box::pin(async move {
            if test(req.user(), Db::default_db()).await? {
                return inner.call(req, args).await;
            }
            redirect_to_login(&req.get_full_path(), login_url.as_deref(), &field)
        })
    })
}

/// `@login_required`.
pub fn login_required<M>(view: impl IntoView<M>) -> View {
    user_passes_test(view, None, super::REDIRECT_FIELD_NAME, Arc::new(|u, _| Box::pin(async move { Ok(u.is_authenticated()) })))
}

/// `@permission_required("app.codename", login_url=..., raise_exception=False)`.
pub fn permission_required<M>(view: impl IntoView<M>, perms: &[&str], raise_exception: bool) -> View {
    let perms: Vec<String> = perms.iter().map(|s| (*s).to_owned()).collect();
    let view = view.into_view();
    view.wrap(move |inner, req: HttpRequest, args| {
        let perms = perms.clone();
        Box::pin(async move {
            let user = req.user();
            let refs: Vec<&str> = perms.iter().map(String::as_str).collect();
            if user.has_perms(&Db::default_db(), &refs).await? {
                return inner.call(req, args).await;
            }
            if raise_exception {
                return Err(Error::PermissionDenied(String::new()));
            }
            redirect_to_login(&req.get_full_path(), None, super::REDIRECT_FIELD_NAME)
        })
    })
}

/// Result type helper for tests.
pub type ViewResult = Result<HttpResponse>;
