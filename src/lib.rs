//! express-rs: Express.js-style HTTP framework, rebuilt in Rust.
//!
//! This crate provides an Express.js-inspired HTTP framework for Rust.
//! It is an independent implementation inspired by Express.js, not an
//! official project.
//!
//! # Phase 2: Application + Routing
//!
//! The current implementation provides:
//! - TCP server with HTTP/1.1 support (via hyper)
//! - Request parsing: method, path, headers, body, query parameters
//! - Response writing: status, headers, text, JSON
//! - Keep-alive connections
//! - Async request processing
//! - Routing: `app.get()`, `app.post()`, etc.
//! - Middleware: `app.use()` with `next()` support
//!
//! # Example
//!
//! ```no_run
//! use express_rs::App;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut app = App::new();
//!
//!     app.get("/", |_req, mut res, _next| async move {
//!         res.status(200).text("Hello, World!");
//!         res
//!     });
//!
//!     app.listen(3000).await?;
//!     Ok(())
//! }
//! ```

pub mod app;
pub mod error;
pub mod request;
pub mod response;
pub mod router;
pub mod server;

pub use app::App;
pub use error::Error;
pub use request::Request;
pub use response::Response;
