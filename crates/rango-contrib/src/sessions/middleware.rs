//! `contrib.sessions.middleware.SessionMiddleware`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_http::cache_utils::patch_vary_headers;
use rango_http::{CookieOptions, HttpRequest, HttpResponse};
use rango_middleware::handlers::Middleware;

use super::backends::backend_from_settings;
use super::Session;

#[derive(Default)]
pub struct SessionMiddleware;

#[async_trait]
impl Middleware for SessionMiddleware {
    async fn process_request(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        let s = settings();
        let key = request.cookies().get(&s.session_cookie_name).cloned();
        let session = Session::new(backend_from_settings()?, key);
        session.load().await?;
        request.set_ext(session);
        Ok(None)
    }

    async fn process_response(&self, request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        let Some(session) = request.ext::<Session>() else { return Ok(response) };
        let s = settings();
        let name = &s.session_cookie_name;
        let accessed = session.accessed();
        let modified = session.modified();
        let empty = session.is_empty();
        if request.cookies().contains_key(name) && empty {
            response.delete_cookie(name, &s.session_cookie_path, s.session_cookie_domain.as_deref(), Some(&s.session_cookie_samesite))?;
            patch_vary_headers(&mut response, &["Cookie"]);
            return Ok(response);
        }
        if accessed {
            patch_vary_headers(&mut response, &["Cookie"]);
        }
        if (modified || s.session_save_every_request) && !empty {
            let (max_age, expires) = if session.get_expire_at_browser_close() {
                (None, None)
            } else {
                (Some(session.get_expiry_age()), Some(session.get_expiry_date().timestamp()))
            };
            if response.status_code != 500 {
                if let Err(e) = session.save().await {
                    tracing::warn!(target: "rango.security.SessionInterrupted", "The request's session was deleted before the request completed: {e}");
                    return Ok(HttpResponse::bad_request(""));
                }
                let opts = CookieOptions {
                    max_age,
                    expires,
                    path: s.session_cookie_path.clone(),
                    domain: s.session_cookie_domain.clone(),
                    secure: s.session_cookie_secure,
                    httponly: s.session_cookie_httponly,
                    samesite: if s.session_cookie_samesite.is_empty() { None } else { Some(s.session_cookie_samesite.clone()) },
                };
                response.set_cookie(name, &session.session_key().unwrap_or_default(), &opts)?;
            }
        }
        Ok(response)
    }
}

rango_middleware::register_middleware!("rango.contrib.sessions.middleware.SessionMiddleware", || Ok(Arc::new(SessionMiddleware)));
