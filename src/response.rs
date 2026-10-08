//! HTTP response abstraction.
//!
//! Provides an idiomatic Rust interface for building HTTP responses,
//! inspired by Express's `res` object. This is the Phase 1 version with
//! basic status, headers, and body. JSON, redirects, and other helpers
//! come in later phases.

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, StatusCode};
use http_body_util::Full;

/// An HTTP response.
///
/// The handler sets the status, headers, and body on this struct, and the
/// server writes it to the client. In Phase 1, this supports basic text
/// responses. JSON, redirects, cookies, and other Express helpers will be
/// added in later phases.
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Option<Bytes>,
}

impl Response {
    /// Create a new response with default status 200 OK.
    pub fn new() -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: None,
        }
    }

    /// Set the HTTP status code.
    ///
    /// Invalid codes (outside 100-999) are silently ignored.
    pub fn status(&mut self, code: u16) -> &mut Self {
        if let Ok(status) = StatusCode::from_u16(code) {
            self.status = status;
        }
        self
    }

    /// Set a response header.
    pub fn set_header(&mut self, name: &str, value: &str) -> &mut Self {
        if let (Ok(name), Ok(value)) = (
            http::header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            self.headers.insert(name, value);
        }
        self
    }

    /// Send a text response with `Content-Type: text/plain; charset=utf-8`.
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        self.body = Some(Bytes::from(text.to_string()));
        self
    }

    /// Send a JSON response with `Content-Type: application/json`.
    ///
    /// In Phase 1, this expects a pre-serialized JSON string. Automatic
    /// serialization from Rust types will be added in Phase 5.
    pub fn json(&mut self, json: &str) -> &mut Self {
        self.headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        self.body = Some(Bytes::from(json.to_string()));
        self
    }

    /// Set the response body to raw bytes.
    pub fn set_body(&mut self, body: Bytes) -> &mut Self {
        self.body = Some(body);
        self
    }

    /// Get the current status code.
    pub fn get_status(&self) -> StatusCode {
        self.status
    }

    /// Get the response headers.
    pub fn get_headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get the response body.
    pub fn get_body(&self) -> Option<&Bytes> {
        self.body.as_ref()
    }

    /// Convert to a hyper response for writing to the wire.
    pub(crate) fn into_hyper_response(self) -> http::Response<Full<Bytes>> {
        let mut builder = http::Response::builder().status(self.status);
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        let body = self.body.unwrap_or_default();
        builder.body(Full::new(body)).unwrap()
    }
}

impl Default for Response {
    fn default() -> Self {
        Self::new()
    }
}
