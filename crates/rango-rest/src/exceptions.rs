//! API exceptions and the default exception handler.

use rango_core::exceptions::Error;
use serde_json::{Value as Json, json};

/// `APIException`: an error that becomes an API response.
#[derive(Debug, Clone)]
pub struct ApiException {
    pub status: u16,
    /// `{"detail": ...}` for most errors; field errors for validation.
    pub body: Json,
    pub code: &'static str,
    /// `WWW-Authenticate` / `Retry-After`.
    pub headers: Vec<(&'static str, String)>,
}

impl std::fmt::Display for ApiException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.body)
    }
}

impl std::error::Error for ApiException {}

impl From<ApiException> for Error {
    fn from(e: ApiException) -> Self {
        Error::Other(Box::new(e))
    }
}

fn detail(status: u16, code: &'static str, msg: impl Into<String>) -> ApiException {
    ApiException { status, body: json!({"detail": msg.into()}), code, headers: vec![] }
}

impl ApiException {
    /// `ValidationError(errors)`: 400 with the errors as the body.
    pub fn validation(errors: Json) -> Self {
        ApiException { status: 400, body: errors, code: "invalid", headers: vec![] }
    }
    pub fn parse_error(msg: &str) -> Self {
        detail(400, "parse_error", msg)
    }
    pub fn authentication_failed(msg: &str) -> Self {
        detail(401, "authentication_failed", msg)
    }
    pub fn not_authenticated() -> Self {
        detail(401, "not_authenticated", "Authentication credentials were not provided.")
    }
    pub fn permission_denied(msg: Option<&str>) -> Self {
        detail(403, "permission_denied", msg.unwrap_or("You do not have permission to perform this action."))
    }
    pub fn not_found(msg: Option<&str>) -> Self {
        detail(404, "not_found", msg.unwrap_or("Not found."))
    }
    pub fn method_not_allowed(method: &str) -> Self {
        detail(405, "method_not_allowed", format!("Method \"{method}\" not allowed."))
    }
    pub fn not_acceptable() -> Self {
        detail(406, "not_acceptable", "Could not satisfy the request Accept header.")
    }
    pub fn unsupported_media_type(media_type: &str) -> Self {
        detail(415, "unsupported_media_type", format!("Unsupported media type \"{media_type}\" in request."))
    }
    pub fn throttled(wait: Option<u64>) -> Self {
        let mut e = detail(
            429,
            "throttled",
            match wait {
                Some(1) => "Request was throttled. Expected available in 1 second.".to_owned(),
                Some(w) => format!("Request was throttled. Expected available in {w} seconds."),
                None => "Request was throttled.".to_owned(),
            },
        );
        if let Some(w) = wait {
            e.headers.push(("Retry-After", w.to_string()));
        }
        e
    }

    /// `exception_handler(exc)`: map any error to an API error (`None` = unhandled, a 500).
    pub fn from_error(e: &Error) -> Option<ApiException> {
        match e {
            Error::Other(b) => b.downcast_ref::<ApiException>().cloned(),
            Error::Http404(m) => Some(Self::not_found((!m.is_empty()).then_some(m.as_str()))),
            Error::ObjectDoesNotExist { .. } => Some(Self::not_found(None)),
            Error::PermissionDenied(m) => Some(Self::permission_denied((!m.is_empty()).then_some(m.as_str()))),
            _ => None,
        }
    }
}
