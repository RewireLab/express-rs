//! Application abstraction.
//!
//! The `App` is the main user-facing type. In Phase 1, it wraps a single
//! handler. In Phase 3, it will hold a router. In Phase 6, it will support
//! middleware.

use crate::error::Error;
use crate::request::Request;
use crate::response::Response;
use crate::server::Server;

/// The Express-rs application.
///
/// # Phase 1
///
/// The app holds a single request handler. All requests are routed to it.
///
/// # Future Phases
///
/// - Phase 3: Router with `app.get()`, `app.post()`, etc.
/// - Phase 6: Middleware with `app.use()`
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
///     app.handler(|_req, mut res| async move {
///         res.status(200).text("Hello, World!");
///         res
///     });
///
///     app.listen(3000).await?;
///     Ok(())
/// }
/// ```
pub struct App {
    server: Server,
}

impl App {
    /// Create a new application.
    pub fn new() -> Self {
        Self {
            server: Server::new(),
        }
    }

    /// Set the request handler.
    ///
    /// In Phase 1, this is a single handler for all requests. It will be
    /// replaced by router-based dispatch in Phase 3.
    pub fn handler<F, Fut>(&mut self, f: F)
    where
        F: Fn(Request, Response) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Response> + Send + 'static,
    {
        self.server.set_handler(f);
    }

    /// Start the server on the given port.
    ///
    /// Runs indefinitely until the process is killed.
    pub async fn listen(self, port: u16) -> Result<(), Error> {
        self.server.listen(port).await
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
