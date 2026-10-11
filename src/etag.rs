//! Entity tag generation.
//!
//! Express's default `etag` setting is `weak`, so `res.send()` emits
//! `W/"..."` tags. The format matches the `etag` package: the body length
//! in hex, a dash, then the first 27 characters of the base64 SHA-1 of
//! the body.

use std::fmt::Write;

use sha1::{Digest, Sha1};

/// Generate a weak entity tag, e.g. `W/"1a-b0aWv0CkplYw1k1RvHb8YBPdSg"`.
pub fn weak(body: &[u8]) -> String {
    tag(body, true)
}

/// Generate a strong entity tag, e.g. `"1a-b0aWv0CkplYw1k1RvHb8YBPdSg"`.
pub fn strong(body: &[u8]) -> String {
    tag(body, false)
}

fn tag(body: &[u8], weak: bool) -> String {
    let mut hasher = Sha1::new();
    hasher.update(body);
    let digest = hasher.finalize();

    let mut out = String::with_capacity(48);
    if weak {
        out.push_str("W/");
    }
    out.push('"');
    write!(&mut out, "{:x}-", body.len()).expect("writing to String cannot fail");
    base64_prefix(&digest, 27, &mut out);
    out.push('"');
    out
}

/// True when `etag` satisfies `if_none_match`.
///
/// `*` always matches. Otherwise the header is a comma-separated list of
/// entity tags, and a weak comparison (Express's `fresh`) ignores the
/// `W/` prefix and quotes.
pub fn matches(etag: &str, if_none_match: &str) -> bool {
    // `If-None-Match: *` matches any existing representation.
    if if_none_match.trim() == "*" {
        return true;
    }

    let candidate = normalize(etag);
    if_none_match
        .split(',')
        .map(normalize)
        .any(|tag| tag == candidate)
}

/// Strip the weakness prefix and quotes for weak comparison.
fn normalize(tag: &str) -> String {
    tag.trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_string()
}

/// Write a prefix of a standard base64 encoding directly into `out`.
fn base64_prefix(bytes: &[u8], limit: usize, out: &mut String) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let start = out.len();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[(n >> 18) as usize & 0x3f] as char);
        if out.len() - start >= limit {
            return;
        }
        out.push(TABLE[(n >> 12) as usize & 0x3f] as char);
        if out.len() - start >= limit {
            return;
        }
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
        if out.len() - start >= limit {
            return;
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
        if out.len() - start >= limit {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_matches_express() {
        assert_eq!(strong(b""), "\"0-2jmj7l5rSw0yVb/vlWAYkK/YBwk\"");
    }

    #[test]
    fn tags_are_27_base64_chars() {
        let tag = strong(b"Hello, World!");
        let inner = tag.trim_matches('"');
        let hash = inner.split('-').nth(1).unwrap();
        assert_eq!(hash.len(), 27);
    }

    #[test]
    fn weak_tag_has_prefix() {
        assert!(weak(b"x").starts_with("W/\""));
    }

    #[test]
    fn star_matches_anything() {
        assert!(matches("\"abc\"", "*"));
    }

    #[test]
    fn weak_comparison_ignores_weakness() {
        assert!(matches("W/\"abc\"", "\"abc\""));
        assert!(matches("\"abc\"", "W/\"abc\""));
    }

    #[test]
    fn list_is_comma_separated() {
        assert!(matches("\"b\"", "\"a\", \"b\" , \"c\""));
    }

    #[test]
    fn no_match_returns_false() {
        assert!(!matches("\"a\"", "\"b\""));
    }
}
