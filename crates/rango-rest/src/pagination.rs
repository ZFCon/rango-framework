//! `PageNumberPagination` and `LimitOffsetPagination`.

use rango_core::exceptions::Result;
use rango_core::utils::http::{quote_plus, urlsplit};
use rango_http::HttpRequest;
use serde_json::{Value as Json, json};

use crate::exceptions::ApiException;

#[derive(Debug, Clone)]
pub enum Pagination {
    PageNumber { page_size: usize, page_query_param: String, page_size_query_param: Option<String>, max_page_size: Option<usize> },
    LimitOffset { default_limit: usize, max_limit: Option<usize> },
}

/// The slice to fetch and how to build the envelope afterwards.
pub struct Window {
    pub offset: usize,
    pub limit: usize,
    pub next: Option<String>,
    pub previous: Option<String>,
}

/// `replace_query_param` / `remove_query_param`: the request URL with one parameter changed (keys sorted, as the REST framework does).
pub fn replace_query_param(request: &HttpRequest, key: &str, value: Option<String>) -> Result<String> {
    let url = request.build_absolute_uri(None)?;
    let mut parts = urlsplit(&url).map_err(|e| rango_core::Error::Other(e.into()))?;
    let mut pairs: Vec<(String, String)> = request.query().lists().flat_map(|(k, v)| v.iter().map(move |x| (k.to_owned(), x.clone()))).filter(|(k, _)| k != key).collect();
    if let Some(v) = value {
        pairs.push((key.to_owned(), v));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    parts.query = pairs.iter().map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v))).collect::<Vec<_>>().join("&");
    Ok(parts.unsplit())
}

impl Pagination {
    pub fn page_number(page_size: usize) -> Self {
        Self::PageNumber { page_size, page_query_param: "page".into(), page_size_query_param: None, max_page_size: None }
    }
    pub fn limit_offset(default_limit: usize) -> Self {
        Self::LimitOffset { default_limit, max_limit: None }
    }

    /// Work out which rows to fetch for this request given the total `count`.
    pub fn window(&self, request: &HttpRequest, count: usize) -> Result<Window> {
        let q = request.query();
        match self {
            Self::PageNumber { page_size, page_query_param, page_size_query_param, max_page_size } => {
                let mut size = *page_size;
                if let Some(v) = page_size_query_param.as_ref().and_then(|p| q.get(p)).and_then(|v| v.parse::<usize>().ok()).filter(|v| *v > 0) {
                    size = max_page_size.map_or(v, |m| v.min(m));
                }
                let num_pages = count.div_ceil(size).max(1);
                let page = match q.get(page_query_param) {
                    None => 1,
                    Some("last") => num_pages,
                    Some(p) => p.parse::<usize>().ok().filter(|n| *n >= 1 && *n <= num_pages).ok_or_else(|| ApiException::not_found(Some("Invalid page.")))?,
                };
                let next = if page < num_pages { Some(replace_query_param(request, page_query_param, Some((page + 1).to_string()))?) } else { None };
                let previous = match page {
                    1 => None,
                    2 => Some(replace_query_param(request, page_query_param, None)?),
                    n => Some(replace_query_param(request, page_query_param, Some((n - 1).to_string()))?),
                };
                Ok(Window { offset: (page - 1) * size, limit: size, next, previous })
            }
            Self::LimitOffset { default_limit, max_limit } => {
                let limit = q.get("limit").and_then(|v| v.parse::<usize>().ok()).filter(|v| *v > 0).map_or(*default_limit, |v| max_limit.map_or(v, |m| v.min(m)));
                let offset = q.get("offset").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
                let with = |o: usize| -> Result<String> {
                    let _ = o;
                    Ok(String::new())
                };
                let _ = with;
                let next = if offset + limit < count {
                    let u = replace_query_param(request, "limit", Some(limit.to_string()))?;
                    Some(set_param(&u, "offset", Some((offset + limit).to_string())))
                } else {
                    None
                };
                let previous = if offset == 0 {
                    None
                } else {
                    let u = replace_query_param(request, "limit", Some(limit.to_string()))?;
                    Some(set_param(&u, "offset", if offset > limit { Some((offset - limit).to_string()) } else { None }))
                };
                Ok(Window { offset, limit, next, previous })
            }
        }
    }

    /// `get_paginated_response(data)`.
    pub fn envelope(count: usize, w: &Window, results: Vec<Json>) -> Json {
        json!({"count": count, "next": w.next, "previous": w.previous, "results": results})
    }

    /// Query parameters for the OpenAPI document: `(name, type, description)`.
    pub fn schema_parameters(&self) -> Vec<(String, &'static str, &'static str)> {
        match self {
            Self::PageNumber { page_query_param, page_size_query_param, .. } => {
                let mut v = vec![(page_query_param.clone(), "integer", "A page number within the paginated result set.")];
                if let Some(p) = page_size_query_param {
                    v.push((p.clone(), "integer", "Number of results to return per page."));
                }
                v
            }
            Self::LimitOffset { .. } => vec![("limit".into(), "integer", "Number of results to return per page."), ("offset".into(), "integer", "The initial index from which to return the results.")],
        }
    }
}

/// Replace/remove `key` in an absolute URL's query string, keeping keys sorted.
fn set_param(url: &str, key: &str, value: Option<String>) -> String {
    let Ok(mut parts) = urlsplit(url) else { return url.to_owned() };
    let mut pairs: Vec<(String, String)> = parts.query.split('&').filter(|s| !s.is_empty()).filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned()))).filter(|(k, _)| k != key).collect();
    if let Some(v) = value {
        pairs.push((key.to_owned(), v));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    parts.query = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    parts.unsplit()
}
