//! Router: dispatches requests to matching handlers.
//!
//! Inspired by the `pillarjs/router` package used by Express 5.
//! The router maintains a stack of layers and iterates through them
//! in order, executing handlers for each matching layer.

use crate::path::PathPattern;
use crate::request::Request;
use crate::response::Response;
use http::Method;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// The type of `next()` call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NextCall {
    /// Continue to the next handler/layer.
    Continue,
    /// Skip to the next route (Express's `next('route')`).
    Route,
}

/// The `next` function passed to handlers.
pub type Next = Arc<dyn Fn(NextCall) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// A request handler with middleware support.
pub type Handler = Arc<
    dyn Fn(
            Arc<Request>,
            Response,
            Next,
            HashMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = Response> + Send>>
        + Send
        + Sync,
>;

/// A single layer in the router stack.
struct Layer {
    pattern: PathPattern,
    method: Option<Method>,
    handlers: Vec<Handler>,
    is_route: bool,
}

/// Router state shared across the dispatch loop.
struct RouterState {
    idx: usize,
    next_called: bool,
    skip_to_route: bool,
}

/// The router.
pub struct Router {
    stack: Vec<Layer>,
    case_sensitive: bool,
    strict: bool,
}

impl Router {
    /// Create a new empty router.
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            case_sensitive: false,
            strict: false,
        }
    }

    /// Create a new router with options.
    pub fn with_options(case_sensitive: bool, strict: bool) -> Self {
        Self {
            stack: Vec::new(),
            case_sensitive,
            strict,
        }
    }

    /// Enable case-sensitive routing.
    pub fn set_case_sensitive(&mut self, enabled: bool) {
        self.case_sensitive = enabled;
    }

    /// Enable strict routing (trailing slash matters).
    pub fn set_strict(&mut self, enabled: bool) {
        self.strict = enabled;
    }

    /// Register a handler for GET requests.
    pub fn get<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, Some(Method::GET), handler);
    }

    /// Register a handler for POST requests.
    pub fn post<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, Some(Method::POST), handler);
    }

    /// Register a handler for PUT requests.
    pub fn put<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, Some(Method::PUT), handler);
    }

    /// Register a handler for PATCH requests.
    pub fn patch<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, Some(Method::PATCH), handler);
    }

    /// Register a handler for DELETE requests.
    pub fn delete<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, Some(Method::DELETE), handler);
    }

    /// Register a handler for OPTIONS requests.
    pub fn options<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, Some(Method::OPTIONS), handler);
    }

    /// Register a handler for HEAD requests.
    pub fn head<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, Some(Method::HEAD), handler);
    }

    /// Register a handler for all HTTP methods.
    pub fn all<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_route(path, None, handler);
    }

    /// Register middleware that matches all paths and methods.
    pub fn r#use<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_middleware("/", handler);
    }

    /// Register middleware that matches a path prefix and all methods.
    pub fn use_with_path<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.add_middleware(path, handler);
    }

    /// Add a route layer.
    fn add_route<F, Fut>(&mut self, path: &str, method: Option<Method>, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let pattern = PathPattern::new(path, self.case_sensitive, self.strict);
        let handler: Handler =
            Arc::new(move |req, res, next, params| Box::pin(handler(req, res, next, params)));

        self.stack.push(Layer {
            pattern,
            method,
            handlers: vec![handler],
            is_route: true,
        });
    }

    /// Add a middleware layer.
    fn add_middleware<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, HashMap<String, String>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let pattern = PathPattern::new(path, self.case_sensitive, self.strict);
        let handler: Handler =
            Arc::new(move |req, res, next, params| Box::pin(handler(req, res, next, params)));

        self.stack.push(Layer {
            pattern,
            method: None,
            handlers: vec![handler],
            is_route: false,
        });
    }

    /// Dispatch a request through the router.
    pub async fn handle(&self, req: Request, res: Response) -> Response {
        let req = Arc::new(req);
        let state = Arc::new(Mutex::new(RouterState {
            idx: 0,
            next_called: true,
            skip_to_route: false,
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

            // Handle next('route') — skip to next route layer
            if state.lock().unwrap().skip_to_route {
                state.lock().unwrap().skip_to_route = false;
                if !layer.is_route {
                    continue;
                }
            }

            // Match path and extract params
            let params = if layer.method.is_some() {
                // Route layer: exact match (tolerating a trailing slash)
                match layer.pattern.match_path(req.path()) {
                    Some(p) => p,
                    None => {
                        state.lock().unwrap().next_called = true;
                        continue;
                    }
                }
            } else {
                // Middleware layer: prefix match, extra segments allowed
                match layer.pattern.match_prefix(req.path()) {
                    Some(p) => p,
                    None => {
                        state.lock().unwrap().next_called = true;
                        continue;
                    }
                }
            };

            // Check HTTP method for route layers
            if let Some(ref method) = layer.method {
                if method != req.method() {
                    state.lock().unwrap().next_called = true;
                    continue;
                }
            }

            // Execute handlers
            state.lock().unwrap().next_called = false;

            for handler in &layer.handlers {
                let state_clone = state.clone();
                let req_clone = req.clone();
                let params_clone = params.clone();

                let next: Next = Arc::new(move |call_type| {
                    let state = state_clone.clone();
                    Box::pin(async move {
                        match call_type {
                            NextCall::Continue => {
                                state.lock().unwrap().next_called = true;
                            }
                            NextCall::Route => {
                                state.lock().unwrap().next_called = true;
                                state.lock().unwrap().skip_to_route = true;
                            }
                        }
                    })
                });

                res = handler(req_clone, res, next, params_clone).await;
            }
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
