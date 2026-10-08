//! Router: dispatches requests to matching handlers.
//!
//! Inspired by the `pillarjs/router` package used by Express 5.
//! The router maintains a stack of layers and iterates through them
//! in order, executing handlers for each matching layer.

use crate::request::Request;
use crate::response::Response;
use http::Method;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// The `next` function passed to handlers.
///
/// When a handler calls `next()`, the router continues to the next
/// matching layer. If `next()` is not called, the request stops at the
/// current handler.
pub type Next = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// A request handler with middleware support.
///
/// Takes a shared `Arc<Request>` (so the request can be shared across
/// middleware, matching Express's behavior), ownership of `Response`,
/// and a `Next` function. Returns the `Response`.
pub type Handler = Arc<
    dyn Fn(Arc<Request>, Response, Next) -> Pin<Box<dyn Future<Output = Response> + Send>>
        + Send
        + Sync,
>;

/// A single layer in the router stack.
///
/// A layer matches a path (and optionally an HTTP method) and contains
/// one or more handlers that are executed in order.
struct Layer {
    path: String,
    method: Option<Method>,
    handlers: Vec<Handler>,
}

impl Layer {
    /// Check if this layer matches the given request.
    fn matches(&self, req: &Request) -> bool {
        // Check HTTP method
        if let Some(ref method) = self.method {
            if method != req.method() {
                return false;
            }
        }

        // Check path
        if self.method.is_some() {
            // Route layer: exact match (with optional trailing slash)
            let path = req.path();
            self.path == path || self.path == path.trim_end_matches('/')
        } else {
            // Middleware layer: prefix match
            req.path().starts_with(&self.path)
        }
    }
}

/// Router state shared across the dispatch loop.
struct RouterState {
    idx: usize,
    next_called: bool,
}

/// The router.
///
/// Maintains an ordered stack of layers and dispatches requests to
/// the first matching layer's handlers.
pub struct Router {
    stack: Vec<Layer>,
}

impl Router {
    /// Create a new empty router.
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// Register a handler for GET requests.
    pub fn get<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, Some(Method::GET), handler);
    }

    /// Register a handler for POST requests.
    pub fn post<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, Some(Method::POST), handler);
    }

    /// Register a handler for PUT requests.
    pub fn put<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, Some(Method::PUT), handler);
    }

    /// Register a handler for PATCH requests.
    pub fn patch<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, Some(Method::PATCH), handler);
    }

    /// Register a handler for DELETE requests.
    pub fn delete<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, Some(Method::DELETE), handler);
    }

    /// Register a handler for OPTIONS requests.
    pub fn options<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, Some(Method::OPTIONS), handler);
    }

    /// Register a handler for HEAD requests.
    pub fn head<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, Some(Method::HEAD), handler);
    }

    /// Register a handler for all HTTP methods.
    pub fn all<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, None, handler);
    }

    /// Register middleware that matches all paths and methods.
    pub fn r#use<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer("/", None, handler);
    }

    /// Register middleware that matches a path prefix and all methods.
    pub fn use_with_path<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_layer(path, None, handler);
    }

    /// Add a layer to the stack.
    fn add_layer<F, Fut>(&mut self, path: &str, method: Option<Method>, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |req, res, next| Box::pin(handler(req, res, next)));

        self.stack.push(Layer {
            path: path.to_string(),
            method,
            handlers: vec![handler],
        });
    }

    /// Dispatch a request through the router.
    ///
    /// Iterates through the layer stack in order. For each matching
    /// layer, executes its handlers sequentially. If a handler calls
    /// `next()`, processing continues to the next handler/layer. If
    /// `next()` is not called, processing stops.
    pub async fn handle(&self, req: Request, res: Response) -> Response {
        let req = Arc::new(req);
        let state = Arc::new(Mutex::new(RouterState {
            idx: 0,
            next_called: true,
        }));
        let mut res = res;

        loop {
            // Check if we should continue
            let layer_idx = {
                let mut s = state.lock().unwrap();
                if !s.next_called || s.idx >= self.stack.len() {
                    return res;
                }
                let idx = s.idx;
                s.idx += 1;
                idx
            };

            let layer = &self.stack[layer_idx];

            // Skip non-matching layers — reset next_called so the loop
            // can continue scanning for the next matching layer.
            if !layer.matches(&req) {
                state.lock().unwrap().next_called = true;
                continue;
            }

            // Execute handlers in this layer.
            // Reset next_called before running handlers so we can
            // detect whether any of them calls next().
            state.lock().unwrap().next_called = false;

            for handler in &layer.handlers {
                let state_clone = state.clone();
                let req_clone = req.clone();

                let next: Next = Arc::new(move || {
                    let state = state_clone.clone();
                    Box::pin(async move {
                        state.lock().unwrap().next_called = true;
                    })
                });

                res = handler(req_clone, res, next).await;
            }
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
