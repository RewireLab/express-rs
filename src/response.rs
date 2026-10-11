//! HTTP response abstraction.
//!
//! Mirrors the parts of Express's `res` object that make sense in Rust:
//! status, header manipulation, content type, `send()`, `json()`,
//! `sendStatus()`, `location()`, `redirect()`, `vary()`, `cookie()`,
//! and the `res.format()` content negotiation.
//!
//! Unlike Express, the handler takes ownership of the `Response`, mutates
//! it, and returns it. Borrowing a `&mut Response` across an `await` point
//! is what this avoids.

use crate::cookies::{self, CookieOptions};
use crate::error::Error;
use crate::etag;
use crate::mime;
use crate::request::Request;
use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use http_body_util::Full;
use serde::Serialize;

/// An HTTP response under construction.
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Option<Bytes>,
    /// The request's `Accept` header, captured by the server so that
    /// `redirect()` can negotiate a body the way Express reaches `req`.
    accept: Option<String>,
    /// Set for HEAD requests: the length the body *would* have been.
    ///
    /// hyper derives framing from the body it is handed, so simply
    /// dropping the body would also drop the `Content-Length` header.
    /// This preserves the length while sending no bytes, which is what
    /// Express does for HEAD.
    head_length: Option<u64>,
}

impl Response {
    /// Create a new response with default status 200 OK.
    pub fn new() -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: None,
            accept: None,
            head_length: None,
        }
    }

    /// Record the request's `Accept` header for content negotiation.
    pub(crate) fn with_accept(mut self, accept: Option<&str>) -> Self {
        self.accept = accept.map(|s| s.to_string());
        self
    }

    /// Set the HTTP status code.
    ///
    /// Codes outside Node's valid range of 100-999 are ignored rather than
    /// throwing, because a handler here returns the `Response` rather than
    /// a `Result` and so has no channel to report the failure.
    pub fn status(&mut self, code: u16) -> &mut Self {
        if let Ok(status) = StatusCode::from_u16(code) {
            self.status = status;
        }
        self
    }

    /// Set a response header.
    ///
    /// A `Content-Type` value is normalised the way `mime-types` does it,
    /// so a bare extension or a charset-less type gains its default
    /// charset.
    pub fn set(&mut self, name: &str, value: &str) -> &mut Self {
        let value = if name.eq_ignore_ascii_case("content-type") {
            mime::content_type(value).unwrap_or_else(|| value.to_string())
        } else {
            value.to_string()
        };

        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.headers.insert(name, value);
        }
        self
    }

    /// Get a response header.
    pub fn get(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    /// Append a value to a response header.
    ///
    /// Existing values are comma-joined, which is what headers such as
    /// `Vary` and `Link` expect. `Set-Cookie` would need separate header
    /// instances and is not handled here.
    pub fn append(&mut self, name: &str, value: &str) -> &mut Self {
        let combined = match self.get(name) {
            Some(prev) if !prev.is_empty() => format!("{}, {}", prev, value),
            _ => value.to_string(),
        };
        self.set(name, &combined)
    }

    /// Remove a response header.
    pub fn remove_header(&mut self, name: &str) -> &mut Self {
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
            self.headers.remove(name);
        }
        self
    }

    /// Set the `Content-Type` from an extension or a full type.
    ///
    /// Aliased as `content_type` to avoid the bare `type` name, which
    /// reads poorly in Rust.
    pub fn content_type(&mut self, value: &str) -> &mut Self {
        let resolved = if value.contains('/') {
            value.to_string()
        } else {
            mime::content_type(value).unwrap_or_else(|| "application/octet-stream".to_string())
        };

        let name = HeaderName::from_static("content-type");
        if let Ok(v) = HeaderValue::from_str(&resolved) {
            self.headers.insert(name, v);
        }
        self
    }

    /// Send a string body.
    ///
    /// Like Express, a string defaults to `text/html` with a UTF-8
    /// charset unless a `Content-Type` is already set, in which case the
    /// charset is applied to whatever is there.
    pub fn send(&mut self, body: &str) -> &mut Self {
        match self
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
        {
            Some(existing) => {
                let with_charset = mime::set_charset(&existing, "utf-8");
                self.set("content-type", &with_charset);
            }
            None => {
                self.content_type("html");
            }
        }
        self.body = Some(Bytes::from(body.to_string()));
        self
    }

    /// Send a binary body.
    ///
    /// A `Content-Type` is only defaulted when none is set, matching how
    /// Express treats a Buffer.
    pub fn send_bytes(&mut self, body: &[u8]) -> &mut Self {
        if !self.headers.contains_key("content-type") {
            self.content_type("bin");
        }
        self.body = Some(Bytes::from(body.to_vec()));
        self
    }

    /// Send a value as JSON.
    ///
    /// Sets `application/json` unless a `Content-Type` is already set,
    /// exactly as Express does.
    pub fn json<T: Serialize>(&mut self, value: &T) -> &mut Self {
        match serde_json::to_string(value) {
            Ok(text) => self.json_str(&text),
            // A serialization failure leaves the body empty; Express has
            // no equivalent because JSON.stringify does not fail.
            Err(_) => {
                self.body = Some(Bytes::new());
                self
            }
        }
    }

    /// Send an already-serialised JSON document.
    pub fn json_str(&mut self, raw: &str) -> &mut Self {
        if !self.headers.contains_key("content-type") {
            self.set("content-type", "application/json");
        }
        self.send(raw)
    }

    /// Set the status and send the standard reason phrase as text.
    pub fn send_status(&mut self, code: u16) -> &mut Self {
        self.status(code);
        let message = reason_phrase(code);
        self.content_type("txt");
        self.send(&message)
    }

    /// Set the `Location` header.
    ///
    /// The URL is percent-encoded the way `encodeURI` does, leaving
    /// reserved URI characters intact.
    pub fn location(&mut self, url: &str) -> &mut Self {
        self.set("location", &encode_uri(url))
    }

    /// Redirect to `url` with `status`.
    ///
    /// Express 5 uses `redirect(status, url)`; the legacy
    /// `redirect(url, status)` order is gone. The body is negotiated
    /// against the request's `Accept` header.
    pub fn redirect(&mut self, status: u16, url: &str) -> &mut Self {
        self.status(status);
        self.location(url);

        let address = self.get("location").unwrap_or_default();
        let accept = self.accept.clone().unwrap_or_default();

        let wants_html = accepts_type(&accept, "text/html");
        let wants_plain = wants_html || accepts_type(&accept, "text/plain");

        let body = if wants_html {
            format!(
                "<!DOCTYPE html><head><title>{}</title></head><body><p>{}. Redirecting to {}</p></body>",
                reason_phrase(status),
                reason_phrase(status),
                escape_html(&address)
            )
        } else if wants_plain {
            format!("{}. Redirecting to {}", reason_phrase(status), address)
        } else {
            String::new()
        };

        self.content_type(if wants_html { "html" } else { "txt" });
        self.set("content-length", &body.len().to_string());
        self.body = Some(Bytes::from(body));
        self
    }

    /// Set a signed cookie, as `res.cookie(name, value, { signed: true })`.
    ///
    /// The value goes out as `s:<encoded>.<signature>`, HMAC-SHA256
    /// over the still-encoded value in the cookie-signature format.
    /// The secret rides in the call rather than on the app, because a
    /// `Response` cannot see the request the way Express's `res`
    /// reaches `req.secret`. Verify on the way in with
    /// [`Request::signed_cookies`](crate::Request::signed_cookies).
    pub fn cookie_signed(
        &mut self,
        name: &str,
        value: &str,
        secret: &str,
        options: &CookieOptions,
    ) -> &mut Self {
        let encoded = cookies::encode_component(value);
        let signed = format!("s:{}.{}", encoded, cookies::sign(&encoded, secret));
        if let Some(field) = cookies::serialize(name, &signed, options) {
            if let Ok(v) = HeaderValue::from_str(&field) {
                self.headers
                    .append(HeaderName::from_static("set-cookie"), v);
            }
        }
        self
    }

    /// Mark the response as an attachment, as `res.attachment()`.
    ///
    /// With a filename, `Content-Disposition` carries it and the
    /// `Content-Type` is taken from the extension (octet-stream when
    /// there is none). A later `send()` supplies the body.
    pub fn attachment(&mut self, filename: Option<&str>) -> &mut Self {
        match filename {
            Some(filename) => {
                let ext = match filename.rsplit_once('.') {
                    Some((prefix, ext)) if !prefix.is_empty() => ext,
                    _ => "",
                };
                self.content_type(ext);
                let disposition = content_disposition(filename);
                self.set("content-disposition", &disposition);
            }
            None => {
                self.set("content-disposition", "attachment");
            }
        }
        self
    }

    /// Set a cookie on the response.
    ///
    /// The value is percent-encoded the way Express's default
    /// `encodeURIComponent` encoder is. Attributes come from
    /// [`CookieOptions`], whose `Path` defaults to `/`. Several cookies
    /// can be set; each becomes its own `Set-Cookie` header, which is
    /// why this does not go through `append()` (it comma-joins).
    ///
    /// A name with characters a cookie cannot carry is skipped instead
    /// of throwing the `TypeError` Express throws.
    pub fn cookie(&mut self, name: &str, value: &str, options: &CookieOptions) -> &mut Self {
        let encoded = cookies::encode_component(value);
        if let Some(field) = cookies::serialize(name, &encoded, options) {
            if let Ok(v) = HeaderValue::from_str(&field) {
                self.headers
                    .append(HeaderName::from_static("set-cookie"), v);
            }
        }
        self
    }

    /// Clear a cookie by sending it expired with an empty value.
    ///
    /// `path` and `domain` must match those the cookie was set with,
    /// so they pass through from `options`. The epoch expiry Express
    /// forces is applied here; a caller-supplied `max_age` is dropped
    /// because a future lifetime cannot clear anything.
    pub fn clear_cookie(&mut self, name: &str, options: &CookieOptions) -> &mut Self {
        self.cookie(name, "", &cookies::clear_options(options))
    }

    /// Add `field` to the `Vary` header if it is not already there.
    pub fn vary(&mut self, field: &str) -> &mut Self {
        if field.is_empty() {
            return self;
        }

        let already = self
            .get("vary")
            .map(|v| v.split(',').any(|f| f.trim().eq_ignore_ascii_case(field)))
            .unwrap_or(false);

        if !already {
            self.append("vary", field);
        }
        self
    }

    /// Apply the finalisation steps Express performs inside `send()`.
    ///
    /// Runs after the handler returns, because the response has no access
    /// to the request method or headers on its own. The order follows
    /// Express: content length, ETag, freshness, body stripping, then HEAD
    /// suppression.
    pub(crate) fn finish(&mut self, req: &Request) {
        // Content-Length, unless the caller manages framing themselves.
        let has_transfer_encoding = self
            .headers
            .get("transfer-encoding")
            .is_some_and(|value| !value.is_empty());
        let generating_etag = !self.headers.contains_key("etag");

        let body_len = self.body.as_ref().map(|b| b.len());
        if !has_transfer_encoding {
            if let Some(len) = body_len {
                self.set("content-length", &len.to_string());
            }
        }

        // ETag, weak by default like Express's `etag: weak` setting.
        if generating_etag {
            if let Some(bytes) = self.body.as_ref() {
                let _ = bytes.len();
                self.set("etag", &etag::weak(bytes));
            }
        }

        // Freshness: a matching conditional request becomes a 304.
        if self.is_fresh(req) && self.status.is_success() {
            self.status = StatusCode::NOT_MODIFIED;
        }

        // 204 and 304 carry no body or content headers.
        if self.status == StatusCode::NO_CONTENT || self.status == StatusCode::NOT_MODIFIED {
            self.headers.remove("content-type");
            self.headers.remove("content-length");
            self.headers.remove("transfer-encoding");
            self.body = None;
        }

        // 205 resets the representation.
        if self.status == StatusCode::RESET_CONTENT {
            self.set("content-length", "0");
            self.headers.remove("transfer-encoding");
            self.body = None;
        }

        // HEAD keeps the headers but sends no body.
        if req.method() == http::Method::HEAD {
            self.head_length = Some(self.body.as_ref().map(|b| b.len() as u64).unwrap_or(0));
            self.body = None;
        }
    }

    /// Express's `req.fresh`: the request is conditionally satisfied.
    fn is_fresh(&self, req: &Request) -> bool {
        // Only safe, cacheable methods participate.
        match *req.method() {
            http::Method::GET | http::Method::HEAD => {}
            _ => return false,
        }

        let if_none_match = req.header("if-none-match");
        let if_modified_since = req.header("if-modified-since");
        if if_none_match.is_none() && if_modified_since.is_none() {
            return false;
        }

        if req
            .header("cache-control")
            .map(|v| v.contains("no-cache"))
            .unwrap_or(false)
        {
            return false;
        }

        if let Some(inm) = if_none_match {
            if inm.trim() != "*" {
                match self.get("etag") {
                    Some(tag) if etag::matches(&tag, inm) => {}
                    _ => return false,
                }
            }
        }

        if let Some(ims) = if_modified_since {
            let fresh = match (
                self.get("last-modified").as_deref(),
                httpdate::parse_http_date(ims),
            ) {
                (Some(lm), Ok(since)) => match httpdate::parse_http_date(lm) {
                    // Whole-second precision: sub-second file mtimes
                    // would otherwise always read as modified.
                    Ok(modified) => secs(modified) <= secs(since),
                    Err(_) => false,
                },
                _ => false,
            };
            if !fresh {
                return false;
            }
        }

        true
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

    /// Set the response body bytes directly, leaving headers untouched.
    pub fn set_body(&mut self, body: Bytes) -> &mut Self {
        self.body = Some(body);
        self
    }

    /// Build the default response for an error nothing handled.
    ///
    /// Express's `finalhandler` honours `err.status`/`err.statusCode`,
    /// sends the message as plain text, and adds the hardening headers
    /// that keep a browser from rendering an error page as HTML.
    pub(crate) fn internal_error(err: &Error, req: &Request) -> Self {
        let mut res = Self::new();
        res.status(err.status());

        // Only 4xx and 5xx are honoured; anything else is a 500.
        if !(400..600).contains(&err.status()) {
            res.status(500);
        }

        res.headers.insert(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("default-src 'none'"),
        );
        res.headers.insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );

        let body = err.message();
        res.set("content-type", "text/plain");
        res.set("content-length", &body.len().to_string());
        res.body = Some(Bytes::from(body));

        // HEAD keeps the headers and drops the body.
        if req.method() == http::Method::HEAD {
            res.head_length = Some(res.body.as_ref().map(|b| b.len() as u64).unwrap_or(0));
            res.body = None;
        }

        res
    }

    /// Convert to a hyper response for writing to the wire.
    pub(crate) fn into_hyper_response(self) -> http::Response<ResponseBody> {
        let mut builder = http::Response::builder().status(self.status);
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }

        let body = match (self.head_length, self.body) {
            // HEAD: report the length, send nothing.
            (Some(len), _) => ResponseBody::Head(HeadBody { len }),
            (None, Some(bytes)) => ResponseBody::Full(Full::new(bytes)),
            (None, None) => ResponseBody::Full(Full::new(Bytes::new())),
        };

        builder.body(body).unwrap()
    }
}

/// The body of an outgoing response.
pub(crate) enum ResponseBody {
    Full(Full<Bytes>),
    Head(HeadBody),
}

impl http_body::Body for ResponseBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match &mut *self {
            ResponseBody::Full(b) => std::pin::Pin::new(b).poll_frame(cx),
            ResponseBody::Head(b) => std::pin::Pin::new(b).poll_frame(cx),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            ResponseBody::Full(b) => b.is_end_stream(),
            ResponseBody::Head(b) => b.is_end_stream(),
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            ResponseBody::Full(b) => b.size_hint(),
            ResponseBody::Head(b) => b.size_hint(),
        }
    }
}

/// A body that yields no data but reports a known length.
///
/// Used for HEAD responses so that `Content-Length` still describes the
/// representation the client would have received.
pub(crate) struct HeadBody {
    len: u64,
}

impl http_body::Body for HeadBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        true
    }

    fn size_hint(&self) -> http_body::SizeHint {
        let mut hint = http_body::SizeHint::default();
        hint.set_exact(self.len);
        hint
    }
}

impl Default for Response {
    fn default() -> Self {
        Self::new()
    }
}

/// A body writer for one offered type, as an object value in
/// Express's `res.format()`.
pub type FormatHandler = Box<dyn FnOnce(Response) -> Result<Response, Error> + Send>;

/// Collects the offered type handlers for [`format`].
pub struct FormatBuilder {
    handlers: Vec<(String, FormatHandler)>,
    fallback: Option<FormatHandler>,
}

impl FormatBuilder {
    fn new() -> Self {
        Self {
            handlers: Vec::new(),
            fallback: None,
        }
    }

    /// Register a handler for a type or an extension, as a key in
    /// Express's `res.format` object.
    pub fn on(
        mut self,
        type_: &str,
        handler: impl FnOnce(Response) -> Result<Response, Error> + Send + 'static,
    ) -> Self {
        self.handlers.push((type_.to_string(), Box::new(handler)));
        self
    }

    /// Register the fallback handler, as the `default` key.
    pub fn default(
        mut self,
        handler: impl FnOnce(Response) -> Result<Response, Error> + Send + 'static,
    ) -> Self {
        self.fallback = Some(Box::new(handler));
        self
    }
}

/// Negotiate the body against the request's `Accept` header, as
/// `res.format()`.
///
/// The registration closure receives the builder; each `on` key is a
/// full type or an extension, and `default` runs when nothing matches.
/// `Vary: Accept` is set before any handler runs, exactly as Express
/// sets it before consulting the header. With no match and no default
/// the returned error is the 406 Express forwards to `next(err)`:
///
/// ```ignore
/// response::format(&req, res, |f| {
///     f.on("json", |res| Ok(res.json(&todo)))
///      .on("html", |res| Ok(res.send("<ul>...</ul>")))
///      .default(|res| Ok(res.send("plain fallback")))
/// })?;
/// ```
pub fn format<F>(req: &Request, res: Response, register: F) -> Result<Response, Error>
where
    F: FnOnce(FormatBuilder) -> FormatBuilder,
{
    let builder = register(FormatBuilder::new());
    let keys: Vec<&str> = builder.handlers.iter().map(|(k, _)| k.as_str()).collect();

    let mut res = res;
    res.vary("Accept");

    let chosen = if keys.is_empty() {
        None
    } else {
        req.accepts(&keys)
    };

    match chosen {
        Some(key) => {
            res.set("content-type", &normalize_type(&key));
            let (_, handler) = builder
                .handlers
                .into_iter()
                .find(|(k, _)| k == &key)
                .ok_or_else(|| Error::with_status(406, "Not Acceptable"))?;
            handler(res)
        }
        None => match builder.fallback {
            Some(handler) => handler(res),
            None => Err(Error::with_status(406, "Not Acceptable")),
        },
    }
}

/// Expand a key the way Express's `normalizeType` does: an extension
/// becomes its MIME type, and parameters are stripped so the handler
/// starts from a bare type.
fn normalize_type(value: &str) -> String {
    let expanded = if value.contains('/') {
        value.to_string()
    } else {
        mime::content_type(value).unwrap_or_else(|| value.to_string())
    };
    expanded
        .split(';')
        .next()
        .unwrap_or(&expanded)
        .trim()
        .to_string()
}

/// The standard reason phrase for a status code.
///
/// Falls back to the numeric code, as Express's `statuses` package does.
fn reason_phrase(code: u16) -> String {
    match code {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        418 => "I'm a Teapot",
        422 => "Unprocessable Entity",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => return code.to_string(),
    }
    .to_string()
}

/// Percent-encode a URL the way `encodeURI` does, leaving the reserved
/// URI characters alone.
fn encode_uri(url: &str) -> String {
    const UNRESERVED: &str = "-_.!~*'()";
    let mut out = String::with_capacity(url.len());

    for byte in url.bytes() {
        let c = byte as char;
        if c.is_ascii_alphanumeric() || UNRESERVED.contains(c) || ";,/?:@&=+$#".contains(c) {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", byte));
        }
    }
    out
}

/// Escape the four characters that matter in HTML text and attributes.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The whole-second part of a timestamp, for fresh comparisons.
fn secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build a `Content-Disposition` value.
///
/// A subset of the `content-disposition` package: the name is quoted
/// with `"` and `\` escaped, and a name carrying characters outside
/// ISO-8859-1 gains a `filename*` parameter percent-encoded the way
/// `encodeURIComponent` does, with a `_`-substituted fallback.
pub(crate) fn content_disposition(filename: &str) -> String {
    let latin1: String = filename
        .chars()
        .map(|c| if (c as u32) <= 0xFF { c } else { '_' })
        .collect();
    let quoted = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));

    if latin1 == filename {
        format!("attachment; filename={}", quoted(filename))
    } else {
        format!(
            "attachment; filename={}; filename*=UTF-8''{}",
            quoted(&latin1),
            cookies::encode_component(filename)
        )
    }
}

/// True when an `Accept` header admits `wanted`.
///
/// Handles the wildcards and `q` values that matter in practice: an
/// absent or empty header accepts anything, `*/*` accepts anything, and
/// `type/*` matches a subtype of that type.
fn accepts_type(accept: &str, wanted: &str) -> bool {
    let accept = accept.trim();
    if accept.is_empty() {
        return true;
    }

    for entry in accept.split(',') {
        let mut parts = entry.split(';');
        let media = match parts.next() {
            Some(m) => m.trim().to_lowercase(),
            None => continue,
        };
        if media.is_empty() {
            continue;
        }

        // A zero quality excludes the entry outright.
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

        if media == wanted || media == "*/*" {
            return true;
        }
        if let Some(prefix) = media.strip_suffix("/*") {
            if wanted.starts_with(prefix) && wanted[prefix.len()..].starts_with('/') {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_phrases_fall_back_to_number() {
        assert_eq!(reason_phrase(302), "Found");
        assert_eq!(reason_phrase(599), "599");
    }

    #[test]
    fn encode_uri_leaves_reserved_characters() {
        assert_eq!(encode_uri("/a/b?c=d"), "/a/b?c=d");
        assert_eq!(encode_uri("http://x.com/p"), "http://x.com/p");
    }

    #[test]
    fn encode_uri_escapes_spaces() {
        assert_eq!(encode_uri("/my file"), "/my%20file");
    }

    #[test]
    fn escape_html_handles_the_dangerous_four() {
        assert_eq!(
            escape_html("<a href=\"x\">&"),
            "&lt;a href=&quot;x&quot;&gt;&amp;"
        );
    }

    #[test]
    fn empty_accept_accepts_everything() {
        assert!(accepts_type("", "text/html"));
    }

    #[test]
    fn wildcard_accepts_everything() {
        assert!(accepts_type("*/*", "application/json"));
    }

    #[test]
    fn subtype_wildcard_matches() {
        assert!(accepts_type("text/*", "text/html"));
        assert!(!accepts_type("text/*", "application/json"));
    }

    #[test]
    fn exact_type_matches() {
        assert!(accepts_type("text/html", "text/html"));
        assert!(!accepts_type("text/html", "text/plain"));
    }

    #[test]
    fn zero_quality_excludes() {
        assert!(!accepts_type("text/html;q=0", "text/html"));
    }

    #[test]
    fn content_disposition_quotes_ascii_names() {
        assert_eq!(
            content_disposition("report.txt"),
            "attachment; filename=\"report.txt\""
        );
    }

    #[test]
    fn content_disposition_escapes_quotes_and_backslashes() {
        assert_eq!(
            content_disposition("a\"b\\c.txt"),
            "attachment; filename=\"a\\\"b\\\\c.txt\""
        );
    }

    #[test]
    fn content_disposition_falls_back_for_non_latin1() {
        assert_eq!(
            content_disposition("\u{30d5}\u{30a1}\u{30a4}\u{30eb}.txt"),
            "attachment; filename=\"____.txt\"; filename*=UTF-8''%E3%83%95%E3%82%A1%E3%82%A4%E3%83%AB.txt"
        );
    }

    #[test]
    fn content_disposition_keeps_latin1_in_the_quoted_name() {
        assert_eq!(
            content_disposition("caf\u{e9}.txt"),
            "attachment; filename=\"caf\u{e9}.txt\""
        );
    }

    #[test]
    fn normalize_type_expands_extensions_without_params() {
        assert_eq!(normalize_type("json"), "application/json");
        assert_eq!(normalize_type("html"), "text/html");
        assert_eq!(normalize_type("png"), "image/png");
    }

    #[test]
    fn normalize_type_strips_params_from_full_types() {
        assert_eq!(normalize_type("application/json"), "application/json");
        assert_eq!(normalize_type("text/html; charset=iso-8859-1"), "text/html");
    }

    #[test]
    fn normalize_type_keeps_unknown_extensions() {
        assert_eq!(normalize_type("nope"), "nope");
    }
}
