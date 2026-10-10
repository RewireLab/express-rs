//! HTTP request abstraction.
//!
//! Provides an idiomatic Rust interface to HTTP request data, inspired by
//! Express's `req` object.

use bytes::Bytes;
use http::{HeaderMap, Method, Uri};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::RwLock;

use crate::body::ParsedBody;

/// An HTTP request.
///
/// Wraps the parsed HTTP request data and provides Express-inspired
/// accessors: method, path, headers, body, query parameters, and route
/// parameters.
pub struct Request {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    query: HashMap<String, String>,
    /// Route parameters extracted from the path (e.g., `:id` → `params["id"]`).
    pub params: HashMap<String, String>,
    /// Body parsed by body-parser middleware, if any ran.
    ///
    /// Express sets `req.body` when a parser runs and leaves it
    /// `undefined` otherwise. The lock holds `None` until a parser
    /// stores a value. Interior mutability is required because handlers
    /// receive the request behind an `Arc`.
    parsed: RwLock<Option<ParsedBody>>,
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
            parsed: RwLock::new(None),
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

    /// Get a request header by name.
    ///
    /// `referer` and `referrer` are treated as interchangeable, matching
    /// the historic misspelling in the HTTP spec that Express honours.
    pub fn header(&self, name: &str) -> Option<&str> {
        let lc = name.to_ascii_lowercase();
        match lc.as_str() {
            "referer" | "referrer" => self
                .headers
                .get("referrer")
                .or_else(|| self.headers.get("referer"))
                .and_then(|v| v.to_str().ok()),
            _ => self.headers.get(&lc).and_then(|v| v.to_str().ok()),
        }
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

    /// Store a parsed body, as body-parser middleware does.
    pub fn set_parsed_body(&self, body: ParsedBody) {
        *self.parsed.write().unwrap() = Some(body);
    }

    /// The parsed body, if a parser ran and the content type matched.
    ///
    /// Returns `None` when no parser ran, the content type did not match,
    /// or the request carried no body — the equivalent of Express's
    /// `req.body` being `undefined`.
    pub fn parsed_body(&self) -> Option<ParsedBody> {
        self.parsed.read().unwrap().clone()
    }

    /// Deserialize a parsed JSON body into `T`.
    ///
    /// Returns `None` when no JSON body was parsed. Returns `Some(Err(..))`
    /// when the value does not fit `T`.
    pub fn body_json<T: DeserializeOwned>(&self) -> Option<Result<T, serde_json::Error>> {
        match self.parsed.read().unwrap().as_ref()? {
            ParsedBody::Json(value) => Some(serde_json::from_value(value.clone())),
            _ => None,
        }
    }

    /// True when the request carries a body.
    ///
    /// Mirrors `type-is`'s `hasBody`: a `Transfer-Encoding` header means a
    /// body may follow, otherwise `Content-Length` must exceed zero.
    pub fn has_body(&self) -> bool {
        if self.headers.contains_key(http::header::TRANSFER_ENCODING) {
            return true;
        }
        self.header("content-length")
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    /// Pick the best match from the offered types against `Accept`.
    ///
    /// Returns `None` when nothing is acceptable, which is the cue to
    /// answer 406. An extension such as `json` is expanded to its MIME
    /// type first, as Express does.
    pub fn accepts(&self, offered: &[&str]) -> Option<String> {
        let accept = self.header("accept").unwrap_or("");

        // Keep the caller's wording alongside its normalised MIME type so
        // the answer reads back as what was offered, like Express.
        let pairs: Vec<(String, String)> = offered
            .iter()
            .map(|t| (t.to_string(), normalize_media(t)))
            .collect();
        let normals: Vec<String> = pairs.iter().map(|(_, n)| n.clone()).collect();

        best_match_owned(accept, &normals).and_then(|chosen| {
            pairs
                .into_iter()
                .find(|(_, n)| *n == chosen)
                .map(|(original, _)| original)
        })
    }

    /// Pick the best match from the offered encodings against
    /// `Accept-Encoding`.
    pub fn accepts_encodings(&self, offered: &[&str]) -> Option<String> {
        best_match(self.header("accept-encoding").unwrap_or(""), offered)
    }

    /// Pick the best match from the offered charsets against
    /// `Accept-Charset`.
    pub fn accepts_charsets(&self, offered: &[&str]) -> Option<String> {
        best_match(self.header("accept-charset").unwrap_or(""), offered)
    }

    /// Pick the best match from the offered languages against
    /// `Accept-Language`.
    pub fn accepts_languages(&self, offered: &[&str]) -> Option<String> {
        best_match(self.header("accept-language").unwrap_or(""), offered)
    }

    /// Check whether the request's `Content-Type` matches any of `types`.
    ///
    /// A `type/*` pattern matches any subtype, and a bare extension is
    /// expanded to its MIME type.
    pub fn is(&self, types: &[&str]) -> Option<String> {
        let content_type = self.header("content-type")?;
        let media = content_type.split(';').next()?.trim().to_lowercase();
        if media.is_empty() {
            return None;
        }

        for t in types {
            let pattern = normalize_media(t).to_lowercase();

            if pattern == media {
                return Some(media);
            }

            if let Some(prefix) = pattern.strip_suffix("/*") {
                if media.starts_with(prefix) && media[prefix.len()..].starts_with('/') {
                    return Some(media);
                }
            }
        }

        None
    }

    /// The `Host` header, port included.
    ///
    /// Express 5 keeps the port; Express 4 stripped it.
    pub fn host(&self) -> Option<&str> {
        self.header("host")
    }

    /// The `Host` header with the port removed, IPv6-literal aware.
    pub fn hostname(&self) -> Option<&str> {
        let host = self.host()?;

        let offset = if host.starts_with('[') {
            host.find(']').map(|i| i + 1).unwrap_or(0)
        } else {
            0
        };

        match host[offset..].find(':') {
            Some(rel) => Some(&host[..offset + rel]),
            None => Some(host),
        }
    }

    /// The protocol, `http` here.
    ///
    /// TLS would report `https`; `X-Forwarded-Proto` is deliberately not
    /// trusted without a trust-proxy setting, which express-rs does not
    /// implement yet.
    pub fn protocol(&self) -> &'static str {
        "http"
    }

    /// Shorthand for `protocol() == "https"`.
    pub fn secure(&self) -> bool {
        self.protocol() == "https"
    }

    /// True when the request carries `X-Requested-With: XMLHttpRequest`.
    pub fn xhr(&self) -> bool {
        self.header("x-requested-with")
            .map(|v| v.eq_ignore_ascii_case("xmlhttprequest"))
            .unwrap_or(false)
    }
}

/// Expand a bare extension to its MIME type, leaving full types alone.
fn normalize_media(t: &str) -> String {
    if t.contains('/') {
        return t.to_string();
    }
    crate::mime::content_type(t)
        .map(|ct| ct.split(';').next().unwrap_or(t).trim().to_string())
        .unwrap_or_else(|| t.to_string())
}

/// Choose the highest-quality acceptable entry from a header.
fn best_match(header: &str, offered: &[&str]) -> Option<String> {
    let owned: Vec<String> = offered.iter().map(|s| s.to_string()).collect();
    best_match_owned(header, &owned)
}

fn best_match_owned(header: &str, offered: &[String]) -> Option<String> {
    let header = header.trim();
    if header.is_empty() {
        return offered.first().map(|s| s.to_string());
    }

    let mut best: Option<(f64, String)> = None;

    for entry in header.split(',') {
        let mut parts = entry.split(';');
        let media = parts.next()?.trim().to_lowercase();
        if media.is_empty() {
            continue;
        }

        let mut quality = 1.0f64;
        for param in parts {
            let param = param.trim();
            if let Some(q) = param.strip_prefix("q=") {
                quality = q.trim().parse().unwrap_or(1.0);
            }
        }
        if quality <= 0.0 {
            continue;
        }

        for candidate in offered {
            let candidate_lc = candidate.to_lowercase();
            let matches = media == candidate_lc
                || media == "*/*"
                || media
                    .strip_suffix("/*")
                    .map(|prefix| {
                        candidate_lc.starts_with(prefix)
                            && candidate_lc[prefix.len()..].starts_with('/')
                    })
                    .unwrap_or(false);

            if !matches {
                continue;
            }

            // Prefer a higher quality, then a more specific pattern.
            let specificity = if media == candidate_lc { 1 } else { 0 };
            let replace = match &best {
                None => true,
                Some((bq, bmedia)) => {
                    quality > *bq || (quality == *bq && specificity == 1 && bmedia != &candidate_lc)
                }
            };

            if replace {
                best = Some((quality, candidate.to_string()));
            }
        }
    }

    best.map(|(_, m)| m)
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

/// Percent-decode a query value (e.g., `%20` → ` `, `+` → ` `).
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

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderName;

    fn req(host: Option<&str>) -> Request {
        let mut headers = HeaderMap::new();
        if let Some(h) = host {
            headers.insert("host", h.parse().unwrap());
        }
        Request::new(Method::GET, "/".parse().unwrap(), headers, Bytes::new())
    }

    fn req_with(headers: &[(&str, &str)]) -> Request {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            if let (Ok(name), Ok(val)) = (HeaderName::from_bytes(k.as_bytes()), v.parse()) {
                map.insert(name, val);
            }
        }
        Request::new(Method::GET, "/".parse().unwrap(), map, Bytes::new())
    }

    #[test]
    fn host_keeps_the_port() {
        assert_eq!(
            req(Some("example.com:3000")).host(),
            Some("example.com:3000")
        );
    }

    #[test]
    fn hostname_strips_the_port() {
        assert_eq!(
            req(Some("example.com:3000")).hostname(),
            Some("example.com")
        );
        assert_eq!(req(Some("example.com")).hostname(), Some("example.com"));
    }

    #[test]
    fn hostname_handles_ipv6_literal() {
        assert_eq!(req(Some("[::1]:8080")).hostname(), Some("[::1]"));
    }

    #[test]
    fn referer_and_referrer_are_interchangeable() {
        assert_eq!(
            req_with(&[("referer", "http://x/")]).header("referrer"),
            Some("http://x/")
        );
        assert_eq!(
            req_with(&[("referrer", "http://x/")]).header("referer"),
            Some("http://x/")
        );
    }

    #[test]
    fn xhr_detects_the_header() {
        assert!(req_with(&[("x-requested-with", "XMLHttpRequest")]).xhr());
        assert!(!req_with(&[("x-requested-with", "fetch")]).xhr());
        assert!(!req(None).xhr());
    }

    #[test]
    fn accepts_expands_extensions() {
        let r = req_with(&[("accept", "application/json")]);
        assert_eq!(r.accepts(&["json"]), Some("json".to_string()));
    }

    #[test]
    fn accepts_returns_none_when_unacceptable() {
        let r = req_with(&[("accept", "text/html")]);
        assert_eq!(r.accepts(&["json"]), None);
    }

    #[test]
    fn accepts_honours_quality() {
        let r = req_with(&[("accept", "text/html;q=0.5, application/json;q=0.9")]);
        assert_eq!(r.accepts(&["json", "html"]), Some("json".to_string()));
    }

    #[test]
    fn accepts_defaults_without_header() {
        assert_eq!(req(None).accepts(&["json"]), Some("json".to_string()));
    }

    #[test]
    fn accepts_wildcard() {
        let r = req_with(&[("accept", "*/*")]);
        assert_eq!(r.accepts(&["json", "html"]), Some("json".to_string()));
    }

    #[test]
    fn is_matches_full_type_and_extension() {
        let r = req_with(&[("content-type", "application/json; charset=utf-8")]);
        assert_eq!(r.is(&["json"]), Some("application/json".to_string()));
        assert_eq!(
            r.is(&["application/json"]),
            Some("application/json".to_string())
        );
        assert_eq!(r.is(&["html"]), None);
    }

    #[test]
    fn is_matches_subtype_wildcard() {
        let r = req_with(&[("content-type", "application/vnd.api+json")]);
        assert_eq!(
            r.is(&["application/*"]),
            Some("application/vnd.api+json".to_string())
        );
    }

    #[test]
    fn protocol_is_http_without_tls() {
        assert_eq!(req(None).protocol(), "http");
        assert!(!req(None).secure());
    }
}
