//! Static file middleware, as `express.static()`.
//!
//! Serves files from a root directory with `Content-Type` lookup,
//! `ETag` and `Last-Modified` caching, conditional requests, single
//! range requests, index files, and trailing-slash redirects. Path
//! traversal is blocked, including through symlinks.
//!
//! Register it with `app.use`, giving the mount point to the factory so
//! it can strip its own prefix:
//!
//! ```ignore
//! app.use(static_files::serve_static_at(
//!     "/assets",
//!     "./public",
//!     StaticOptions::default(),
//! ));
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::body::BodyFuture;
use crate::error::Error;
use crate::etag;
use crate::mime;
use crate::request::Request;
use crate::response::Response;
use crate::router::{Next, NextCall, Params};

/// How to treat files or directories whose name starts with a dot.
///
/// Express 5 defaults to ignoring them, including dotfiles nested in
/// hidden directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dotfiles {
    /// Pretend the file does not exist.
    #[default]
    Ignore,
    /// Serve the file.
    Allow,
    /// Answer 403.
    Deny,
}

/// Extra headers applied to every served file, as `setHeaders`.
pub type HeaderHook = Arc<dyn Fn(&mut Response, &FileInfo) + Send + Sync>;

/// Metadata about a file about to be served.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// The resolved filesystem path.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Last modification time, when the platform reports one.
    pub modified: Option<SystemTime>,
}

/// Options for static file serving.
#[derive(Clone)]
pub struct StaticOptions {
    /// Serve `index.html` (or the named files) for directories.
    ///
    /// An empty list disables index files, as `index: false` does.
    /// Defaults to `["index.html"]`.
    pub index: Vec<String>,
    /// Hidden file policy. Defaults to ignoring them.
    pub dotfiles: Dotfiles,
    /// `Cache-Control` lifetime in seconds. Defaults to 0.
    ///
    /// Express accepts milliseconds or strings like `"1d"`; this takes
    /// seconds directly.
    pub max_age_secs: u64,
    /// Append `immutable` to `Cache-Control`. Defaults to false.
    pub immutable: bool,
    /// Send `ETag`. Defaults to true.
    pub etag: bool,
    /// Send `Last-Modified`. Defaults to true.
    pub last_modified: bool,
    /// Send `Accept-Ranges`. Defaults to true.
    pub accept_ranges: bool,
    /// Send `Cache-Control`. Defaults to true.
    pub cache_control: bool,
    /// Redirect bare directories to a trailing slash. Defaults to true.
    pub redirect: bool,
    /// Call `next()` on a miss instead of answering 404. Defaults to true.
    pub fallthrough: bool,
    /// Extensions to try when the path has none. Defaults to empty.
    pub extensions: Vec<String>,
    /// Hook run after the default headers are set.
    pub on_headers: Option<HeaderHook>,
}

impl Default for StaticOptions {
    fn default() -> Self {
        Self {
            index: vec!["index.html".to_string()],
            dotfiles: Dotfiles::Ignore,
            max_age_secs: 0,
            immutable: false,
            etag: true,
            last_modified: true,
            accept_ranges: true,
            cache_control: true,
            redirect: true,
            fallthrough: true,
            extensions: Vec::new(),
            on_headers: None,
        }
    }
}

impl StaticOptions {
    /// Options with the default `index.html` file, as `send` uses.
    pub fn with_index() -> Self {
        Self {
            index: vec!["index.html".to_string()],
            ..Self::default()
        }
    }
}

/// Serve files from `root` at the `/` mount point.
pub fn serve_static(
    root: impl Into<PathBuf>,
) -> impl Fn(Arc<Request>, Response, Next, Params) -> BodyFuture + Send + Sync + 'static {
    serve_static_at("/", root, StaticOptions::with_index())
}

/// Serve files from `root` under `mount` with `options`.
///
/// The middleware strips `mount` from the request path itself, so
/// register it with plain `app.use`.
pub fn serve_static_at(
    mount: &str,
    root: impl Into<PathBuf>,
    options: StaticOptions,
) -> impl Fn(Arc<Request>, Response, Next, Params) -> BodyFuture + Send + Sync + 'static {
    let mount = normalize_mount(mount);
    let root = root.into();
    let options = options;

    move |req, res, next, _params| {
        let mount = mount.clone();
        let root = root.clone();
        let options = options.clone();
        Box::pin(async move { serve(&req, res, &next, &mount, &root, &options).await })
    }
}

/// Normalize a mount point: empty becomes `/`, trailing slashes go.
fn normalize_mount(mount: &str) -> String {
    let trimmed = mount.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Answer one request from the static middleware.
async fn serve(
    req: &Request,
    mut res: Response,
    next: &Next,
    mount: &str,
    root: &Path,
    options: &StaticOptions,
) -> Result<Response, Error> {
    // Only GET and HEAD are served. Anything else falls through, or is a
    // 405 when fallthrough is off — exactly as serve-static does.
    if *req.method() != http::Method::GET && *req.method() != http::Method::HEAD {
        if options.fallthrough {
            next(NextCall::Continue).await;
            return Ok(res);
        }
        res.status(405);
        res.set("allow", "GET, HEAD");
        return Ok(res);
    }

    // Strip the mount point. Anything outside it is not ours.
    let remainder = match strip_mount(req.path(), mount) {
        Some(r) => r,
        None => {
            next(NextCall::Continue).await;
            return Ok(res);
        }
    };

    // Decode percent escapes. Failure is a 400, as in send.
    let decoded = match percent_decode_path(&remainder) {
        Some(d) => d,
        None => return Err(Error::with_status(400, "failed to decode path")),
    };
    if decoded.contains('\0') {
        return Err(Error::with_status(400, "invalid path"));
    }

    let segments: Vec<&str> = decoded.split('/').filter(|s| !s.is_empty()).collect();

    // A `..` segment is a traversal attempt: 403, as send answers.
    if segments.contains(&"..") {
        return Err(Error::with_status(403, "forbidden"));
    }

    // Hidden files or directories: allow, deny with 403, or ignore.
    if segments.iter().any(|s| s.len() > 1 && s.starts_with('.')) {
        match options.dotfiles {
            Dotfiles::Allow => {}
            Dotfiles::Deny => return Err(Error::with_status(403, "forbidden")),
            Dotfiles::Ignore => return miss(req, res, next, options).await,
        }
    }

    // Resolve under the root and confine symlinks to it.
    //
    // send only checks `..` lexically and follows symlinks wherever they
    // lead. Confining the canonical path is stricter and safer; a symlink
    // pointing outside answers 404 here instead of leaking the target.
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|_| Error::with_status(404, "not found"))?;
    let mut joined = canonical_root.clone();
    for seg in &segments {
        joined.push(seg);
    }
    let resolved = match tokio::fs::canonicalize(&joined).await {
        Ok(p) => p,
        Err(_) => {
            return miss_with_extensions(req, res, next, mount, root, &segments, options).await
        }
    };
    if !resolved.starts_with(&canonical_root) {
        return miss(req, res, next, options).await;
    }

    let meta = match tokio::fs::metadata(&resolved).await {
        Ok(m) => m,
        Err(_) => return miss(req, res, next, options).await,
    };

    if meta.is_dir() {
        return serve_directory(req, res, next, &resolved, &decoded, options).await;
    }
    if !meta.is_file() {
        return miss(req, res, next, options).await;
    }

    // A file addressed with a trailing slash is a 404, as in send.
    if decoded.ends_with('/') && !segments.is_empty() {
        return miss(req, res, next, options).await;
    }

    send_file(req, res, &resolved, &meta, options).await
}

/// Try `path + '.' + ext` fallbacks, then treat the miss normally.
async fn miss_with_extensions(
    req: &Request,
    res: Response,
    next: &Next,
    _mount: &str,
    root: &Path,
    segments: &[&str],
    options: &StaticOptions,
) -> Result<Response, Error> {
    let has_ext = segments.last().map(|s| s.contains('.')).unwrap_or(false);

    if !options.extensions.is_empty() && !has_ext {
        let canonical_root = tokio::fs::canonicalize(root)
            .await
            .map_err(|_| Error::with_status(404, "not found"))?;
        let mut rel = PathBuf::new();
        for seg in segments {
            rel.push(seg);
        }
        for ext in &options.extensions {
            let mut candidate = rel.clone();
            let name = candidate
                .file_name()
                .map(|n| format!("{}.{}", n.to_string_lossy(), ext));
            if let Some(name) = name {
                candidate.set_file_name(name);
            }
            let joined = canonical_root.join(&candidate);
            if let Ok(resolved) = tokio::fs::canonicalize(&joined).await {
                if !resolved.starts_with(&canonical_root) {
                    continue;
                }
                if let Ok(meta) = tokio::fs::metadata(&resolved).await {
                    if meta.is_file() {
                        return send_file(req, res, &resolved, &meta, options).await;
                    }
                }
            }
        }
    }

    miss(req, res, next, options).await
}

/// A miss calls `next()` when fallthrough is on, else answers 404.
async fn miss(
    req: &Request,
    res: Response,
    next: &Next,
    options: &StaticOptions,
) -> Result<Response, Error> {
    let _ = req;
    if options.fallthrough {
        next(NextCall::Continue).await;
        return Ok(res);
    }
    Err(Error::with_status(404, "not found"))
}

/// Serve a directory: index files, or a trailing-slash redirect.
async fn serve_directory(
    req: &Request,
    res: Response,
    next: &Next,
    dir: &Path,
    decoded: &str,
    options: &StaticOptions,
) -> Result<Response, Error> {
    if !decoded.ends_with('/') {
        if options.redirect {
            return redirect_to_trailing_slash(req, res);
        }
        return miss(req, res, next, options).await;
    }

    for index in &options.index {
        let candidate = dir.join(index);
        if let Ok(meta) = tokio::fs::metadata(&candidate).await {
            if meta.is_file() {
                return send_file(req, res, &candidate, &meta, options).await;
            }
        }
    }

    miss(req, res, next, options).await
}

/// Answer 301 with the trailing slash appended, as serve-static does.
fn redirect_to_trailing_slash(req: &Request, mut res: Response) -> Result<Response, Error> {
    let location = format!("{}/", req.path());
    res.status(301);
    res.location(&location);
    res.set("content-type", "text/html");
    res.set("content-security-policy", "default-src 'none'");
    res.set("x-content-type-options", "nosniff");
    let doc = redirect_document(&location);
    res.set("content-length", &doc.len().to_string());
    res.set_body(bytes::Bytes::from(doc));
    Ok(res)
}

/// Minimal redirect document, matching serve-static's shape.
fn redirect_document(location: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Redirecting</title>\n</head>\n<body>\n<pre>Redirecting to {}</pre>\n</body>\n</html>\n",
        escape_html(location)
    )
}

/// Escape the characters that matter in HTML text.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Send one file with caching headers, freshness, and range support.
async fn send_file(
    req: &Request,
    mut res: Response,
    path: &Path,
    meta: &std::fs::Metadata,
    options: &StaticOptions,
) -> Result<Response, Error> {
    let len = meta.len();
    let modified = meta.modified().ok();
    let modified_secs =
        modified.and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()));

    // Content type from the extension, defaulting to octet-stream.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let content_type = if ext.is_empty() {
        "application/octet-stream".to_string()
    } else {
        mime::content_type(ext).unwrap_or_else(|| "application/octet-stream".to_string())
    };
    res.set("content-type", &content_type);

    if options.accept_ranges {
        res.set("accept-ranges", "bytes");
    }

    if options.cache_control {
        let mut cache = format!("public, max-age={}", options.max_age_secs);
        if options.immutable {
            cache.push_str(", immutable");
        }
        res.set("cache-control", &cache);
    }

    if options.last_modified {
        if let Some(modified) = modified {
            res.set("last-modified", &httpdate::fmt_http_date(modified));
        }
    }

    if options.etag {
        let tag = stat_etag(len, modified_secs);
        res.set("etag", &tag);
    }

    if let Some(hook) = &options.on_headers {
        let info = FileInfo {
            path: path.to_path_buf(),
            size: len,
            modified,
        };
        hook(&mut res, &info);
    }

    // Preconditions come before freshness, as in send.
    if (req.header("if-match").is_some() || req.header("if-unmodified-since").is_some())
        && precondition_failed(req, &res, modified_secs)
    {
        return Err(Error::with_status(412, "precondition failed"));
    }

    if (req.header("if-none-match").is_some() || req.header("if-modified-since").is_some())
        && is_fresh(req, &res, modified_secs)
    {
        res.status(304);
        res.remove_header("content-type");
        res.remove_header("content-length");
        res.remove_header("transfer-encoding");
        return Ok(res);
    }

    // Single ranges become 206. Anything else serves the whole file.
    let mut status = 200u16;
    let mut offset = 0u64;
    let mut length = len;

    if options.accept_ranges {
        if let Some(range_header) = req.header("range") {
            if range_header.trim_start().starts_with("bytes=")
                && is_range_fresh(req, &res, modified_secs)
            {
                match parse_range(range_header, len) {
                    RangeOutcome::Single(range) => {
                        status = 206;
                        res.set(
                            "content-range",
                            &format!("bytes {}-{}/{}", range.start, range.end, len),
                        );
                        offset = range.start;
                        length = range.end - range.start + 1;
                    }
                    RangeOutcome::Unsatisfiable => {
                        return Err(Error::with_status(416, "requested range not satisfiable"));
                    }
                    RangeOutcome::Full => {}
                }
            }
        }
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| Error::with_status(500, "failed to read file"))?;
    let end = (offset + length).min(bytes.len() as u64) as usize;
    let start = offset.min(bytes.len() as u64) as usize;
    let slice = bytes.get(start..end).unwrap_or(&[]);

    res.status(status);
    res.set("content-length", &slice.len().to_string());
    res.set_body(bytes::Bytes::from(slice.to_vec()));
    Ok(res)
}

/// True when `If-Match` or `If-Unmodified-Since` fails.
fn precondition_failed(req: &Request, res: &Response, modified_secs: Option<u64>) -> bool {
    if let Some(if_match) = req.header("if-match") {
        if if_match.trim() != "*" {
            match res.get("etag") {
                Some(tag) if etag::matches(&tag, if_match) => {}
                _ => return true,
            }
        }
    }

    if let Some(ius) = req.header("if-unmodified-since") {
        if let Ok(since) = httpdate::parse_http_date(ius) {
            let since_secs = since
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            match modified_secs {
                Some(mtime) if mtime <= since_secs => {}
                _ => return true,
            }
        }
    }

    false
}

/// True when the client's cache is fresh.
fn is_fresh(req: &Request, res: &Response, modified_secs: Option<u64>) -> bool {
    if let Some(inm) = req.header("if-none-match") {
        if inm.trim() != "*" {
            match res.get("etag") {
                Some(tag) if etag::matches(&tag, inm) => {}
                _ => return false,
            }
        }
    } else if let Some(ims) = req.header("if-modified-since") {
        let since_secs = httpdate::parse_http_date(ims)
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()));
        match (since_secs, modified_secs) {
            (Some(since), Some(mtime)) if mtime <= since => {}
            _ => return false,
        }
    } else {
        return false;
    }

    if req
        .header("cache-control")
        .map(|v| v.contains("no-cache"))
        .unwrap_or(false)
    {
        return false;
    }

    true
}

/// True when an `If-Range` header still matches the file.
fn is_range_fresh(req: &Request, res: &Response, modified_secs: Option<u64>) -> bool {
    let Some(if_range) = req.header("if-range") else {
        return true;
    };

    if if_range.contains('"') {
        return match res.get("etag") {
            Some(tag) => if_range.contains(&tag),
            None => false,
        };
    }

    let file_secs = modified_secs;
    let range_secs = httpdate::parse_http_date(if_range)
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()));
    match (file_secs, range_secs) {
        (Some(file), Some(given)) => file <= given,
        _ => false,
    }
}

/// A parsed byte range, endpoints inclusive.
#[derive(Debug, PartialEq, Eq)]
struct ByteRange {
    start: u64,
    end: u64,
}

/// How a `Range` header resolves against a file of `len` bytes.
#[derive(Debug, PartialEq, Eq)]
enum RangeOutcome {
    /// Serve the whole file.
    Full,
    /// Serve one slice.
    Single(ByteRange),
    /// Nothing satisfies the range: 416.
    Unsatisfiable,
}

/// Parse a `Range` header for a file of `len` bytes.
///
/// Only `bytes=` ranges are honoured. Malformed headers and multi-range
/// requests serve the whole file, as `send` does.
fn parse_range(header: &str, len: u64) -> RangeOutcome {
    let spec = match header.trim_start().strip_prefix("bytes=") {
        Some(s) => s,
        None => return RangeOutcome::Full,
    };
    if spec.contains(',') {
        return RangeOutcome::Full;
    }

    let (start_s, end_s) = match spec.split_once('-') {
        Some(pair) => pair,
        None => return RangeOutcome::Full,
    };

    // Suffix range: the last N bytes.
    if start_s.trim().is_empty() {
        let Ok(n) = end_s.trim().parse::<u64>() else {
            return RangeOutcome::Full;
        };
        if n == 0 || len == 0 {
            return RangeOutcome::Full;
        }
        let n = n.min(len);
        return RangeOutcome::Single(ByteRange {
            start: len - n,
            end: len - 1,
        });
    }

    let Ok(start) = start_s.trim().parse::<u64>() else {
        return RangeOutcome::Full;
    };
    if start >= len {
        return RangeOutcome::Unsatisfiable;
    }

    if end_s.trim().is_empty() {
        return RangeOutcome::Single(ByteRange {
            start,
            end: len - 1,
        });
    }
    let Ok(mut end) = end_s.trim().parse::<u64>() else {
        return RangeOutcome::Full;
    };
    if end < start {
        return RangeOutcome::Unsatisfiable;
    }
    end = end.min(len - 1);

    RangeOutcome::Single(ByteRange { start, end })
}

/// Weak entity tag from file size and modification time, as `send` makes.
///
/// `send` tags stat results rather than hashing content, so large files
/// never pay for a digest.
pub fn stat_etag(len: u64, mtime_secs: Option<u64>) -> String {
    match mtime_secs {
        Some(ms) => format!("W/\"{:x}-{:x}\"", len, ms),
        None => format!("W/\"{:x}\"", len),
    }
}

/// Strip `mount` from `path`, requiring a segment boundary.
fn strip_mount(path: &str, mount: &str) -> Option<String> {
    if mount == "/" {
        return Some(path.to_string());
    }
    let rest = path.strip_prefix(mount)?;
    if rest.is_empty() {
        return Some("/".to_string());
    }
    if rest.starts_with('/') {
        return Some(rest.to_string());
    }
    None
}

/// Percent-decode a path, rejecting malformed escapes.
///
/// Unlike query decoding, `+` stays a plus here.
fn percent_decode_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }

    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_strip_requires_a_boundary() {
        assert_eq!(
            strip_mount("/static/a.txt", "/static"),
            Some("/a.txt".to_string())
        );
        assert_eq!(strip_mount("/static", "/static"), Some("/".to_string()));
        assert_eq!(strip_mount("/staticfiles/a", "/static"), None);
        assert_eq!(strip_mount("/other", "/static"), None);
        assert_eq!(strip_mount("/a.txt", "/"), Some("/a.txt".to_string()));
    }

    #[test]
    fn path_decoding_rejects_bad_escapes() {
        assert_eq!(percent_decode_path("/a%20b"), Some("/a b".to_string()));
        assert_eq!(percent_decode_path("/a%2Fb"), Some("/a/b".to_string()));
        assert_eq!(percent_decode_path("/a%zz"), None);
        assert_eq!(percent_decode_path("/a%2"), None);
        // Plus stays a plus in paths.
        assert_eq!(percent_decode_path("/a+b"), Some("/a+b".to_string()));
    }

    #[test]
    fn stat_etag_matches_send_shape() {
        assert_eq!(stat_etag(0x1a, Some(0x1234)), "W/\"1a-1234\"");
        assert_eq!(stat_etag(16, None), "W/\"10\"");
    }

    #[test]
    fn range_parses_start_end() {
        assert_eq!(
            parse_range("bytes=0-499", 1000),
            RangeOutcome::Single(ByteRange { start: 0, end: 499 })
        );
    }

    #[test]
    fn range_open_end_clamps_to_length() {
        assert_eq!(
            parse_range("bytes=500-", 1000),
            RangeOutcome::Single(ByteRange {
                start: 500,
                end: 999
            })
        );
        assert_eq!(
            parse_range("bytes=0-9999", 1000),
            RangeOutcome::Single(ByteRange { start: 0, end: 999 })
        );
    }

    #[test]
    fn range_suffix_takes_the_tail() {
        assert_eq!(
            parse_range("bytes=-500", 1000),
            RangeOutcome::Single(ByteRange {
                start: 500,
                end: 999
            })
        );
        assert_eq!(
            parse_range("bytes=-2000", 1000),
            RangeOutcome::Single(ByteRange { start: 0, end: 999 })
        );
    }

    #[test]
    fn range_past_the_end_is_unsatisfiable() {
        assert_eq!(
            parse_range("bytes=1000-", 1000),
            RangeOutcome::Unsatisfiable
        );
        assert_eq!(
            parse_range("bytes=900-100", 1000),
            RangeOutcome::Unsatisfiable
        );
    }

    #[test]
    fn malformed_and_multi_ranges_serve_whole_file() {
        assert_eq!(parse_range("bytes=abc", 1000), RangeOutcome::Full);
        assert_eq!(parse_range("bytes=0-1,3-4", 1000), RangeOutcome::Full);
        assert_eq!(parse_range("items=0-1", 1000), RangeOutcome::Full);
    }
}
