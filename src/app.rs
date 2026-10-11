//! Application abstraction.
//!
//! The `App` is the main user-facing type. It provides an Express-like
//! API for registering routes, middleware, and error-handling middleware.

use crate::error::Error;
use crate::request::Request;
use crate::response::Response;
use crate::router::{Next, Params, Router};
use crate::server::Server;
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
///         Ok(res)
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
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.get(path, handler);
    }

    /// Register a handler for POST requests.
    pub fn post<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.post(path, handler);
    }

    /// Register a handler for PUT requests.
    pub fn put<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.put(path, handler);
    }

    /// Register a handler for PATCH requests.
    pub fn patch<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.patch(path, handler);
    }

    /// Register a handler for DELETE requests.
    pub fn delete<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.delete(path, handler);
    }

    /// Register a handler for OPTIONS requests.
    pub fn options<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.options(path, handler);
    }

    /// Register a handler for HEAD requests.
    pub fn head<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.head(path, handler);
    }

    /// Register a handler for GET requests with a route-scoped
    /// error handler, as `app.get(path, fn, errFn)`.
    ///
    /// The error handler runs only when this route's own handler
    /// fails; an error raised elsewhere still skips the whole layer,
    /// reaching ordinary error middleware instead.
    pub fn get_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.get_with_error(path, handler, error_handler);
    }

    /// Register a handler for POST requests with a route-scoped
    /// error handler, as `app.post(path, fn, errFn)`.
    ///
    /// The error handler runs only when this route's own handler
    /// fails; an error raised elsewhere still skips the whole layer,
    /// reaching ordinary error middleware instead.
    pub fn post_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.post_with_error(path, handler, error_handler);
    }

    /// Register a handler for PUT requests with a route-scoped
    /// error handler, as `app.put(path, fn, errFn)`.
    ///
    /// The error handler runs only when this route's own handler
    /// fails; an error raised elsewhere still skips the whole layer,
    /// reaching ordinary error middleware instead.
    pub fn put_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.put_with_error(path, handler, error_handler);
    }

    /// Register a handler for PATCH requests with a route-scoped
    /// error handler, as `app.patch(path, fn, errFn)`.
    ///
    /// The error handler runs only when this route's own handler
    /// fails; an error raised elsewhere still skips the whole layer,
    /// reaching ordinary error middleware instead.
    pub fn patch_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.patch_with_error(path, handler, error_handler);
    }

    /// Register a handler for DELETE requests with a route-scoped
    /// error handler, as `app.delete(path, fn, errFn)`.
    ///
    /// The error handler runs only when this route's own handler
    /// fails; an error raised elsewhere still skips the whole layer,
    /// reaching ordinary error middleware instead.
    pub fn delete_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.delete_with_error(path, handler, error_handler);
    }

    /// Register a handler for OPTIONS requests with a route-scoped
    /// error handler, as `app.options(path, fn, errFn)`.
    ///
    /// The error handler runs only when this route's own handler
    /// fails; an error raised elsewhere still skips the whole layer,
    /// reaching ordinary error middleware instead.
    pub fn options_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.options_with_error(path, handler, error_handler);
    }

    /// Register a handler for HEAD requests with a route-scoped
    /// error handler, as `app.head(path, fn, errFn)`.
    ///
    /// The error handler runs only when this route's own handler
    /// fails; an error raised elsewhere still skips the whole layer,
    /// reaching ordinary error middleware instead.
    pub fn head_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.head_with_error(path, handler, error_handler);
    }

    /// Register a handler for all HTTP methods with a route-scoped
    /// error handler, as `app.all(path, fn, errFn)`.
    pub fn all_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.all_with_error(path, handler, error_handler);
    }

    /// Register a handler for all HTTP methods.
    pub fn all<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.all(path, handler);
    }

    /// Register middleware that matches all paths and methods.
    pub fn r#use<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.r#use(handler);
    }

    /// Register middleware that matches a path prefix and all methods.
    pub fn use_with_path<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.use_with_path(path, handler);
    }

    /// Register a parameter callback, as `app.param()`.
    ///
    /// The callback runs before the handlers of any matched layer that
    /// binds a parameter of this name, and at most once per request.
    /// It reads the bound value from `params`, calls `next()` to
    /// proceed, returns its own response to short-circuit, and returns
    /// `Err` to reach the error middleware.
    ///
    /// ```ignore
    /// app.param("id", |req, mut res, next, params| async move {
    ///     if params.get("id").map_or(true, |v| v.parse::<u64>().is_err()) {
    ///         return Err(Error::with_status(400, "invalid id"));
    ///     }
    ///     next(NextCall::Continue).await;
    ///     Ok(res)
    /// });
    /// ```
    pub fn param<F, Fut>(&mut self, name: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.param(name, handler);
    }

    /// Mount a sub-router at a path prefix, as `app.use('/api', apiRouter)`.
    ///
    /// Build the sub-router with [`express_rs::router::Router::new`] and
    /// its ordinary `get`/`post`/`use` methods; its paths are relative
    /// to the prefix. When the sub-router falls through without
    /// answering, the parent's stack continues, and `next('router')`
    /// from inside the sub-router exits it early.
    ///
    /// ```ignore
    /// let mut api = express_rs::router::Router::new();
    /// api.get("/users", |_req, mut res, _next, _params| async move {
    ///     res.json(&users);
    ///     Ok(res)
    /// });
    /// app.mount("/api", api);
    /// ```
    pub fn mount(&mut self, prefix: &str, router: Router) {
        self.router.mount(prefix, router);
    }

    /// Register error-handling middleware.
    ///
    /// Takes the error as its first argument, which is what distinguishes
    /// it from ordinary middleware — the same four-argument convention
    /// Express uses, but enforced by the compiler.
    ///
    /// ```ignore
    /// app.use_error_handler(|err, _req, mut res, _next, _params| async move {
    ///     res.status(err.status()).send(&err.message());
    ///     Ok(res)
    /// });
    /// ```
    pub fn use_error_handler<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Error, Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.router.use_error_handler(handler);
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
