//! `views.static`: `serve()` for development use.

use std::path::Path;

use rango_core::exceptions::{Error, Result};
use rango_core::utils::http::{http_date, parse_http_date_safe};
use rango_core::utils::os::{normpath, safe_join};
use rango_http::{FileResponse, HttpRequest, HttpResponse};

/// `was_modified_since(header, mtime)`.
pub fn was_modified_since(header: Option<&str>, mtime: i64) -> bool {
    match header.and_then(|h| h.split(';').next()).and_then(parse_http_date_safe) {
        Some(since) => mtime > since,
        None => true,
    }
}

/// `serve(request, path, document_root, show_indexes)`.
pub async fn serve(request: &HttpRequest, path: &str, document_root: &Path, show_indexes: bool) -> Result<HttpResponse> {
    let norm = normpath(Path::new(path));
    let rel = norm.to_string_lossy().trim_start_matches('/').to_owned();
    let full = safe_join(document_root, &[&rel])?;
    if full.is_dir() {
        if show_indexes {
            return directory_index(&rel, &full);
        }
        return Err(Error::Http404("Directory indexes are not allowed here.".into()));
    }
    let meta = tokio::fs::metadata(&full).await.map_err(|_| Error::Http404(format!("“{}” does not exist", full.display())))?;
    let mtime = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    if !was_modified_since(request.meta().get("HTTP_IF_MODIFIED_SINCE").map(String::as_str), mtime) {
        return Ok(HttpResponse::not_modified());
    }
    let mut response = FileResponse::open(&full, false, None).await.map_err(|e| Error::Other(Box::new(e)))?;
    response.set_header("Last-Modified", &http_date(Some(mtime as f64)))?;
    Ok(response)
}

fn directory_index(path: &str, fullpath: &Path) -> Result<HttpResponse> {
    let mut names: Vec<String> = std::fs::read_dir(fullpath)
        .map_err(|e| Error::Other(Box::new(e)))?
        .filter_map(|e| e.ok())
        .map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() { format!("{n}/") } else { n }
        })
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    if !path.is_empty() {
        names.insert(0, "../".into());
    }
    let items: String = names.iter().map(|n| format!("<li><a href=\"{0}\">{0}</a></li>\n", rango_core::utils::html::escape(n))).collect();
    let title = rango_core::utils::html::escape(&format!("Index of {path}/"));
    Ok(HttpResponse::new(format!(
        "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta http-equiv=\"Content-type\" content=\"text/html; charset=utf-8\">\n    <meta http-equiv=\"Content-Language\" content=\"en-us\">\n    <meta name=\"robots\" content=\"NONE,NOARCHIVE\">\n    <title>{title}</title>\n  </head>\n  <body>\n    <h1>{title}</h1>\n    <ul>\n{items}    </ul>\n  </body>\n</html>\n"
    )))
}
