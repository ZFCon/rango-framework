//! The request cycle shared by every API view.

use std::future::Future;

use rango_contrib::auth::CurrentUser;
use rango_core::exceptions::Result;
use rango_http::{HttpRequest, HttpResponse};
use serde_json::{Value as Json, json};

use crate::auth::{Authentication, Permission, Throttle};
use crate::exceptions::ApiException;

/// `rango::rest::Request`.
#[derive(Clone)]
pub struct ApiRequest {
    pub http: HttpRequest,
    /// `request.data`: the parsed body (JSON, form or multipart).
    pub data: Json,
    /// `request.user`.
    pub user: CurrentUser,
    /// `request.auth` (the token for token authentication).
    pub auth: Option<String>,
}

impl ApiRequest {
    /// `request.query_params.get(name)`.
    pub fn query(&self, name: &str) -> Option<&str> {
        self.http.query().get(name)
    }
}

/// `rango::rest::Response`.
#[derive(Debug, Clone)]
pub struct Response {
    /// `None` renders an empty body (204).
    pub data: Option<Json>,
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl Response {
    pub fn new(data: Json) -> Self {
        Response { data: Some(data), status: 200, headers: vec![] }
    }
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
    pub fn created(data: Json) -> Self {
        Response::new(data).status(201)
    }
    pub fn no_content() -> Self {
        Response { data: None, status: 204, headers: vec![] }
    }
}

/// Per-view policy (`authentication_classes`, `permission_classes`, `throttle_classes`); `None` = the `REST_FRAMEWORK` default.
#[derive(Clone, Default)]
pub struct ViewConfig {
    pub authentication: Option<Vec<Authentication>>,
    pub permissions: Option<Vec<Permission>>,
    pub throttles: Option<Vec<Throttle>>,
}

impl ViewConfig {
    pub fn authenticators(&self) -> Vec<Authentication> {
        self.authentication.clone().unwrap_or_else(crate::settings::default_authentication)
    }
    pub fn permission_list(&self) -> Vec<Permission> {
        self.permissions.clone().unwrap_or_else(crate::settings::default_permissions)
    }
    pub fn throttle_list(&self) -> Vec<Throttle> {
        self.throttles.clone().unwrap_or_else(crate::settings::default_throttles)
    }
}

/// Parse the request body by content type (`JSONParser`, `FormParser`, `MultiPartParser`).
pub async fn parse_body(http: &HttpRequest) -> Result<Json> {
    let body = http.body()?;
    match http.content_type() {
        "application/json" => {
            if body.is_empty() {
                return Ok(json!({}));
            }
            serde_json::from_slice(body).map_err(|e| ApiException::parse_error(&format!("JSON parse error - {e}")).into())
        }
        "application/x-www-form-urlencoded" | "multipart/form-data" => {
            let post = http.post().await?;
            Ok(Json::Object(post.lists().map(|(k, v)| (k.to_owned(), if v.len() == 1 { Json::String(v[0].clone()) } else { Json::Array(v.iter().cloned().map(Json::String).collect()) })).collect()))
        }
        _ if body.is_empty() => Ok(json!({})),
        other => Err(ApiException::unsupported_media_type(other).into()),
    }
}

/// `JSONRenderer`: compact, UTF-8.
pub fn render(response: Response, allow: &[&str]) -> HttpResponse {
    let mut r = match &response.data {
        Some(d) => HttpResponse::new(rango_core::utils::json::dumps_with(d, true, false).unwrap_or_default()).content_type("application/json"),
        None => {
            let mut r = HttpResponse::new("");
            r.headers.remove("Content-Type");
            r
        }
    };
    let _ = r.set_status_code(response.status);
    let _ = r.set_header("Allow", &allow.join(", "));
    let _ = r.set_header("Vary", "Accept");
    for (k, v) in &response.headers {
        let _ = r.set_header(k, v);
    }
    r
}

/// The REST framework request cycle: authenticate, check permissions and throttles, parse the body, run
/// `handler`, turn errors into API responses and render JSON.
///
/// `allowed` lists the methods the view implements; `name` is used by the `OPTIONS` metadata.
pub async fn dispatch<F, Fut>(config: &ViewConfig, http: HttpRequest, allowed: &[&str], name: &str, handler: F) -> HttpResponse
where
    F: FnOnce(ApiRequest) -> Fut,
    Fut: Future<Output = Result<Response>>,
{
    let mut allow: Vec<&str> = allowed.to_vec();
    if allow.contains(&"GET") {
        allow.push("HEAD");
    }
    allow.push("OPTIONS");
    let auths = config.authenticators();
    let method = http.method().to_owned();
    let result: Result<Response> = async {
        // perform_authentication
        let (mut user, mut auth) = (CurrentUser::default(), None);
        for a in &auths {
            if let Some((u, t)) = a.authenticate(&http).await? {
                (user, auth) = (u, t);
                break;
            }
        }
        let mut req = ApiRequest { http: http.clone(), data: Json::Null, user, auth };
        // check_permissions
        for p in config.permission_list() {
            if !p.has_permission(&req).await {
                return Err(if req.user.is_authenticated() { ApiException::permission_denied(None) } else { ApiException::not_authenticated() }.into());
            }
        }
        // check_throttles
        for t in config.throttle_list() {
            if let Err(wait) = t.check(&req).await? {
                return Err(ApiException::throttled(Some(wait)).into());
            }
        }
        if method == "OPTIONS" {
            return Ok(Response::new(json!({"name": name, "description": "", "renders": ["application/json"], "parses": ["application/json", "application/x-www-form-urlencoded", "multipart/form-data"]})));
        }
        if !allow.contains(&method.as_str()) {
            return Err(ApiException::method_not_allowed(&method).into());
        }
        if matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
            req.data = parse_body(&http).await?;
        }
        handler(req).await
    }
    .await;
    let response = match result {
        Ok(r) => r,
        Err(e) => match ApiException::from_error(&e) {
            Some(mut api) => {
                // 401 needs a challenge; without one (session auth first) the REST framework answers 403.
                if api.status == 401 {
                    match auths.first().and_then(Authentication::authenticate_header) {
                        Some(h) => api.headers.push(("WWW-Authenticate", h)),
                        None => api.status = 403,
                    }
                }
                Response { data: Some(api.body), status: api.status, headers: api.headers.into_iter().map(|(k, v)| (k.to_owned(), v)).collect() }
            }
            None => {
                // Unhandled: let the framework's 500 machinery deal with it.
                return rango_middleware::handlers::response_for_exception(&http, &e).await;
            }
        },
    };
    let mut out = render(response, &allow);
    if method == "HEAD" {
        out.set_content("");
    }
    out
}

/// `@api_view(["GET", "POST"])`: wrap an async function as an API view.
pub fn api_view<F, Fut>(methods: &[&'static str], config: ViewConfig, handler: F) -> rango_urls::View
where
    F: Fn(ApiRequest, rango_urls::ViewArgs) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Response>> + Send + 'static,
{
    let methods: Vec<&'static str> = methods.to_vec();
    rango_urls::View::new("rango.rest.api_view", move |http, args| {
        let (config, methods, handler) = (config.clone(), methods.clone(), handler.clone());
        Box::pin(async move { Ok(dispatch(&config, http, &methods, "Api", move |req| handler(req, args)).await) })
    })
    .with_flags(|f| f.csrf_exempt = true)
}
