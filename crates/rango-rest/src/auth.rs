//! Authentication, permissions and throttling.

use std::sync::Arc;

use base64::Engine as _;
use rango_contrib::auth::{self, Credentials, CurrentUser, RequestUserExt};
use rango_core::cache::{Timeout, cache};
use rango_core::exceptions::Result;
use rango_db::models::Model;
use rango_db::Db;
use rango_http::HttpRequest;
use rango_middleware::handlers::Middleware;

use crate::exceptions::ApiException;
use crate::views::ApiRequest;

fn user_of(u: Box<dyn auth::AbstractBaseUser>) -> CurrentUser {
    CurrentUser(Some(Arc::new(tokio::sync::Mutex::new(u))))
}

/// A custom authentication scheme.
#[async_trait::async_trait]
pub trait Authenticator: Send + Sync {
    /// `Ok(None)` = not attempted; `Err` = attempted and failed.
    async fn authenticate(&self, request: &HttpRequest) -> Result<Option<(CurrentUser, Option<String>)>>;
    /// Value of `WWW-Authenticate` for 401 responses (`None` turns them into 403).
    fn authenticate_header(&self) -> Option<String> {
        None
    }
    /// `(scheme name, OpenAPI security scheme object)`.
    fn openapi(&self) -> Option<(String, serde_json::Value)> {
        None
    }
}

/// `authentication_classes` entries.
#[derive(Clone)]
pub enum Authentication {
    /// `SessionAuthentication`: the user logged in through the session (CSRF is enforced on unsafe methods).
    Session,
    /// `BasicAuthentication`.
    Basic,
    /// `TokenAuthentication` (`Authorization: Token <key>`, see [`crate::authtoken`]).
    Token,
    Custom(Arc<dyn Authenticator>),
}

impl Authentication {
    pub fn from_name(class: &str) -> Option<Self> {
        match class {
            "SessionAuthentication" => Some(Self::Session),
            "BasicAuthentication" => Some(Self::Basic),
            "TokenAuthentication" => Some(Self::Token),
            _ => None,
        }
    }

    pub fn authenticate_header(&self) -> Option<String> {
        match self {
            Self::Session => None,
            Self::Basic => Some("Basic realm=\"api\"".into()),
            Self::Token => Some("Token".into()),
            Self::Custom(c) => c.authenticate_header(),
        }
    }

    pub async fn authenticate(&self, request: &HttpRequest) -> Result<Option<(CurrentUser, Option<String>)>> {
        let db = Db::default_db();
        let header = request.meta().get("HTTP_AUTHORIZATION").cloned().unwrap_or_default();
        let mut parts = header.split_whitespace();
        let scheme = parts.next().unwrap_or("").to_lowercase();
        let rest: Vec<&str> = parts.collect();
        match self {
            Self::Custom(c) => c.authenticate(request).await,
            Self::Session => {
                let user = request.user();
                if !user.is_authenticated() || !user.snapshot(|u| u.is_active()).await.unwrap_or(false) {
                    return Ok(None);
                }
                if !matches!(request.method(), "GET" | "HEAD" | "OPTIONS" | "TRACE") {
                    // `enforce_csrf`: API views are csrf-exempt for the middleware, session logins are checked here.
                    let mw = rango_middleware::middleware::csrf::CsrfViewMiddleware { never_reject: false };
                    let probe = rango_urls::View::new("rest.csrf", |_, _| Box::pin(async { Ok(rango_http::HttpResponse::new("")) }));
                    if let Some(resp) = mw.process_view(request, &probe, &rango_urls::ViewArgs::default()).await? {
                        let reason = String::from_utf8_lossy(resp.content()?).into_owned();
                        let short = rango_views_reason(&reason);
                        return Err(ApiException::permission_denied(Some(&format!("CSRF Failed: {short}"))).into());
                    }
                }
                Ok(Some((user, None)))
            }
            Self::Basic => {
                if scheme != "basic" {
                    return Ok(None);
                }
                let fail = |m: &str| ApiException::authentication_failed(m);
                if rest.is_empty() {
                    return Err(fail("Invalid basic header. No credentials provided.").into());
                }
                if rest.len() > 1 {
                    return Err(fail("Invalid basic header. Credentials string should not contain spaces.").into());
                }
                let decoded = base64::engine::general_purpose::STANDARD.decode(rest[0]).ok().and_then(|b| String::from_utf8(b).ok()).ok_or_else(|| fail("Invalid basic header. Credentials not correctly base64 encoded."))?;
                let (userid, password) = decoded.split_once(':').ok_or_else(|| fail("Invalid basic header. Credentials not correctly base64 encoded."))?;
                match auth::authenticate(&db, &Credentials::new(userid, password)).await? {
                    None => Err(fail("Invalid username/password.").into()),
                    Some((u, _)) if !u.is_active() => Err(fail("User inactive or deleted.").into()),
                    Some((u, _)) => Ok(Some((user_of(u), None))),
                }
            }
            Self::Token => {
                if scheme != "token" {
                    return Ok(None);
                }
                let fail = |m: &str| ApiException::authentication_failed(m);
                if rest.is_empty() {
                    return Err(fail("Invalid token header. No credentials provided.").into());
                }
                if rest.len() > 1 {
                    return Err(fail("Invalid token header. Token string should not contain spaces.").into());
                }
                let token = match crate::authtoken::Token::objects().filter_kw("key", rest[0]).get(&db).await {
                    Ok(t) => t,
                    Err(rango_core::Error::ObjectDoesNotExist { .. }) => return Err(fail("Invalid token.").into()),
                    Err(e) => return Err(e),
                };
                match auth::get_user_model()?.get_by_pk(&db, &rango_db::Value::Int(token.user_id)).await? {
                    Some(u) if u.is_active() => Ok(Some((user_of(u), Some(token.key)))),
                    _ => Err(fail("User inactive or deleted.").into()),
                }
            }
        }
    }

    /// `(name, security scheme)` for the OpenAPI document (the names).
    pub fn openapi(&self) -> Option<(String, serde_json::Value)> {
        use serde_json::json;
        match self {
            Self::Session => Some(("cookieAuth".into(), json!({"type": "apiKey", "in": "cookie", "name": rango_core::conf::settings().session_cookie_name}))),
            Self::Basic => Some(("basicAuth".into(), json!({"type": "http", "scheme": "basic"}))),
            Self::Token => Some(("tokenAuth".into(), json!({"type": "apiKey", "in": "header", "name": "Authorization", "description": "Token-based authentication with required prefix \"Token\""}))),
            Self::Custom(c) => c.openapi(),
        }
    }
}

/// The CSRF failure page mentions the reason; keep only that line.
fn rango_views_reason(page: &str) -> String {
    use rango_middleware::middleware::csrf as _;
    for r in ["CSRF cookie not set.", "CSRF token missing.", "Referer checking failed - no Referer.", "Referer checking failed"] {
        if page.contains(r) {
            return r.to_owned();
        }
    }
    "CSRF token missing or incorrect.".to_owned()
}

/// `permission_classes` entries.
#[derive(Clone)]
pub enum Permission {
    AllowAny,
    IsAuthenticated,
    IsAdminUser,
    IsAuthenticatedOrReadOnly,
    /// A custom check (`has_permission`).
    Custom(Arc<dyn Fn(&ApiRequest) -> bool + Send + Sync>),
}

impl Permission {
    pub fn from_name(class: &str) -> Option<Self> {
        match class {
            "AllowAny" => Some(Self::AllowAny),
            "IsAuthenticated" => Some(Self::IsAuthenticated),
            "IsAdminUser" => Some(Self::IsAdminUser),
            "IsAuthenticatedOrReadOnly" => Some(Self::IsAuthenticatedOrReadOnly),
            _ => None,
        }
    }

    pub async fn has_permission(&self, request: &ApiRequest) -> bool {
        let safe = matches!(request.http.method(), "GET" | "HEAD" | "OPTIONS");
        match self {
            Self::AllowAny => true,
            Self::IsAuthenticated => request.user.is_authenticated(),
            Self::IsAdminUser => request.user.is_staff().await,
            Self::IsAuthenticatedOrReadOnly => safe || request.user.is_authenticated(),
            Self::Custom(f) => f(request),
        }
    }

    /// Whether anonymous users can pass (decides if the OpenAPI operation lists `{}` as a security option).
    pub fn allows_anonymous(&self, safe_method: bool) -> bool {
        match self {
            Self::AllowAny => true,
            Self::IsAuthenticatedOrReadOnly => safe_method,
            _ => false,
        }
    }
}

/// `throttle_classes` entries: `rate` like `"100/day"` (default from `DEFAULT_THROTTLE_RATES`).
#[derive(Clone, Debug)]
pub enum Throttle {
    /// `AnonRateThrottle` (scope `anon`): unauthenticated requests by IP.
    Anon(Option<String>),
    /// `UserRateThrottle` (scope `user`): by user id, or IP when anonymous.
    User(Option<String>),
}

impl Throttle {
    pub fn from_name(class: &str) -> Option<Self> {
        match class {
            "AnonRateThrottle" => Some(Self::Anon(None)),
            "UserRateThrottle" => Some(Self::User(None)),
            _ => None,
        }
    }

    fn parse_rate(rate: &str) -> Option<(usize, f64)> {
        let (num, period) = rate.split_once('/')?;
        let secs = match period.chars().next()? {
            's' => 1.0,
            'm' => 60.0,
            'h' => 3600.0,
            'd' => 86400.0,
            _ => return None,
        };
        Some((num.trim().parse().ok()?, secs))
    }

    /// `allow_request`: `Err(wait seconds)` when over the limit.
    pub async fn check(&self, request: &ApiRequest) -> Result<std::result::Result<(), u64>> {
        let (scope, rate) = match self {
            Self::Anon(r) => ("anon", r.clone()),
            Self::User(r) => ("user", r.clone()),
        };
        let Some((limit, duration)) = rate.or_else(|| crate::settings::throttle_rate(scope)).as_deref().and_then(Self::parse_rate) else { return Ok(Ok(())) };
        let authed = request.user.is_authenticated();
        if matches!(self, Self::Anon(_)) && authed {
            return Ok(Ok(()));
        }
        let ident = if authed {
            request.user.pk().await.map(|p| format!("{p:?}")).unwrap_or_default()
        } else {
            let m = request.http.meta();
            m.get("HTTP_X_FORWARDED_FOR").map(|x| x.split(',').next().unwrap_or("").trim().to_owned()).or_else(|| m.get("REMOTE_ADDR").cloned()).unwrap_or_default()
        };
        let key = format!("throttle_{scope}_{ident}");
        let c = cache()?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let mut history: Vec<f64> = c.get(&key).await?.unwrap_or_default();
        history.retain(|t| *t > now - duration);
        if history.len() >= limit {
            let wait = history.last().map_or(duration, |oldest| duration - (now - oldest));
            return Ok(Err(wait.ceil().max(1.0) as u64));
        }
        history.insert(0, now);
        c.set(&key, &history, Timeout::Secs(duration)).await?;
        Ok(Ok(()))
    }
}
