//! MIME type helpers.
//!
//! A compact stand-in for the `mime-types` package Express uses, covering
//! the content types that matter for `res.type()`, `res.send()` charset
//! handling, and `res.format()` negotiation.
//!
//! Express 5 resolves `.js` to `text/javascript` rather than
//! `application/javascript`, matching current mime-db.

/// A content type and, when the type is textual, its default charset.
#[derive(Debug, Clone, Copy)]
pub struct ContentType {
    pub mime: &'static str,
    pub charset: Option<&'static str>,
}

/// Look up an extension or bare type name, e.g. `html`, `.json`, `txt`.
pub fn lookup(ext: &str) -> Option<ContentType> {
    let key = ext.trim_start_matches('.').to_ascii_lowercase();
    let entry = match key.as_str() {
        "html" | "htm" => ("text/html", Some("utf-8")),
        "txt" | "text" => ("text/plain", Some("utf-8")),
        "css" => ("text/css", Some("utf-8")),
        "js" | "mjs" => ("text/javascript", Some("utf-8")),
        "json" => ("application/json", None),
        "map" => ("application/json", None),
        "xml" => ("application/xml", Some("utf-8")),
        "rss" => ("application/rss+xml", Some("utf-8")),
        "atom" => ("application/atom+xml", Some("utf-8")),
        "csv" => ("text/csv", Some("utf-8")),
        "md" | "markdown" => ("text/markdown", Some("utf-8")),
        "yaml" | "yml" => ("text/yaml", Some("utf-8")),
        "pdf" => ("application/pdf", None),
        "zip" => ("application/zip", None),
        "gz" | "gzip" => ("application/gzip", None),
        "tar" => ("application/x-tar", None),
        "wasm" => ("application/wasm", None),
        "png" => ("image/png", None),
        "jpg" | "jpeg" => ("image/jpeg", None),
        "gif" => ("image/gif", None),
        "webp" => ("image/webp", None),
        "avif" => ("image/avif", None),
        "bmp" => ("image/bmp", None),
        "svg" => ("image/svg+xml", None),
        "ico" => ("image/x-icon", None),
        "woff" => ("font/woff", None),
        "woff2" => ("font/woff2", None),
        "ttf" => ("font/ttf", None),
        "otf" => ("font/otf", None),
        "mp4" => ("video/mp4", None),
        "webm" => ("video/webm", None),
        "ogg" | "ogv" => ("video/ogg", None),
        "mp3" => ("audio/mpeg", None),
        "wav" => ("audio/wav", None),
        "weba" => ("audio/webm", None),
        "oga" => ("audio/ogg", None),
        "flac" => ("audio/flac", None),
        "bin" | "exe" | "dll" | "so" | "class" => ("application/octet-stream", None),
        _ => return None,
    };
    Some(ContentType {
        mime: entry.0,
        charset: entry.1,
    })
}

/// Resolve a type the way `mime-types`' `contentType()` does: a value
/// containing `/` is used verbatim, anything else is treated as an
/// extension. A default charset is appended for textual types that do
/// not already carry one.
///
/// Returns `None` when an extension is unknown.
pub fn content_type(value: &str) -> Option<String> {
    if value.contains('/') {
        if value.contains(';') {
            return Some(value.to_string());
        }
        let media = value.trim();
        return Some(match charset_for(media) {
            Some(cs) => format!("{}; charset={}", media, cs),
            None => media.to_string(),
        });
    }

    lookup(value).map(|ct| match ct.charset {
        Some(cs) => format!("{}; charset={}", ct.mime, cs),
        None => ct.mime.to_string(),
    })
}

/// The default charset mime-db records for a media type: `UTF-8` for
/// textual types, nothing for the rest.
fn charset_for(media: &str) -> Option<&'static str> {
    let media = media.trim().to_ascii_lowercase();
    if media.starts_with("text/") || media == "application/javascript" {
        Some("utf-8")
    } else {
        None
    }
}

/// Split `type/subtype; param=value` into its media type and parameters.
pub fn parse_content_type(value: &str) -> Option<ContentTypeParts<'_>> {
    let mut parts = value.split(';');
    let media = parts.next()?.trim();
    if media.is_empty() || !media.contains('/') {
        return None;
    }
    Some(ContentTypeParts { media, rest: value })
}

/// The pieces of a parsed Content-Type header.
pub struct ContentTypeParts<'a> {
    /// The `type/subtype` portion.
    pub media: &'a str,
    /// The full original header value.
    pub rest: &'a str,
}

/// Set the charset on a Content-Type, replacing any existing charset.
///
/// Mirrors Express's `setCharset`: `text/html` becomes
/// `text/html; charset=utf-8`, and an existing charset is overwritten
/// rather than duplicated.
pub fn set_charset(value: &str, charset: &str) -> String {
    if value.is_empty() || charset.is_empty() {
        return value.to_string();
    }

    let media = value.split(';').next().unwrap_or(value).trim().to_string();

    let mut params: Vec<(String, String)> = Vec::new();
    for part in value.split(';').skip(1) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = match part.split_once('=') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => (part.to_lowercase(), String::new()),
        };
        if k != "charset" {
            params.push((k, v));
        }
    }

    let mut out = format!("{}; charset={}", media, charset);
    for (k, v) in params {
        out.push_str(&format!("; {}={}", k, v));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_common_extensions() {
        assert_eq!(content_type("html").unwrap(), "text/html; charset=utf-8");
        assert_eq!(content_type("json").unwrap(), "application/json");
        assert_eq!(content_type(".txt").unwrap(), "text/plain; charset=utf-8");
        assert_eq!(content_type("png").unwrap(), "image/png");
    }

    #[test]
    fn javascript_is_text_javascript_in_express5() {
        assert_eq!(
            content_type("js").unwrap(),
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn full_type_passes_through() {
        assert_eq!(
            content_type("application/vnd.api+json").unwrap(),
            "application/vnd.api+json"
        );
    }

    #[test]
    fn unknown_extension_is_none() {
        assert!(content_type("nope").is_none());
    }

    #[test]
    fn set_charset_appends_and_replaces() {
        assert_eq!(
            set_charset("text/html", "utf-8"),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            set_charset("text/html; charset=iso-8859-1", "utf-8"),
            "text/html; charset=utf-8"
        );
        assert_eq!(set_charset("", "utf-8"), "");
    }

    #[test]
    fn set_charset_keeps_other_params() {
        assert_eq!(
            set_charset("text/html; boundary=x", "utf-8"),
            "text/html; charset=utf-8; boundary=x"
        );
    }
}
