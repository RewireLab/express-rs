//! Router: dispatches requests to matching handlers.
//!
//! Inspired by the `pillarjs/router` package used by Express 5.
//! The router maintains a stack of layers and iterates through them
//! in order, executing handlers for each matching layer.
//!
//! # Error flow
//!
//! Express distinguishes error-handling middleware by arity: a function
//! taking four arguments `(err, req, res, next)` is an error handler, and
//! `Layer#handleError` refuses to call anything else. This router keeps
//! that distinction in the type system instead — [`ErrorHandler`] takes
//! the error as its first parameter and can only be registered through
//! [`Router::use_error_handler`].
//!
//! While an error is pending, route layers are skipped entirely, exactly
//! as `Router#handle` does in the original. Only middleware can catch an
//! error, so an error handler must be registered with `use`.

use crate::error::Error;
use crate::path::PathPattern;
use crate::request::Request;
use crate::response::Response;
use http::Method;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// The type of `next()` call.
#[derive(Debug)]
pub enum NextCall {
    /// Continue to the next handler/layer.
    Continue,
    /// Skip to the next route (Express's `next('route')`).
    Route,
    /// Forward an error to the error-handling middleware (`next(err)`).
    Error(Error),
}

/// The `next` function passed to handlers.
pub type Next = Arc<dyn Fn(NextCall) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Route parameters captured from the path.
pub type Params = HashMap<String, String>;

/// A request handler or piece of middleware.
///
/// Returns `Err` to signal failure, which the router forwards exactly as
/// Express forwards a rejected promise.
pub type RequestHandler = Arc<
    dyn Fn(
            Arc<Request>,
            Response,
            Next,
            Params,
        ) -> Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>
        + Send
        + Sync,
>;

/// Error-handling middleware.
///
/// Recognised by taking the error as its first parameter, mirroring
/// Express's four-argument convention `(err, req, res, next)`.
///
/// The error is passed by value rather than by reference. A borrowed
/// argument cannot be captured by an `async` block, which would make
/// `async fn` error handlers impossible to write.
pub type ErrorHandler = Arc<
    dyn Fn(
            Error,
            Arc<Request>,
            Response,
            Next,
            Params,
        ) -> Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>
        + Send
        + Sync,
>;

/// What a layer's handler does when reached.
enum HandlerFn {
    Request(RequestHandler),
    Error(ErrorHandler),
}

/// A single layer in the router stack.
struct Layer {
    pattern: PathPattern,
    method: Option<Method>,
    handler: HandlerFn,
    is_route: bool,
}

/// Router state shared across the dispatch loop.
struct RouterState {
    idx: usize,
    next_called: bool,
    skip_to_route: bool,
    error: Option<Error>,
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
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, Some(Method::GET), handler);
    }

    /// Register a handler for POST requests.
    pub fn post<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, Some(Method::POST), handler);
    }

    /// Register a handler for PUT requests.
    pub fn put<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, Some(Method::PUT), handler);
    }

    /// Register a handler for PATCH requests.
    pub fn patch<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, Some(Method::PATCH), handler);
    }

    /// Register a handler for DELETE requests.
    pub fn delete<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, Some(Method::DELETE), handler);
    }

    /// Register a handler for OPTIONS requests.
    pub fn options<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, Some(Method::OPTIONS), handler);
    }

    /// Register a handler for HEAD requests.
    pub fn head<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, Some(Method::HEAD), handler);
    }

    /// Register a handler for all HTTP methods.
    pub fn all<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route(path, None, handler);
    }

    /// Register middleware that matches all paths and methods.
    pub fn r#use<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_middleware("/", HandlerFn::Request(boxed_request(handler)));
    }

    /// Register middleware that matches a path prefix and all methods.
    pub fn use_with_path<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_middleware(path, HandlerFn::Request(boxed_request(handler)));
    }

    /// Register error-handling middleware.
    ///
    /// The handler takes the error as its first argument, which is what
    /// distinguishes it from ordinary middleware — the same four-argument
    /// convention Express uses, made explicit in the signature.
    ///
    /// Error handlers only run while an error is pending, and are reached
    /// only through middleware, because a route layer is skipped as soon
    /// as an error exists.
    pub fn use_error_handler<F, Fut>(&mut self, handler: F)
    where
        F: Fn(Error, Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_middleware("/", HandlerFn::Error(boxed_error(handler)));
    }

    /// Add a route layer to the stack.
    fn add_route<F, Fut>(&mut self, path: &str, method: Option<Method>, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        let pattern = PathPattern::new(path, self.case_sensitive, self.strict);
        self.stack.push(Layer {
            pattern,
            method,
            handler: HandlerFn::Request(boxed_request(handler)),
            is_route: true,
        });
    }

    /// Add a middleware layer to the stack.
    fn add_middleware(&mut self, path: &str, handler: HandlerFn) {
        let pattern = PathPattern::new(path, self.case_sensitive, self.strict);
        self.stack.push(Layer {
            pattern,
            method: None,
            handler,
            is_route: false,
        });
    }

    /// Dispatch a request through the router.
    ///
    /// Iterates through the layer stack in order. A layer runs only when
    /// its handler kind matches the current error state: ordinary handlers
    /// run while no error is pending, error handlers only once one is.
    ///
    /// A handler that returns `Err`, or that calls
    /// `next(NextCall::Error(..))`, puts an error into play. If the stack
    /// runs out with an error still pending, the router answers with a
    /// default 500, as Express's `finalhandler` does.
    pub async fn handle(&self, req: Arc<Request>, res: Response) -> Response {
        let state = Arc::new(Mutex::new(RouterState {
            idx: 0,
            next_called: true,
            skip_to_route: false,
            error: None,
        }));
        let mut res = res;

        loop {
            let layer_idx = {
                let mut s = state.lock().unwrap();
                if !s.next_called || s.idx >= self.stack.len() {
                    // End of the stack. An unhandled error becomes a 500.
                    if let Some(err) = s.error.take() {
                        drop(s);
                        return Response::internal_error(&err, &req);
                    }
                    return res;
                }
                let idx = s.idx;
                s.idx += 1;
                idx
            };

            let layer = &self.stack[layer_idx];

            // next('route'): skip middleware until the next route layer.
            // Express converts the 'route' sentinel into "no error" and
            // only route layers can satisfy the search.
            if state.lock().unwrap().skip_to_route {
                if !layer.is_route {
                    state.lock().unwrap().next_called = true;
                    continue;
                }
                state.lock().unwrap().skip_to_route = false;
            }

            // Match the path.
            let params = if layer.is_route {
                match layer.pattern.match_path(req.path()) {
                    Some(p) => p,
                    None => {
                        state.lock().unwrap().next_called = true;
                        continue;
                    }
                }
            } else {
                match layer.pattern.match_prefix(req.path()) {
                    Some(p) => p,
                    None => {
                        state.lock().unwrap().next_called = true;
                        continue;
                    }
                }
            };

            // A pending error skips every route layer, so only middleware
            // can catch it. This is `if (layerError) { match = false }` in
            // the original router.
            if layer.is_route && state.lock().unwrap().error.is_some() {
                state.lock().unwrap().next_called = true;
                continue;
            }

            // A route's method must match. Express falls back from HEAD to
            // GET when no HEAD handler is registered.
            if let Some(ref method) = layer.method {
                let method_matches = *method == *req.method()
                    || (*req.method() == Method::HEAD && *method == Method::GET);

                if !method_matches {
                    state.lock().unwrap().next_called = true;
                    continue;
                }
            }

            // Only a handler whose kind matches the error state may run.
            let error_pending = state.lock().unwrap().error.is_some();
            match (&layer.handler, error_pending) {
                (HandlerFn::Request(_), false) | (HandlerFn::Error(_), true) => {}
                _ => {
                    state.lock().unwrap().next_called = true;
                    continue;
                }
            }

            state.lock().unwrap().next_called = false;

            let outcome = match &layer.handler {
                HandlerFn::Request(handler) => {
                    let state_clone = state.clone();
                    let req_clone = req.clone();

                    let next: Next = Arc::new(move |call| {
                        let state = state_clone.clone();
                        Box::pin(async move { apply_next_call(&state, call) })
                    });

                    handler(req_clone, res, next, params).await
                }
                HandlerFn::Error(handler) => {
                    // Take the error so the handler owns it; a subsequent
                    // next(err) from inside the handler installs a new one.
                    let err = match state.lock().unwrap().error.take() {
                        Some(e) => e,
                        None => {
                            state.lock().unwrap().next_called = true;
                            continue;
                        }
                    };

                    let state_clone = state.clone();
                    let req_clone = req.clone();

                    let next: Next = Arc::new(move |call| {
                        let state = state_clone.clone();
                        Box::pin(async move { apply_next_call(&state, call) })
                    });

                    handler(err, req_clone, res, next, params).await
                }
            };

            match outcome {
                Ok(next_res) => res = next_res,
                Err(e) => {
                    // The failing handler consumed the response, so rebuild
                    // it: the ownership model gives no way to return both a
                    // partial response and an error.
                    res = Response::new();

                    // A failure advances dispatch, exactly as Express's
                    // `ret.then(null, error => next(error))` does.
                    let mut s = state.lock().unwrap();
                    s.error = Some(e);
                    s.next_called = true;
                }
            }
        }
    }
}

/// Apply a `next()` call to the router state.
fn apply_next_call(state: &Arc<Mutex<RouterState>>, call: NextCall) {
    match call {
        NextCall::Continue => {
            state.lock().unwrap().next_called = true;
        }
        NextCall::Route => {
            let mut s = state.lock().unwrap();
            s.next_called = true;
            s.skip_to_route = true;
        }
        NextCall::Error(e) => {
            state.lock().unwrap().error = Some(e);
            state.lock().unwrap().next_called = true;
        }
    }
}

/// Box a request handler into the shared trait object.
fn boxed_request<F, Fut>(handler: F) -> RequestHandler
where
    F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Response, Error>> + Send + 'static,
{
    Arc::new(move |req, res, next, params| Box::pin(handler(req, res, next, params)))
}

/// Box an error handler into the shared trait object.
fn boxed_error<F, Fut>(handler: F) -> ErrorHandler
where
    F: Fn(Error, Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Response, Error>> + Send + 'static,
{
    Arc::new(move |err, req, res, next, params| Box::pin(handler(err, req, res, next, params)))
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
