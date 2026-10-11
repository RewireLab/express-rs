//! HTTP server foundation.
//!
//! Provides the lowest-level framework foundation: TCP listener, HTTP/1.1
//! connection handling, request parsing, response writing, and keep-alive.
//! Built on top of hyper for HTTP protocol handling.

use crate::error::Error;
use crate::router::Router;
use crate::{Request, Response};
use http::Request as HyperRequest;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

/// The HTTP server.
///
/// Owns the TCP listener and spawns connection tasks. Each connection is
/// handled by hyper's HTTP/1.1 implementation, which provides keep-alive,
/// pipelining, and proper connection management.
pub struct Server {
    router: Option<Router>,
    /// Express stamps `X-Powered-By: Express` on every response by
    /// default; `app.disable("x-powered-by")` turns it off here.
    x_powered_by: bool,
}

impl Server {
    /// Create a new server with no router.
    pub fn new() -> Self {
        Self {
            router: None,
            x_powered_by: true,
        }
    }

    /// Set the request router.
    pub fn set_router(&mut self, router: Router) {
        self.router = Some(router);
    }

    /// Set whether responses carry `X-Powered-By: Express`.
    pub fn set_x_powered_by(&mut self, enabled: bool) {
        self.x_powered_by = enabled;
    }

    /// Start the server and listen for connections.
    ///
    /// This method runs indefinitely, accepting connections and spawning
    /// tasks to handle them. It returns only if the server fails to start.
    pub async fn listen(self, port: u16) -> Result<(), Error> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr).await?;

        let router = self.router.ok_or_else(|| Error::new("No router set"))?;
        let x_powered_by = self.x_powered_by;

        let router = Arc::new(router);

        loop {
            let (stream, _) = listener.accept().await?;
            let router = router.clone();

            tokio::spawn(async move {
                let io = TokioIo::new(stream);

                let service = service_fn(move |req: HyperRequest<Incoming>| {
                    let router = router.clone();
                    async move {
                        let (parts, body) = req.into_parts();
                        let collected = body.collect().await.unwrap_or_default();
                        let body_bytes = collected.to_bytes();

                        let request = Arc::new(Request::new(
                            parts.method,
                            parts.uri,
                            parts.headers,
                            body_bytes,
                        ));
                        let response = Response::new().with_accept(request.header("accept"));

                        let mut response = router.handle(request.clone(), response).await;
                        response.finish(&request);
                        // Applied after finalisation so error responses
                        // carry it too, and never over a handler's own
                        // value.
                        if x_powered_by && response.get("x-powered-by").is_none() {
                            response.set("x-powered-by", "Express");
                        }
                        let hyper_response = response.into_hyper_response();
                        Ok::<_, Infallible>(hyper_response)
                    }
                });

                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
