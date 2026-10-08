//! HTTP request abstraction.
//!
//! Provides an idiomatic Rust interface to HTTP request data, inspired by
//! Express's `req` object. This is the Phase 1 version with basic data
//! access; route parameters and body parsing come in later phases.

use bytes::Bytes;
use http::{HeaderMap, Method, Uri};
use std::collections::HashMap;

/// An HTTP request.
///
/// Wraps the parsed HTTP request data and provides Express-inspired
/// accessors. In Phase 1, this exposes method, path, headers, body, and
/// query parameters. Route parameters (`req.params`) and body parsing
/// (`req.body`) will be added in later phases.
pub struct Request {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    query: HashMap<String, String>,
    /// Route parameters extracted from the path (e.g., `:id` → `params["id"]`).
    pub params: HashMap<String, String>,
}

impl Request {
    /// Create a new request from raw parts.
    pub fn new(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Self {
        let query = parse_query(&uri);
        Self {
            method,
            uri,
            headers,
            body,
            query,
            params: HashMap::new(),
        }
    }

    /// The HTTP method (GET, POST, etc.).
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The request path (without query string).
    pub fn path(&self) -> &str {
        self.uri.path()
    }

    /// The full request URI.
    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    /// All request headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a header value by name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// The raw request body.
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// The request body as a UTF-8 string (lossy).
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// Parsed query parameters.
    pub fn query(&self) -> &HashMap<String, String> {
        &self.query
    }

    /// Get a query parameter by name.
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(|s| s.as_str())
    }

    /// Get a route parameter by name.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(|s| s.as_str())
    }
}

/// Parse query parameters from a URI.
fn parse_query(uri: &Uri) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or("");
            let value = parts.next().unwrap_or("");
            if !key.is_empty() {
                map.insert(percent_decode(key), percent_decode(value));
            }
        }
    }
    map
}

/// Percent-decode a string (e.g., `%20` → ` `, `+` → ` `).
fn percent_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h = chars.next();
            let l = chars.next();
            if let (Some(h), Some(l)) = (h, l) {
                let hex = [h, l];
                if let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&hex).unwrap_or(""), 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            result.push('%');
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}
