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
    /// Leave the current mounted router and continue the parent's
    /// stack (Express's `next('router')`).
    Router,
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
    /// A mounted sub-router, as `app.use('/api', apiRouter)`. The
    /// layer matches the prefix; the sub-router dispatches the
    /// remainder.
    Mounted {
        pattern: PathPattern,
        router: Arc<Router>,
    },
}

/// A single layer in the router stack.
struct Layer {
    pattern: PathPattern,
    method: Option<Method>,
    handler: HandlerFn,
    is_route: bool,
    /// A route-scoped error handler, as the second function in
    /// Express's `app.get(path, fn, errFn)`. It catches failures of
    /// this layer's own handler; errors raised elsewhere still skip
    /// the layer entirely.
    route_error: Option<ErrorHandler>,
}

/// Router state shared across the dispatch loop.
struct RouterState {
    idx: usize,
    next_called: bool,
    skip_to_route: bool,
    /// Set by `next('router')`: abandon this router's remaining stack
    /// and hand control back to the mounting parent.
    exit_router: bool,
    error: Option<Error>,
    /// Parameter callbacks already run for this request, with the error
    /// one of them produced, if any. A name present here is not run
    /// again, which is what keeps a validation callback from firing
    /// twice when several layers bind the same parameter.
    param_outcomes: HashMap<String, Option<Error>>,
}

/// What one pass of [`Router::dispatch`] decided.
enum DispatchOutcome {
    /// A handler took ownership and never called `next`: dispatch is
    /// over, and a mounting parent stops here.
    Done(Response),
    /// The stack ran out — or `next('router')` was called — with no
    /// error: a mounted parent continues with its own stack, and the
    /// top level stops.
    Continue(Response),
    /// The stack ran out with an error still pending: a mounted parent
    /// continues in error mode, and the top level answers with the
    /// default error response.
    ContinueError(Response, Error),
}

/// The router.
pub struct Router {
    stack: Vec<Layer>,
    /// Parameter callbacks by name, as `app.param()`. Consulted at
    /// dispatch time, so a callback registered after a route still
    /// applies to it.
    param_callbacks: HashMap<String, RequestHandler>,
    case_sensitive: bool,
    strict: bool,
}

impl Router {
    /// Create a new empty router.
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            param_callbacks: HashMap::new(),
            case_sensitive: false,
            strict: false,
        }
    }

    /// Create a new router with options.
    pub fn with_options(case_sensitive: bool, strict: bool) -> Self {
        Self {
            stack: Vec::new(),
            param_callbacks: HashMap::new(),
            case_sensitive,
            strict,
        }
    }

    /// Register a parameter callback, as `app.param()`.
    ///
    /// The callback runs before the handlers of any matched layer that
    /// binds a parameter of this name, and at most once per request.
    /// It reads the bound value from `params`, calls `next()` to
    /// proceed, returns its own response to short-circuit, and returns
    /// `Err` to reach the error middleware.
    pub fn param<F, Fut>(&mut self, name: &str, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.param_callbacks
            .insert(name.to_string(), boxed_request(handler));
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

    /// Register a handler for GET requests with a route-scoped
    /// error handler, as `app.get(path, fn, errFn)`.
    pub fn get_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_error(path, Some(Method::GET), handler, error_handler);
    }

    /// Register a handler for POST requests with a route-scoped
    /// error handler, as `app.post(path, fn, errFn)`.
    pub fn post_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_error(path, Some(Method::POST), handler, error_handler);
    }

    /// Register a handler for PUT requests with a route-scoped
    /// error handler, as `app.put(path, fn, errFn)`.
    pub fn put_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_error(path, Some(Method::PUT), handler, error_handler);
    }

    /// Register a handler for PATCH requests with a route-scoped
    /// error handler, as `app.patch(path, fn, errFn)`.
    pub fn patch_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_error(path, Some(Method::PATCH), handler, error_handler);
    }

    /// Register a handler for DELETE requests with a route-scoped
    /// error handler, as `app.delete(path, fn, errFn)`.
    pub fn delete_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_error(path, Some(Method::DELETE), handler, error_handler);
    }

    /// Register a handler for OPTIONS requests with a route-scoped
    /// error handler, as `app.options(path, fn, errFn)`.
    pub fn options_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_error(path, Some(Method::OPTIONS), handler, error_handler);
    }

    /// Register a handler for HEAD requests with a route-scoped
    /// error handler, as `app.head(path, fn, errFn)`.
    pub fn head_with_error<F, Fut, E, EFut>(&mut self, path: &str, handler: F, error_handler: E)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_error(path, Some(Method::HEAD), handler, error_handler);
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
        self.add_route_error(path, None, handler, error_handler);
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

    /// Mount a sub-router at a path prefix, as `app.use(path, subApp)`.
    ///
    /// The prefix is matched with prefix semantics and stripped before
    /// the sub-router's layers see the path, so a router built with
    /// `Router::new()` and relative paths mounts unchanged. When the
    /// sub-router's stack runs out without answering — or one of its
    /// handlers calls `next('router')` — the parent's stack continues.
    pub fn mount(&mut self, prefix: &str, router: Router) {
        let pattern = PathPattern::new(prefix, self.case_sensitive, self.strict);
        self.stack.push(Layer {
            pattern: pattern.clone(),
            method: None,
            handler: HandlerFn::Mounted {
                pattern,
                router: Arc::new(router),
            },
            is_route: false,
            route_error: None,
        });
    }

    /// Add a route layer to the stack.
    fn add_route<F, Fut>(&mut self, path: &str, method: Option<Method>, handler: F)
    where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_full(
            path,
            method,
            handler,
            |err, _req, _res, _next, _params| async move { Err::<Response, Error>(err) },
        );
    }

    /// Add a route layer with a route-scoped error handler, as
    /// `app.get(path, fn, errFn)`.
    pub fn add_route_error<F, Fut, E, EFut>(
        &mut self,
        path: &str,
        method: Option<Method>,
        handler: F,
        error_handler: E,
    ) where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        self.add_route_full(path, method, handler, error_handler);
    }

    fn add_route_full<F, Fut, E, EFut>(
        &mut self,
        path: &str,
        method: Option<Method>,
        handler: F,
        error_handler: E,
    ) where
        F: Fn(Arc<Request>, Response, Next, Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, Error>> + Send + 'static,
        E: Fn(Error, Arc<Request>, Response, Next, Params) -> EFut + Send + Sync + 'static,
        EFut: Future<Output = Result<Response, Error>> + Send + 'static,
    {
        let pattern = PathPattern::new(path, self.case_sensitive, self.strict);
        self.stack.push(Layer {
            pattern,
            method,
            handler: HandlerFn::Request(boxed_request(handler)),
            is_route: true,
            route_error: Some(boxed_error(error_handler)),
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
            route_error: None,
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
        let path = req.path().to_string();
        match self.dispatch(&req, res, &path, HashMap::new()).await {
            DispatchOutcome::Done(res) | DispatchOutcome::Continue(res) => res,
            DispatchOutcome::ContinueError(_res, err) => Response::internal_error(&err, &req),
        }
    }

    /// Dispatch against `path` — which a mounting parent has already
    /// stripped of its prefix — with `mount_params` merged into every
    /// matched layer's parameters.
    ///
    /// The returned outcome tells a mounting parent what to do: stop
    /// because a handler answered, continue its own stack because this
    /// router fell through, or continue in error mode because an error
    /// is still pending. At the top level those last two collapse into
    /// "stop".
    async fn dispatch(
        &self,
        req: &Arc<Request>,
        res: Response,
        path: &str,
        mount_params: Params,
    ) -> DispatchOutcome {
        let state = Arc::new(Mutex::new(RouterState {
            idx: 0,
            next_called: true,
            skip_to_route: false,
            exit_router: false,
            error: None,
            param_outcomes: HashMap::new(),
        }));
        let mut res = res;

        loop {
            let layer_idx = {
                let mut s = state.lock().unwrap();
                if s.exit_router {
                    // next('router'): hand control back to the parent.
                    return match s.error.take() {
                        Some(err) => DispatchOutcome::ContinueError(res, err),
                        None => DispatchOutcome::Continue(res),
                    };
                }
                if !s.next_called {
                    // A handler took ownership without calling next.
                    return match s.error.take() {
                        Some(err) => DispatchOutcome::ContinueError(res, err),
                        None => DispatchOutcome::Done(res),
                    };
                }
                if s.idx >= self.stack.len() {
                    // End of the stack: fall through, or bubble the error.
                    return match s.error.take() {
                        Some(err) => DispatchOutcome::ContinueError(res, err),
                        None => DispatchOutcome::Continue(res),
                    };
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

            // Match the path. A mounted layer hands its sub-path and
            // bound parameters to the sub-router; every other layer
            // matches against the (possibly mount-stripped) path.
            let mut mount_rest: Option<String> = None;
            let mut params = if let HandlerFn::Mounted { pattern, .. } = &layer.handler {
                match pattern.match_prefix_rest(path) {
                    Some((mount_params, rest)) => {
                        mount_rest = Some(rest);
                        mount_params
                    }
                    None => {
                        state.lock().unwrap().next_called = true;
                        continue;
                    }
                }
            } else if layer.is_route {
                match layer.pattern.match_path(path) {
                    Some(p) => p,
                    None => {
                        state.lock().unwrap().next_called = true;
                        continue;
                    }
                }
            } else {
                let matched = match layer.pattern.match_prefix(path) {
                    Some(p) => p,
                    None => {
                        state.lock().unwrap().next_called = true;
                        continue;
                    }
                };
                matched
            };
            // Mount parameters ride along on every layer, with the
            // layer's own bindings taking precedence.
            params.extend(mount_params.clone());

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
            // A mounted router is skipped while an error is pending, so
            // the parent's error middleware catches sub-router errors.
            let error_pending = state.lock().unwrap().error.is_some();
            match (&layer.handler, error_pending) {
                (HandlerFn::Request(_), false)
                | (HandlerFn::Error(_), true)
                | (HandlerFn::Mounted { .. }, false) => {}
                _ => {
                    state.lock().unwrap().next_called = true;
                    continue;
                }
            }

            // Parameter callbacks run for every matched layer whose
            // bound parameters name one, before the layer's own
            // handlers — as `process_params` does in the original
            // router, for routes and middleware alike.
            if !self.param_callbacks.is_empty() {
                let wanted: Vec<String> = layer
                    .pattern
                    .param_names()
                    .into_iter()
                    .filter(|n| params.contains_key(n) && self.param_callbacks.contains_key(n))
                    .collect();

                if !wanted.is_empty() {
                    state.lock().unwrap().next_called = false;
                    match self
                        .run_param_callbacks(&state, req, res, &params, wanted)
                        .await
                    {
                        ParamRun::Failed => {
                            // The failing callback consumed the response;
                            // rebuild it as the Err arm below does.
                            res = Response::new();
                            continue;
                        }
                        ParamRun::Proceed(next_res) => {
                            res = next_res;
                            if !state.lock().unwrap().next_called {
                                // A callback returned a final response
                                // without calling next; the loop top
                                // returns it.
                                continue;
                            }
                        }
                    }
                }
            }

            state.lock().unwrap().next_called = false;

            let outcome = match &layer.handler {
                HandlerFn::Mounted { router, .. } => {
                    let rest = mount_rest.take().unwrap_or_else(|| "/".to_string());
                    let router = router.clone();
                    // Boxed because a mounted router dispatches again:
                    // the indirection keeps the future's size finite.
                    match Box::pin(router.dispatch(req, res, &rest, params.clone())).await {
                        DispatchOutcome::Done(next_res) => {
                            // The sub-router answered; the loop top sees
                            // next_called off and stops here.
                            state.lock().unwrap().next_called = false;
                            Ok(next_res)
                        }
                        DispatchOutcome::Continue(next_res) => {
                            state.lock().unwrap().next_called = true;
                            Ok(next_res)
                        }
                        DispatchOutcome::ContinueError(next_res, err) => {
                            let mut s = state.lock().unwrap();
                            s.error = Some(err);
                            s.next_called = true;
                            Ok(next_res)
                        }
                    }
                }
                HandlerFn::Request(handler) => {
                    let state_clone = state.clone();
                    let req_clone = req.clone();

                    let next: Next = Arc::new(move |call| {
                        let state = state_clone.clone();
                        Box::pin(async move { apply_next_call(&state, call) })
                    });

                    handler(req_clone, res, next, params.clone()).await
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

                    handler(err, req_clone, res, next, params.clone()).await
                }
            };

            match outcome {
                Ok(next_res) => res = next_res,
                Err(e) => {
                    // The failing handler consumed the response, so rebuild
                    // it: the ownership model gives no way to return both a
                    // partial response and an error.
                    res = Response::new();

                    // A route-scoped error handler catches its own
                    // route's failure, as the second function of
                    // `app.get(path, fn, errFn)` does. An error raised
                    // elsewhere still skips this layer entirely, which
                    // is the router's own skip rule.
                    let caught = match (&layer.handler, &layer.route_error) {
                        (HandlerFn::Request(_), Some(route_error)) if layer.is_route => {
                            state.lock().unwrap().next_called = false;
                            let state_clone = state.clone();
                            let next: Next = Arc::new(move |call| {
                                let state = state_clone.clone();
                                Box::pin(async move { apply_next_call(&state, call) })
                            });
                            match route_error(e.clone(), req.clone(), res, next, params.clone())
                                .await
                            {
                                Ok(next_res) => {
                                    res = next_res;
                                    true
                                }
                                Err(e2) => {
                                    // The error handler itself failed:
                                    // rebuild and forward the new error.
                                    res = Response::new();
                                    let mut s = state.lock().unwrap();
                                    s.error = Some(e2);
                                    s.next_called = true;
                                    true
                                }
                            }
                        }
                        _ => false,
                    };

                    if !caught {
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
}

/// What running the parameter callbacks decided.
enum ParamRun {
    /// Every callback called `next()`; the layer's handlers may run.
    Proceed(Response),
    /// An error is now pending; skip the layer and let the error flow.
    Failed,
}

impl Router {
    /// Run the parameter callbacks for a matched layer, in path order.
    ///
    /// A name already run for this request is skipped — unless it
    /// failed, in which case the failure is replayed into the error
    /// state so no later layer can proceed either.
    async fn run_param_callbacks(
        &self,
        state: &Arc<Mutex<RouterState>>,
        req: &Arc<Request>,
        mut res: Response,
        params: &Params,
        names: Vec<String>,
    ) -> ParamRun {
        for name in names {
            if let Some(cached) = state.lock().unwrap().param_outcomes.get(&name).cloned() {
                if let Some(err) = cached {
                    let mut s = state.lock().unwrap();
                    s.error = Some(err);
                    s.next_called = true;
                    return ParamRun::Failed;
                }
                continue;
            }

            let callback = match self.param_callbacks.get(&name) {
                Some(cb) => cb.clone(),
                None => continue,
            };

            state.lock().unwrap().next_called = false;

            let state_clone = state.clone();
            let next: Next = Arc::new(move |call| {
                let state = state_clone.clone();
                Box::pin(async move { apply_next_call(&state, call) })
            });

            match callback(req.clone(), res, next, params.clone()).await {
                Ok(next_res) => {
                    res = next_res;
                    let called_next = state.lock().unwrap().next_called;
                    state.lock().unwrap().param_outcomes.insert(name, None);
                    if !called_next {
                        // A callback that never calls next owns the
                        // response; the loop top sees next_called off and
                        // returns it.
                        return ParamRun::Proceed(res);
                    }
                }
                Err(err) => {
                    let mut s = state.lock().unwrap();
                    s.param_outcomes.insert(name, Some(err.clone()));
                    s.error = Some(err);
                    s.next_called = true;
                    drop(s);
                    return ParamRun::Failed;
                }
            }
        }

        // Reaching here, every callback that ran called next(); the
        // cached skips never touched the flag, so restore it for the
        // layer's own handlers.
        state.lock().unwrap().next_called = true;
        ParamRun::Proceed(res)
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
        NextCall::Router => {
            let mut s = state.lock().unwrap();
            s.next_called = true;
            s.exit_router = true;
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
