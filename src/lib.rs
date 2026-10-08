//! express-rs: Express.js-style HTTP framework, rebuilt in Rust.
//!
//! This crate provides an Express.js-inspired HTTP framework for Rust.
//! It is an independent implementation inspired by Express.js, not an
//! official project.
//!
//! # Phase 1: HTTP Foundation
//!
//! The current implementation provides the HTTP foundation:
//! - TCP server with HTTP/1.1 support
//! - Request parsing: method, path, headers, body, query parameters
//! - Response writing: status, headers, body
//! - Keep-alive connections
//! - Basic async request processing
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
//!     app.handler(|_req, mut res| async move {
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
pub mod server;

pub use app::App;
pub use error::Error;
pub use request::Request;
pub use response::Response;
