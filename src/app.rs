//! Application abstraction.
//!
//! The `App` is the main user-facing type. It provides an Express-like
//! API for registering routes and middleware.

use crate::error::Error;
use crate::request::Request;
use crate::response::Response;
use crate::router::{Next, Router};
use crate::server::Server;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

/// The Express-rs application.
///
/// # Example
///
/// ```no_run
/// use express_rs::App;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let mut app = App::new();
///
///     app.get("/", |_req, mut res, _next, _params| async move {
///         res.status(200).send("Hello, World!");
///         res
///     });
///
///     app.listen(3000).await?;
///     Ok(())
/// }
/// ```
pub struct App {
    server: Server,
    router: Router,
}

impl App {
    /// Create a new application.
    pub fn new() -> Self {
        Self {
            server: Server::new(),
            router: Router::new(),
        }
    }

    /// Enable case-sensitive routing.
    pub fn case_sensitive(&mut self) {
        self.router.set_case_sensitive(true);
    }

    /// Enable strict routing (trailing slash matters).
    pub fn strict(&mut self) {
        self.router.set_strict(true);
    }

    /// Register a handler for GET requests.
    pub fn get<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.get(path, handler);
    }

    /// Register a handler for POST requests.
    pub fn post<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.post(path, handler);
    }

    /// Register a handler for PUT requests.
    pub fn put<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.put(path, handler);
    }

    /// Register a handler for PATCH requests.
    pub fn patch<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.patch(path, handler);
    }

    /// Register a handler for DELETE requests.
    pub fn delete<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.delete(path, handler);
    }

    /// Register a handler for OPTIONS requests.
    pub fn options<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.options(path, handler);
    }

    /// Register a handler for HEAD requests.
    pub fn head<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.head(path, handler);
    }

    /// Register a handler for all HTTP methods.
    pub fn all<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.all(path, handler);
    }

    /// Register middleware that matches all paths and methods.
    pub fn r#use<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.r#use(handler);
    }

    /// Register middleware that matches a path prefix and all methods.
    pub fn use_with_path<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router.use_with_path(path, handler);
    }

    /// Start the server on the given port.
    ///
    /// Runs indefinitely until the process is killed.
    pub async fn listen(mut self, port: u16) -> Result<(), Error> {
        self.server.set_router(self.router);
        self.server.listen(port).await
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
