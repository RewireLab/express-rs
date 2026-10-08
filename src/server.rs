//! HTTP server foundation.
//!
//! Provides the lowest-level framework foundation: TCP listener, HTTP/1.1
//! connection handling, request parsing, response writing, and keep-alive.
//! Built on top of hyper for HTTP protocol handling.

use crate::error::Error;
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

/// A request handler function.
///
/// In Phase 1, the handler receives the request and response, sets the
/// response, and returns it. In Phase 6, this will gain a `next()` parameter
/// for middleware support.
pub type Handler = Arc<
    dyn Fn(
            Request,
            Response,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Response> + Send>>
        + Send
        + Sync,
>;

/// The HTTP server.
///
/// Owns the TCP listener and spawns connection tasks. Each connection is
/// handled by hyper's HTTP/1.1 implementation, which provides keep-alive,
/// pipelining, and proper connection management.
pub struct Server {
    handler: Option<Handler>,
}

impl Server {
    /// Create a new server with no handler.
    pub fn new() -> Self {
        Self { handler: None }
    }

    /// Set the request handler.
    ///
    /// In Phase 1, there is a single handler for all requests. In Phase 3,
    /// this will be replaced by a router.
    pub fn set_handler<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Request, Response) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Response> + Send + 'static,
    {
        self.handler = Some(Arc::new(move |req, res| Box::pin(handler(req, res))));
    }

    /// Start the server and listen for connections.
    ///
    /// This method runs indefinitely, accepting connections and spawning
    /// tasks to handle them. It returns only if the server fails to start.
    pub async fn listen(self, port: u16) -> Result<(), Error> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr).await?;

        let handler = self
            .handler
            .ok_or_else(|| Error::Custom("No handler set".to_string()))?;

        loop {
            let (stream, _) = listener.accept().await?;
            let handler = handler.clone();

            tokio::spawn(async move {
                let io = TokioIo::new(stream);

                let service = service_fn(move |req: HyperRequest<Incoming>| {
                    let handler = handler.clone();
                    async move {
                        let (parts, body) = req.into_parts();
                        let collected = body.collect().await.unwrap_or_default();
                        let body_bytes = collected.to_bytes();

                        let request =
                            Request::new(parts.method, parts.uri, parts.headers, body_bytes);
                        let response = Response::new();

                        let response = handler(request, response).await;
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
