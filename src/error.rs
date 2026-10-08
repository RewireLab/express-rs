//! Error types for express-rs.
//!
//! This module defines the error model for the framework. It will expand
//! in later phases to include HTTP errors, routing errors, and parsing errors.

use std::fmt;

/// The primary error type for express-rs.
#[derive(Debug)]
pub enum Error {
    /// I/O error (e.g., failed to bind to a port).
    Io(std::io::Error),
    /// Hyper HTTP error.
    Hyper(hyper::Error),
    /// HTTP protocol error.
    Http(http::Error),
    /// Custom error message.
    Custom(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO error: {}", e),
            Error::Hyper(e) => write!(f, "Hyper error: {}", e),
            Error::Http(e) => write!(f, "HTTP error: {}", e),
            Error::Custom(s) => write!(f, "{}", s),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Hyper(e) => Some(e),
            Error::Http(e) => Some(e),
            Error::Custom(_) => None,
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
