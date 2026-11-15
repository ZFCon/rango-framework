//! `middleware.clickjacking.XFrameOptionsMiddleware`.

use std::sync::Arc;

use async_trait::async_trait;
use rango_core::conf::settings;
use rango_core::exceptions::Result;
use rango_http::{HttpRequest, HttpResponse};

use crate::handlers::Middleware;

#[derive(Default)]
pub struct XFrameOptionsMiddleware;

#[async_trait]
impl Middleware for XFrameOptionsMiddleware {
    async fn process_response(&self, _request: &HttpRequest, mut response: HttpResponse) -> Result<HttpResponse> {
        if response.header("X-Frame-Options").is_some() || response.flags.contains("xframe_options_exempt") {
            return Ok(response);
        }
        response.set_header("X-Frame-Options", &settings().x_frame_options.to_uppercase())?;
        Ok(response)
    }
}

crate::register_middleware!("rango.middleware.clickjacking.XFrameOptionsMiddleware", || Ok(Arc::new(XFrameOptionsMiddleware)));
