//! Body parsing middleware.
//!
//! Mirrors the `body-parser` package Express 5 bundles as `express.json()`,
//! `express.text()`, and `express.urlencoded()`: content-type gating,
//! `Content-Encoding` decompression, charset checks, size limits, and
//! typed errors. A parser that runs stores its result on the request;
//! when the content type does not match, or the request carries no body,
//! it calls `next()` and leaves the request untouched.
//!
//! Only JSON, text, and URL-encoded forms are supported. There is no raw
//! parser yet, and no `zstd` support.

use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::error::Error;
use crate::request::Request;
use crate::response::Response;
use crate::router::{Next, NextCall, Params};

/// Default body size limit: 100kb, as in body-parser.
pub const DEFAULT_LIMIT: usize = 102_400;

/// Default cap on URL-encoded pairs: 1000, as in body-parser.
pub const DEFAULT_PARAMETER_LIMIT: usize = 1_000;

/// Default nesting depth for extended URL-encoded bodies: 32.
pub const DEFAULT_DEPTH: usize = 32;

/// A parsed request body.
///
/// Express exposes `req.body` as a value whose shape depends on the
/// parser. This names the shapes explicitly instead.
#[derive(Debug, Clone)]
pub enum ParsedBody {
    /// A JSON document.
    Json(Value),
    /// A decoded text body.
    Text(String),
    /// A URL-encoded form, as an object of strings and arrays.
    Form(Value),
}

/// Boxed future returned by body-parser middleware.
pub type BodyFuture = Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>>;

/// Verification callback, as `verify` in body-parser.
///
/// Receives the request, the raw body bytes, and the charset. Returning
/// `Err` rejects the body with a 403.
pub type VerifyFn = Arc<dyn Fn(&Request, &[u8], &str) -> Result<(), String> + Send + Sync>;

/// Options for the JSON parser.
#[derive(Clone)]
pub struct JsonOptions {
    /// Maximum body size in bytes. Defaults to 100kb.
    pub limit: usize,
    /// Only objects and arrays are accepted. Defaults to true.
    pub strict: bool,
    /// Decompress `gzip`, `deflate`, and `br` bodies. Defaults to true.
    pub inflate: bool,
    /// Content types to parse. Defaults to `application/json`.
    pub types: Vec<String>,
    /// Optional verification callback.
    pub verify: Option<VerifyFn>,
}

impl Default for JsonOptions {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            strict: true,
            inflate: true,
            types: vec!["application/json".to_string()],
            verify: None,
        }
    }
}

/// Options for the text parser.
#[derive(Clone)]
pub struct TextOptions {
    /// Maximum body size in bytes. Defaults to 100kb.
    pub limit: usize,
    /// Decompress `gzip`, `deflate`, and `br` bodies. Defaults to true.
    pub inflate: bool,
    /// Content types to parse. Defaults to `text/plain`.
    pub types: Vec<String>,
    /// Charset used when the header names none. Defaults to `utf-8`.
    pub default_charset: String,
    /// Optional verification callback.
    pub verify: Option<VerifyFn>,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            inflate: true,
            types: vec!["text/plain".to_string()],
            default_charset: "utf-8".to_string(),
            verify: None,
        }
    }
}

/// Options for the URL-encoded parser.
#[derive(Clone)]
pub struct FormOptions {
    /// Maximum body size in bytes. Defaults to 100kb.
    pub limit: usize,
    /// Decompress `gzip`, `deflate`, and `br` bodies. Defaults to true.
    pub inflate: bool,
    /// Content types to parse. Defaults to the form type.
    pub types: Vec<String>,
    /// Expand `a[b]` nesting. Defaults to false, as in Express 5.
    pub extended: bool,
    /// Maximum number of pairs. Defaults to 1000.
    pub parameter_limit: usize,
    /// Maximum nesting depth when extended. Defaults to 32.
    pub depth: usize,
    /// Charset used when the header names none. Defaults to `utf-8`.
    pub default_charset: String,
    /// Optional verification callback.
    pub verify: Option<VerifyFn>,
}

impl Default for FormOptions {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            inflate: true,
            types: vec!["application/x-www-form-urlencoded".to_string()],
            extended: false,
            parameter_limit: DEFAULT_PARAMETER_LIMIT,
            depth: DEFAULT_DEPTH,
            default_charset: "utf-8".to_string(),
            verify: None,
        }
    }
}

/// JSON body-parsing middleware, as `express.json()`.
///
/// Stores a [`ParsedBody::Json`] value on the request.
pub fn json(
    options: JsonOptions,
) -> impl Fn(Arc<Request>, Response, Next, Params) -> BodyFuture + Send + Sync + 'static {
    move |req, res, next, _params| {
        let opts = options.clone();
        Box::pin(async move {
            let prepared = match prepare(
                &req,
                &opts.types,
                opts.inflate,
                opts.limit,
                CharsetRule::UtfOnly,
                "utf-8",
                &opts.verify,
            )? {
                Some(p) => p,
                None => {
                    next(NextCall::Continue).await;
                    return Ok(res);
                }
            };

            let text = decode_bytes(&prepared.bytes, &prepared.charset);
            let value = parse_json(&text, opts.strict)?;
            req.set_parsed_body(ParsedBody::Json(value));
            next(NextCall::Continue).await;
            Ok(res)
        })
    }
}

/// Text body-parsing middleware, as `express.text()`.
///
/// Stores a [`ParsedBody::Text`] value on the request.
pub fn text(
    options: TextOptions,
) -> impl Fn(Arc<Request>, Response, Next, Params) -> BodyFuture + Send + Sync + 'static {
    move |req, res, next, _params| {
        let opts = options.clone();
        Box::pin(async move {
            let prepared = match prepare(
                &req,
                &opts.types,
                opts.inflate,
                opts.limit,
                CharsetRule::Any,
                &opts.default_charset,
                &opts.verify,
            )? {
                Some(p) => p,
                None => {
                    next(NextCall::Continue).await;
                    return Ok(res);
                }
            };

            let decoded = decode_bytes(&prepared.bytes, &prepared.charset);
            req.set_parsed_body(ParsedBody::Text(decoded));
            next(NextCall::Continue).await;
            Ok(res)
        })
    }
}

/// URL-encoded body-parsing middleware, as `express.urlencoded()`.
///
/// Stores a [`ParsedBody::Form`] value on the request.
pub fn urlencoded(
    options: FormOptions,
) -> impl Fn(Arc<Request>, Response, Next, Params) -> BodyFuture + Send + Sync + 'static {
    move |req, res, next, _params| {
        let opts = options.clone();
        Box::pin(async move {
            if opts.default_charset != "utf-8" && opts.default_charset != "iso-8859-1" {
                return Err(Error::new(
                    "option defaultCharset must be either utf-8 or iso-8859-1",
                ));
            }

            let prepared = match prepare(
                &req,
                &opts.types,
                opts.inflate,
                opts.limit,
                CharsetRule::UtfOrLatin1,
                &opts.default_charset,
                &opts.verify,
            )? {
                Some(p) => p,
                None => {
                    next(NextCall::Continue).await;
                    return Ok(res);
                }
            };

            let text = decode_bytes(&prepared.bytes, &prepared.charset);
            let value = parse_form(
                &text,
                &prepared.charset,
                opts.extended,
                opts.parameter_limit,
                opts.depth,
            )?;
            req.set_parsed_body(ParsedBody::Form(value));
            next(NextCall::Continue).await;
            Ok(res)
        })
    }
}

/// Which charsets a parser accepts.
enum CharsetRule {
    /// Only `utf-*`, as the JSON parser requires.
    UtfOnly,
    /// `utf-8` or `iso-8859-1`, as the form parser requires.
    UtfOrLatin1,
    /// Anything, as the text parser allows.
    Any,
}

/// A body that passed gating and is ready to parse.
#[derive(Debug)]
struct Prepared {
    bytes: Vec<u8>,
    charset: String,
}

/// Gate a request the way `read` in body-parser does.
///
/// Returns `Ok(None)` when the parser must skip the request: no
/// content type, a non-matching type, or no body. Otherwise returns the
/// (possibly decompressed) bytes and the charset.
fn prepare(
    req: &Request,
    types: &[String],
    inflate: bool,
    limit: usize,
    charsets: CharsetRule,
    default_charset: &str,
    verify: &Option<VerifyFn>,
) -> Result<Option<Prepared>, Error> {
    // No content type means nothing to parse.
    let content_type = match req.header("content-type") {
        Some(ct) => ct.to_string(),
        None => return Ok(None),
    };

    // Skip content types this parser does not handle.
    let refs: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
    if req.is(&refs).is_none() {
        let _ = content_type;
        return Ok(None);
    }

    // Skip requests without bodies.
    if !req.has_body() {
        return Ok(None);
    }

    // Decompress when the client encoded the body.
    let encoding = req
        .header("content-encoding")
        .unwrap_or("identity")
        .to_ascii_lowercase();
    let bytes: Vec<u8> = match encoding.as_str() {
        "identity" => req.body().to_vec(),
        "gzip" | "deflate" | "br" => {
            if !inflate {
                return Err(Error::with_kind(
                    415,
                    "encoding.unsupported",
                    "content encoding unsupported",
                ));
            }
            decompress(&encoding, req.body(), limit)?
        }
        other => {
            return Err(Error::with_kind(
                415,
                "encoding.unsupported",
                format!("unsupported content encoding \"{}\"", other),
            ));
        }
    };

    if bytes.len() > limit {
        return Err(Error::with_kind(
            413,
            "entity.too.large",
            "request entity too large",
        ));
    }

    // Determine and validate the charset.
    let charset = content_type
        .split(';')
        .skip(1)
        .find_map(|part| {
            let part = part.trim();
            part.split_once('=').and_then(|(k, v)| {
                if k.trim().eq_ignore_ascii_case("charset") {
                    Some(v.trim().to_ascii_lowercase())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| default_charset.to_ascii_lowercase());

    let charset_ok = match charsets {
        CharsetRule::UtfOnly => charset.starts_with("utf-"),
        CharsetRule::UtfOrLatin1 => charset == "utf-8" || charset == "iso-8859-1",
        CharsetRule::Any => true,
    };
    if !charset_ok {
        return Err(Error::with_kind(
            415,
            "charset.unsupported",
            format!("unsupported charset \"{}\"", charset.to_ascii_uppercase()),
        ));
    }

    if let Some(check) = verify {
        if let Err(message) = check(req, &bytes, &charset) {
            return Err(Error::with_kind(403, "entity.verify.failed", message));
        }
    }

    Ok(Some(Prepared { bytes, charset }))
}

/// Decompress a body, reading at most `limit + 1` bytes.
///
/// The cap keeps a compression bomb from exhausting memory: anything
/// past the limit is a 413 either way.
fn decompress(encoding: &str, input: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let reader: Box<dyn Read> = match encoding {
        "gzip" => Box::new(flate2::read::MultiGzDecoder::new(input)),
        "deflate" => Box::new(flate2::read::ZlibDecoder::new(input)),
        "br" => Box::new(brotli::Decompressor::new(input, 4096)),
        _ => {
            return Err(Error::with_kind(
                415,
                "encoding.unsupported",
                format!("unsupported content encoding \"{}\"", encoding),
            ));
        }
    };

    let mut out = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| Error::with_kind(400, "entity.parse.failed", "failed to decompress body"))?;

    if out.len() > limit {
        return Err(Error::with_kind(
            413,
            "entity.too.large",
            "request entity too large",
        ));
    }

    Ok(out)
}

/// Decode bytes with a small set of charsets.
///
/// `utf-8` (and its aliases) decodes lossily, as `iconv-lite` does rather
/// than failing. `iso-8859-1` maps each byte to its code point directly.
/// `windows-1252` maps the `0x80-0x9F` range through its table. Anything
/// else falls back to lossy UTF-8.
fn decode_bytes(bytes: &[u8], charset: &str) -> String {
    match charset {
        "utf-8" | "utf8" | "ascii" | "us-ascii" => String::from_utf8_lossy(bytes).into_owned(),
        "iso-8859-1" | "latin1" => bytes.iter().map(|&b| b as char).collect(),
        "windows-1252" => decode_windows_1252(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Decode Windows-1252, whose `0x80-0x9F` range differs from Latin-1.
///
/// Undefined positions become U+FFFD.
fn decode_windows_1252(bytes: &[u8]) -> String {
    const TABLE: [char; 32] = [
        '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}',
        '\u{017D}', '\u{FFFD}', '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
    ];

    bytes
        .iter()
        .map(|&b| {
            if (0x80..0xA0).contains(&b) {
                TABLE[(b - 0x80) as usize]
            } else {
                b as char
            }
        })
        .collect()
}

/// Parse a JSON document.
///
/// An empty body becomes `{}`, matching body-parser's special case.
/// Strict mode requires the first non-whitespace character to open an
/// object or an array.
fn parse_json(text: &str, strict: bool) -> Result<Value, Error> {
    if text.is_empty() {
        return Ok(Value::Object(Default::default()));
    }

    if strict {
        let first = text
            .chars()
            .find(|c| !matches!(c, ' ' | '\t' | '\n' | '\r'));
        match first {
            Some('{') | Some('[') => {}
            _ => {
                return Err(Error::with_kind(
                    400,
                    "entity.parse.failed",
                    "strict violation: body must be a JSON object or array",
                ));
            }
        }
    }

    serde_json::from_str(text)
        .map_err(|e| Error::with_kind(400, "entity.parse.failed", e.to_string()))
}

/// Parse a URL-encoded form.
///
/// An empty body becomes `{}`. Each pair beyond `parameter_limit` is a
/// 413. With `extended`, `a[b]` keys nest up to `depth` levels deep;
/// without it, keys stay literal.
fn parse_form(
    text: &str,
    charset: &str,
    extended: bool,
    parameter_limit: usize,
    depth: usize,
) -> Result<Value, Error> {
    if text.is_empty() {
        return Ok(Value::Object(Default::default()));
    }

    let pair_count = text.matches('&').count() + 1;
    if pair_count > parameter_limit {
        return Err(Error::with_kind(
            413,
            "parameters.too.many",
            "too many parameters",
        ));
    }

    let mut root = Value::Object(Default::default());

    for pair in text.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        if raw_key.is_empty() {
            continue;
        }

        let key = decode_form_piece(raw_key, charset);
        let value = decode_form_piece(raw_value, charset);

        if extended {
            let segments = split_key(&key);
            insert_nested(&mut root, &segments, value, depth)?;
        } else {
            set_leaf(
                root.as_object_mut()
                    .unwrap()
                    .entry(key)
                    .or_insert(Value::Null),
                value,
            );
        }
    }

    Ok(root)
}

/// Decode one form key or value: `+` becomes a space, `%XX` becomes a
/// byte, then the bytes decode with the body charset.
fn decode_form_piece(piece: &str, charset: &str) -> String {
    let bytes = piece.as_bytes();
    let mut raw = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    raw.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        if bytes[i] == b'+' {
            raw.push(b' ');
        } else {
            raw.push(bytes[i]);
        }
        i += 1;
    }

    decode_bytes(&raw, charset)
}

/// One segment of a bracketed form key.
enum KeySeg {
    Field(String),
    Push,
    Index(usize),
}

/// Split `a[b][0][]` into its segments.
fn split_key(key: &str) -> Vec<KeySeg> {
    let mut segments = Vec::new();
    let mut rest = key;

    match rest.find('[') {
        Some(i) => {
            segments.push(KeySeg::Field(rest[..i].to_string()));
            rest = &rest[i..];
        }
        None => {
            segments.push(KeySeg::Field(key.to_string()));
            return segments;
        }
    }

    while !rest.is_empty() {
        if !rest.starts_with('[') {
            break;
        }
        match rest.find(']') {
            Some(end) => {
                let inner = &rest[1..end];
                if inner.is_empty() {
                    segments.push(KeySeg::Push);
                } else if let Ok(index) = inner.parse::<usize>() {
                    segments.push(KeySeg::Index(index));
                } else {
                    segments.push(KeySeg::Field(inner.to_string()));
                }
                rest = &rest[end + 1..];
            }
            // An unclosed bracket stays literal.
            None => {
                segments.push(KeySeg::Field(rest.to_string()));
                break;
            }
        }
    }

    segments
}

/// Insert a value into a nested form tree.
///
/// A scalar that meets another scalar becomes an array. A container
/// that meets a conflicting shape is replaced, with the newcomer
/// winning. Nesting past `depth` is a 400.
fn insert_nested(
    node: &mut Value,
    segs: &[KeySeg],
    value: String,
    depth: usize,
) -> Result<(), Error> {
    if segs.len() - 1 > depth {
        return Err(Error::with_kind(
            400,
            "querystring.parse.rangeError",
            "The input exceeded the depth",
        ));
    }

    match &segs[0] {
        KeySeg::Field(name) => {
            if name.is_empty() {
                return Ok(());
            }
            if segs.len() == 1 {
                if !node.is_object() {
                    *node = Value::Object(Default::default());
                }
                let slot = node
                    .as_object_mut()
                    .unwrap()
                    .entry(name.clone())
                    .or_insert(Value::Null);
                set_leaf(slot, value);
                return Ok(());
            }

            let child = node.as_object_mut().map(|obj| {
                let slot = obj.entry(name.clone()).or_insert(Value::Null);
                if !slot.is_null() && !slot.is_object() && !slot.is_array() {
                    *slot = Value::Null;
                }
                slot as &mut Value
            });
            match child {
                Some(child) => insert_nested(child, &segs[1..], value, depth),
                None => {
                    *node = Value::Object(Default::default());
                    let child = node
                        .as_object_mut()
                        .unwrap()
                        .entry(name.clone())
                        .or_insert(Value::Null);
                    insert_nested(child, &segs[1..], value, depth)
                }
            }
        }
        KeySeg::Push => {
            ensure_array(node);
            if segs.len() == 1 {
                node.as_array_mut().unwrap().push(Value::String(value));
                return Ok(());
            }
            node.as_array_mut().unwrap().push(Value::Null);
            let last = node.as_array_mut().unwrap().len() - 1;
            insert_nested(
                &mut node.as_array_mut().unwrap()[last],
                &segs[1..],
                value,
                depth,
            )
        }
        KeySeg::Index(index) => {
            ensure_array(node);
            let arr = node.as_array_mut().unwrap();
            while arr.len() <= *index {
                arr.push(Value::Null);
            }
            if segs.len() == 1 {
                set_leaf(&mut arr[*index], value);
                return Ok(());
            }
            insert_nested(&mut arr[*index], &segs[1..], value, depth)
        }
    }
}

/// Turn a slot into an array when `[]` or `[n]` addressing needs one.
///
/// An existing scalar is preserved as the first element. A conflicting
/// object is replaced, with the newcomer winning.
fn ensure_array(node: &mut Value) {
    match node {
        Value::Null => *node = Value::Array(Vec::new()),
        Value::Array(_) => {}
        Value::String(s) => {
            let old = std::mem::take(s);
            *node = Value::Array(vec![Value::String(old)]);
        }
        _ => *node = Value::Array(Vec::new()),
    }
}

/// Store a scalar, merging repeats into an array.
///
/// An object in the slot is replaced: the newcomer wins.
fn set_leaf(slot: &mut Value, value: String) {
    match slot {
        Value::Null => *slot = Value::String(value),
        Value::String(old) => {
            let old = std::mem::take(old);
            *slot = Value::Array(vec![Value::String(old), Value::String(value)]);
        }
        Value::Array(items) => items.push(Value::String(value)),
        _ => *slot = Value::String(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use http::{HeaderMap, Method};

    fn req(content_type: Option<&str>, body: &[u8]) -> Request {
        let mut headers = HeaderMap::new();
        if let Some(ct) = content_type {
            headers.insert("content-type", ct.parse().unwrap());
        }
        headers.insert("content-length", body.len().to_string().parse().unwrap());
        Request::new(
            Method::POST,
            "/".parse().unwrap(),
            headers,
            bytes::Bytes::from(body.to_vec()),
        )
    }

    #[test]
    fn json_parses_an_object() {
        let value = parse_json(r#"{"a":1}"#, true).unwrap();
        assert_eq!(value["a"], 1);
    }

    #[test]
    fn json_empty_body_becomes_empty_object() {
        assert_eq!(
            parse_json("", true).unwrap(),
            Value::Object(Default::default())
        );
    }

    #[test]
    fn json_strict_rejects_primitives() {
        let err = parse_json("42", true).unwrap_err();
        assert_eq!(err.status(), 400);
        assert_eq!(err.kind(), Some("entity.parse.failed"));
    }

    #[test]
    fn json_non_strict_accepts_primitives() {
        assert_eq!(parse_json("42", false).unwrap(), Value::from(42));
    }

    #[test]
    fn json_strict_skips_leading_whitespace() {
        assert!(parse_json("  \n\t{\"a\":1}", true).is_ok());
        assert!(parse_json("  42", true).is_err());
    }

    #[test]
    fn json_malformed_is_a_400() {
        let err = parse_json("{oops", true).unwrap_err();
        assert_eq!(err.status(), 400);
        assert_eq!(err.kind(), Some("entity.parse.failed"));
    }

    #[test]
    fn form_flat_pairs_and_repeats() {
        let value = parse_form("a=1&b=2&a=3", "utf-8", false, 1000, 32).unwrap();
        assert_eq!(value["b"], "2");
        assert_eq!(value["a"], Value::from(vec!["1", "3"]));
    }

    #[test]
    fn form_decodes_plus_and_percent() {
        let value = parse_form("q=hello+world%21", "utf-8", false, 1000, 32).unwrap();
        assert_eq!(value["q"], "hello world!");
    }

    #[test]
    fn form_empty_body_becomes_empty_object() {
        assert_eq!(
            parse_form("", "utf-8", false, 1000, 32).unwrap(),
            Value::Object(Default::default())
        );
    }

    #[test]
    fn form_too_many_parameters_is_a_413() {
        let err = parse_form("a=1&b=2&c=3", "utf-8", false, 2, 32).unwrap_err();
        assert_eq!(err.status(), 413);
        assert_eq!(err.kind(), Some("parameters.too.many"));
    }

    #[test]
    fn form_simple_mode_keeps_brackets_literal() {
        let value = parse_form("a[b]=c", "utf-8", false, 1000, 32).unwrap();
        assert_eq!(value["a[b]"], "c");
    }

    #[test]
    fn form_extended_nests_brackets() {
        let value = parse_form("a[b]=c", "utf-8", true, 1000, 32).unwrap();
        assert_eq!(value["a"]["b"], "c");
    }

    #[test]
    fn form_extended_appends_with_empty_brackets() {
        let value = parse_form("a[]=1&a[]=2", "utf-8", true, 1000, 32).unwrap();
        assert_eq!(value["a"], Value::from(vec!["1", "2"]));
    }

    #[test]
    fn form_depth_limit_is_a_400() {
        let err = parse_form("a[b][c]=1", "utf-8", true, 1000, 1).unwrap_err();
        assert_eq!(err.status(), 400);
        assert_eq!(err.kind(), Some("querystring.parse.rangeError"));
    }

    #[test]
    fn latin1_decodes_high_bytes() {
        assert_eq!(decode_bytes(&[0xE9], "iso-8859-1"), "\u{e9}");
    }

    #[test]
    fn windows_1252_maps_the_quoting_range() {
        assert_eq!(
            decode_bytes(&[0x93, 0x94], "windows-1252"),
            "\u{201c}\u{201d}"
        );
    }

    #[test]
    fn prepare_skips_without_content_type() {
        let r = req(None, b"{}");
        let out = prepare(
            &r,
            &["application/json".to_string()],
            true,
            100,
            CharsetRule::UtfOnly,
            "utf-8",
            &None,
        )
        .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn prepare_skips_unmatched_type() {
        let r = req(Some("text/plain"), b"{}");
        let out = prepare(
            &r,
            &["application/json".to_string()],
            true,
            100,
            CharsetRule::UtfOnly,
            "utf-8",
            &None,
        )
        .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn prepare_rejects_non_utf_charset_for_json() {
        let r = req(Some("application/json; charset=iso-8859-1"), b"{}");
        let err = prepare(
            &r,
            &["application/json".to_string()],
            true,
            100,
            CharsetRule::UtfOnly,
            "utf-8",
            &None,
        )
        .unwrap_err();
        assert_eq!(err.status(), 415);
        assert_eq!(err.kind(), Some("charset.unsupported"));
    }

    #[test]
    fn prepare_rejects_unknown_encoding() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("content-length", "2".parse().unwrap());
        headers.insert("content-encoding", "compress".parse().unwrap());
        let r = Request::new(
            Method::POST,
            "/".parse().unwrap(),
            headers,
            bytes::Bytes::from("{}"),
        );
        let err = prepare(
            &r,
            &["application/json".to_string()],
            true,
            100,
            CharsetRule::UtfOnly,
            "utf-8",
            &None,
        )
        .unwrap_err();
        assert_eq!(err.status(), 415);
        assert_eq!(err.kind(), Some("encoding.unsupported"));
    }

    #[test]
    fn prepare_enforces_the_limit() {
        let r = req(Some("application/json"), b"123456");
        let err = prepare(
            &r,
            &["application/json".to_string()],
            true,
            4,
            CharsetRule::UtfOnly,
            "utf-8",
            &None,
        )
        .unwrap_err();
        assert_eq!(err.status(), 413);
        assert_eq!(err.kind(), Some("entity.too.large"));
    }

    #[test]
    fn prepare_runs_verify_and_maps_failure_to_403() {
        let r = req(Some("application/json"), b"{}");
        let verify: VerifyFn = Arc::new(|_, _, _| Err("nope".to_string()));
        let err = prepare(
            &r,
            &["application/json".to_string()],
            true,
            100,
            CharsetRule::UtfOnly,
            "utf-8",
            &Some(verify),
        )
        .unwrap_err();
        assert_eq!(err.status(), 403);
        assert_eq!(err.kind(), Some("entity.verify.failed"));
    }

    #[test]
    fn gzip_roundtrip_through_decompress() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"{\"a\":1}").unwrap();
        let compressed = encoder.finish().unwrap();

        let out = decompress("gzip", &compressed, 100).unwrap();
        assert_eq!(out, b"{\"a\":1}");
    }

    #[test]
    fn decompress_caps_output_at_the_limit() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"0123456789").unwrap();
        let compressed = encoder.finish().unwrap();

        let err = decompress("gzip", &compressed, 4).unwrap_err();
        assert_eq!(err.status(), 413);
    }
}
