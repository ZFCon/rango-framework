//! The `REST_FRAMEWORK` setting with the defaults.

use rango_core::conf::settings;
use serde_json::Value as Json;

use crate::auth::{Authentication, Permission, Throttle};
use crate::pagination::Pagination;

fn get(key: &str) -> Option<Json> {
    settings().rest_framework.get(key).cloned()
}

fn names(key: &str) -> Option<Vec<String>> {
    get(key).and_then(|v| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.rsplit('.').next().unwrap_or(s).to_owned())).collect()))
}

/// `DEFAULT_AUTHENTICATION_CLASSES` (Session + Basic unless configured).
pub fn default_authentication() -> Vec<Authentication> {
    match names("DEFAULT_AUTHENTICATION_CLASSES") {
        Some(n) => n.iter().filter_map(|c| Authentication::from_name(c)).collect(),
        None => vec![Authentication::Session, Authentication::Basic],
    }
}

/// `DEFAULT_PERMISSION_CLASSES` (`AllowAny` unless configured).
pub fn default_permissions() -> Vec<Permission> {
    match names("DEFAULT_PERMISSION_CLASSES") {
        Some(n) => n.iter().filter_map(|c| Permission::from_name(c)).collect(),
        None => vec![Permission::AllowAny],
    }
}

/// `DEFAULT_THROTTLE_CLASSES` with rates from `DEFAULT_THROTTLE_RATES`.
pub fn default_throttles() -> Vec<Throttle> {
    names("DEFAULT_THROTTLE_CLASSES").unwrap_or_default().iter().filter_map(|c| Throttle::from_name(c)).collect()
}

/// `DEFAULT_THROTTLE_RATES[scope]`, e.g. `"100/day"`.
pub fn throttle_rate(scope: &str) -> Option<String> {
    get("DEFAULT_THROTTLE_RATES").and_then(|r| r.get(scope).and_then(|v| v.as_str().map(str::to_owned)))
}

/// `DEFAULT_PAGINATION_CLASS` + `PAGE_SIZE`.
pub fn default_pagination() -> Option<Pagination> {
    let class = get("DEFAULT_PAGINATION_CLASS")?.as_str()?.rsplit('.').next()?.to_owned();
    let size = get("PAGE_SIZE").and_then(|v| v.as_u64()).map(|n| n as usize);
    match class.as_str() {
        "PageNumberPagination" => size.map(Pagination::page_number),
        "LimitOffsetPagination" => size.map(Pagination::limit_offset),
        _ => None,
    }
}

pub fn search_param() -> String {
    get("SEARCH_PARAM").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "search".into())
}

pub fn ordering_param() -> String {
    get("ORDERING_PARAM").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "ordering".into())
}

pub fn non_field_errors_key() -> String {
    get("NON_FIELD_ERRORS_KEY").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "non_field_errors".into())
}
