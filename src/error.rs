//! Error types.
//!
//! Express passes any thrown value to error-handling middleware, and the
//! convention is an object with a `message` and an optional
//! `status`/`statusCode`. This mirrors that with an enum for framework
//! errors plus an application variant carrying both fields.

use std::fmt;

/// The status code used when an error does not name one.
pub const DEFAULT_STATUS: u16 = 500;

/// The primary error type for express-rs.
#[derive(Debug)]
pub enum Error {
    /// I/O error (e.g., failed to bind to a port).
    Io(std::io::Error),
    /// Hyper HTTP error.
    Hyper(hyper::Error),
    /// HTTP protocol error.
    Http(http::Error),
    /// An application error with an optional status code.
    App(AppError),
}

/// An error raised by application code.
#[derive(Debug)]
pub struct AppError {
    message: String,
    status: Option<u16>,
    /// Machine-readable category, e.g. `entity.parse.failed`.
    ///
    /// Express attaches `.type` to errors from body parsing. The name
    /// `kind` avoids confusion with the Rust `type` keyword.
    kind: Option<String>,
}

impl Error {
    /// Create an application error with no status, defaulting to 500.
    pub fn new(message: impl Into<String>) -> Self {
        Error::App(AppError {
            message: message.into(),
            status: None,
            kind: None,
        })
    }

    /// Create an application error with an explicit status code.
    pub fn with_status(status: u16, message: impl Into<String>) -> Self {
        Error::App(AppError {
            message: message.into(),
            status: Some(status),
            kind: None,
        })
    }

    /// Create an application error with status, category, and message.
    pub fn with_kind(status: u16, kind: impl Into<String>, message: impl Into<String>) -> Self {
        Error::App(AppError {
            message: message.into(),
            status: Some(status),
            kind: Some(kind.into()),
        })
    }

    /// The error's message.
    ///
    /// Framework errors report their own `Display`; application errors
    /// report whatever the handler supplied.
    pub fn message(&self) -> String {
        match self {
            Error::Io(e) => e.to_string(),
            Error::Hyper(e) => e.to_string(),
            Error::Http(e) => e.to_string(),
            Error::App(e) => e.message.clone(),
        }
    }

    /// The HTTP status this error should produce.
    ///
    /// `err.status` or `err.statusCode` in Express, falling back to 500.
    pub fn status(&self) -> u16 {
        match self {
            Error::App(e) => e.status.unwrap_or(DEFAULT_STATUS),
            _ => DEFAULT_STATUS,
        }
    }

    /// The machine-readable category, if one was set.
    pub fn kind(&self) -> Option<&str> {
        match self {
            Error::App(e) => e.kind.as_deref(),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{}", e),
            Error::Hyper(e) => write!(f, "{}", e),
            Error::Http(e) => write!(f, "{}", e),
            Error::App(e) => write!(f, "{}", e.message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Hyper(e) => Some(e),
            Error::Http(e) => Some(e),
            Error::App(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<hyper::Error> for Error {
    fn from(e: hyper::Error) -> Self {
        Error::Hyper(e)
    }
}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Self {
        Error::Http(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_error_defaults_to_500() {
        let e = Error::new("boom");
        assert_eq!(e.message(), "boom");
        assert_eq!(e.status(), 500);
    }

    #[test]
    fn application_error_carries_its_status() {
        let e = Error::with_status(404, "missing");
        assert_eq!(e.message(), "missing");
        assert_eq!(e.status(), 404);
        assert_eq!(e.kind(), None);
    }

    #[test]
    fn application_error_carries_its_kind() {
        let e = Error::with_kind(400, "entity.parse.failed", "bad json");
        assert_eq!(e.status(), 400);
        assert_eq!(e.kind(), Some("entity.parse.failed"));
    }

    #[test]
    fn framework_errors_report_500() {
        let e = Error::Http(http::Error::from(
            http::status::StatusCode::from_u16(99).unwrap_err(),
        ));
        assert_eq!(e.status(), 500);
        assert!(!e.message().is_empty());
    }

    #[test]
    fn display_matches_message() {
        assert_eq!(Error::new("nope").to_string(), "nope");
    }
}
