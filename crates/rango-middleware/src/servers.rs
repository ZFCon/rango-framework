//! Development server (`core.servers.basehttp`) on hyper.

use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use rango_core::exceptions::Error;
use rango_http::{Body, HttpRequest};
use tokio::net::TcpListener;

use crate::handlers::Handler;

type RespBody = UnsyncBoxBody<Bytes, io::Error>;

/// `request.META`-style construction from a hyper request.
pub async fn request_from_hyper(req: hyper::Request<Incoming>, local: SocketAddr, remote: SocketAddr, secure: bool) -> Result<HttpRequest, Error> {
    let (parts, body) = req.into_parts();
    let bytes = body.collect().await.map_err(|e| Error::RequestAborted(e.to_string()))?.to_bytes();
    let uri = parts.uri.path_and_query().map_or("/", |p| p.as_str()).to_owned();
    let mut b = HttpRequest::builder(parts.method.as_str(), &uri);
    b.body(bytes).remote_addr(&remote.ip().to_string()).server(&local.ip().to_string(), local.port());
    if secure {
        b.scheme("https");
    }
    for (name, value) in &parts.headers {
        b.header(name.as_str(), &String::from_utf8_lossy(value.as_bytes()));
    }
    b.build()
}

/// Convert a Rango response into a hyper response (cookies become `Set-Cookie` headers).
pub fn response_to_hyper(response: rango_http::HttpResponse) -> hyper::Response<RespBody> {
    let (status, _reason, headers, cookies, body, closers) = response.into_body();
    let mut builder = hyper::Response::builder().status(status);
    for (k, v) in headers.iter() {
        builder = builder.header(k, v);
    }
    for (k, m) in &cookies {
        builder = builder.header("Set-Cookie", m.to_header(k));
    }
    let body: RespBody = match body {
        Body::Bytes(b) => Full::new(b).map_err(|never: Infallible| match never {}).boxed_unsync(),
        Body::Stream(s) => StreamBody::new(s.map_ok(Frame::data)).boxed_unsync(),
    };
    for c in closers {
        c();
    }
    builder.body(body).unwrap_or_else(|_| hyper::Response::new(Full::new(Bytes::new()).map_err(|n: Infallible| match n {}).boxed_unsync()))
}

fn log_line(remote: &SocketAddr, method: &str, target: &str, version: hyper::Version, status: u16, size: Option<&str>) {
    let ts = chrono::Local::now().format("%d/%b/%Y %H:%M:%S");
    eprintln!("[{ts}] \"{method} {target} {version:?}\" {status} {}", size.unwrap_or("-"));
    let _ = remote;
}

/// Serve `handler` on `listener` until Ctrl-C.
pub async fn serve(handler: Arc<Handler>, listener: TcpListener, log_requests: bool) -> io::Result<()> {
    let local = listener.local_addr()?;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        let (stream, remote) = tokio::select! {
            r = listener.accept() => r?,
            _ = &mut shutdown => return Ok(()),
        };
        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            let svc = service_fn(move |req: hyper::Request<Incoming>| {
                let handler = Arc::clone(&handler);
                async move {
                    let (method, target, version) = (req.method().to_string(), req.uri().to_string(), req.version());
                    let response = match request_from_hyper(req, local, remote, false).await {
                        Ok(request) => handler.get_response(request).await,
                        Err(e) => {
                            let mut r = rango_http::HttpResponse::bad_request(e.to_string());
                            r.status_code = 400;
                            r
                        }
                    };
                    let size = response.header("Content-Length").map(str::to_owned);
                    let status = response.status_code;
                    if log_requests {
                        log_line(&remote, &method, &target, version, status, size.as_deref());
                    }
                    Ok::<_, Infallible>(response_to_hyper(response))
                }
            });
            let _ = auto::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(stream), svc).await;
        });
    }
}
